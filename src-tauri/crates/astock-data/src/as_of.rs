//! 时间旅行（As-Of）上下文 — 运行时实现
//!
//! DTO 类型（AsOfContext、AsOfSource 等）的权威定义在 `axagent_harness::as_of`，
//! 本模块 re-export 它们并附加运行时状态管理。
//!
//! 通过 `tokio::task_local!` 注入当前任务的 `AsOfContext`，所有 vendor 调用
//! 都可以隐式读取截止日，从而过滤或降级数据。Live 模式下该 task-local 为 None.
//!
//! # 双层存储：task_local + 进程级全局回退
//!
//! `tokio::task_local!` 不会跨 `tokio::spawn` / `JoinSet::spawn` 边界传播，
//! 而 `run_stock_workflow_inner` 与 `WorkEngine::run_workflow` 中分别有
//! 一次 `tokio::spawn` / `JoinSet::spawn`，导致 vendor 工具调用时
//! `current_as_of()` 返回 None、`truncate_*_by_asof` 兜底失效、数据
//! 穿透到 as-of 之后。
//!
//! 为此引入 `static GLOBAL_AS_OF: Mutex<Option<AsOfContext>>` 作为
//! 进程级回退。读取时优先 task_local（更精确、嵌套感知），没有再读全局；
//! 写入时 `with_optional_asof` 同步写全局，spawn 出去的 future 即可读到。

use chrono::{Duration, Utc};
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

// 共享 DTO 类型（AsOfContext / AsOfSource / AsOfDataScope / AsOfDataKind /
// DegradationEntry / AsOfError）的权威定义在 `axagent_harness::as_of`，
// 本模块通过 pub use re-export，不重复定义。
// 运行时状态管理（task_local、全局 Mutex、降级日志等）仍保留在本模块。
pub use axagent_harness::as_of::{
    AsOfContext, AsOfDataKind, AsOfDataScope, AsOfError, AsOfSource, DegradationEntry,
};

tokio::task_local! {
    /// 当前任务内的 AsOfContext；None 表示 live 模式
    pub static AS_OF: Option<AsOfContext>;
}

// ─── 进程级全局回退 ─────────────────────────────────────────────
//
// 设计要点：
// 1. 用 `parking_lot::Mutex`（同步锁），不持锁跨 await，不会破坏
//    tokio 调度器，也不会出现"未来日期 guard 跨越 await"之类问题。
// 2. `OnceLock<Mutex<...>>` 延迟初始化，避免构造期全局状态问题。
// 3. `current_as_of()` 以 task_local 的**可见性**为准：可见（含 `Ok(None)` =
//    显式声明 live）就用它的值，只有不可见（跨 spawn）才回退全局。**不可**把
//    「显式 live」与「task_local 不可见」合并处理 —— 合并会让全局残留穿透显式
//    live 声明（2026-09-19 修复，详见 `current_as_of()` 的文档注释）。
// 4. `with_optional_asof` 压入一层作用域；离开时按 **token** 移除自己那一层
//    （而不是"写回进入前读到的值"），确保同一进程内多次/并发回放互不污染。
//    为什么不能用"写回旧值"，见 `AsOfScopeStack` 的文档。

/// 进程级全局 AsOf **作用域栈**：task_local 不可见时的兜底。
///
/// 为什么是栈，而不是「单槽 + 退出时写回进入前读到的值」：
/// 后者在**并发**下必然残留。设外层作用域 A 仍持有全局时，
/// `stock_workflow/core.rs` 把 DAG `tokio::spawn` 出去，子任务进入自己的
/// `with_optional_asof` 时读到 `prev = A` —— 而 A **不是它的前一个值，是别人的当前值**。
/// 于是：外层先退出（写回 A 的前一值 None）、子任务后退出（写回 A）
/// ⇒ 全局永久停在 A，撤销了外层的恢复。
///
/// 后果不是"少恢复一次"，而是**整进程被锁进回放模式**：
/// 2026-09-23 实测（`axagent-batch-rerun --since 2026-07-21 --apply --reflect-after`）——
/// 4 条 as-of(2026-07-21) 分析全部 `completed` 且落库正确，但紧随其后的批量反思
/// （在 as-of 作用域**之外**执行）拿到的 K 线被截断在 07-21 ⇒ 4/4 报
/// 「在 2026-07-21 之后无K线数据」⇒ 降级为无行情反思
/// ⇒ `deterministic_was_correct` 恒 `None` ⇒ **一条 `strategy_performance` 都不写**，
/// 反思→规则→反哺闭环（阶段四）整体空转 —— 而日志里每条分析的结论都"正常"。
///
/// 栈语义：进入 = `push(token, ctx)`，退出 = 按 **token** 移除自己那一层
/// （不按位置、不按值比较），栈顶即当前生效值。
/// 并发交错、嵌套、乱序退出三种情形都自洽 —— 每个作用域只负责移除**自己**。
struct AsOfScopeStack {
    /// (token, ctx)；token 单调递增且唯一，用于精确移除自己那一层
    entries: Vec<(u64, Option<AsOfContext>)>,
    next_token: u64,
}

/// 进程级全局 AsOf 作用域栈
static GLOBAL_AS_OF: OnceLock<Mutex<AsOfScopeStack>> = OnceLock::new();

#[inline]
fn global_lock() -> &'static Mutex<AsOfScopeStack> {
    GLOBAL_AS_OF.get_or_init(|| Mutex::new(AsOfScopeStack { entries: Vec::new(), next_token: 0 }))
}

/// 压入一层全局作用域，返回该层的 token（退出时用它精确移除自己）。
///
/// 注意：这是同步调用，**不**会跨 await 持锁；用于在 `tokio::spawn`
/// 之前先同步写入，使 spawn 出去的 future 通过 `current_as_of()`
/// 也能读到截止日。
fn push_global_asof(ctx: Option<AsOfContext>) -> u64 {
    let mut g = global_lock().lock();
    g.next_token += 1;
    let token = g.next_token;
    g.entries.push((token, ctx));
    token
}

/// 按 token 精确移除自己那一层；该层已被 `clear_global_asof` 清掉时无操作。
///
/// **不可**退化为"移除栈顶"或"按值查找"：并发交错时退出顺序与进入顺序无关，
/// 只有 token 能唯一定位"我压的那一层"。
fn pop_global_asof(token: u64) {
    let mut g = global_lock().lock();
    if let Some(pos) = g.entries.iter().position(|(t, _)| *t == token) {
        g.entries.remove(pos);
    }
}

/// 同步写入全局 AsOfContext（**重置为单层**：清空栈后压入一层），
/// 返回设置前**生效**的值（即原栈顶）。
///
/// 注意：这是同步调用，**不**会跨 await 持锁；用于在 `tokio::spawn`
/// 之前先同步写入，使 spawn 出去的 future 通过 `current_as_of()`
/// 也能读到截止日。
///
/// 之所以"重置为单层"而不是压栈：本函数的语义是**显式设定当前模式**，
/// 调用方（`enter_global_asof` / 测试）期望"我设的值立即生效"。
/// 需要可并存的嵌套/并发作用域请走 `with_optional_asof` / `enter_global_asof`，
/// 它们压栈并按 token 退出。
pub fn set_global_asof(ctx: Option<AsOfContext>) -> Option<AsOfContext> {
    let mut g = global_lock().lock();
    let prev = g.entries.last().and_then(|(_, c)| *c);
    g.entries.clear();
    g.next_token += 1;
    let token = g.next_token;
    g.entries.push((token, ctx));
    prev
}

/// 同步读取当前生效的全局 AsOfContext（栈顶；不影响 task_local 优先级）
pub fn peek_global_asof() -> Option<AsOfContext> {
    let g = global_lock().lock();
    g.entries.last().and_then(|(_, c)| *c)
}

/// 同步清空全局 AsOf 作用域栈，返回清空前生效的值
pub fn clear_global_asof() -> Option<AsOfContext> {
    let mut g = global_lock().lock();
    let prev = g.entries.last().and_then(|(_, c)| *c);
    g.entries.clear();
    prev
}

/// RAII 守卫：构造时压入一层全局作用域，drop 时**按 token 移除自己那一层**。
///
/// 用法（推荐用于 Tauri command 入口）：
/// ```ignore
/// let _guard = as_of::enter_global_asof(Some(ctx));
/// // 此作用域内 current_as_of() 在任何 task（包括 spawn 出去的）都可见 ctx
/// // 作用域结束自动恢复（恢复 = 移除自己那一层，露出下面一层）
/// ```
///
/// ⚠ 与旧实现的差别：drop 时**不再写回"进入前读到的值"**。
/// 写回旧值在并发下会把自己不拥有的一层重新装回去（详见 `AsOfScopeStack`），
/// 是"进程被永久锁进回放模式"的根因。
pub struct AsOfScopeGuard {
    /// 自己那一层的 token；`drop` 时用它精确移除
    token: u64,
}

impl Drop for AsOfScopeGuard {
    fn drop(&mut self) {
        pop_global_asof(self.token);
    }
}

pub fn enter_global_asof(ctx: Option<AsOfContext>) -> AsOfScopeGuard {
    AsOfScopeGuard { token: push_global_asof(ctx) }
}

/// 读取当前任务的 AsOfContext。
///
/// 判据是 task_local 的**可见性**，不是它的取值 —— 这两种情况必须分开：
///
/// - `Ok(ctx)`（**含 `Ok(None)`**）⇒ 本任务已显式声明模式，一切以它为准。
///   `Ok(None)` = 显式声明 live，**必须屏蔽全局回退**：否则同一进程内别的执行流
///   留在全局里的 AsOf 会穿透这条声明（2026-09-19 修复；原先写作
///   `if let Ok(Some(c)) = …`，把「显式 live」与「task_local 不可见」压成同一类）。
/// - `Err`（task_local 不可见 —— 跨 `tokio::spawn` / `JoinSet::spawn` 边界）
///   ⇒ 回落进程级全局。这是全局回退**唯一**存在的理由。
pub fn current_as_of() -> Option<AsOfContext> {
    match AS_OF.try_with(|c| *c) {
        Ok(ctx) => ctx,
        // spawn 边界兜底：task_local 不可见时读进程级全局
        Err(_) => peek_global_asof(),
    }
}

/// 获取 as-of 日期作为 YYYY-MM-DD 字符串，无 as-of 时返回系统当前日期
pub fn current_date_or_now() -> String {
    match current_as_of() {
        Some(ctx) => ctx.as_of_date.format("%Y-%m-%d").to_string(),
        // 修复 M22: 统一按北京时间（UTC+8）取日期，避免部署机本地时区导致 as-of 缺省日期偏移
        // （对齐 calendar.rs::beijing_now 口径；其余模块均固定 FixedOffset::east_opt(8*3600)）
        None => {
            let beijing = Utc::now() + Duration::hours(8);
            beijing.format("%Y-%m-%d").to_string()
        },
    }
}

/// 判断当前是否处于时间旅行模式
pub fn is_asof_active() -> bool {
    current_as_of().is_some()
}

/// 判断"指定数据种类"是否受当前 as-of 影响
///
/// 决策矩阵(借鉴 TradingAgents-CN README 202 行):
/// | scope         | Structured 工具 | Unstructured 工具 | Rank 工具 |
/// |---------------|----------------|-------------------|-----------|
/// | All           | ✅ 受影响       | ✅ 受影响          | ✅ 受影响  |
/// | Structured    | ✅ 受影响       | ❌ 实时(穿透)     | ❌ 实时   |
/// | 无 as-of(live) | ❌ 实时       | ❌ 实时            | ❌ 实时   |
///
/// 用法(供 vendor 调用处判断):
/// ```ignore
/// if as_of::is_asof_active_for(AsOfDataKind::Structured) {
///     let adjusted = current_as_of().unwrap();
///     vendor.fetch_as_of(adjusted.as_of_date);
/// } else {
///     vendor.fetch_live();
/// }
/// ```
pub fn is_asof_active_for(kind: AsOfDataKind) -> bool {
    match current_as_of() {
        None => false,
        Some(ctx) => match ctx.data_scope {
            AsOfDataScope::All => true,
            AsOfDataScope::Structured => matches!(kind, AsOfDataKind::Structured),
        },
    }
}

/// 在指定 scope 内运行闭包，并提供降级日志的 task_local 容器
pub async fn with_degradation_log<F, T>(f: F) -> T
where
    F: std::future::Future<Output = T>,
{
    DEGRADATION_LOG.scope(std::cell::RefCell::new(Vec::new()), f).await
}

/// 当前数据新鲜度描述(供工作流 prompt `{{data_freshness}}` 变量注入)
///
/// 返回中文短语,如:
/// - live 模式 → "实时数据(无时间锚定)"
/// - Structured + as_of=X → "价格/技术/财务 截至 X,新闻/公告 实时"
/// - All + as_of=X       → "全数据截至 X(回放模式)"
pub fn data_freshness_description() -> String {
    match current_as_of() {
        None => "实时数据(无时间锚定)".to_string(),
        Some(ctx) => {
            let date = ctx.as_string();
            match ctx.data_scope {
                AsOfDataScope::All => format!("全数据截至 {date}(回放模式)"),
                AsOfDataScope::Structured => {
                    format!("价格/技术/财务 截至 {date},新闻/公告 实时")
                },
            }
        },
    }
}

/// 生成当前 AsOf 的 cache key 后缀（live 模式返回 "live"）
pub fn cache_suffix() -> String {
    current_as_of()
        .map(|c| format!("asof-{}", c.as_of_date.format("%Y%m%d")))
        .unwrap_or_else(|| "live".to_string())
}

// DegradationEntry 的权威定义在 `axagent_harness::as_of`，本模块顶部已 pub use。
// 此处仅保留运行时降级日志缓冲（task_local + 全局环形缓冲）。

// 任务级降级日志：每个 tokio 任务一个 Vec，scope 结束时不重置，
// 由 workflow 节点通过 take_asof_degradation_report() 一次性消费并清空。
tokio::task_local! {
    static DEGRADATION_LOG: std::cell::RefCell<Vec<DegradationEntry>>;
}

/// 全局降级环形缓冲(缺陷 E 修复):供前端 poll 实时显示降级数量/详情。
/// 不依赖 task_local 作用域(全局可见),cap 256 条,满了弹出最早。
///
/// 每条携带「最后一次被记录的序号」(`GLOBAL_DEGRADATION_SEQ` 水位):
/// 重复条目不追加、只刷新其 seq —— 这样「本次运行是否又降级了」可以按
/// `seq > watermark` 精确切片(见 `take_global_degradations_since`),
/// 同时前端面板仍保持按 `(vendor, method, reason)` 去重不刷屏。
static GLOBAL_DEGRADATION_LOG: Mutex<VecDeque<(u64, DegradationEntry)>> =
    Mutex::new(VecDeque::new());
static GLOBAL_DEGRADATION_TOTAL: AtomicU64 = AtomicU64::new(0);
/// 降级**事件**计数(每次 `record_degradation` 都递增,含重复条目)。
/// 与 `GLOBAL_DEGRADATION_TOTAL`(只数不同条目)语义不同,勿混用。
static GLOBAL_DEGRADATION_SEQ: AtomicU64 = AtomicU64::new(0);
/// 当前工作流运行的降级基线（事件 seq 水位）。由 `stock_workflow/core.rs` 在
/// 运行入口与 `deg_watermark` 同处写入。
///
/// 为什么需要（R5，2026-09-26 实证）：data-quality 的豁免判据若只按 as_of 日期
/// 过滤缓冲，**同截止日的修复前旧运行**残留的条目会被当成本轮降级 ⇒ 全 10 维
/// 无差别豁免、真工具故障也被抹掉。基线 = 「本轮开始那一刻」的 seq。
/// 前提与工作流水位本身一致：工作流有全局 permit 串行，单基线成立。
static GLOBAL_DEGRADATION_BASELINE: AtomicU64 = AtomicU64::new(0);

const GLOBAL_DEGRADATION_CAP: usize = 256;

/// 记录一次降级(仅在 as-of 模式下有效，live 模式直接忽略)
///
/// **按 `(vendor, method, reason)` 去重**：一次回放里同一维度常被多个分析师节点反复调用
/// （如 `t-news-data` 与决策节点都调 `get_news`），逐次追加会让前端降级面板被同一条目刷屏，
/// 反而看不出"到底哪几个维度降级了"。⇒ 同一条目只记一次，累计总数也不重复计数。
/// 需要区分「同一 method 的不同对象」时，把对象写进 reason（如 `get_news` 带股票代码）。
pub fn record_degradation(vendor: &str, method: &str, reason: &str) {
    let as_of = match current_as_of() {
        Some(c) => c.as_string(),
        None => return, // live 模式无降级概念
    };
    let entry = DegradationEntry {
        vendor: vendor.to_string(),
        method: method.to_string(),
        reason: reason.to_string(),
        as_of,
    };
    let is_duplicate = |e: &DegradationEntry| {
        e.vendor == entry.vendor && e.method == entry.method && e.reason == entry.reason
    };
    // 事件序号：每次 record（含重复）都消费一个，供运行边界水位切片用
    let seq = GLOBAL_DEGRADATION_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    // 任务级尝试：若没在 task_local scope 中，单独初始化一个新 scope
    let _ = DEGRADATION_LOG.try_with(|cell| {
        let mut log = cell.borrow_mut();
        if !log.iter().any(is_duplicate) {
            log.push(entry.clone());
        }
    });
    // 全局环形缓冲: 累计总数只按「不同条目」增长；重复条目刷新 last_seq
    {
        let mut g = GLOBAL_DEGRADATION_LOG.lock();
        if let Some(existing) = g.iter_mut().find(|(_, e)| is_duplicate(e)) {
            // 刷新为本次事件的序号：即使内容早已在缓冲里，
            // 「本运行内再次降级」这一事实也必须能被水位切片检出。
            existing.0 = seq;
            return;
        }
        if g.len() >= GLOBAL_DEGRADATION_CAP {
            g.pop_front();
        }
        g.push_back((seq, entry));
        GLOBAL_DEGRADATION_TOTAL.fetch_add(1, Ordering::Relaxed);
    }
}

/// 在可选 AsOfContext 包裹下运行闭包。
///
/// - `ctx = None`  → 直接执行 `f`（live 模式，零开销）
/// - `ctx = Some(c)` → `AS_OF.scope(Some(c), f)`（让 vendor 调用能读到 task_local）
///
/// **同时同步写入进程级全局**，使 `tokio::spawn` / `JoinSet::spawn`
/// 出去的 future 通过 `current_as_of()` 也能读到截止日。
///
/// ⚠️ 2026-08-01 实锤修复：原实现"故意不恢复全局"，要求调用方显式
/// `set_global_asof(None)` 清理，但实际 90% 调用方（backtest_reco_strategies、
/// backtest_analysis、recommend_stocks 等）都没清理 → `GLOBAL_AS_OF` 永久残留
/// as-of 上下文 → 之后用户在**同一进程**跑 Serenity 趋势智选时：
/// `search_stock` 直接走 as-of 分支 `return Ok(vec![])`（空）、
/// `get_industry_ranking` 走 as-of 分支查每日快照（凌晨无快照）+ vendor
/// NoHistoricalSemantic 全跳过 → 也空 → 上游节点全空、0 候选。
///
/// 现改为 **RAII 语义**（与 `enter_global_asof` 一致）。
///
/// ⚠️ 2026-09-23 再修（第二次踩同一族坑，方向相反）：上一版 RAII 实现是
/// 「退出时把进入前读到的值写回全局」，在**并发**下必然残留 —— 外层作用域仍持有
/// 全局时，本函数内 `tokio::spawn` 出去的 fire-and-forget 任务（如
/// `stock_workflow/core.rs` 的 DAG）读到 `prev = 外层值`（那是**别人的当前值**，
/// 不是它的前值）；外层先退出写回 None、子任务后退出把外层值写回
/// ⇒ 全局永久停在那个 as-of，整进程被锁进回放模式。
/// 实测后果见 `AsOfScopeStack` 的文档（批量重跑后反思全部拿不到行情）。
///
/// 故现改为：**压栈进入、按 token 移除自己那一层**。
/// 这使三种情形都自洽：嵌套（LIFO）、并发交错、以及"进入顺序与退出顺序相反"。
/// 每个作用域只负责移除自己压的那一层，不假定"前一个值"归自己所有。
pub async fn with_optional_asof<F, T>(ctx: Option<AsOfContext>, f: F) -> T
where
    F: std::future::Future<Output = T>,
{
    // 用具名 guard 而非"手动在末尾恢复"：异常展开路径也必须移除本层。
    let _layer = AsOfScopeGuard { token: push_global_asof(ctx) };
    match ctx {
        Some(c) => AS_OF.scope(Some(c), f).await,
        None => f.await,
    }
}

/// 消费并清空当前任务的降级日志。返回累积的降级条目。
/// 必须在 with_degradation_log scope 内调用，否则返回空 Vec。
pub fn take_asof_degradation_report() -> Vec<DegradationEntry> {
    DEGRADATION_LOG.try_with(|cell| std::mem::take(&mut *cell.borrow_mut())).unwrap_or_else(|e| {
        tracing::warn!("[as_of] take_asof_degradation_report 访问失败: {e}");
        Vec::new()
    })
}

/// 仅快照全局降级日志(不清空,供前端 poll 显示)。
/// 返回按时间顺序排列(旧 → 新)的最近 256 条。
pub fn peek_global_degradation_report() -> Vec<DegradationEntry> {
    let g = GLOBAL_DEGRADATION_LOG.lock();
    g.iter().map(|(_, e)| e.clone()).collect()
}

/// 当前降级事件水位。运行入口在 spawn 内调用一次，结束时把它传给
/// `take_global_degradations_since` 即得「本次运行期间记录的降级」切片。
pub fn global_degradation_seq_watermark() -> u64 {
    GLOBAL_DEGRADATION_SEQ.load(Ordering::Relaxed)
}

/// 取水位之后（含重复刷新）的全局降级条目，并合并当前任务的 task-local
/// 条目（按 `(vendor, method, reason)` 去重）。
///
/// 为什么不能只用 `take_asof_degradation_report()`（2026-09-26 修复，缺陷 R1）：
/// 分析师节点各自 `tokio::spawn`，`record_degradation` 对 task-local 的
/// `try_with` 在子任务里静默失败，只有全局环形缓冲收到 ⇒ 父任务的
/// task-local 消费端永远拿到空表 ⇒ 回放明明降级 6+ 维度、面板却显示
/// 「0 个降级」。这与 2026-09-19/09-23 修过的 AS_OF 跨 spawn 问题是同族。
///
/// 已知边界：缓冲 cap 256，单次运行内新增不同条目超过 256 时最早条目被
/// 挤出、切片会漏（实际降级维度远小于该量级）。
pub fn take_global_degradations_since(watermark: u64) -> Vec<DegradationEntry> {
    let mut out = take_asof_degradation_report();
    let g = GLOBAL_DEGRADATION_LOG.lock();
    for (seq, e) in g.iter() {
        if *seq > watermark
            && !out
                .iter()
                .any(|x| x.vendor == e.vendor && x.method == e.method && x.reason == e.reason)
        {
            out.push(e.clone());
        }
    }
    out
}

/// 当前累计降级总数(从进程启动起算,跨 live/replay 切换)。
pub fn global_degradation_count() -> u64 {
    GLOBAL_DEGRADATION_TOTAL.load(Ordering::Relaxed)
}

/// 设定当前运行的降级基线（工作流入口调用一次）。
pub fn set_global_degradation_baseline(baseline: u64) {
    GLOBAL_DEGRADATION_BASELINE.store(baseline, Ordering::Relaxed);
}

/// 基线之后被记录（含重复刷新）的降级方法名清单，供 data-quality 节点
/// 判定「哪些维度本轮确实发生了设计性降级」（R5 豁免收紧的判据源）。
pub fn global_degraded_methods_since_baseline() -> Vec<String> {
    let baseline = GLOBAL_DEGRADATION_BASELINE.load(Ordering::Relaxed);
    let g = GLOBAL_DEGRADATION_LOG.lock();
    let mut out: Vec<String> = Vec::new();
    for (seq, e) in g.iter() {
        if *seq > baseline && !out.contains(&e.method) {
            out.push(e.method.clone());
        }
    }
    out
}

/// 清空全局降级缓冲(切换到 live 模式时由前端触发,避免过期条目一直显示)。
pub fn reset_global_degradation_log() {
    let mut g = GLOBAL_DEGRADATION_LOG.lock();
    g.clear();
    // total 不重置,保留"曾经降级过多少项"作为历史指标
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Local, NaiveDate};
    use serial_test::serial;

    #[tokio::test]
    #[serial(asof)]
    async fn current_as_of_returns_none_outside_scope() {
        // 清理全局，确保测试隔离
        let _ = clear_global_asof();
        assert!(current_as_of().is_none());
        assert!(!is_asof_active());
    }

    #[tokio::test]
    #[serial(asof)]
    async fn current_as_of_returns_value_inside_scope() {
        let _ = clear_global_asof();
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let got = AS_OF.scope(Some(ctx), async { current_as_of() }).await;
        assert_eq!(got.unwrap().as_of_date, date);
        assert_eq!(got.unwrap().source, AsOfSource::UserReplay);
    }

    /// 回归（2026-09-19）：**显式 live 声明必须屏蔽进程级全局残留**。
    ///
    /// 缺陷形态：`current_as_of()` 原写作 `if let Ok(Some(c)) = AS_OF.try_with(..)`，
    /// 把「本任务显式声明 live（`Ok(None)`）」与「task_local 不可见（`Err`）」压成
    /// 同一类 ⇒ 只要进程里残留一个全局 AsOf（例如兄弟用例装完没还原），显式
    /// `scope(None, …)` 也会读到它，`vendors_for_live_*` /
    /// `kline_cache_key_live_no_effective_suffix` 一族断言随之全红。
    #[tokio::test]
    #[serial(asof)]
    async fn explicit_live_scope_shields_global_residue() {
        let leaked =
            AsOfContext::new(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(), AsOfSource::UserReplay)
                .unwrap();
        let _ = set_global_asof(Some(leaked));

        // 被测行为：scope 内显式声明 live ⇒ 必须为 None，无视全局残留。
        let inside = AS_OF.scope(None, async { current_as_of() }).await;
        assert!(
            inside.is_none(),
            "显式 scope(None)（声明 live）不得回落到进程级全局残留，实际读到 {:?}",
            inside.map(|c| c.as_of_date.to_string())
        );

        // 负控（先证判据会告警）：**同一份残留**下，无 scope 的读取必须读到全局 ——
        // 否则上一条断言只是「全局恰好为空」带来的平凡真，证明不了屏蔽行为。
        assert!(
            current_as_of().is_some(),
            "负控失效：全局残留未被读到 ⇒ 上一条断言对「屏蔽」无区分力"
        );

        let _ = clear_global_asof();
    }

    #[tokio::test]
    async fn validate_rejects_future_date() {
        let future = Local::now().date_naive() + Duration::days(7);
        let result = AsOfContext::new(future, AsOfSource::UserReplay);
        assert!(matches!(result, Err(AsOfError::FutureDate { .. })));
    }

    #[tokio::test]
    async fn validate_rejects_empty_string() {
        let result = AsOfContext::parse("");
        assert!(matches!(result, Err(AsOfError::InvalidFormat { .. })));
    }

    #[tokio::test]
    async fn validate_rejects_invalid_format() {
        let result = AsOfContext::parse("2026/06/01");
        assert!(matches!(result, Err(AsOfError::InvalidFormat { .. })));
    }

    #[tokio::test]
    #[serial(asof)]
    async fn today_is_accepted() {
        let today = Local::now().date_naive();
        let ctx = AsOfContext::new(today, AsOfSource::UserReplay).unwrap();
        assert_eq!(ctx.as_of_date, today);
    }

    #[tokio::test]
    async fn parse_roundtrip() {
        let today = Local::now().date_naive();
        let s = today.format("%Y-%m-%d").to_string();
        let ctx = AsOfContext::parse(&s).unwrap();
        assert_eq!(ctx.as_string(), s);
    }

    #[test]
    fn parse_optional_none_is_live() {
        let r = AsOfContext::parse_optional(None).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn parse_optional_empty_is_live() {
        assert!(AsOfContext::parse_optional(Some("")).unwrap().is_none());
        assert!(AsOfContext::parse_optional(Some("   ")).unwrap().is_none());
    }

    #[test]
    fn parse_optional_past_date_is_replay() {
        let today = Local::now().date_naive();
        let past = today - Duration::days(7);
        let s = past.format("%Y-%m-%d").to_string();
        let r = AsOfContext::parse_optional(Some(&s)).unwrap();
        assert!(r.is_some());
        assert_eq!(r.unwrap().as_string(), s);
    }

    #[test]
    #[serial(asof)]
    fn parse_optional_future_date_rejected() {
        let future = Local::now().date_naive() + Duration::days(7);
        let s = future.format("%Y-%m-%d").to_string();
        let r = AsOfContext::parse_optional(Some(&s));
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("as_of_date 解析失败"));
    }

    #[test]
    fn parse_optional_invalid_format_rejected() {
        assert!(AsOfContext::parse_optional(Some("2026/06/01")).is_err());
        assert!(AsOfContext::parse_optional(Some("garbage")).is_err());
    }

    #[tokio::test]
    #[serial(asof)]
    async fn cache_suffix_returns_live_outside_scope() {
        let _ = clear_global_asof();
        let s = cache_suffix();
        assert_eq!(s, "live");
    }

    #[tokio::test]
    #[serial(asof)]
    async fn cache_suffix_includes_date_inside_scope() {
        let _ = clear_global_asof();
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let s = AS_OF.scope(Some(ctx), async { cache_suffix() }).await;
        assert_eq!(s, "asof-20260601");
    }

    #[tokio::test]
    #[serial(asof)]
    async fn nested_scope_uses_inner_value() {
        let _ = clear_global_asof();
        let outer_date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let inner_date = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
        let outer = AsOfContext::new(outer_date, AsOfSource::UserReplay).unwrap();
        let inner = AsOfContext::new(inner_date, AsOfSource::UserReplay).unwrap();
        let result = AS_OF
            .scope(Some(outer), async {
                let outer_val = current_as_of().unwrap().as_of_date;
                let inner_val =
                    AS_OF.scope(Some(inner), async { current_as_of().unwrap().as_of_date }).await;
                // 内层 scope 结束后，外层值恢复
                let after_inner = current_as_of().unwrap().as_of_date;
                (outer_val, inner_val, after_inner)
            })
            .await;
        assert_eq!(result, (outer_date, inner_date, outer_date));
    }

    // ── 降级日志(spec §4.1 统一降级协议) ────────────────────────
    // 实时性方法在 as-of 模式下跳过时，必须把降级原因写入日志供 workflow 消费

    #[tokio::test]
    #[serial(asof)]
    async fn record_degradation_ignored_in_live_mode() {
        let _ = clear_global_asof();
        record_degradation("vendor", "method", "test");
        // live 模式下没有 task_local scope，take 返回空
        let report = take_asof_degradation_report();
        assert!(report.is_empty());
    }

    #[tokio::test]
    #[serial(asof)]
    async fn record_degradation_captures_entries_in_asof_scope() {
        let _ = clear_global_asof();
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        // 正确用法：AS_OF.scope 包裹 with_degradation_log，workflow 节点调用入口
        AS_OF
            .scope(Some(ctx), async {
                with_degradation_log(async {
                    record_degradation("eastmoney", "get_hot_stocks", "no historical semantics");
                    record_degradation("tencent", "get_cls_flash", "future-only feed");
                    let report = take_asof_degradation_report();
                    assert_eq!(report.len(), 2);
                    assert_eq!(report[0].vendor, "eastmoney");
                    assert_eq!(report[0].as_of, "2026-06-01");
                    assert_eq!(report[1].method, "get_cls_flash");
                    // 消费后清空
                    let second = take_asof_degradation_report();
                    assert!(second.is_empty(), "take 应当清空日志");
                })
                .await
            })
            .await;
    }

    /// 回归（2026-09-25）：同一 `(vendor, method, reason)` 只记一条。
    ///
    /// 实测缺陷形态：一次回放里 `get_news` 被两个节点各调一次，同一原因追加两条
    /// ⇒ 前端降级面板 5 条里有 2 条是同一件事，看不出真实降级维度数。
    #[tokio::test]
    #[serial(asof)]
    async fn record_degradation_dedups_identical_entries() {
        let _ = clear_global_asof();
        reset_global_degradation_log();
        let before_total = global_degradation_count();
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        AS_OF
            .scope(Some(ctx), async {
                with_degradation_log(async {
                    for _ in 0..3 {
                        record_degradation("astock-data", "get_news", "as-of 模式无历史新闻");
                    }
                    // 不同 reason（如不同个股）必须各自留痕，去重不能吞掉真实信息
                    record_degradation("astock-data", "get_news", "as-of 模式无 600519 历史新闻");
                    let report = take_asof_degradation_report();
                    assert_eq!(report.len(), 2, "同条目应去重，异条目应保留: {report:?}");
                    assert_eq!(report[0].reason, "as-of 模式无历史新闻");
                })
                .await
            })
            .await;
        let global = peek_global_degradation_report();
        assert_eq!(
            global.iter().filter(|e| e.reason == "as-of 模式无历史新闻").count(),
            1,
            "全局环形缓冲同样不得重复追加"
        );
        assert_eq!(
            global_degradation_count() - before_total,
            2,
            "累计总数只按「不同条目」增长，重复调用不得虚增"
        );
        reset_global_degradation_log();
    }

    // ── 进程级全局回退(缺陷: spawn 边界穿透) ──────────────────

    /// task_local 优先级：在两层都设值时，task_local 胜出
    #[tokio::test]
    #[serial(asof)]
    async fn current_asof_prefers_task_local() {
        let _ = clear_global_asof();
        let task_date = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
        let global_date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let task_ctx = AsOfContext::new(task_date, AsOfSource::UserReplay).unwrap();
        let global_ctx = AsOfContext::new(global_date, AsOfSource::ScheduledReplay).unwrap();
        set_global_asof(Some(global_ctx));
        let got = AS_OF.scope(Some(task_ctx), async { current_as_of().unwrap() }).await;
        assert_eq!(got.as_of_date, task_date, "task_local 必须胜过全局");
        assert_eq!(got.source, AsOfSource::UserReplay);
        let _ = clear_global_asof();
    }

    /// 跨 spawn 边界全局回退：spawn 出去的 future 应当读到全局值
    #[serial(asof)]
    #[tokio::test]
    async fn global_fallback_survives_tokio_spawn() {
        let _ = clear_global_asof();
        let date = NaiveDate::from_ymd_opt(2026, 6, 8).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        set_global_asof(Some(ctx));

        // 在新 spawn 的任务中（无 task_local scope），current_as_of 应当
        // 通过全局回退读到 ctx
        let got = tokio::spawn(async move { current_as_of() }).await.unwrap();
        assert!(got.is_some(), "spawn 边界后必须能读到全局 AsOfContext");
        assert_eq!(got.unwrap().as_of_date, date);
        let _ = clear_global_asof();
    }

    /// 没有 task_local 也没有全局时，current_as_of 返回 None
    #[serial(asof)]
    #[tokio::test]
    async fn current_asof_falls_back_to_global_outside_scope() {
        let _ = clear_global_asof();
        // 仅有 task_local，无全局：scope 内读到值
        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::BacktestSweep).unwrap();
        let in_scope = AS_OF.scope(Some(ctx), async { current_as_of().is_some() }).await;
        assert!(in_scope);
        // scope 外（且全局已清空）应返回 None
        assert!(current_as_of().is_none());
    }

    /// RAII 守卫：早返回也必须恢复原值
    #[tokio::test]
    #[serial(asof)]
    async fn raii_guard_restores_on_early_return() {
        let _ = clear_global_asof();
        let today = Local::now().date_naive();
        let original_date = today - Duration::days(90);
        let temp_date = today - Duration::days(7);
        let original = AsOfContext::new(original_date, AsOfSource::UserReplay).unwrap();
        let temp = AsOfContext::new(temp_date, AsOfSource::ScheduledReplay).unwrap();
        set_global_asof(Some(original));

        async fn inner(temp: AsOfContext) -> Option<AsOfContext> {
            let _guard = enter_global_asof(Some(temp));
            assert_eq!(current_as_of().unwrap().as_of_date, temp.as_of_date);
            // 早返回：_guard 仍会 drop，恢复原值
            current_as_of()
        }

        let in_temp = inner(temp).await;
        assert_eq!(in_temp.unwrap().as_of_date, temp_date);
        // inner 返回后，全局应当恢复为 original
        let after = current_as_of().unwrap();
        assert_eq!(after.as_of_date, original_date, "RAII 必须恢复原值");
        let _ = clear_global_asof();
    }

    /// 嵌套守卫：内层 drop 后必须 LIFO 恢复到外层值
    #[tokio::test]
    #[serial(asof)]
    async fn nested_guards_restore_lifo() {
        let _ = clear_global_asof();
        let outer_date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let inner_date = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
        let outer = AsOfContext::new(outer_date, AsOfSource::UserReplay).unwrap();
        let inner = AsOfContext::new(inner_date, AsOfSource::ScheduledReplay).unwrap();
        set_global_asof(Some(outer));

        let _g_outer = enter_global_asof(Some(outer));
        assert_eq!(current_as_of().unwrap().as_of_date, outer_date);

        {
            let _g_inner = enter_global_asof(Some(inner));
            assert_eq!(current_as_of().unwrap().as_of_date, inner_date);
            // _g_inner 在此 block 结束时 drop
        }

        // 回到 outer
        assert_eq!(current_as_of().unwrap().as_of_date, outer_date, "内层 drop 后必须恢复 outer");
        // 2026-09-19 修复：必须**先显式 drop 外层 guard，再清空全局**。
        //   原写法（clear 在前、_g_outer 由函数返回时 drop）的真实执行序是：
        //     clear_global_asof() → GLOBAL_AS_OF = None
        //     _g_outer.drop()     → set_global_asof(prev)，而 prev 是 :570 创建 guard 时
        //                           读到的 Some(outer)（因为 :568 已先把全局设成 outer）
        //                       ⇒ 把 outer 又**装回去**，等于撤销这次 clear。
        //   后果：进程级 GLOBAL_AS_OF 永久残留 outer(2026-01-01)，
        //         污染同二进制内**其后**所有读全局的用例。
        //   实测（`--test-threads=1` 强制顺序）：本用例 + `asof_realtime_degrade_tests::
        //   is_asof_active_false_in_live` ⇒ 后者 FAILED（读到 asof-20260101 ⇒ 判成 replay）；
        //   全量并行时另有 5 条 live 断言同因变红（`vendors_for_live_*` / `kline_cache_key_live_*`
        //   / `should_use_asof_live_is_false` / `try_vendor_with_asof_live_returns_none`）。
        //   为什么一直没暴露：全量串行按字典序跑，本用例之后还有十几条 `as_of::tests`
        //   用例**开头都 clear_global_asof()**，顺手当了清道夫 ⇒ 掩盖成假绿。
        drop(_g_outer);
        let _ = clear_global_asof();
    }

    /// 回归（2026-09-23）：**并发/乱序退出的作用域不得把 as-of 残留进进程级全局**。
    ///
    /// 缺陷形态（实测于 `axagent-batch-rerun --since 2026-07-21 --apply --reflect-after`）：
    /// `with_optional_asof` 上一版以「退出时写回进入前读到的值」实现 RAII。
    /// `stock_workflow/core.rs` 在 as-of 作用域内把 DAG `tokio::spawn` 出去，
    /// 子任务进入时读到 `prev = 外层的当前值`（那是**别人的**值，不是它的前值）；
    /// 外层先退出（写回 None）、子任务后退出（把那层的值写回）
    /// ⇒ 全局永久停留在该 as-of ⇒ 紧随其后的批量反思在 07-21 被截断 K 线
    /// ⇒ 4/4 降级为「无行情反思」⇒ 不写 strategy_performance。
    ///
    /// 用例用 push/pop 显式构造「进入序 A→B、退出序 A→B」的**乱序**，
    /// 不依赖线程调度，故无 flaky 风险。
    #[tokio::test]
    #[serial(asof)]
    async fn overlapping_scopes_leave_no_global_residue() {
        let _ = clear_global_asof();
        let date = NaiveDate::from_ymd_opt(2026, 7, 21).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();

        // 负控（先证判据会告警）：压入一层且不移除 ⇒ 必须读到 as-of。
        // 否则下面那条 `is_none()` 只是「栈恰好为空」的平凡真，证明不了移除行为。
        let ctl = push_global_asof(Some(ctx));
        assert!(
            current_as_of().is_some(),
            "负控失效：push 之后应读到 as-of，否则本用例对「移除」无区分力"
        );
        pop_global_asof(ctl);

        // 外层进入（等价 `with_optional_asof(Some(ctx), …)`）
        let outer = push_global_asof(Some(ctx));
        // 子任务在外层**仍持有全局时**进入
        // （等价 `core.rs` 那个 fire-and-forget spawn 内部再 `with_optional_asof`）
        let inner = push_global_asof(Some(ctx));

        // 外层**先**退出：只移除自己那一层 ⇒ 子任务那层仍生效
        pop_global_asof(outer);
        assert_eq!(
            current_as_of().map(|c| c.as_string()),
            Some("2026-07-21".to_string()),
            "外层先退出时，子任务的作用域必须仍然有效"
        );

        // 子任务后退出：移除自己那层后栈空 ⇒ 必须回到 live
        pop_global_asof(inner);
        assert!(
            current_as_of().is_none(),
            "全部作用域退出后不得残留 as-of（残留会把整进程锁进回放模式），实际读到 {:?}",
            current_as_of().map(|c| c.as_string())
        );

        // 正常 LIFO 退出同样必须无残留（防「只修乱序、修坏常规嵌套」）
        let a = push_global_asof(Some(ctx));
        let b = push_global_asof(Some(ctx));
        pop_global_asof(b);
        pop_global_asof(a);
        assert!(current_as_of().is_none(), "正常 LIFO 退出同样不得残留");

        let _ = clear_global_asof();
    }

    // ── 混合 as-of 模式(Phase 1:数据范围分离) ──────────────────
    // 设计动机：参考 TradingAgents-CN README 的"价格截止 + 社交/新闻实时"。
    // 用户回放个股时，价格/技术/财务按 as_of 截止，但想看当时的新闻/公告
    // 是否还有后效（如事件影响持续到回放日期之后）。本组测试覆盖该模式。

    /// 默认行为兼容：未设置 data_scope 时等同于 All（保持旧语义）
    #[test]
    fn data_scope_default_is_all_compatible() {
        assert_eq!(AsOfDataScope::default(), AsOfDataScope::All);
        let today = Local::now().date_naive();
        let ctx = AsOfContext::new(today, AsOfSource::UserReplay).unwrap();
        assert_eq!(ctx.data_scope, AsOfDataScope::All);
    }

    /// with_data_scope 是消费式 API，链式调用应保留日期/来源不变
    #[test]
    fn with_data_scope_chains_correctly() {
        let today = Local::now().date_naive();
        let ctx = AsOfContext::new(today, AsOfSource::UserReplay)
            .unwrap()
            .with_data_scope(AsOfDataScope::Structured);
        assert_eq!(ctx.data_scope, AsOfDataScope::Structured);
        assert_eq!(ctx.source, AsOfSource::UserReplay);
        assert_eq!(ctx.as_of_date, today);
    }

    /// data_scope=All 时，所有 kind 都被 as-of 拦截
    #[tokio::test]
    #[serial(asof)]
    async fn data_scope_all_blocks_all_kinds() {
        let _ = clear_global_asof();
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let result = AS_OF
            .scope(Some(ctx), async {
                let structured = is_asof_active_for(AsOfDataKind::Structured);
                let unstructured = is_asof_active_for(AsOfDataKind::Unstructured);
                let rank = is_asof_active_for(AsOfDataKind::Rank);
                (structured, unstructured, rank)
            })
            .await;
        assert_eq!(result, (true, true, true), "All 模式必须拦截所有 kind");
    }

    /// data_scope=Structured 时，仅结构化数据被拦截，新闻/公告/排行保持实时
    #[tokio::test]
    #[serial(asof)]
    async fn data_scope_structured_blocks_only_structured() {
        let _ = clear_global_asof();
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay)
            .unwrap()
            .with_data_scope(AsOfDataScope::Structured);
        let result = AS_OF
            .scope(Some(ctx), async {
                let structured = is_asof_active_for(AsOfDataKind::Structured);
                let unstructured = is_asof_active_for(AsOfDataKind::Unstructured);
                let rank = is_asof_active_for(AsOfDataKind::Rank);
                (structured, unstructured, rank)
            })
            .await;
        assert_eq!(
            result,
            (true, false, false),
            "Structured 模式：仅结构化数据走 as-of，新闻/排行放行"
        );
    }

    /// live 模式（无 as_of）下，is_asof_active_for 对所有 kind 都返回 false
    #[tokio::test]
    #[serial(asof)]
    async fn live_mode_blocks_nothing() {
        let _ = clear_global_asof();
        let result = AS_OF
            .scope(None, async {
                (
                    is_asof_active_for(AsOfDataKind::Structured),
                    is_asof_active_for(AsOfDataKind::Unstructured),
                    is_asof_active_for(AsOfDataKind::Rank),
                )
            })
            .await;
        assert_eq!(result, (false, false, false));
    }

    /// is_asof_active_for 必须能跨越 spawn 边界（依赖全局回退）
    #[serial(asof)]
    #[tokio::test]
    async fn is_asof_active_for_works_across_spawn_boundary() {
        let _ = clear_global_asof();
        let date = NaiveDate::from_ymd_opt(2026, 6, 8).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay)
            .unwrap()
            .with_data_scope(AsOfDataScope::Structured);
        set_global_asof(Some(ctx));

        // spawn 出去的新任务无 task_local scope，必须能通过全局回退读到 kind 决策
        let (structured, unstructured) = tokio::spawn(async move {
            (
                is_asof_active_for(AsOfDataKind::Structured),
                is_asof_active_for(AsOfDataKind::Unstructured),
            )
        })
        .await
        .unwrap();
        assert!(structured, "spawn 后 Structured 仍走 as-of");
        assert!(!unstructured, "spawn 后 Unstructured 不应被 as-of 拦截");
        let _ = clear_global_asof();
    }

    /// data_freshness_description 应当分别覆盖 live / All / Structured 三种文案
    #[tokio::test]
    #[serial(asof)]
    async fn data_freshness_description_live() {
        let _ = clear_global_asof();
        let s = data_freshness_description();
        assert!(s.contains("实时"), "live 文案必须含『实时』: {s}");
    }

    #[tokio::test]
    #[serial(asof)]
    async fn data_freshness_description_all() {
        let _ = clear_global_asof();
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let s = AS_OF.scope(Some(ctx), async { data_freshness_description() }).await;
        assert!(s.contains("2026-06-01"), "All 文案必须含日期: {s}");
        assert!(s.contains("全数据"), "All 文案必须含『全数据』: {s}");
    }

    #[tokio::test]
    #[serial(asof)]
    async fn data_freshness_description_structured() {
        let _ = clear_global_asof();
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay)
            .unwrap()
            .with_data_scope(AsOfDataScope::Structured);
        let s = AS_OF.scope(Some(ctx), async { data_freshness_description() }).await;
        assert!(s.contains("2026-06-01"));
        assert!(s.contains("新闻"), "Structured 文案必须说明新闻是实时的: {s}");
    }

    /// serde 向后兼容：旧的 JSON 文本没有 data_scope 字段时，解析结果为 All
    #[test]
    fn asof_ctx_backward_compatible_serde() {
        // 旧版本序列化格式（只有两个字段）
        let legacy = r#"{"as_of_date":"2026-06-01","source":"user_replay"}"#;
        let ctx: AsOfContext = serde_json::from_str(legacy).unwrap();
        assert_eq!(ctx.as_of_date.to_string(), "2026-06-01");
        assert_eq!(ctx.source, AsOfSource::UserReplay);
        // data_scope 走 serde default -> All
        assert_eq!(ctx.data_scope, AsOfDataScope::All);
    }

    /// serde 正向：新格式带 snake_case 枚举字符串
    #[test]
    fn asof_ctx_serde_with_data_scope() {
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay)
            .unwrap()
            .with_data_scope(AsOfDataScope::Structured);
        let s = serde_json::to_string(&ctx).unwrap();
        assert!(s.contains("\"data_scope\":\"structured\""), "序列化必须小写枚举名: {s}");
        let back: AsOfContext = serde_json::from_str(&s).unwrap();
        assert_eq!(back, ctx);
    }

    // ── R1 回归（2026-09-26）：降级报告跨 spawn 丢失 →「0 个降级」假象 ──

    /// 缺陷形态（实测于 300642 as_of=2026-09-22 回放）：分析师节点各自
    /// `tokio::spawn`，`record_degradation` 对 task-local 的 `try_with` 在子任务
    /// 里静默失败，只有全局环形缓冲收到；而父任务消费端只读 task-local
    /// ⇒ 回放降级 6+ 维度、面板却显示「0 个降级」。
    #[tokio::test]
    #[serial(asof)]
    async fn watermark_slice_captures_spawned_child_degradations() {
        let _ = clear_global_asof();
        reset_global_degradation_log();
        let date = NaiveDate::from_ymd_opt(2026, 9, 22).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        // with_optional_asof（真实工作流入口用的就是它）：task_local + 全局同步写入，
        // 子任务里的 record_degradation 才能通过全局回退确认"处于 as-of 模式"
        with_optional_asof(Some(ctx), async {
            with_degradation_log(async {
                let watermark = global_degradation_seq_watermark();
                let child = tokio::spawn(async move {
                    record_degradation("astock-data", "search_stock", "as-of 模式搜索不可用");
                });
                child.await.unwrap();
                // 负控（先证缺陷形态仍在）：task-local 消费端拿不到子任务的记录
                assert!(
                    take_asof_degradation_report().is_empty(),
                    "task-local 竟收到子任务记录 ⇒ spawn 前提失效，本用例失去区分力"
                );
                // 被测行为：水位切片必须捕获
                let slice = take_global_degradations_since(watermark);
                assert_eq!(slice.len(), 1, "水位切片应捕获子任务降级: {slice:?}");
                assert_eq!(slice[0].method, "search_stock");
            })
            .await;
        })
        .await;
        reset_global_degradation_log();
    }

    /// 重复条目必须刷新 last_seq：同一 `(vendor, method, reason)` 在**上一轮**
    /// 已留在缓冲里，本轮再次降级时若不刷新序号，水位切片会检不出来。
    #[tokio::test]
    #[serial(asof)]
    async fn duplicate_record_refreshes_seq_for_watermark_slice() {
        let _ = clear_global_asof();
        reset_global_degradation_log();
        let date = NaiveDate::from_ymd_opt(2026, 9, 22).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        // 上一轮运行：同一降级已写入全局缓冲（无 task-local scope）
        with_optional_asof(Some(ctx), async {
            record_degradation("astock-data", "get_cls_flash", "as-of 快讯不可用");
        })
        .await;
        // 本轮运行：子任务再次记录同一降级
        with_optional_asof(Some(ctx), async {
            with_degradation_log(async {
                let watermark = global_degradation_seq_watermark();
                let child = tokio::spawn(async move {
                    record_degradation("astock-data", "get_cls_flash", "as-of 快讯不可用");
                });
                child.await.unwrap();
                let slice = take_global_degradations_since(watermark);
                assert_eq!(
                    slice.iter().filter(|e| e.method == "get_cls_flash").count(),
                    1,
                    "重复条目刷新 seq 后，本轮水位切片必须检得: {slice:?}"
                );
                // 累计总数仍只按「不同条目」计，重复不虚增
                assert_eq!(
                    peek_global_degradation_report()
                        .iter()
                        .filter(|e| e.method == "get_cls_flash")
                        .count(),
                    1,
                    "前端面板不得因重复记录刷屏"
                );
            })
            .await;
        })
        .await;
        reset_global_degradation_log();
    }

    /// R5 回归（2026-09-26，实证于 1ad42f59 重跑）：豁免判据必须按**本轮基线**过滤，
    /// 同截止日旧运行残留的条目不得再触发豁免。
    #[tokio::test]
    #[serial(asof)]
    async fn baseline_excludes_previous_runs_degradations() {
        let _ = clear_global_asof();
        reset_global_degradation_log();
        let date = NaiveDate::from_ymd_opt(2026, 9, 22).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        // 上一轮运行：get_money_flow 已降级并留在缓冲（同 as_of 日期）
        with_optional_asof(Some(ctx), async {
            record_degradation("astock-data", "get_money_flow", "as-of 无历史资金流(旧轮)");
        })
        .await;
        // 本轮运行：入口设基线；只有 get_cls_flash 在本轮降级
        with_optional_asof(Some(ctx), async {
            with_degradation_log(async {
                set_global_degradation_baseline(global_degradation_seq_watermark());
                let child = tokio::spawn(async move {
                    record_degradation("astock-data", "get_cls_flash", "as-of 快讯不可用(本轮)");
                });
                child.await.unwrap();
                let methods = global_degraded_methods_since_baseline();
                assert!(
                    !methods.iter().any(|m| m == "get_money_flow"),
                    "基线前(旧运行)的条目不得进入本轮豁免清单: {methods:?}"
                );
                assert!(
                    methods.iter().any(|m| m == "get_cls_flash"),
                    "本轮子任务降级必须进入清单: {methods:?}"
                );
            })
            .await;
        })
        .await;
        reset_global_degradation_log();
    }
}
