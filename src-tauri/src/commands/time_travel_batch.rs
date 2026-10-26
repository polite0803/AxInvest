//! 时间旅行批次复盘（运行态内存 + `time_travel_batch_runs` 历史表双写）。
//!
//! 状态存储：
//! - **运行态**：进程内 `BATCH_RUNS`（轮询低延迟，互斥语义见下）；
//! - **历史态**：每次状态变更同步 upsert 到 `time_travel_batch_runs` 表
//!   （实体声明见 `axagent_entities::time_travel_batch_runs`，读写统一收口在
//!   `axagent_dao::repo::time_travel_batch_runs`），重启后
//!   `get_time_travel_batch` 从库中还原历史；`list_time_travel_batches` 给任务中心
//!   提供倒序历史。进程异常退出遗留的 `running` 行在新进程首次访问时收敛为
//!   `interrupted`——as_of 封闭世界栈是进程级的，不支持跨进程续跑。
//!
//! 流水线（见 `.trae/documents/time-travel-batch_plan.md`）：
//! 1. 选定历史交易日 `as_of`，进入 [`as_of::with_optional_asof`] 封闭世界；
//! 2. **guard 内**一轮统一回放两条选股链（顶层平级，批次不区分）：
//!    a. 荐股链：候选**落 `reco_picks`，每行带 `as_of_date` = 本批截止日**。铁律 B2
//!    于 2026-10-12 改写——回放的候选必须持久化才有样本可供反思，防污染的手段从
//!    「跳过写库」换成「写库 + 打代际标记」；
//!    b. 趋势智选 serenity 原工作流（同口径落库带代际，但**不写进程内全局种子**，
//!    那层状态无代际可打）；两链候选合并去重（同 code 留置信最高一条）后，
//!    经 [`run_analysis_for_candidates`] 逐只 replay 分析
//!    （`analysis_kind='replay'`，分析日 / pending 锚点均为 `as_of`）；
//!    serenity 链 best-effort：失败只降级批次终态，不拖垮荐股链；
//! 3. **退出 guard 后**再做四档反思（铁律 B3）：guard 内「今天」= as_of 会导致
//!    2/5/28/90 交易日档全部判未到期；
//! 4. 到期档（hindsight ≤ 北京今日）立即反思；未到期档留 pending，由既有
//!    18:00 batch-reflection cron 自然补做。
//!
//! 互斥：AppState 上的全局批次锁 + live 流水线时间窗拒绝
//! （23:50-02:10 / 17:50-18:10，北京时间）——as_of 栈是进程级的，
//! 批次既不能与 live 流水线并发，也不能与另一个批次并发。

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};

use axagent_agent_macro::agent_command;
use axagent_astock_data::AStockClient;
use axagent_astock_data::as_of::{self, AsOfContext, AsOfSource};
use axagent_astock_data::calendar;
use axagent_harness::{DatabaseConnection, holding_period::Period};
use axagent_search::vector_store::VectorStore;
use axagent_trajectory::TrajectoryStorage;
use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::sync::{Mutex, OwnedMutexGuard};

use super::error::ErrorResponse;
use super::error_code::time_travel as ec;
use super::pool_scan::{
    PoolCandidate, PoolCandidateResult, PoolScanOutcome, dedup_explicit_candidates,
    run_analysis_for_candidates,
};
use super::stock_workflow::{ReflectionFilter, run_batch_reflection_inner};
use crate::app_state::AppState;

// ── 批次状态（进程内单例；MVP 不建批次表，重启即清空）─────────────────────────

/// batch_id → 批次运行状态。锁用 tokio Mutex（临界区内无跨 await 持有的 std guard，
/// 且全仓禁用 std 同步锁）。
static BATCH_RUNS: LazyLock<Arc<Mutex<HashMap<String, BatchRunState>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));

// 批次终态 / 阶段常量
const STATUS_RUNNING: &str = "running";
const STATUS_COMPLETED: &str = "completed";
const STATUS_PARTIAL: &str = "partial";
const STATUS_FAILED: &str = "failed";
/// 进程重启后，上次运行中未终态的批次被收敛为此状态（不可续跑，仅留痕）
const STATUS_INTERRUPTED: &str = "interrupted";
const PHASE_ANALYSIS: &str = "analysis";
const PHASE_REFLECTION: &str = "reflection";

/// 发起时间旅行批次的请求（camelCase 与前端契约对齐）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeTravelBatchRequest {
    /// 固定锚点日 `YYYY-MM-DD`；与 `trading_days_ago` 二选一（date 优先）
    #[serde(default)]
    pub date: Option<String>,
    /// 相对北京今日回看的交易日数（90 = 90 个交易日前）
    #[serde(default)]
    pub trading_days_ago: Option<i32>,
    /// 参与的周期档名（`ultra_short`/`short`/`mid`/`long`，兼容 `ultrashort`）；
    /// 空 = 全四档
    #[serde(default)]
    pub periods: Vec<String>,
    /// 荐股最低置信度（0-100）
    #[serde(default = "default_min_confidence")]
    pub min_confidence: u8,
    /// 每档 top-N（0 = 荐股侧不限）
    #[serde(default = "default_top_n")]
    pub top_n: usize,
    /// 是否在 guard 外立即反思已到期档（false = 只分析，反思全部留给 cron）
    #[serde(default = "default_true")]
    pub reflect: bool,
    /// 是否执行**逐只 replay 分析**（false = 只跑选股链，把候选落库后即结束）。
    ///
    /// ## 为什么需要这个开关（R-2，2026-10-25）
    ///
    /// 逐只分析要跑 LLM 工作流，实测单日 30~60 分钟，而**根因不是本地计算慢，
    /// 是 LLM 供应商限速**（AgnesAI 免费档 429，节点重试 + fallback 全被拒）。
    /// 与此同时，**盈利能力样本并不依赖分析阶段** ——
    /// 那条路是 `reco_picks → decision_validations`，只用 K 线算收益；
    /// 分析阶段喂的是 `stock_analyses.outcome`（决策质量口径）。
    ///
    /// ⇒ 「每日 +1 个回放日」在开着分析时**不可行**（限速 + 撞锁），
    /// 关掉分析后可行（选股阶段实测 < 1 分钟）。
    ///
    /// 默认 **true**（保持既有语义不变）；CLI / cron 按需显式传 false。
    #[serde(default = "default_true")]
    pub analyze: bool,
}

fn default_min_confidence() -> u8 {
    50
}
fn default_top_n() -> usize {
    20
}
fn default_true() -> bool {
    true
}

/// 批次运行状态（序列化给前端轮询；同时可从历史表 JSON 反序列化还原）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchRunState {
    pub batch_id: String,
    pub as_of: String,
    pub trading_days_ago: Option<i32>,
    /// running / completed / partial / failed / interrupted
    pub status: String,
    /// analysis / reflection
    pub phase: String,
    /// 人话补充（如「当日荐股无候选」）
    pub message: Option<String>,
    pub total: usize,
    pub ok: usize,
    pub fail: usize,
    /// 智能荐股链产出的候选行数（合并去重前；四档各一排）
    pub reco_picks: usize,
    /// 趋势智选链产出的逐档候选行数（合并去重前；mid/long 两档展开）
    pub serenity_picks: usize,
    /// 趋势智选工作流失败原因（best-effort：荐股链成功时批次仍继续，终态降级 partial）
    pub serenity_error: Option<String>,
    pub analyses: Vec<PoolCandidateResult>,
    pub horizons: Vec<HorizonSummary>,
    /// batch_reflection_inner 返回的原始 JSON 摘要（调试/明细用）
    pub reflection_summary: Option<String>,
    pub error_code: Option<String>,
    pub error_detail: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    /// 发起请求快照：仅用于落库 request_json，不向前端输出、从库还原时为 None
    #[serde(skip)]
    pub request: Option<TimeTravelBatchRequest>,
}

/// 单个复盘档的批次级汇总。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HorizonSummary {
    /// ultra_short / short / mid / long
    pub period: String,
    pub holding_days: u32,
    /// 该档名义 hindsight（交易日外推；同档各行同日）
    pub hindsight_date: String,
    /// hindsight 是否已到期（≤ 北京今日）
    pub due: bool,
    pub total: usize,
    pub completed: usize,
    pub pending: usize,
    pub failed: usize,
}

impl BatchRunState {
    fn new(
        batch_id: String,
        as_of: String,
        trading_days_ago: Option<i32>,
        request: TimeTravelBatchRequest,
    ) -> Self {
        let now = now_ms();
        Self {
            batch_id,
            as_of,
            trading_days_ago,
            status: STATUS_RUNNING.to_string(),
            phase: PHASE_ANALYSIS.to_string(),
            message: None,
            total: 0,
            ok: 0,
            fail: 0,
            reco_picks: 0,
            serenity_picks: 0,
            serenity_error: None,
            analyses: Vec::new(),
            horizons: Vec::new(),
            reflection_summary: None,
            error_code: None,
            error_detail: None,
            created_at_ms: now,
            updated_at_ms: now,
            finished_at_ms: None,
            request: Some(request),
        }
    }
}

/// 运行状态 → 历史表 upsert 载体。
///
/// `request` / analyses / horizons 的 JSON 序列化失败按 None 落库（与原实现同口径：
/// 落库失败/缺明细不阻断批次流水线，内存态仍是权威）。
fn batch_record_from_state(
    s: &BatchRunState,
) -> axagent_dao::repo::time_travel_batch_runs::BatchRunRecord {
    axagent_dao::repo::time_travel_batch_runs::BatchRunRecord {
        id: s.batch_id.clone(),
        as_date: s.as_of.clone(),
        trading_days_ago: s.trading_days_ago,
        status: s.status.clone(),
        phase: s.phase.clone(),
        message: s.message.clone(),
        total: s.total as i32,
        ok: s.ok as i32,
        fail: s.fail as i32,
        reco_picks: s.reco_picks as i32,
        serenity_picks: s.serenity_picks as i32,
        serenity_error: s.serenity_error.clone(),
        request_json: s.request.as_ref().and_then(|r| serde_json::to_string(r).ok()),
        analyses_json: serde_json::to_string(&s.analyses).ok(),
        horizons_json: serde_json::to_string(&s.horizons).ok(),
        reflection_summary: s.reflection_summary.clone(),
        error_code: s.error_code.clone(),
        error_detail: s.error_detail.clone(),
        created_at_ms: s.created_at_ms,
        updated_at_ms: s.updated_at_ms,
        finished_at_ms: s.finished_at_ms,
    }
}

/// 更新内存态并尽力同步落库（落库失败只告警，不阻断批次流水线；
/// 内存态是运行轮询的权威，库是重启后的历史权威）。
async fn save_state(db: &DatabaseConnection, mut s: BatchRunState) {
    s.updated_at_ms = now_ms();
    // 落库逻辑下沉 dao（分层护栏：命令层不得直连 sea_orm / entities）
    let record = batch_record_from_state(&s);
    if let Err(e) = axagent_dao::repo::time_travel_batch_runs::upsert_batch_run(db, &record).await {
        tracing::warn!("[time-travel-batch] 批次 {} 状态落库失败（不影响运行）: {e}", s.batch_id);
    }
    BATCH_RUNS.lock().await.insert(s.batch_id.clone(), s);
}

/// 进程首次访问时收敛上次进程遗留的 running 批次。
///
/// as_of 封闭世界栈是进程级的（见模块头），重启后旧批次不可能续跑，
/// 库内 `running` 只会是异常退出的僵尸。本函数每进程只执行一次：
/// 首次成功发起批次之前必先经过这里（run 命令先 reap 再 insert），
/// 因此不会误伤当前进程的存活批次。
static STALE_REAP_DONE: AtomicBool = AtomicBool::new(false);

async fn ensure_stale_reaped(db: &DatabaseConnection) {
    if STALE_REAP_DONE.swap(true, Ordering::SeqCst) {
        return;
    }
    // 收敛 SQL 下沉 dao（分层护栏：命令层不得直连 sea_orm / entity）；
    // 状态值与中断说明仍由命令层给（状态语义属于流水线，不属于 repo）。
    let result = axagent_dao::repo::time_travel_batch_runs::mark_stale_batches(
        db,
        now_ms(),
        STATUS_RUNNING,
        STATUS_INTERRUPTED,
        "应用重启：批次运行中进程退出，as-of 封闭世界无法跨进程续跑",
    )
    .await;
    match result {
        Ok(0) => {},
        Ok(n) => tracing::info!(
            "[time-travel-batch] 启动收敛：{n} 个遗留 running 批次标记为 interrupted"
        ),
        Err(e) => tracing::warn!("[time-travel-batch] 遗留批次收敛失败: {e}"),
    }
}

/// 历史表行还原为运行状态。`summary_only=true` 时不反序列化大 JSON 明细
/// （列表场景；analyses/horizons/reflection_summary 留空，计数列仍完整）。
fn state_from_model(
    m: axagent_dao::repo::time_travel_batch_runs::BatchRunModel,
    summary_only: bool,
) -> BatchRunState {
    let (analyses, horizons, reflection_summary) = if summary_only {
        (Vec::new(), Vec::new(), None)
    } else {
        let analyses = m
            .analyses_json
            .and_then(|j| serde_json::from_str::<Vec<PoolCandidateResult>>(&j).ok())
            .unwrap_or_default();
        let horizons = m
            .horizons_json
            .and_then(|j| serde_json::from_str::<Vec<HorizonSummary>>(&j).ok())
            .unwrap_or_default();
        (analyses, horizons, m.reflection_summary)
    };
    BatchRunState {
        batch_id: m.id,
        as_of: m.as_date,
        trading_days_ago: m.trading_days_ago,
        status: m.status,
        phase: m.phase,
        message: m.message,
        total: m.total.max(0) as usize,
        ok: m.ok.max(0) as usize,
        fail: m.fail.max(0) as usize,
        reco_picks: m.reco_picks.max(0) as usize,
        serenity_picks: m.serenity_picks.max(0) as usize,
        serenity_error: m.serenity_error,
        analyses,
        horizons,
        reflection_summary,
        error_code: m.error_code,
        error_detail: m.error_detail,
        created_at_ms: m.created_at_ms,
        updated_at_ms: m.updated_at_ms,
        finished_at_ms: m.finished_at_ms,
        request: None,
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// 北京当前日期（UTC+8 固定偏移，与 reflection/calendar 既有口径一致）。
fn beijing_today() -> chrono::NaiveDate {
    (chrono::Utc::now() + chrono::Duration::hours(8)).date_naive()
}

/// 是否落在 live 流水线时间窗（23:50-02:10 / 17:50-18:10，北京时间）。
///
/// 边界与四个 live cron 的启动窗对齐：窗内启动批次，全局 as_of 栈可能串扰正在跑的
/// live 任务（荐股 / pool-scan / batch-reflection），故一律拒绝。
///
/// `pub(crate)`：漏检补分析批次（`mover_recall_analysis`）共用同一 as-of 进程栈，
/// 必须共用同一时间窗判据（禁区 12：判据单点，不得各抄一份）。
pub(crate) fn in_live_window() -> bool {
    let bj = chrono::Utc::now() + chrono::Duration::hours(8);
    let hm = bj.format("%H:%M").to_string();
    let (h, m): (u32, u32) = {
        let mut it = hm.split(':').map(|x| x.parse::<u32>().unwrap_or(0));
        (it.next().unwrap_or(0), it.next().unwrap_or(0))
    };
    let mins = h * 60 + m;
    // 23:50-24:00 或 00:00-02:10
    mins >= 23 * 60 + 50 || mins <= 2 * 60 + 10 || (17 * 60 + 50..=18 * 60 + 10).contains(&mins)
}

/// 进 guard 前的前置校验：该 as-of 日是否有建池两源的本地快照。
///
/// 为什么必须显式失败而不是让它跑出零候选：热股榜与行业榜都申报 `NoHistoricalSemantic`，
/// 它们在回放里的唯一历史正门是本地每日快照（live 采集累积而来，**早于累积起点的日期
/// 物理上不可回补**）。缺快照 ⇒ 建池得到空池 ⇒ 整批候选为 0，而批次只在终态留一句
/// 「均无候选（可能当日数据缺失或阈值过高）」——那把「这个日期根本不可回放」和
/// 「阈值太高所以没选出来」挤进了同一句（AGENTS.md「结构性缺口不得在 UI 造成歧义」）。
///
/// 现网实证：历史 5 次批次全取 2026-05-27，而热股榜快照最早只到 2026-09-30。
///
/// ## 判据：两源**都缺** ⇒ 拒绝；缺**一**源 ⇒ 放行但标注降级（2026-10-26 改）
///
/// 旧实现要求两源都有快照才放行 ⇒ 2026-05-27 / 06-10 / 06-11 / 08-18 / 09-04 / 09-07 /
/// 09-09 这些日期**只有 `get_industry_ranking` 一源**（实测快照表），全部不可回放。
///
/// 而「缺一源」并不等于「池子空」：
/// ① 缺的那源在 as-of 下**仍会走 vendor 的历史通道**（如同花顺热榜回溯），能取到就取，
///    取不到返回**空榜**（`Ok(vec![])`，不是失败）；
/// ② 建池是 `get_hot_stocks(30) ∪ get_industry_ranking 龙头(20) ∪ FALLBACK_STOCKS`，
///    **FALLBACK 是静态大盘股列表**（茅台/五粮液/宁德时代…，与日期无关 ⇒ **无前视偏差**），
///    所以少一源时池子仍有真实成分，不是空池。
///
/// 为什么必须放宽：**长档（T+90）的成熟判定只看「行情库里决策日之后有没有 90 根 K 线」，
/// 不看今天**（`compute_market_snapshots` 取最近 500 根，`exit_idx = entry_idx + expected_days`）。
/// 实测行情库有 6575 个交易日，2026-06-10 之后**恰好 90 根** ⇒ 回放 06-10 立刻能让
/// long 档成熟并产出校准样本；09-07 之后恰好 28 根 ⇒ mid 档同理。
/// 不放宽 ⇒ long/mid 档的校准样本只能等 wall-clock（long 需 T+90 ⇒ 2027-01），
/// 而 `CALIBRATION_MIN_SAMPLES = 30` 的门槛又不允许降 ⇒ 长期档**永远校准不了**
/// ⇒ 置信被永久封顶 ⇒ 顶层「买入」永远为 0（R-3h 死循环）。
///
/// ⚠ 两源**都**缺时仍拒绝：那时池子 100% 是 FALLBACK_STOCKS ⇒ 回放不再是「当时的候选池」，
/// 而「按固定名单选股」——与系统真实行为不同构，混进来会污染校准样本。
///
/// ## ⚠⚠ 代价必须如实说明（2026-10-26 实测，不可当副作用略过）
///
/// 实测「快照里有没有**有效成分**」（`get_industry_ranking` 看 `leaderCode`、
/// `get_hot_stocks` 看 `stockCode`）：
///
/// | 快照日 | 源 | 行数 | **有效成分** |
/// |---|---|---:|---:|
/// | 05-27 ~ 09-24（8 个日） | industry | 18~31 | **0** |
/// | **09-28** | hot_stocks | 19 | **19** |
/// | 09-30 起 | 两个源 | 20~100 | 全量 |
///
/// ⇒ **09-28 之前的快照虽存在，但一个股票成分都没有**（只有行业名 + 涨跌）。
/// ⇒ 所以「放宽一源」**只**让这些日期**能跑**（回放不再被拒），
/// **不等于**它们的池是「当时的候选池」——那些日期的池 **100% 是静态 FALLBACK_STOCKS**。
///
/// **这个取舍是可接受的，因为两件事被刻意分开**：
/// · **档位成熟判定**（long/mid 能不能有 T+90/T+28 的成熟样本）⇒ **只看 K 线，与池成分无关**
///   ⇒ 放宽**确实**解锁了 long/mid 的校准样本（这正是 R-3h 要的）；
/// · **选股能力**（池里的票是否代表当时的候选）⇒ 那部分**仍然不成立**，
///   所以这些日期的样本**不得**被当作「系统选股能力」的证据，只用于「档位动作 → 胜率」映射。
///
/// ⇒ 批次消息里的降级说明必须把这句话说清楚（见下方 `pool_degradation_note`）。
async fn require_pool_snapshot_coverage(
    db: &DatabaseConnection,
    as_of: &chrono::NaiveDate,
) -> Result<Option<String>, String> {
    use axagent_analysis_engine::recommender::pool::POOL_SNAPSHOT_METHODS;

    let as_of_str = as_of.format("%Y-%m-%d").to_string();
    // 查的就是建池真实取的那两源 —— 清单由 pool.rs 单点导出，这里不另抄一份
    let missing = axagent_dao::repo::astock_daily_snapshot::missing_snapshot_sources(
        db,
        &as_of_str,
        &POOL_SNAPSHOT_METHODS,
    )
    .await
    .map_err(|e| format!("读 astock_daily_snapshot 覆盖失败: {e}"))?;
    if missing.is_empty() {
        return Ok(None);
    }

    // 两源都缺 ⇒ 池子只能是静态 FALLBACK_STOCKS ⇒ 该日不是「当时的候选池」⇒ 拒绝。
    if missing.len() >= POOL_SNAPSHOT_METHODS.len() {
        // 给出「最早能回放的那一天」，拒绝才具备可操作性
        let earliest = axagent_dao::repo::astock_daily_snapshot::earliest_snapshot_date(
            db,
            POOL_SNAPSHOT_METHODS[0],
        )
        .await
        .map_err(|e| format!("读最早快照日期失败: {e}"))?;
        let hint = match earliest.as_deref() {
            Some(d) => format!("本地热股榜快照最早只到 {d}，请把批次日期改到 {d} 或之后"),
            None => "本地还没有任何热股榜快照（每日采集尚未跑过），当前无日期可回放".to_string(),
        };
        return Err(format!(
            "{as_of_str} 缺**全部**建池快照（{}）⇒ 该日候选池只能由静态兜底名单构成，\
             不是当时的候选池，不予回放。热股榜/行业榜无历史通道，{hint}；\
             这不是阈值问题，改阈值不会让候选变出来",
            missing.join(", ")
        ));
    }

    // 缺一源 ⇒ 放行，但把降级事实**说清楚**（不静默）：该源会走 vendor 历史通道，
    // 取不到就是空榜；池子由另一源 + 静态兜底名单组成。
    //
    // ⚠ 措辞必须写明「**该日样本不得当作选股能力的证据**」——
    //   实测 09-28 之前的 industry 快照 `leaderCode` **全为 null**（只有行业名+涨跌）
    //   ⇒ 那些日期的「另一源」也取不到股票 ⇒ 池 100% 是静态兜底名单。
    //   放宽买到的只是「**能跑**」，不是「池是当时的候选池」。
    Ok(Some(format!(
        "{as_of_str} 缺建池快照之一（{}）⇒ 该源在回放中为空榜（会先尝试 vendor 历史通道），\
         候选池由另一源 + 静态兜底名单构成；⚠ 该日样本只用于「档位动作→胜率」映射，\
         **不得**当作选股能力的证据（历史快照的成分字段可能为空）",
        missing.join(", ")
    )))
}

// ── Tauri 命令 ────────────────────────────────────────────────────────────────

/// 时间旅行批次定时任务的 `task_type` 标识 —— 权威值在 `commands::constants`
/// （`scheduled_task` 的「立即执行」白名单、`services.rs` 的 executor 分支与
/// 播种三处共读一份，留在这里会逼出横向互调，撞分层护栏规则 2）。
///
/// 与 `DECISION_BACKTEST_TASK_TYPE` 同形：调度层按 task_type 分发到实现，
/// **cron 不调命令**（命令要 `State<'_, AppState>`，cron 拿不到）。
pub use crate::commands::constants::TIME_TRAVEL_BATCH_TASK_TYPE;

/// 列出「今天可回放」的 as_of 日：建池快照**至少一源**齐全 **且** 至少已过 `min_holding_days` 个交易日。
///
/// ## 为什么要「至少过 N 个交易日」而不是「快照齐全即可」
///
/// 回放的产物要能被判定才有价值，而判定 T+N 的前提是 **as_of + N 个交易日 ≤ 今天**。
/// 快照齐全但持有期未到期的日子，跑出来只会得到一堆判不出胜负的样本 ——
/// 那种样本进不了胜率分母，只是白跑一遍选股与分析。
///
/// ## 返回
///
/// `[(as_of, 距今交易日数)]`，按日期升序。距今交易日数 ≤ 0 的日期（今天本身）
/// 会被过滤掉（没有任何持有期可判）。
pub async fn list_replayable_as_of_dates(
    db: &DatabaseConnection,
    min_holding_days: i32,
) -> Result<Vec<(String, i32)>, String> {
    use axagent_analysis_engine::recommender::pool::POOL_SNAPSHOT_METHODS;

    let today = beijing_today();
    // ⚠ 用「**至少一**源有快照」而不是「全部源」（2026-10-26，R-3h）。
    // 实测 `astock_daily_snapshot`：05-27 / 06-10 / 06-11 / 08-18 / 09-04 / 09-07 / 09-09
    // 这些日期**只有 `get_industry_ranking` 一源**（热股榜快照最早到 09-28），
    // 用「全部源」判会把它们全部排除 ⇒ 只能从 09-28 起回放。
    //
    // 而档位成熟判定只看「行情库里决策日之后有没有 N 根 K 线」，**不看今天**
    // ⇒ 回放 06-10 立刻能让 long 档（T+90）成熟、09-07 能让 mid 档（T+28）成熟
    // ⇒ long/mid 的校准样本不必等 wall-clock（详见 `list_snapshot_dates_with_any_method` 文档）。
    //
    // ⚠ 零源日（两源都没快照）**仍排除**：那时池子 100% 是静态 FALLBACK_STOCKS，
    //   不是「当时的候选池」⇒ 回放与系统真实行为不同构，混进来会污染校准样本。
    let covered_any =
        axagent_dao::repo::astock_daily_snapshot::list_snapshot_dates_with_any_method(
            db,
            &POOL_SNAPSHOT_METHODS,
        )
        .await
        .map_err(|e| {
            ErrorResponse::err_with_detail(ec::QUERY_FAILED, format!("读快照覆盖失败: {e}"))
        })?;

    let mut out = Vec::new();
    for (d, _srcs) in covered_any {
        let Ok(nd) = chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d") else {
            tracing::warn!("[time-travel-batch] 快照日期 `{d}` 不是 ISO 日期，跳过");
            continue;
        };
        if !calendar::is_trading_day(&nd) {
            continue;
        }
        // 零源日排除（两源都没快照 ⇒ 池子只能是静态兜底名单）
        let missing = axagent_dao::repo::astock_daily_snapshot::missing_snapshot_sources(
            db,
            &d,
            &POOL_SNAPSHOT_METHODS,
        )
        .await
        .map_err(|e| {
            ErrorResponse::err_with_detail(ec::QUERY_FAILED, format!("读快照覆盖失败: {e}"))
        })?;
        if missing.len() >= POOL_SNAPSHOT_METHODS.len() {
            continue;
        }
        let ago = calendar::trading_days_between(nd, today);
        if ago >= min_holding_days {
            out.push((d, ago));
        }
    }
    Ok(out)
}

/// 发起一次时间旅行批次复盘；立即返回 batch_id，执行在后台进行，
/// 用 [`get_time_travel_batch`] 轮询进度与结果。
/// 发起时间旅行批次的**实现**（命令层是它的薄包装）。
///
/// 抽出来的原因与 `run_batch_reflection_inner` / `run_decision_backtest_inner` 同源：
/// **命令要 `State<'_, AppState>`，而 cron handler 与独立 CLI 都拿不到 `State`**。
/// 本函数收**各个组件**（而不是整个 `AppState`）⇒ 命令 / cron / CLI 三条调用路径
/// 共用同一实现，不会出现「命令能跑、cron 走另一份逻辑」的分叉。
///
/// 参数逐项对应 `execute_batch` 需要的依赖，外加 `batch_lock`（全局批次互斥锁）。
pub async fn run_time_travel_batch_inner(
    db: &DatabaseConnection,
    client: &Arc<AStockClient>,
    engine: &Arc<axagent_rt_workflow::work_engine::WorkEngine>,
    vector_store: &Arc<VectorStore>,
    master_key: &[u8; 32],
    trajectory_storage: &Arc<TrajectoryStorage>,
    batch_lock: &Arc<Mutex<()>>,
    request: TimeTravelBatchRequest,
) -> Result<String, String> {
    // 批次恒同时回放「智能荐股 + 趋势智选」两条平级选股链（请求体无开关）。

    // 1. 周期档解析（空 = 全四档）
    let periods: Vec<Period> = if request.periods.is_empty() {
        Period::ALL.to_vec()
    } else {
        let mut parsed = Vec::with_capacity(request.periods.len());
        for raw in &request.periods {
            match raw.parse::<Period>() {
                Ok(p) => parsed.push(p),
                Err(e) => {
                    return Err(ErrorResponse::err_with_detail(
                        ec::INVALID_PARAMS,
                        format!("周期档无法识别 {raw:?}: {e}"),
                    ));
                },
            }
        }
        parsed
    };

    // 2. as_of 锚点解析（date 优先；trading_days_ago 从北京今日按交易日回退）
    let today = beijing_today();
    let (as_of, trading_days_ago) = match (request.date.as_deref(), request.trading_days_ago) {
        (Some(d), _) => {
            let nd = chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").map_err(|e| {
                ErrorResponse::err_with_detail(ec::INVALID_PARAMS, format!("date 解析失败: {e}"))
            })?;
            // 回显时估算交易日距离（失败不阻断）
            let ago = calendar::trading_days_between(nd, today);
            (nd, Some(ago.max(0)))
        },
        (None, Some(n)) if n > 0 => (calendar::nth_trading_day(today, -n), Some(n)),
        (None, Some(_)) => {
            return Err(ErrorResponse::err_with_detail(
                ec::INVALID_PARAMS,
                "tradingDaysAgo 必须为正整数",
            ));
        },
        (None, None) => {
            return Err(ErrorResponse::err_with_detail(
                ec::INVALID_PARAMS,
                "必须提供 date 或 tradingDaysAgo 之一",
            ));
        },
    };

    // 锚点必须是交易日（周末 / 假日不能作为回放交易日）
    if !calendar::is_trading_day(&as_of) {
        return Err(ErrorResponse::err_with_detail(
            ec::ASOF_INVALID,
            format!("{as_of} 不是 A 股交易日，请选择最近的交易日"),
        ));
    }
    // AsOfContext::new 内含「不得晚于今天」的契约校验
    let ctx = AsOfContext::new(as_of, AsOfSource::UserReplay)
        .map_err(|e| ErrorResponse::err_with_detail(ec::ASOF_INVALID, e.to_string()))?;

    // 建池快照覆盖校验（先于 live 窗判断与抢锁）：不可回放的日期当场拒绝并给出可用日期，
    // 不能放批次进去跑完只留一句含糊的「均无候选（可能当日数据缺失或阈值过高）」。
    // ⚠ 返回 `Option<String>` = 降级说明（缺一源时非空）⇒ 必须**往下传**并落进批次消息，
    //   否则「池子少了一个真实榜源」这件事在界面上完全不可见（结构性缺口不得静默）。
    let pool_degradation_note = require_pool_snapshot_coverage(db, &as_of)
        .await
        .map_err(|e| ErrorResponse::err_with_detail(ec::ASOF_INVALID, e))?;
    if let Some(note) = pool_degradation_note.as_deref() {
        tracing::warn!("[time-travel-batch] {note}");
    }

    // 3. live 时间窗拒绝（先于抢锁，给出可操作的拒绝原因）
    if in_live_window() {
        return Err(ErrorResponse::err_with_detail(
            ec::LIVE_WINDOW,
            "当前处于 live 流水线时间窗（23:50-02:10 / 17:50-18:10），请避开后再发起时间旅行批次",
        ));
    }

    // 4. 全局批次互斥（try_lock：已在跑直接拒绝，不排队）
    let permit =
        batch_lock.clone().try_lock_owned().map_err(|_| ErrorResponse::err(ec::ALREADY_RUNNING))?;

    let db = db.clone();
    // 新进程首次发起批次前，收敛上次进程异常退出遗留的 running 行
    // （必须在本行 insert 之前，避免把本批次误判为僵尸）。
    ensure_stale_reaped(&db).await;

    // 5. 建批次状态并后台执行（内存 + 历史表双写）
    let batch_id = uuid::Uuid::new_v4().to_string();
    save_state(
        &db,
        BatchRunState::new(
            batch_id.clone(),
            as_of.format("%Y-%m-%d").to_string(),
            trading_days_ago,
            request.clone(),
        ),
    )
    .await;
    // spawn 会 move batch_id，返回给调用方的 id 单独留一份
    let batch_id_to_return = batch_id.clone();

    let client = client.clone();
    let engine = engine.clone();
    let vector_store = vector_store.clone();
    let master_key = *master_key;
    let trajectory_storage = trajectory_storage.clone();
    let reflect = request.reflect;
    let analyze = request.analyze;
    let min_confidence = request.min_confidence;
    let top_n = request.top_n;
    // ⚠ 不能用 tauri::async_runtime::spawn：Tauri 全局 runtime 的 worker 栈只有默认 2MB，
    // debug 构建下本批次 future（荐股扫描 → serenity 回放 → 逐只工作流分析 → 多路
    // agent/LLM 流）轮询到深层时直接栈溢出（STATUS_ACCESS_VIOLATION，
    // 2026-10-12 实测三复现）。走独立多线程 runtime + 32MB 大栈
    // （必须 multi-thread：引擎/工具链路含 block_in_place，current-thread runtime 会 panic）。
    const BATCH_STACK: usize = 32 * 1024 * 1024;
    crate::spawn_fire_and_forget_multithread(
        "time_travel_batch",
        BATCH_STACK,
        4,
        BATCH_STACK,
        async move {
            execute_batch(
                batch_id,
                ctx,
                periods,
                min_confidence,
                top_n,
                reflect,
                analyze,
                db,
                client,
                engine,
                vector_store,
                master_key,
                trajectory_storage,
                permit,
                pool_degradation_note,
            )
            .await;
            tracing::info!("[time-travel-batch] 批次结束");
        },
    );

    Ok(batch_id_to_return)
}

/// 发起一次时间旅行批次复盘；立即返回 batch_id，执行在后台进行，
/// 用 [`get_time_travel_batch`] 轮询进度与结果。
#[agent_command(domain = time_travel, safety = Caution, call_mode = StateInput, description = "发起时间旅行批次复盘")]
#[tauri::command]
pub async fn run_time_travel_batch(
    state: State<'_, AppState>,
    request: TimeTravelBatchRequest,
) -> Result<String, String> {
    let master_key = state.harness.master_key_owned();
    run_time_travel_batch_inner(
        state.harness.db(),
        &state.astock_client,
        &state.work_engine,
        &state.vector_store,
        &master_key,
        &state.trajectory_storage,
        &state.time_travel_batch_lock,
        request,
    )
    .await
}

/// 查询批次运行状态：优先内存运行态；重启后内存已失则从历史表还原完整明细。
/// 批次不存在返回 null。
#[agent_command(domain = time_travel, safety = Safe, call_mode = StateInput, description = "查询时间旅行批次状态")]
#[tauri::command]
pub async fn get_time_travel_batch(
    state: State<'_, AppState>,
    batch_id: String,
) -> Result<Option<BatchRunState>, String> {
    let db = state.harness.db();
    ensure_stale_reaped(db).await;

    if let Some(s) = BATCH_RUNS.lock().await.get(&batch_id).cloned() {
        return Ok(Some(s));
    }

    let model =
        axagent_dao::repo::time_travel_batch_runs::fetch_batch_by_id(db, &batch_id).await.map_err(
            |e| ErrorResponse::err_with_detail(ec::QUERY_FAILED, format!("查询批次历史失败: {e}")),
        )?;
    Ok(model.map(|m| state_from_model(m, false)))
}

/// 查询批次复盘历史（任务中心用）：按创建时间倒序，默认 50 条；
/// 摘要不含 analyses / horizons 明细，明细走 [`get_time_travel_batch`]。
#[agent_command(domain = time_travel, safety = Safe, call_mode = StateInput, description = "查询时间旅行批次历史")]
#[tauri::command]
pub async fn list_time_travel_batches(
    state: State<'_, AppState>,
    limit: Option<u64>,
    status_filter: Option<String>,
    as_of: Option<String>,
) -> Result<Vec<BatchRunState>, String> {
    let db = state.harness.db();
    ensure_stale_reaped(db).await;

    // 查询下沉 dao（分层护栏：命令层不得直连 sea_orm / entity）
    let rows = axagent_dao::repo::time_travel_batch_runs::list_recent_batch_runs(
        db,
        limit.unwrap_or(50).min(200),
        status_filter.as_deref(),
        as_of.as_deref(),
    )
    .await
    .map_err(|e| {
        ErrorResponse::err_with_detail(ec::QUERY_FAILED, format!("查询批次历史失败: {e}"))
    })?;

    Ok(rows.into_iter().map(|m| state_from_model(m, true)).collect())
}

// ── 后台执行体 ────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn execute_batch(
    batch_id: String,
    ctx: AsOfContext,
    periods: Vec<Period>,
    min_confidence: u8,
    top_n: usize,
    reflect: bool,
    // 是否执行逐只 replay 分析（R-2：关闭后只落选股产物，见 `TimeTravelBatchRequest::analyze`）
    analyze: bool,
    db: DatabaseConnection,
    client: Arc<AStockClient>,
    engine: Arc<axagent_rt_workflow::work_engine::WorkEngine>,
    vector_store: Arc<VectorStore>,
    master_key: [u8; 32],
    trajectory_storage: Arc<TrajectoryStorage>,
    // 仅用于把全局批次锁持有到批次结束：drop 即释放。
    _permit: OwnedMutexGuard<()>,
    // 建池降级说明（缺一源快照时非空）⇒ 落进批次消息，不静默。
    pool_degradation_note: Option<String>,
) {
    let as_of_str = ctx.as_of_date.format("%Y-%m-%d").to_string();
    // run 命令在 spawn 前已把初始态写入内存；缺失说明状态异常，不再兜底构造
    // （new 需要完整请求参数），直接放弃本轮后台执行。
    let mut s = match BATCH_RUNS.lock().await.get(&batch_id).cloned() {
        Some(s) => s,
        None => {
            tracing::error!(
                "[time-travel-batch] {batch_id} 内存运行态缺失，放弃后台执行（历史表保留 interrupted 由下次启动收敛）"
            );
            return;
        },
    };

    // ── 阶段 1-3：guard 内两链统一回放 → 合并去重 → 逐只 replay 分析 ───────────
    // 整个 pipeline future 在 with_optional_asof 内执行；future 返回即 guard drop，
    // 之后阶段 4 的反思天然落在 guard 外（铁律 B3）。
    let pipeline = async {
        // 链 A：内存荐股（与荐股 cron 共用模板变量加载，缺陷 D2：两条入口必须同一套权重/阈值）。
        let vars = match axagent_analysis_engine::recommender::reco_loop::load_reco_served_vars_db(
            &db,
        )
        .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    "[time-travel-batch] {as_of_str} 读取荐股模板变量失败 ({e})，按硬编码默认参数执行"
                );
                Vec::new()
            },
        };
        let prior = crate::commands::stock_analysis::reco_horizon_prior(&db, &vars).await;
        let scan = axagent_analysis_engine::recommender::run_recommendation_scan(
            client.clone(),
            &periods,
            &vars,
            min_confidence,
            top_n,
            prior,
        )
        .await;

        // 链 A 落库（铁律 B2 于 2026-10-12 改写：回放候选必须持久化才有样本可供反思）：
        // 写同一张 reco_picks，每行带 `as_of_date` = 本批截止日 ⇒ 与实盘同表但不混码。
        // 命令层那条 `persist_reco_response_picks` 在这里走不到（批次直连引擎的
        // `run_recommendation_scan`），所以按荐股 cron 分支的同形态在这里落一次。
        //
        // ⚠ **落库用 `picks_by_period` 而不是 `picks`**（R-3d，2026-10-26）：
        // `scan.picks` 是**通知口径** —— 四档合并后按 stock_code 去重、再 top-N
        // ⇒ 同一只票在多档命中时只留 confidence 最高的那一档 ⇒ **档位被压平成一档**。
        // 实测：回放批次落库 230 行只覆盖 58 只票，`period` **全是 `short`**，
        // 其余三档的票全被去重吃掉 ⇒ 反思按档分档、命中率按档统计、
        // `decision_validations` 的 `period ↔ t_plus_n` 匹配**全部失真**。
        //
        // 落库口径要的是「各档各自判了什么」⇒ 用按档结果（不跨档去重、不截断）。
        // ⚠ 通知口径（`scan.picks`）**不受影响** —— 那是给用户看的「这几只票」，
        // 同一只票推四遍是噪声。
        let reco_generated_at = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3f").to_string();
        let reco_strategy_weights_json = vars
            .iter()
            .find(|(k, _)| k == "reco_strategy_weights")
            .and_then(|(_, v)| v.is_object().then(|| v.to_string()));
        // 按档展平：同一只票在不同档可以各落一行（`period` 列即档位标记）。
        let reco_picks_by_period: Vec<axagent_analysis_engine::recommender::types::RecoPick> =
            scan.picks_by_period.iter().flat_map(|(_, ps)| ps.iter().cloned()).collect();
        let reco_written = crate::commands::_shared_reco_pool::persist_reco_picks(
            &db,
            &reco_picks_by_period,
            scan.seed_pool_snapshot.clone(),
            reco_strategy_weights_json,
            &reco_generated_at,
            Some(&as_of_str),
            crate::commands::_shared_reco_pool::purpose::TIME_TRAVEL_REPLAY,
        )
        .await;
        tracing::info!(
            "[time-travel-batch] {as_of_str} 链 A 落库 reco_picks: {reco_written}/{} 行\
            （按档：{} 档，as_of_date={as_of_str}）",
            reco_picks_by_period.len(),
            scan.picks_by_period.len(),
        );
        // 批次级池快照（含 0 票回放批次）：与 picks 同代际，供漏检归因判「池子空/没扫过」。
        crate::commands::_shared_reco_pool::persist_reco_pool_batch(
            &db,
            "reco",
            "all",
            scan.seed_pool_snapshot.as_deref(),
            &reco_generated_at,
            Some(&as_of_str),
            crate::commands::_shared_reco_pool::purpose::TIME_TRAVEL_REPLAY,
        )
        .await;
        // L3 截断留痕逐档落库（同样带代际）。缺这一段会让回放的漏检归因整批落
        // `Unexplained`——与实盘不同口径，正是 `reco_scan_audit` 文档要求避免的形态。
        for (audit_period, trimmed) in &scan.period_audit {
            let audit_written = crate::commands::_shared_reco_pool::persist_reco_scan_audit(
                &db,
                trimmed,
                *audit_period,
                &reco_generated_at,
                Some(&as_of_str),
                crate::commands::_shared_reco_pool::purpose::TIME_TRAVEL_REPLAY,
            )
            .await;
            tracing::info!(
                "[time-travel-batch] {as_of_str} 链 A 落库 scan_audit: {audit_written}/{} 行\
                （档 {}，as_of_date={as_of_str}）",
                trimmed.len(),
                audit_period.as_str()
            );
        }

        // picks 同时映射为内存候选队列，供下方与链 B 合并去重后逐只 replay 分析。
        let reco_candidates: Vec<PoolCandidate> = scan
            .picks
            .iter()
            .map(|p| PoolCandidate {
                stock_code: p.stock_code.clone(),
                stock_name: p.stock_name.clone(),
                period: p.period,
                confidence: p.confidence as i32,
                anchor_event_id: None,
            })
            .collect();
        let reco_count = reco_candidates.len();
        tracing::info!(
            "[time-travel-batch] {as_of_str} 链A 荐股 {reco_count} 只候选（已落 reco_picks，\
             代际 as_of_date={as_of_str}）"
        );

        // 链 B：趋势智选 serenity 原工作流（静默 app=None；内部工作流节点自带同锚点
        // as-of scope，与本 guard 嵌套安全）。as_of_ctx=Some 时其落库分支把截止日写进行上的
        // `as_of_date`（代际标记），与实盘同表不混码；进程内全局种子仍不写（无代际可打）。
        // best-effort：失败不阻断批次，候选只用荐股链。
        let (serenity_candidates, serenity_error) =
            match serenity_replay(&db, &client, &engine, ctx).await {
                Ok(v) => (v, None),
                Err(e) => {
                    tracing::warn!(
                        "[time-travel-batch] {as_of_str} 链B 趋势智选回放失败，降级为仅荐股链: {e}"
                    );
                    (Vec::new(), Some(e))
                },
            };
        let serenity_count = serenity_candidates.len();
        tracing::info!(
            "[time-travel-batch] {as_of_str} 链B 趋势智选 {serenity_count} 只逐档候选（已落 \
             reco_picks，代际 as_of_date={as_of_str}）"
        );

        // 两链合并去重：同 code 多条（四档 + mid/long、两链撞车）只留置信最高一条，
        // 口径与 live pool-scan 同一个纯函数（dedup_explicit_candidates）。
        let mut merged = reco_candidates;
        merged.extend(serenity_candidates);
        let candidates = dedup_explicit_candidates(merged);
        let merged_count = candidates.len();

        // R-2（2026-10-25）：可关闭逐只分析。
        // 候选**此时已落 `reco_picks`**（上面两链各自 persist），所以关闭分析不会丢选股产物 ——
        // 而盈利能力样本走的正是 `reco_picks → decision_validations` 那条路。
        //
        // 关掉分析的实际意义：逐只分析要跑 LLM 工作流，免费档限速下实测单日 30~60 分钟，
        // 而选股阶段 < 1 分钟 ⇒ 「每日 +1 个回放日」只有关掉分析才可行。
        if !analyze {
            tracing::info!(
                "[time-travel-batch] {as_of_str} 两链去重后 {merged_count} 只候选已落库，\
                 按请求**跳过逐只 replay 分析**（盈利能力样本不依赖该阶段）"
            );
            let outcome = PoolScanOutcome {
                candidates: merged_count,
                ok: 0,
                fail: 0,
                skipped_analyzed_today: 0,
                truncated: false,
                timed_out: false,
                pending_remaining: merged_count,
                first_error: None,
                details: vec![format!(
                    "按请求跳过逐只分析（{merged_count} 只候选已落 reco_picks）"
                )],
                results: Vec::new(),
            };
            return (reco_count, serenity_count, serenity_error, outcome);
        }

        tracing::info!(
            "[time-travel-batch] {as_of_str} 两链去重后 {merged_count} 只进入逐只 replay 分析"
        );

        let outcome =
            run_analysis_for_candidates(&db, &client, &engine, candidates, "time_travel").await;
        (reco_count, serenity_count, serenity_error, outcome)
    };

    let (reco_count, serenity_count, serenity_error, outcome) =
        as_of::with_optional_asof(Some(ctx), pipeline).await;

    // 成功分析的 analysis_id 集合 —— 阶段 4 反思范围严格收口于此
    let analysis_ids: Vec<String> =
        outcome.results.iter().filter_map(|r| r.analysis_id.clone()).collect();

    s.total = outcome.candidates;
    s.ok = outcome.ok;
    s.fail = outcome.fail;
    s.reco_picks = reco_count;
    s.serenity_picks = serenity_count;
    s.serenity_error = serenity_error.clone();
    s.analyses = outcome.results;
    if reco_count == 0 && serenity_count == 0 {
        s.message =
            Some(format!("{as_of_str} 荐股与趋势智选均无候选（可能当日数据缺失或阈值过高）"));
    } else if s.total == 0 {
        s.message = Some("候选队列执行异常：0 只进入分析".to_string());
    } else {
        // 两链候选数 + 去重后入队数，让用户看到平级两链各自的产出与撞车规模。
        let mut msg = format!(
            "{as_of_str} 荐股 {reco_count} + 趋势智选 {serenity_count} 只候选，去重后 {total} 只进入分析",
            total = s.total
        );
        if serenity_error.is_some() {
            msg.push_str("；趋势智选回放失败已降级（仅计荐股链候选）");
        }
        // 建池降级必须说清楚：少了一个真实榜源 ⇒ 池子成分与「当时真实候选池」不同构。
        if let Some(note) = pool_degradation_note.as_deref() {
            msg.push('；');
            msg.push_str(note);
        }
        s.message = Some(msg);
    }
    save_state(&db, s.clone()).await;

    // ── 阶段 4：guard 外四档反思 ─────────────────────────────────────────────
    // 此时 as_of 栈已退出；run_reflection_workflow 内部会按各行 hindsight_date
    // 自行包 AsOfContext（hindsight 锚点），与本批次互不冲突。
    let mut reflection_error: Option<String> = None;
    if reflect {
        if analysis_ids.is_empty() {
            tracing::info!("[time-travel-batch] {batch_id} 无成功分析，跳过反思阶段");
        } else {
            s.phase = PHASE_REFLECTION.to_string();
            save_state(&db, s.clone()).await;

            // 反思前快照（主要为拿到各档 due/pending 分布）
            s.horizons =
                summarize_horizons(&db, &analysis_ids, beijing_today()).await.unwrap_or_default();
            save_state(&db, s.clone()).await;

            // 只反思「本批 + 已到期」的 pending 行；未到期的留给 18:00 cron。
            // 预算给到 分析数×4+8（每分析最多四档行），保证本批到期行一轮跑完。
            let due_filter = ReflectionFilter {
                period: None,
                due_only: true,
                min_confidence: None,
                depth_override: None,
            };
            let budget = (analysis_ids.len() * 4 + 8) as u32;
            match run_batch_reflection_inner(
                &db,
                &client,
                &engine,
                &vector_store,
                &master_key,
                Some(budget),
                Some(&trajectory_storage),
                Some(&due_filter),
                Some(&analysis_ids),
            )
            .await
            {
                Ok(summary) => {
                    s.reflection_summary = Some(summary.to_string());
                },
                Err(e) => {
                    tracing::warn!("[time-travel-batch] {batch_id} guard 外批次反思失败: {e}");
                    reflection_error = Some(e);
                },
            }

            // 反思后刷新各档终态计数
            s.horizons =
                summarize_horizons(&db, &analysis_ids, beijing_today()).await.unwrap_or_default();
        }
    }

    // ── 终态汇总 ─────────────────────────────────────────────────────────────
    let any_horizon_failed = s.horizons.iter().any(|h| h.failed > 0);
    // ⚠ 「按请求跳过逐只分析」**不是失败**（R-2）。
    // 跳过时 `s.total = 候选数 > 0` 而 `s.ok = 0`（没有分析成功数）⇒ 若走下面的
    // 「全部个股分析失败」分支，会把一次**成功**的选股批次标成 `failed`（实测 12 条误判）。
    // ⇒ 跳过分析时直接判 completed，并在 message 里写明范围。
    s.status = if !analyze {
        s.message = Some(match s.message.clone() {
            Some(m) => format!("{m}（按请求跳过逐只 replay 分析：候选已落 reco_picks）"),
            None => "按请求跳过逐只 replay 分析：候选已落 reco_picks".to_string(),
        });
        STATUS_COMPLETED
    } else if s.total > 0 && s.ok == 0 {
        // 全部个股分析失败 = 批次硬失败
        s.error_code = Some(ec::EXEC_FAILED.to_string());
        s.error_detail = Some("全部候选分析失败".to_string());
        STATUS_FAILED
    } else if s.fail > 0
        || any_horizon_failed
        || reflection_error.is_some()
        || serenity_error.is_some()
    {
        // 趋势智选链失败 / 部分个股分析失败 / 反思失败均为降级成功（partial）
        if let Some(e) = reflection_error {
            s.error_detail = Some(format!("批次反思失败: {e}"));
        } else if let Some(e) = serenity_error {
            // 仅 serenity 链失败时给出详情；反思失败优先展示反思详情
            s.error_detail = Some(format!("趋势智选回放失败（荐股链已正常完成）: {e}"));
        }
        STATUS_PARTIAL
    } else {
        STATUS_COMPLETED
    }
    .to_string();
    s.finished_at_ms = Some(now_ms());
    save_state(&db, s).await;
}

/// 批次专用：以静默方式（无 AppHandle、不发 IPC 事件）按 as-of 锚点回放一轮
/// 趋势智选工作流，并把逐档候选映射为批次候选队列。
///
/// 复用 `run_serenity_screening_inner`（与手动 `run_serenity_screening` 命令同一实现）：
/// inner 内部工作流节点自带同锚点 as-of scope（与批次外层 guard 嵌套安全），
/// `as_of_ctx=Some` 时候选**照落** `reco_picks`（代际 = 本批截止日，见 2026-10-12 裁定），
/// 只跳过全局 serenity 种子同步 —— 种子是进程内单槽、无代际维度，回放写它会换掉 live 的建池输入。
async fn serenity_replay(
    db: &DatabaseConnection,
    client: &AStockClient,
    engine: &Arc<axagent_rt_workflow::work_engine::WorkEngine>,
    ctx: AsOfContext,
) -> Result<Vec<PoolCandidate>, String> {
    use super::stock_workflow::serenity::{
        SERENITY_TEMPLATE_IDS, SerenityScreeningArgs, run_serenity_screening_inner,
    };
    let value = run_serenity_screening_inner(SerenityScreeningArgs {
        db,
        client,
        engine: engine.clone(),
        app: None,
        as_of_ctx: Some(ctx),
        themes: None,
        run_id: None,
        replay_purpose: crate::commands::_shared_reco_pool::purpose::TIME_TRAVEL_REPLAY,
        template_id: SERENITY_TEMPLATE_IDS[0].to_string(),
    })
    .await?;
    Ok(serenity_candidates_from_json(&value))
}

/// 把 `run_serenity_screening_inner` 返回 JSON 的 `candidates`（mid/long 逐档展开）
/// 映射为 [`PoolCandidate`]。
///
/// 注意键名是**混合形态**：候选本体来自 LLM 工作流，`stock_code`/`stock_name`/
/// `confidence` 为 snake_case；逐档覆盖字段（`period`/`winRate` 等）为 camelCase。
/// 置信度优先取 `winRate`（逐档折算胜率，与 reco_picks.confidence 同量纲，
/// 供两链合并去重比较）；period 缺失/无法识别的行跳过 —— 空展开回退分支
/// （退回无档位原始数组）不该进入逐只分析。
fn serenity_candidates_from_json(value: &serde_json::Value) -> Vec<PoolCandidate> {
    let Some(arr) = value.get("candidates").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(arr.len());
    for c in arr {
        let Some(code) =
            c.get("stock_code").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty())
        else {
            continue;
        };
        let Some(period) =
            c.get("period").and_then(|v| v.as_str()).and_then(|p| p.parse::<Period>().ok())
        else {
            tracing::warn!("[time-travel-batch] 趋势智选候选 {code} 缺周期档/档名无法识别，跳过");
            continue;
        };
        let name = c.get("stock_name").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        // winRate = 该档折算胜率（与落库 confidence 同量纲）；缺失时退回原始候选评分。
        let confidence =
            c.get("winRate").or_else(|| c.get("confidence")).and_then(|v| v.as_i64()).unwrap_or(50)
                as i32;
        out.push(PoolCandidate {
            stock_code: code.to_string(),
            stock_name: name,
            period,
            confidence,
            anchor_event_id: None,
        });
    }
    out
}

/// 按四档汇总本批 pending/completed/failed 行（同档各行 hindsight 相同，取首行）。
async fn summarize_horizons(
    db: &DatabaseConnection,
    analysis_ids: &[String],
    today: chrono::NaiveDate,
) -> Result<Vec<HorizonSummary>, String> {
    // 反思行查询下沉 dao（分层护栏：命令层不得直连 sea_orm / entities）
    let rows =
        axagent_dao::repo::stock_lesson_queries::fetch_by_original_analysis_ids(db, analysis_ids)
            .await
            .map_err(|e| format!("查询批次反思行失败: {e}"))?;

    // horizon 字符串 → 行集合（本批 pending 行建点即写 horizon，None 老行不计入四档汇总）
    let mut groups: BTreeMap<String, Vec<_>> = BTreeMap::new();
    for r in &rows {
        if let Some(h) = r.horizon.as_deref() {
            groups.entry(h.to_string()).or_default().push(r);
        }
    }

    let mut out = Vec::new();
    for period in Period::ALL {
        let key = period.as_str();
        // horizon 列历史写法 ultrashort 也应认到 UltraShort（parse 已兼容）
        let grp = groups
            .iter()
            .find(|(k, _)| k.parse::<Period>().is_ok_and(|p| p.as_str() == period.as_str()));
        let Some((_, grp)) = grp else {
            continue;
        };

        let hindsight_date = grp.first().map(|r| r.hindsight_date.clone()).unwrap_or_default();
        let due = chrono::NaiveDate::parse_from_str(&hindsight_date, "%Y-%m-%d")
            .is_ok_and(|h| h <= today);

        let mut completed = 0usize;
        let mut pending = 0usize;
        let mut failed = 0usize;
        for r in grp {
            match r.status.as_str() {
                "completed" => completed += 1,
                "pending" | "running" => pending += 1,
                _ => failed += 1,
            }
        }

        out.push(HorizonSummary {
            period: key.to_string(),
            holding_days: period.default_holding_days(),
            hindsight_date,
            due,
            total: grp.len(),
            completed,
            pending,
            failed,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_dao::repo::time_travel_batch_runs as repo;

    /// serenity inner 返回 JSON 的键名是混合形态（候选本体 snake_case +
    /// 逐档覆盖 camelCase）；置信度必须优先取 winRate（折算胜率，与 reco_picks
    /// 落库口径同量纲），无 winRate 才回退原始 confidence；无档位 / 空 code 丢弃。
    #[test]
    fn serenity_json_maps_snake_case_body_prefers_winrate() {
        let value = serde_json::json!({
            "status": "completed",
            "candidates": [
                { "stock_code": "600001", "stock_name": "甲", "confidence": 91, "period": "mid", "winRate": 72 },
                { "stock_code": "600002", "stock_name": "乙", "confidence": 80, "period": "long" },
                // 空展开回退分支：无逐档 period → 不进逐只分析
                { "stock_code": "600003", "stock_name": "丙", "confidence": 80 },
                // 残缺 code 丢弃
                { "stock_code": "  ", "stock_name": "丁", "period": "mid" },
            ]
        });
        let rows = serenity_candidates_from_json(&value);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].stock_code, "600001");
        assert_eq!(rows[0].stock_name, "甲");
        assert_eq!(rows[0].period, Period::Mid);
        assert_eq!(rows[0].confidence, 72, "winRate 优先于原始候选评分");
        assert_eq!(rows[1].stock_code, "600002");
        assert_eq!(rows[1].period, Period::Long);
        assert_eq!(rows[1].confidence, 80, "无 winRate 回退原始 confidence");
    }

    /// 返回体没有 candidates 段（异常形态）→ 空队列而非 panic
    #[test]
    fn serenity_json_missing_candidates_yields_empty() {
        let value = serde_json::json!({ "status": "completed" });
        assert!(serenity_candidates_from_json(&value).is_empty());
    }

    // ── 持久化（time_travel_batch_runs）─────────────────────────────────────────

    /// 构造带两条分析结果 / 一个档汇总的测试状态（created_at 显式可控，便于验倒序）。
    fn make_persist_state(
        batch_id: &str,
        as_of: &str,
        status: &str,
        created_at_ms: i64,
    ) -> BatchRunState {
        let mut s = BatchRunState::new(
            batch_id.to_string(),
            as_of.to_string(),
            Some(9),
            TimeTravelBatchRequest {
                date: Some(as_of.to_string()),
                trading_days_ago: None,
                periods: vec![],
                min_confidence: 55,
                top_n: 10,
                reflect: false,
                analyze: true,
            },
        );
        s.status = status.to_string();
        s.created_at_ms = created_at_ms;
        s.updated_at_ms = created_at_ms;
        if status != STATUS_RUNNING {
            s.finished_at_ms = Some(created_at_ms + 5_000);
        }
        s.total = 2;
        s.ok = 1;
        s.fail = 1;
        s.reco_picks = 3;
        s.serenity_picks = 2;
        s.analyses = vec![
            PoolCandidateResult {
                stock_code: "600001".to_string(),
                stock_name: "甲".to_string(),
                period: "mid".to_string(),
                confidence: 80,
                analysis_id: Some("a-1".to_string()),
                error: None,
            },
            PoolCandidateResult {
                stock_code: "600002".to_string(),
                stock_name: "乙".to_string(),
                period: "long".to_string(),
                confidence: 70,
                analysis_id: None,
                error: Some("数据缺失".to_string()),
            },
        ];
        s.horizons = vec![HorizonSummary {
            period: "mid".to_string(),
            holding_days: 14,
            hindsight_date: "2026-05-15".to_string(),
            due: true,
            total: 1,
            completed: 1,
            pending: 0,
            failed: 0,
        }];
        s
    }

    async fn test_db() -> DatabaseConnection {
        axagent_dao::db::create_test_pool().await.expect("测试：创建连接池应成功").conn
    }

    /// upsert 后从库还原（模拟重启：不经内存）全字段一致；同一批次二次 upsert
    /// 为更新而非重复插入。
    #[tokio::test]
    async fn persist_then_restore_roundtrip_and_upsert_updates() {
        let db = test_db().await;
        let mut s = make_persist_state("b1", "2026-05-01", STATUS_COMPLETED, 1_000);
        repo::upsert_batch_run(&db, &batch_record_from_state(&s))
            .await
            .expect("首次 insert 应成功");

        // 同批次状态推进（completed 计数变化）→ upsert 更新
        s.ok = 2;
        s.fail = 0;
        s.updated_at_ms = 2_000;
        repo::upsert_batch_run(&db, &batch_record_from_state(&s))
            .await
            .expect("冲突 upsert 应成功");

        let m =
            repo::fetch_batch_by_id(&db, "b1").await.expect("查询应成功").expect("批次行应存在");
        assert_eq!(m.ok, 2, "upsert 应覆盖计数列");
        assert_eq!(m.created_at_ms, 1_000, "创建时间不被更新覆盖");

        let restored = state_from_model(m, false);
        assert_eq!(restored.batch_id, "b1");
        assert_eq!(restored.as_of, "2026-05-01");
        assert_eq!(restored.total, 2);
        assert_eq!(restored.reco_picks, 3);
        assert_eq!(restored.analyses.len(), 2, "明细 JSON 应完整还原");
        assert_eq!(restored.analyses[0].stock_code, "600001");
        assert_eq!(restored.analyses[1].error.as_deref(), Some("数据缺失"));
        assert_eq!(restored.horizons[0].period, "mid");
        assert_eq!(restored.horizons[0].completed, 1);

        // 全表仍只有一行（upsert 未产生重复）
        let count = repo::list_recent_batch_runs(&db, 100, None, None).await.unwrap().len();
        assert_eq!(count, 1);
    }

    /// 列表按 created_at 倒序、默认上限与 status/as_of 过滤生效；
    /// 摘要模式不携带明细 JSON。
    #[tokio::test]
    async fn list_returns_desc_summary_with_filters() {
        let db = test_db().await;
        // 三条：两条 completed（不同日期）、一条 running
        for (id, date, ts) in [
            ("old", "2026-04-01", 1_000_i64),
            ("new", "2026-05-01", 3_000),
            ("run", "2026-05-01", 2_000),
        ] {
            let status = if id == "run" {
                STATUS_RUNNING
            } else {
                STATUS_COMPLETED
            };
            repo::upsert_batch_run(
                &db,
                &batch_record_from_state(&make_persist_state(id, date, status, ts)),
            )
            .await
            .unwrap();
        }

        // 走命令层同一查询（list_time_travel_batches 调的就是它）
        let all: Vec<_> = repo::list_recent_batch_runs(&db, 50, None, None)
            .await
            .unwrap()
            .into_iter()
            .map(|m| state_from_model(m, true))
            .collect();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].batch_id, "new", "倒序：最新创建在前");
        assert_eq!(all[1].batch_id, "run");
        assert_eq!(all[2].batch_id, "old");
        assert!(all[0].analyses.is_empty(), "摘要模式不携带 analyses 明细");
        assert!(all[0].horizons.is_empty());
        assert_eq!(all[0].total, 2, "但标量计数仍完整");

        // status 过滤
        let completed =
            repo::list_recent_batch_runs(&db, 50, Some(STATUS_COMPLETED), None).await.unwrap();
        assert_eq!(completed.len(), 2);

        // as_of 过滤
        let on_day = repo::list_recent_batch_runs(&db, 50, None, Some("2026-05-01")).await.unwrap();
        assert_eq!(on_day.len(), 2);
    }

    /// 模拟重启：库内遗留 running 行被一次性收敛为 interrupted，
    /// 非 running 终态行不受影响。
    #[tokio::test]
    async fn stale_running_rows_are_interrupted() {
        let db = test_db().await;
        repo::upsert_batch_run(
            &db,
            &batch_record_from_state(&make_persist_state(
                "zombie",
                "2026-05-01",
                STATUS_RUNNING,
                1_000,
            )),
        )
        .await
        .unwrap();
        repo::upsert_batch_run(
            &db,
            &batch_record_from_state(&make_persist_state(
                "done",
                "2026-05-02",
                STATUS_COMPLETED,
                2_000,
            )),
        )
        .await
        .unwrap();

        // 与 ensure_stale_reaped 同一调用形态（状态/文案由命令层给）
        let reap = || async {
            repo::mark_stale_batches(
                &db,
                now_ms(),
                STATUS_RUNNING,
                STATUS_INTERRUPTED,
                "应用重启：批次运行中进程退出，as-of 封闭世界无法跨进程续跑",
            )
            .await
            .unwrap()
        };
        assert_eq!(reap().await, 1, "仅 running 行被收敛");

        let zombie = repo::fetch_batch_by_id(&db, "zombie").await.unwrap().unwrap();
        assert_eq!(zombie.status, STATUS_INTERRUPTED);
        assert!(zombie.finished_at_ms.is_some(), "interrupted 也写完成时刻");
        assert!(zombie.message.unwrap().contains("重启"));

        let done = repo::fetch_batch_by_id(&db, "done").await.unwrap().unwrap();
        assert_eq!(done.status, STATUS_COMPLETED, "终态行不受收敛影响");

        // 再执行一次：无遗留 running，幂等返回 0
        assert_eq!(reap().await, 0);
    }
}
