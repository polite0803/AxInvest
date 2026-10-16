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
//! 本模块把它们按 **as-of（原 `analysis_date`）原地重跑**，使其产出真正的决策。
//!
//! ## 为什么是 as-of 而不是 live
//!
//! 中断的行原本是**当天实时分析**，结论只对那天有意义。用 live（今天）重跑，
//! 等于把「当时的判断」换成「今天的判断」，与库内其它同日记录不可比，
//! 也无法作为那天的反思样本。而 as-of 重跑恰好命中 `run_stock_workflow_inner`
//! 的「replay 同日覆盖」分支（`stock_workflow/core.rs:450-458`）⇒ **原地 UPDATE
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

use axagent_entities::stock_analyses;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::scheduler::restore::STOCK_ANALYSIS_STATUS_INTERRUPTED;

/// 内置任务类型标识（cron executor 路由键，勿改）。
///
/// ⚠ 名字**必须带域前缀**，不能叫 `TASK_TYPE`：门禁测试
/// `stock_automation_cron_seed::every_builtin_task_type_is_handled_by_the_executor`
/// 会把路径常量取**最后一段**（`rsplit("::")`）去 `init/services.rs` 里做子串查找，
/// 而 `TASK_TYPE` 是 `POOL_SCAN_TASK_TYPE` 的子串 ⇒ 门禁会因他人名字而恒真，
/// 真正「seed 有任务、executor 没分支」的缺陷就检不出来了。
pub const STOCK_ANALYSIS_RERUN_TASK_TYPE: &str = "stock-analysis-rerun";

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
        serde_json::from_str(raw)
            .map_err(|e| format!("解析 {STOCK_ANALYSIS_RERUN_TASK_TYPE} 配置失败: {e}"))
    }

    /// 序列化（种子任务写入 `CronJob.prompt` 用，与 `PoolScanConfig::to_json` 同形）。
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self)
            .map_err(|e| format!("序列化 {STOCK_ANALYSIS_RERUN_TASK_TYPE} 配置失败: {e}"))
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

/// 统计命中给定状态的行数。
async fn count_by_status(db: &DatabaseConnection, statuses: &[&str]) -> Result<usize, String> {
    stock_analyses::Entity::find()
        .filter(stock_analyses::Column::Status.is_in(statuses.to_vec()))
        .count(db)
        .await
        .map(|n| n as usize)
        .map_err(|e| format!("统计待重跑分析失败: {e}"))
}

/// 取出本轮要处理的行，按 `(analysis_date, stock_code)` **升序**。
///
/// 排序是确定性要求：同一批在两次运行里顺序一致，日志才可比对。
/// 升序 = 先补最老的 —— 否则持续产生的新中断会把老记录永远挤在队尾。
async fn load_batch(
    db: &DatabaseConnection,
    statuses: &[&str],
    max_per_run: usize,
) -> Result<Vec<stock_analyses::Model>, String> {
    let mut q = stock_analyses::Entity::find()
        .filter(stock_analyses::Column::Status.is_in(statuses.to_vec()))
        .order_by_asc(stock_analyses::Column::AnalysisDate)
        .order_by_asc(stock_analyses::Column::StockCode);
    if max_per_run > 0 {
        q = q.limit(Some(max_per_run as u64));
    }
    q.all(db).await.map_err(|e| format!("查询待重跑分析失败: {e}"))
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
        let status = stock_analyses::Entity::find_by_id(analysis_id)
            .one(db)
            .await
            .ok()
            .flatten()
            .map(|m| m.status)
            .unwrap_or_else(|| "<记录消失>".to_string());
        if status != "running" {
            return status;
        }
        if started.elapsed() >= timeout {
            return format!("timeout（等待 {}s 仍未落定）", timeout.as_secs());
        }
        tokio::time::sleep(SETTLE_POLL_INTERVAL).await;
    }
}

/// 重跑一条：按 as-of 原地覆盖，并等到落定。返回该条的终态 status（或错误说明）。
async fn rerun_one(
    app: Option<tauri::AppHandle>,
    state: &AppState,
    db: &DatabaseConnection,
    row: &stock_analyses::Model,
) -> Result<String, String> {
    // `as_of_date` 用**该行自己的 analysis_date** ⇒ 命中 replay 同日覆盖分支，原地 UPDATE。
    //
    // ⚠ `template_id` 同样取该行的原值：中断行可能是快速链（`stock-analysis-fast`）产出，
    //   强行传完整链会把「一条快速链记录」重跑成完整链结论 —— 那是语义变更，不是补跑。
    //   原值为 NULL 时传 None，由 `run_stock_workflow_inner` 走它的缺省解析。
    let settled = crate::commands::stock_workflow::core::run_stock_workflow_inner(
        app,
        state,
        row.stock_code.clone(),
        None,
        Some(row.analysis_date.clone()),
        Some(row.id.clone()),
        None,
        None,
        row.template_id.clone(),
    )
    .await;

    match settled {
        Ok(_) => Ok(wait_analysis_settled(db, &row.id, SETTLE_TIMEOUT).await),
        // 调用本身失败 ⇒ DAG 没起来，**不必等**（等了也只是白等一个不会变的状态）。
        Err(e) => Err(e),
    }
}

/// 执行一次完整的重跑（内含多轮循环，直到收敛或达 `max_rounds`）。
///
/// 返回的 [`RerunOutcome`] 由调用方（cron executor / 启动钩子）渲染并落库 ——
/// 本函数**不**自己 `record_run`，因为两种调用方的记录面不同。
pub async fn run_rerun(
    app: Option<tauri::AppHandle>,
    state: &AppState,
    config: &RerunConfig,
) -> RerunOutcome {
    let db = state.harness.db().clone();
    let mut outcome = RerunOutcome::default();

    // 起始读数用**可重试全集**（含 failed/timeout），因为它才是收敛目标；
    // 若只用 interrupted 计数，会低估「这一轮之后还剩多少没结论的记录」。
    outcome.pending_at_start = count_by_status(&db, RETRY_ROUND_STATUSES).await.unwrap_or(0);
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
            outcome.attempted += 1;
            let label = format!("{} {}", row.stock_code, row.stock_name);
            tracing::info!(
                "[stock-analysis-rerun] 重跑 {label}（as_of={}, id={}）",
                row.analysis_date,
                row.id
            );
            match rerun_one(app.clone(), state, &db, row).await {
                Ok(status) => {
                    if status != "completed" {
                        let msg =
                            format!("{label} 重跑后状态={status}（as_of={}）", row.analysis_date);
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
}
