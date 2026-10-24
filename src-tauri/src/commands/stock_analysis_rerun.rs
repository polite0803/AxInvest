// SPDX-License-Identifier: AGPL-3.0-only

//! 中断分析的自动重跑 —— 内置任务类型 `stock-analysis-rerun`。
//!
//! ## 它解决什么
//!
//! `stock_analyses.status = 'interrupted'` 的行，是「进程在分析跑完前消失、
//! 终态从未写入」的遗留（成因与判据见 `crate::scheduler::restore` 的
//! `reclaim_stale_stock_analyses`）。这类行**不会自己好**：永远没有结论，
//! 历史面板里点开也只是一句「未产出结论」。
//!
//! 本模块把它们按 **as-of 锚**（行内 `as_of_date`，空则以 `analysis_date` 回退）原地
//! 重跑，使其产出真正的决策。
//!
//! ## 为什么是 as-of 而不是 live
//!
//! 中断的行原本是**当天实时分析**，结论只对那天有意义。用 live（今天）重跑，
//! 等于把「当时的判断」换成「今天的判断」，与库内其它同日记录不可比，
//! 也无法作为那天的反思样本。而 as-of 重跑恰好命中 `run_stock_workflow_inner`
//! 的「replay 同日覆盖」分支（`stock_workflow/core.rs:451-459`）⇒ **原地 UPDATE
//! 同一 `analysis_id`**，不新建版本行、不污染历史。
//!
//! 附带收益（与 `axagent-batch-rerun` 同）：重跑后的分析其 `hindsight_date`
//! （= as_of + 期望持有期）**已经落在过去** ⇒ 落库即可进反思队列。
//!
//! ## 为什么串行、且**不进**交易日闸门
//!
//! - **串行**：本项目有「高并发触发 LLM 供应商限流降级」的历史教训（同
//!   `axagent-batch-rerun.rs:879-881` 的取舍）。重跑是长任务，宁可慢，
//!   也不要把降级样本混进反思数据集。
//! - **不进闸门**：`init/services.rs` 的 A 股链交易日闸门是**显式白名单**
//!   （`matches!` 逐个列 task_type），其注释写明「只拦『本轮产物依赖当日行情』
//!   的任务」。本任务是**补历史**，与「今天是不是交易日」无关 ⇒ 故意不登记进去。
//!   ⚠ 若将来有人把它加进那个 `matches!`，补跑会在周末静默停摆。
//!
//! ## 与离线工具 `axagent-batch-rerun` 的分工（不是两份实现）
//!
//! | | 离线 bin | 本模块 |
//! |---|---|---|
//! | 筛选口径 | `template_version`（按**代际**对齐全库） | `status`（按**状态**收敛未完成） |
//! | 触发 | 人工跑一次 | 应用内常驻（启动 + 定时），无人值守 |
//! | 场景 | 算法升版后的一次性数据修复 | 中断记录的持续自愈 |
//!
//! 两者都调 `run_stock_workflow_inner` 的 replay 同日覆盖分支。
//!
//! ⚠ **已知重复（登记，勿再抄第三份）**：本文件 [`wait_analysis_settled`] 与
//! `src/bin/axagent-batch-rerun.rs::wait_until_settled` 是同语义实现。bin 是独立
//! target 且 `mod commands` 在 lib 中为私有 ⇒ 无法直接引用，故**本文件是权威实现**。
//! 收敛方向：在 `lib.rs` 增一条 `pub use`，让 bin 改调此处（属接口面变更，另行处理）。

use axagent_dao::repo::stock_analysis_rerun_queue::{
    RerunCandidate, count_analyses_by_status, fetch_analysis_status, has_any_in_status,
    load_rerun_candidates,
};
// ⚠ 走 `axagent_dao::DatabaseConnection`（dao/lib.rs 的既有 re-export）而**不是**
// `sea_orm::DatabaseConnection`：本文件在 `src/commands/**` 内，被
// `commands-no-direct-db` 门禁扫描，import `sea_orm` 即计数（棘轮基线按文件登记，
// 新文件从 0 起算 ⇒ 直接越界）。类型本身完全同一个，不是替身。
use axagent_dao::DatabaseConnection;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::commands::error::ErrorResponse;
use crate::commands::error_code::common as common_err;
use crate::scheduler::restore::STOCK_ANALYSIS_STATUS_INTERRUPTED;

/// 内置任务类型标识（cron executor 路由键，勿改）—— **权威定义在
/// `commands::constants`**（与其它内置任务 task_type 同处，见该模块头部的 ⚠：
/// 留在业务模块会逼出 `crate::commands::stock_analysis_rerun::` 横向互调）。
/// 此处只做再导出，本模块内部一律引用本名字。
pub use crate::commands::constants::STOCK_ANALYSIS_RERUN_TASK_TYPE;

/// 同进程内补跑批次的单飞锁（`run_rerun` 第一件事就是拿它）。
///
/// 用 `try_lock` 而不是 `lock`：**排队等于待会儿照跑一遍**，而锁外那一层（启动钩子与
/// cron 兜底轮）本来就各有一条到点路径，重复跑同一批正是要防的事。拿不到就让路并如实
/// 记一次未收口，比排队重烧 35 × 30 次 LLM 调用便宜。
///
/// `tokio::sync::Mutex` 而非 `std::sync`：本仓 `clippy.toml` 的 `disallowed-types`
/// 禁 std 锁，且这里的持锁期横跨整批 await。
static RERUN_SINGLE_FLIGHT: std::sync::OnceLock<tokio::sync::Mutex<()>> =
    std::sync::OnceLock::new();

/// 单条分析落定的等待上限。
///
/// 取值与 `axagent-batch-rerun` 一致（30 分钟）：按「实盘单条分析的最坏耗时」留足。
/// 超时**不**等于该条失败 —— 它只是本轮的等待结束，下一轮会按状态重新决定要不要再跑。
const SETTLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// 落定轮询间隔。
const SETTLE_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// 第一轮处理的状态集合：**只碰** `interrupted`。
///
/// 刻意最保守：优先补「从未跑起来」的，不去动「跑过了但失败」的 —— 后者可能
/// 是数据源或模型侧的真实问题，盲目重跑只会重复烧配额。
const FIRST_ROUND_STATUSES: &[&str] = &[STOCK_ANALYSIS_STATUS_INTERRUPTED];

/// 第二轮起纳入的状态集合：追加 `failed` / `timeout`。
///
/// - `failed` / `timeout`：跑过了但没出结论 ⇒ 按用户要求「失败自动重试直到成功」纳入。
/// - `cancelled`：**永不纳入** —— 那是用户**主动取消**的明确意图，
///   自动重跑等于替用户推翻他自己的决定。
/// - `completed`：当然不纳入。
const RETRY_ROUND_STATUSES: &[&str] = &[STOCK_ANALYSIS_STATUS_INTERRUPTED, "failed", "timeout"];

/// 重跑配置（JSON 存 `CronJob.prompt`，与 `pool-scan` 同机制）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RerunConfig {
    /// 单轮最多重跑多少条；`0` = 不限（默认：一次清空）。
    #[serde(default)]
    pub max_per_run: usize,
    /// 最多跑几轮。一轮里失败/超时的会在下一轮重试，直到全部产出结论或达上限。
    #[serde(default = "default_max_rounds")]
    pub max_rounds: u32,
}

fn default_max_rounds() -> u32 {
    3
}

impl Default for RerunConfig {
    fn default() -> Self {
        Self { max_per_run: 0, max_rounds: default_max_rounds() }
    }
}

impl RerunConfig {
    /// 解析配置。**空串 = 用默认值**（种子任务不写 `prompt` 时的正常形态，不是错误）。
    pub fn from_json(raw: &str) -> Result<Self, String> {
        if raw.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_json::from_str(raw).map_err(|e| {
            ErrorResponse::err_with_detail(
                common_err::INVALID_INPUT,
                format!("解析 {STOCK_ANALYSIS_RERUN_TASK_TYPE} 配置失败: {e}"),
            )
        })
    }

    /// 序列化（种子任务写入 `CronJob.prompt` 用，与 `PoolScanConfig::to_json` 同形）。
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| {
            ErrorResponse::err_with_detail(
                common_err::INTERNAL,
                format!("序列化 {STOCK_ANALYSIS_RERUN_TASK_TYPE} 配置失败: {e}"),
            )
        })
    }
}

/// 一次重跑的汇总（供 cron executor 渲染成 `TaskRunResult`）。
#[derive(Debug, Default)]
pub struct RerunOutcome {
    /// 进入本函数时，库里「未产出结论」的条数（`interrupted` + 可重试的失败态）。
    pub pending_at_start: usize,
    /// 本次真正发起过重跑的**次数**（跨轮累计，同一行可能被发起两次）。
    pub attempted: usize,
    /// 结束时仍未产出结论的条数。
    pub still_pending: usize,
    /// 实际跑了几轮。
    pub rounds: u32,
    /// 首个错误。**不吞**：哪怕最终全部收敛，也把首个错误带出去供排查。
    pub first_error: Option<String>,
}

impl RerunOutcome {
    /// 收敛掉的条数（起始待处理 − 结束时剩余）。饱和减，防计数为负。
    pub fn resolved(&self) -> usize {
        self.pending_at_start.saturating_sub(self.still_pending)
    }

    /// 是否算成功：**全部收敛**才算。
    ///
    /// 刻意不用「跑完没报错」当成功 —— 那会把「每一条都失败但流程走完了」报成成功，
    /// 正是本仓 `axagent-batch-rerun` 用 `not_completed` 单列计数防的那种虚高。
    pub fn is_success(&self) -> bool {
        self.still_pending == 0
    }

    pub fn render(&self) -> String {
        let mut s = format!(
            "中断分析重跑：起始 {} 条待处理 → 收敛 {} 条，剩余 {} 条；共 {} 轮、发起 {} 次",
            self.pending_at_start,
            self.resolved(),
            self.still_pending,
            self.rounds,
            self.attempted
        );
        if let Some(e) = &self.first_error {
            s.push_str(&format!("；首个错误：{e}"));
        }
        s
    }
}

/// 统计命中给定状态的行数（持久化在 dao 的 `count_analyses_by_status`）。
async fn count_by_status(db: &DatabaseConnection, statuses: &[&str]) -> Result<usize, String> {
    count_analyses_by_status(db, statuses).await.map_err(|e| {
        ErrorResponse::err_with_detail(common_err::INTERNAL, format!("统计待重跑分析失败: {e}"))
    })
}

/// 取出本轮要处理的行，按 `(analysis_date, stock_code, created_at, id)` 升序，同键已去重。
///
/// 排序、同键去重与取件上限都是 **dao 侧的契约**（见 `stock_analysis_rerun_queue` 的
/// 模块文档），不是本命令的偏好 —— 换调用方不得换顺序。
async fn load_batch(
    db: &DatabaseConnection,
    statuses: &[&str],
    max_per_run: usize,
) -> Result<Vec<RerunCandidate>, String> {
    load_rerun_candidates(db, statuses, max_per_run).await.map_err(|e| {
        ErrorResponse::err_with_detail(common_err::INTERNAL, format!("查询待重跑分析失败: {e}"))
    })
}

/// 等待某条分析离开 `running`，返回其**终态** status 字符串（或超时说明）。
///
/// ## 为什么必须等
///
/// `run_stock_workflow_inner` 是 **fire-and-forget**：它把 DAG `tokio::spawn` 到后台后
/// **立即返回**（`stock_workflow/core.rs:976`）。那是**为前端设计**的形态 ——
/// 前端拿到 `analysisId` 后自己轮询 `status`。而本模块是无人值守的批处理，
/// 必须自己负责等待，否则会「静默地什么都没做成，却照样输出成功报告」。
///
/// 这不是理论风险：`axagent-batch-rerun.rs:220-239` 记着 2026-09-23 的实测 ——
/// 不等就返回时每条只耗 2.7~10.6 秒（正常单条 ≥1 分钟）、`status` 停在 `running`、
/// 日志出现 `task was cancelled`。
///
/// 轮询 DB 而不是拿内部句柄：DAG 已 `spawn` 走、句柄不可得；而 `status` 正是
/// **前端也在用**的同一权威落地信号 ⇒ 轮询它与「用户看到的状态」严格一致。
pub async fn wait_analysis_settled(
    db: &DatabaseConnection,
    analysis_id: &str,
    timeout: std::time::Duration,
) -> String {
    let started = std::time::Instant::now();
    loop {
        // `None` 与「某个 status 值」是两回事：前者 = 行已被删除，同样要停止等待，
        // 但报告文案必须能把它与真终态区分开（dao 的 `fetch_analysis_status` 文档已述）。
        let status = match fetch_analysis_status(db, analysis_id).await {
            Ok(Some(s)) => s,
            Ok(None) => "<记录消失>".to_string(),
            Err(e) => format!("<查询失败: {e}>"),
        };
        if status != "running" {
            return status;
        }
        if started.elapsed() >= timeout {
            return format!("timeout（等待 {}s 仍未落定）", timeout.as_secs());
        }
        tokio::time::sleep(SETTLE_POLL_INTERVAL).await;
    }
}

/// 本次重跑**真正落库的那条记录的 id** —— 等待必须打在它身上，不能打在来源行上。
///
/// `run_stock_workflow_inner` 是 fire-and-forget：它把 DAG `spawn` 走后立刻返回，返回体里
/// 的 `analysisId` 才是它实际写入的行。而补跑的来源行 `row.id` 走 as-of 回放时**不会被复用**：
/// 同日无回放行 ⇒ 新建 UUID 版本（`stock_workflow/core.rs:591`），有同日回放行 ⇒ 覆盖的是
/// 那条 target 行。两种情形下 `row.id` 的状态都停在 `interrupted`（≠ `running`）⇒
/// 按它轮询第一次读就判定「已落定」并立即返回，整批以**毫秒级** tick 完。
///
/// 实证（2026-10-17 09:07）：59 个父 DAG 在 43 秒内全部起跑 ⇒ LLM 提供商 429 全线失败
/// （71 条股吧正常的运行 0 条出决策），同时把本机 IP 打到东财股吧限流，之后单只手动分析
/// 也因 `t-sentiment-data` 失败而被阻断。缺了这层等待，本模块文档里「必须自己负责等待」
/// 那句话就是空的。
fn settle_target_id(result: &serde_json::Value, fallback: &str) -> String {
    result
        .get("analysisId")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

/// 重跑一条：按 as-of 原地覆盖，并等到落定。返回该条的终态 status（或错误说明）。
async fn rerun_one(
    app: Option<tauri::AppHandle>,
    state: &AppState,
    db: &DatabaseConnection,
    row: &RerunCandidate,
) -> Result<String, String> {
    // as-of 锚取**该行解析后的锚点** `row.as_of`（回放行 = `as_of_date`，实盘行回退
    // 行内 `analysis_date`）⇒ 命中 replay 同日覆盖分支，原地 UPDATE。
    //
    // ⚠ 不得改回按行内执行日发起：回放行的执行日（如 10-17）≠ 它复现的行情日（如 10-14），
    //   按执行日会解析出错误的覆盖目标 ⇒ 新建一条无关记录，而不是原地覆盖原回放行
    //   （2026-10-17 实测 `d3a74ec8` 形态）。
    //
    // ⚠ `template_id` 同样取该行的原值：中断行可能是快速链（`stock-analysis-fast`）产出，
    //   强行传完整链会把「一条快速链记录」重跑成完整链结论 —— 那是语义变更，不是补跑。
    //   原值为 NULL 时传 None，由 `run_stock_workflow_inner` 走它的缺省解析。
    // ⚠ 走 `crate::` 的 lib 层 re-export（`lib.rs:83`），**不**写
    // `crate::commands::stock_workflow::core::…`：后者是跨模块直调兄弟 commands，
    // 被 `commands-no-sibling-call` 分层门拦（bin 侧早就是走 re-export 的，
    // 这里必须同口径 —— 门禁不是「bin 例外、应用内宽松」）。
    let settled = crate::run_stock_workflow_inner(
        app,
        state,
        row.stock_code.clone(),
        None,
        Some(row.as_of.clone()),
        Some(row.id.clone()),
        None,
        None,
        row.template_id.clone(),
    )
    .await;

    match settled {
        Ok(result) => {
            let target = settle_target_id(&result, &row.id);
            Ok(wait_analysis_settled(db, &target, SETTLE_TIMEOUT).await)
        },
        // 调用本身失败 ⇒ DAG 没起来，**不必等**（等了也只是白等一个不会变的状态）。
        Err(e) => Err(e),
    }
}

/// 执行一次完整的重跑（内含多轮循环，直到收敛或达 `max_rounds`）。
///
/// 返回的 [`RerunOutcome`] 由调用方（cron executor / 启动钩子）渲染并落库 ——
/// 本函数**不**自己 `record_run`，因为两种调用方的记录面不同。
///
/// ## 同进程单飞（本函数第一件事）
///
/// 发起方有两条且**住在同一个进程**：启动钩子 `start_interrupted_analysis_rerun`
/// 与内置任务/系统兜底轮走的 cron executor。`EXTERNAL_RERUN_LOCK_FILE` 那把锁只防
/// 离线 bin，防不到同进程的两次 spawn。
/// 实证（2026-10-17 12:38–12:52）：多批各自在起点载入**同一份** 35 行清单、按同一
/// `(analysis_date, stock_code)` 顺序前进 ⇒ lockstep：同一行 `d3a74ec8`（603466）被
/// 连跑 3 次，每一次的终态落库都在 2 秒后被下一次的 `status=running` 覆盖，
/// 面板永远看不到收口的结论，而每跑一次约 30 次 LLM 调用。
pub async fn run_rerun(
    app: Option<tauri::AppHandle>,
    state: &AppState,
    config: &RerunConfig,
) -> RerunOutcome {
    let db = state.harness.db().clone();

    // ## 闸门 0（2026-10-18）：先做一次只查主键的廉价预检，再谈计数
    //
    // `pending_at_start` 的口径是「含去重的可重试全集」，而计算它要走
    // `pending_rows` —— 那条路径**必然**把 `parent_analysis_id IS NOT NULL`（54 行 /
    // 80 MB）与 `analysis_kind='replay'`（40 行 / 49 MB）两批整行拉回并反序列化。
    // 实测（本机生产库）：这张宽表 `blackboard_snapshot` 合计 504 MB、单行最大 7.4 MB，
    // 于是一次 `run_rerun` 光取件口就要解压 129 MB TOAST（一轮 2 次 × 3 轮 + 首尾 ≈ 900 MB）。
    //
    // 而启动钩子**每次启动**都会跑这一遍（sleep 10s 后），即使库里根本没有待补跑的记录
    // —— 实测启动日志 `无待补跑的中断分析` 出现时刻比 `setup_async_complete` 晚 10.58 s，
    // 净耗时≈0.6 s，全花在「为了得到 0 而先拉 129 MB」上。
    //
    // 三道判据只删不增 ⇒ 原始命中为空即蕴含全集为空，故本闸门**不改变补跑口径**。
    // 查询失败按「可能有」继续（`unwrap_or(true)`），退回原路径而不是静默跳过整批。
    if !has_any_in_status(&db, RETRY_ROUND_STATUSES).await.unwrap_or(true) {
        tracing::info!("[stock-analysis-rerun] 可重试状态无任何记录（廉价预检）⇒ 本轮跳过");
        return RerunOutcome::default();
    }

    // 起始读数用**可重试全集**（含 failed/timeout），因为它才是收敛目标；
    // 若只用 interrupted 计数，会低估「这一轮之后还剩多少没结论的记录」。
    let mut outcome = RerunOutcome {
        pending_at_start: count_by_status(&db, RETRY_ROUND_STATUSES).await.unwrap_or(0),
        ..Default::default()
    };

    let flight = RERUN_SINGLE_FLIGHT.get_or_init(|| tokio::sync::Mutex::new(()));
    let Ok(_guard) = flight.try_lock() else {
        // 让路时把两个读数都填成「原样剩着」：`is_success()` 因此为假，任务记录不会把
        // 「什么都没做」记成一次成功 —— 那正是本模块反复防的那种虚高。
        outcome.still_pending = outcome.pending_at_start;
        outcome.first_error =
            Some("已有补跑批次在跑 ⇒ 本次让路（同进程多驱动会 lockstep 重跑同一批）".to_string());
        tracing::warn!("[stock-analysis-rerun] {}", outcome.first_error.as_deref().unwrap_or(""));
        return outcome;
    };

    if outcome.pending_at_start == 0 {
        return outcome;
    }

    let max_rounds = config.max_rounds.max(1);
    for round in 1..=max_rounds {
        let statuses = if round == 1 {
            FIRST_ROUND_STATUSES
        } else {
            RETRY_ROUND_STATUSES
        };
        let batch = match load_batch(&db, statuses, config.max_per_run).await {
            Ok(b) => b,
            Err(e) => {
                outcome.first_error.get_or_insert(e);
                break;
            },
        };
        if batch.is_empty() {
            break;
        }
        outcome.rounds = round;
        tracing::info!(
            "[stock-analysis-rerun] 第 {round}/{max_rounds} 轮：处理 {} 条（状态集合 {:?}）",
            batch.len(),
            statuses
        );

        for row in &batch {
            // 发起前重取一次状态：批次清单是本轮开头载入的，期间别的通路（另一条驱动、
            // 用户手动分析、上一次补跑的落库）可能已经把这条处理掉。
            // 单飞锁挡同进程重复批，这一句挡**清单过期**——两者挡的不是同一种竞态。
            let current = fetch_analysis_status(&db, &row.id).await.ok().flatten();
            if current.as_deref().is_some_and(|s| !statuses.contains(&s)) {
                tracing::info!(
                    "[stock-analysis-rerun] 跳过 {}：当前状态={} 已不在本轮可重试集合内",
                    row.stock_code,
                    current.as_deref().unwrap_or("?")
                );
                continue;
            }
            outcome.attempted += 1;
            let label = row.stock_code.clone();
            tracing::info!(
                "[stock-analysis-rerun] 重跑 {label}（as_of={}, id={}）",
                row.as_of,
                row.id
            );
            match rerun_one(app.clone(), state, &db, row).await {
                Ok(status) => {
                    if status != "completed" {
                        let msg = format!("{label} 重跑后状态={status}（as_of={}）", row.as_of);
                        tracing::warn!("[stock-analysis-rerun] {msg}");
                        outcome.first_error.get_or_insert(msg);
                    }
                },
                Err(e) => {
                    let msg = format!("{label} 重跑发起失败: {e}");
                    tracing::error!("[stock-analysis-rerun] {msg}");
                    outcome.first_error.get_or_insert(msg);
                },
            }
        }

        let left = count_by_status(&db, RETRY_ROUND_STATUSES).await.unwrap_or(0);
        tracing::info!("[stock-analysis-rerun] 第 {round} 轮结束，剩余 {left} 条未产出结论");
        if left == 0 {
            break;
        }
    }

    outcome.still_pending = count_by_status(&db, RETRY_ROUND_STATUSES).await.unwrap_or(0);
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 空配置串必须解析成默认值 —— 种子任务不写 `prompt` 是**正常形态**，
    /// 若在这里报错，任务到点会记一次「配置解析失败」，而用户什么都没配错。
    #[test]
    fn empty_config_falls_back_to_default() {
        let c = RerunConfig::from_json("").expect("空串应回退默认值");
        assert_eq!(c.max_per_run, 0, "默认不限条数");
        assert_eq!(c.max_rounds, 3, "默认 3 轮");
    }

    /// `maxRounds` 缺省时由 serde 默认值补齐（局部配置不该把整份配置打回默认）。
    #[test]
    fn partial_config_keeps_serde_default_for_missing_field() {
        let c = RerunConfig::from_json(r#"{"maxPerRun":7}"#).expect("局部配置应可解析");
        assert_eq!(c.max_per_run, 7);
        assert_eq!(c.max_rounds, 3, "缺省字段走 serde default，而不是 0");
    }

    /// 成功判据是「全部收敛」，不是「跑完了」。
    /// 反向锁：若有人把 `is_success` 改成恒 `true`，这条会红。
    #[test]
    fn success_requires_full_convergence() {
        let partial = RerunOutcome {
            pending_at_start: 35,
            attempted: 35,
            still_pending: 1,
            rounds: 3,
            first_error: None,
        };
        assert!(!partial.is_success(), "还剩 1 条未产出结论时不得报成功");
        assert_eq!(partial.resolved(), 34);

        let full = RerunOutcome {
            pending_at_start: 35,
            attempted: 40,
            still_pending: 0,
            rounds: 3,
            first_error: Some("某条曾失败".into()),
        };
        assert!(full.is_success(), "全部收敛即成功，首个错误不改变成功判据");
        assert_eq!(full.resolved(), 35);
    }

    /// `resolved` 必须饱和减：`still_pending > pending_at_start` 是可能的
    /// （起始读数与结束读数之间，别处又产生了新的中断行），减法不能下溢 panic。
    #[test]
    fn resolved_saturates_when_new_interruptions_appear() {
        let o = RerunOutcome {
            pending_at_start: 2,
            attempted: 2,
            still_pending: 5,
            rounds: 1,
            first_error: None,
        };
        assert_eq!(o.resolved(), 0, "后产生的新中断不得让 resolved 变成负数");
    }

    /// 等待目标必须是**返回体里的 `analysisId`**，不是来源行 id。
    /// 反向锁：若把 `settle_target_id` 改成直接回 `fallback`，这条红。
    #[test]
    fn settle_target_prefers_the_id_actually_written() {
        let r = serde_json::json!({ "analysisId": "new-replay-uuid", "stockCode": "301302" });
        assert_eq!(
            settle_target_id(&r, "source-row-id"),
            "new-replay-uuid",
            "补跑新建的是回放版本，等来源行等于不等待"
        );
    }

    /// 返回体缺键 / 空串时必须退回来源行 id —— 退回空串会让轮询查一条不存在的行，
    /// 又是一次「立即落定」，等于把上一个缺陷换个形态放回去。
    #[test]
    fn settle_target_falls_back_instead_of_polling_empty_id() {
        assert_eq!(
            settle_target_id(&serde_json::json!({ "stockCode": "301302" }), "source-row-id"),
            "source-row-id",
            "缺 analysisId 时退回来源行，不能返回空串"
        );
        assert_eq!(
            settle_target_id(&serde_json::json!({ "analysisId": "" }), "source-row-id"),
            "source-row-id",
            "空串同样要退回"
        );
    }

    /// 接线锁：`rerun_one` 里不得再按来源行 id 等待。
    /// 上面两条只锁纯函数；本条锁「纯函数真的被接在等待那一步上」。
    #[test]
    fn rerun_one_waits_on_the_settled_target_not_the_source_row() {
        let src = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/commands/stock_analysis_rerun.rs"
        ))
        .expect("读取本文件失败");
        let body = src
            .split("async fn rerun_one(")
            .nth(1)
            .expect("找不到 rerun_one")
            .split("\n}\n")
            .next()
            .expect("rerun_one 函数体切不出");
        assert!(body.contains("settle_target_id("), "rerun_one 必须用 settle_target_id 求等待目标");
        assert!(
            !body.contains("wait_analysis_settled(db, &row.id"),
            "等待目标不得是来源行 id（该 id 在补跑路径下永不进入 running）"
        );
    }

    /// 接线锁：`rerun_one` 的 as-of 必须取**解析后的锚点** `row.as_of`，而不是行内执行日。
    /// 上面 dao 侧的用例只锁投影；本条锁「命令真的把锚传给了工作流」。
    ///
    /// 反向锁：若把第 5 参改回行内执行日，本用例红 —— 按执行日发起会新建一条无关记录，
    /// 而不是命中同日覆盖分支原地 UPDATE 原回放行。
    ///
    /// ⚠ 断言扫的是 `rerun_one` 函数体（含体内注释）⇒ 该函数体内任何位置（包括注释）
    /// 都不得出现字面量 `row.analysis_date`。
    #[test]
    fn rerun_one_anchors_on_resolved_as_of_not_execution_date() {
        let src = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/commands/stock_analysis_rerun.rs"
        ))
        .expect("读取本文件失败");
        let body = src
            .split("async fn rerun_one(")
            .nth(1)
            .expect("找不到 rerun_one")
            .split("\n}\n")
            .next()
            .expect("rerun_one 函数体切不出");
        assert!(
            body.contains("Some(row.as_of.clone())"),
            "rerun_one 必须以解析后的锚点 `row.as_of` 发起重跑"
        );
        assert!(
            !body.contains("row.analysis_date"),
            "rerun_one 内不得残留执行日 `row.analysis_date`（按执行日发起会打错覆盖目标）"
        );
    }
}
