// SPDX-License-Identifier: AGPL-3.0-only

//! 进化计划定时聚合（P1/P4，`PLAN-batch-automation-mover-recall.md` P1/P4）。
//!
//! 每天（内置 20:00，在 18:00 到期反思之后）聚合近窗口内新增的三类样本：
//! 1. 已到期反思（`stock_reflections.status=completed`）
//! 2. 决策回写验证（`decision_validations`）
//! 3. 妖股漏检核查（`mover_recall_runs` 的业务类漏检条目数）
//!
//! 有**业务类**样本才产出计划，并以 `status=pending_approval` 落 `stock_evolution_history`；
//! **不执行任何变异**（不调 `run_evolution`，不回写参数/权重/模板）。接受/拒绝
//! 由 P4 审批命令（`commands::stock_evolution_approval`）处理。
//!
//! ## P4 硬规则：数据可达性失败分流
//!
//! 失败归因为「拿不到数据」的样本（行情缺失 / 判据不可用 / vendor 限流致候选池降级、
//! 见 [`reflection_data_blocked`] / [`validation_data_blocked`] /
//! [`classify_mover_misses`]）**只进数据质量分桶**：id 写入 trigger_sample_json 的
//! `dataBlocked`，永不计入驱动进化的样本数，**永不触发模板结构（节点/边）变异**。
//! 只有业务质量类样本（看错、漏信号、评分截断）才允许驱动进化。
//!
//! 两条防抖规则：
//! - 窗口内业务类样本全 0 ⇒ skip（任务成功但不产计划，避免每晚空计划轰炸）；
//! - 窗口内已有未审批计划 ⇒ skip（审批未处理前不重复堆计划）。
//!
//! 计划类型固定 `parameter`（权重/阈值/horizon_prior 类调参）；模板结构（节点/边）
//! 变异属高风险动作，**未来开放时必须只在业务类样本上进行**（见 `is_data_reachability_blocked`）。
//!
//! ## 2026-10-19 阶段 3 / 决-3：结构层根因不塞进参数计划
//!
//! 反思的 `implementation_tier`（权威 = `analysis_engine::reflection_tier`）判为 **L2/L3**
//! 的那些行，其根因是「门形状 / 腿集合 / 提示词判据」，不是某个标量取值。本聚合器把它们
//! **从驱动样本里分出**、id 落 `trigger_sample_json.structuralReflectionIds` 进审批抽屉待裁区，
//! 而不是留在 `reflections` 里 —— 让它们驱动参数计划会造成两种坏：① 网格朝一个与根因无关的
//! 数字收敛；② 审批人以为接受计划就等于处置了本窗口的全部反思。
//! 层级未标注（NULL，含本列引入前的存量）按既有行为继续参与，见 `may_enter_param_grid`。

use axagent_analysis_engine::StockReflectionReport;
use axagent_analysis_engine::evolution_drift::load_performance_window;
use axagent_analysis_engine::evolution_optimizer::PARAM_EVOLUTION_LOOKBACK_DAYS;
use axagent_analysis_engine::stock_self_evolution::{
    EvolutionTrigger, EvolutionType, StockSelfEvolutionEngine,
};
use axagent_dao::DatabaseConnection;
use axagent_dao::repo::decision_validations::{self, ValidationModel};
use axagent_dao::repo::mover_recall_runs;
use axagent_dao::repo::stock_evolution_history::{self, PendingProposalRecord};
use axagent_dao::repo::stock_lesson_queries::{self, ReflectionModel};
use axagent_harness::holding_period::HORIZON_BRANCH_GENERATION_FLOOR;
use serde::{Deserialize, Serialize};

/// 定时任务 task_type（与前端任务中心白名单、services.rs 执行分支逐字对齐）。
pub use crate::commands::constants::EVOLUTION_PROPOSAL_TASK_TYPE;

/// 默认样本窗口（小时）。
const DEFAULT_WINDOW_HOURS: i64 = 24;

/// 任务配置（JSON 存 `CronJob.prompt`，空串 = 全默认）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EvolutionProposalConfig {
    /// 聚合样本的回看窗口（小时），缺省 24。
    pub window_hours: Option<i64>,
}

impl Default for EvolutionProposalConfig {
    fn default() -> Self {
        Self { window_hours: Some(DEFAULT_WINDOW_HOURS) }
    }
}

impl EvolutionProposalConfig {
    pub fn from_json(s: &str) -> Result<Self, String> {
        if s.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_json::from_str(s).map_err(|e| format!("evolution-proposal 配置解析失败: {e}"))
    }

    pub fn window_hours_or_default(&self) -> i64 {
        self.window_hours.unwrap_or(DEFAULT_WINDOW_HOURS).max(1)
    }
}

/// 本轮聚合到的样本计数（业务类 = 驱动进化；data_blocked = 数据可达性失败，只入质量看板）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProposalSampleCounts {
    /// 窗口内新增的已到期反思行（业务类）
    pub reflections: u64,
    /// 窗口内新增的决策验证行（业务类）
    pub validations: u64,
    /// 窗口内各核查轮次的业务类漏检条目数（剔除 pool_degraded）
    pub mover_misses: u64,
    /// 数据可达性失败样本总数（三类来源合计；**不进 [`Self::total`]**）
    pub data_blocked: u64,
}

impl ProposalSampleCounts {
    /// 驱动进化的样本总量。**刻意不含 data_blocked** —— 数据可达性失败永不驱动进化。
    fn total(&self) -> u64 {
        self.reflections + self.validations + self.mover_misses
    }
}

/// 触发样本留痕（落 `stock_evolution_history.trigger_sample_json`）。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct TriggerSampleSnapshot {
    /// 驱动进化的业务类样本 id
    reflection_ids: Vec<String>,
    validation_ids: Vec<String>,
    mover_run_ids: Vec<String>,
    /// 数据可达性失败样本（只进数据质量看板，不驱动进化）
    data_blocked: DataBlockedSamples,
    /// 结构层根因样本（阶段 3 / 决-3）：反思判为 L2/L3 ⇒ 不驱动参数进化，
    /// 落本字段进审批抽屉的待裁区，等人处置而不是被网格折算。
    structural_reflection_ids: Vec<String>,
    counts: SnapshotCounts,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct DataBlockedSamples {
    reflection_ids: Vec<String>,
    validation_ids: Vec<String>,
    mover_run_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotCounts {
    reflections: u64,
    validations: u64,
    mover_misses: u64,
    data_blocked: u64,
}

/// `mover_recall_runs.misses_json` 单条目的最小反序列化形态（只取归因层）。
/// 落库形态是 `MoverMiss` 数组（serde snake_case，layer 字段），
/// 这里不依赖 analysis-engine 的完整类型，避免跨层耦合。
#[derive(Debug, Deserialize)]
struct MissEntry {
    #[serde(default)]
    layer: Option<String>,
}

/// 妖股漏检的**数据可达性**归因层（与 `MissLayer::PoolDegraded` 的 as_str 逐字对齐）。
const MOVER_POOL_DEGRADED: &str = "pool_degraded";

/// 反思行是否属于数据可达性失败（硬规则：永不驱动任何进化，含未来结构变异）。
///
/// - `mover_label = no_market_data`：到反思点行情快照不可得；
/// - `mover_label = rule_unavailable`：阈值判据不可用（变量缺失/读取失败）；
/// - `blackboard_snapshot` 含 `__untrusted` 标记：工作流自证该结论基于不可信/降级数据。
fn reflection_data_blocked(r: &ReflectionModel) -> bool {
    matches!(r.mover_label.as_deref(), Some("no_market_data") | Some("rule_unavailable"))
        || r.blackboard_snapshot.as_deref().is_some_and(|s| s.contains("__untrusted"))
}

/// 决策验证行是否属于数据可达性失败：T+N 价格或最终收益算不出来
/// （行情缺失），该行没有可用于评价业务质量的结论。
fn validation_data_blocked(v: &ValidationModel) -> bool {
    v.t_plus_n_price.is_none() || v.final_return_pct.is_none()
}

/// 解析一轮 mover 核查的 `misses_json`，按归因层拆分
/// 返回 `(业务类漏检条目数, pool_degraded 数据可达性条目数)`。
fn classify_mover_misses(misses_json: Option<&str>) -> (u64, u64) {
    let Some(raw) = misses_json.filter(|s| !s.trim().is_empty()) else {
        return (0, 0);
    };
    let Ok(entries) = serde_json::from_str::<Vec<MissEntry>>(raw) else {
        // 解析不出来不能假装「没有数据失败」：保守地整轮不记业务账，留给数据质量排查。
        tracing::warn!("[evolution-proposal] misses_json 解析失败，该轮漏检不驱动进化");
        return (0, 0);
    };
    let mut business = 0u64;
    let mut blocked = 0u64;
    for e in entries {
        if e.layer.as_deref() == Some(MOVER_POOL_DEGRADED) {
            blocked += 1;
        } else {
            business += 1;
        }
    }
    (business, blocked)
}

/// 聚合结果摘要（写进 TaskRunResult.output）。
#[derive(Debug, Clone)]
pub struct ProposalSummary {
    /// true = 按规则跳过（无业务样本 / 已有待审批计划），未落新计划
    pub skipped: bool,
    /// 新落的计划 id（skipped=true 时为 None）
    pub plan_id: Option<String>,
    pub counts: ProposalSampleCounts,
    pub message: String,
}

impl std::fmt::Display for ProposalSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} reflections={} validations={} mover_misses={} data_blocked={}",
            self.message,
            self.counts.reflections,
            self.counts.validations,
            self.counts.mover_misses,
            self.counts.data_blocked,
        )?;
        if let Some(pid) = &self.plan_id {
            write!(f, " plan_id={pid}")?;
        }
        Ok(())
    }
}

/// 跑一轮进化计划聚合。cron 执行分支与（未来的）手动命令共用此实现。
pub async fn run_evolution_proposal_inner(
    db: &DatabaseConnection,
    engine: &StockSelfEvolutionEngine,
    config: &EvolutionProposalConfig,
) -> Result<ProposalSummary, String> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let since_ms = now_ms - config.window_hours_or_default() * 3600 * 1000;
    // decision_validations.validated_at 是 YYYY-MM-DD 文本，窗口下界取当天日期串
    let since_date = chrono::DateTime::from_timestamp_millis(since_ms)
        .unwrap_or_default()
        .with_timezone(&chrono::Utc)
        .format("%Y-%m-%d")
        .to_string();

    // 1. 三类样本拉行分类（数据可达性失败单独分桶，不驱动进化）
    let reflection_rows = stock_lesson_queries::fetch_completed_reflections_since(db, since_ms)
        .await
        .map_err(|e| format!("读取到期反思失败: {e}"))?;

    let mut reflection_ids = Vec::new();
    let mut blocked_reflection_ids = Vec::new();
    let mut structural_reflection_ids = Vec::new();
    for r in &reflection_rows {
        if reflection_data_blocked(r) {
            blocked_reflection_ids.push(r.id.clone());
        } else if !axagent_analysis_engine::reflection_tier::may_enter_param_grid(
            axagent_analysis_engine::reflection_tier::parse_reflection_tier(
                r.implementation_tier.as_deref(),
            ),
        ) {
            // 阶段 3 / 决-3：结构层根因（L2 改公式 / L3 改提示词）**不驱动参数进化计划**。
            // 本聚合器产出的计划类型固定 `parameter`，而审批接受会真的执行
            // `execute_pending_plan` 并要求 `parameter_result` ⇒ 把结构层塞进来等于
            // 用调参去修一个不是参数的问题（更坏的是它必然在执行期被打成 rejected）。
            // NULL（未判层级 / 本列引入前的存量）按既有行为继续参与，理由见
            // `reflection_tier::may_enter_param_grid`。
            structural_reflection_ids.push(r.id.clone());
        } else {
            reflection_ids.push(r.id.clone());
        }
    }
    if !structural_reflection_ids.is_empty() {
        tracing::info!(
            "[evolution-proposal] {} 条反思判为结构层（L2/L3），已从参数进化样本中分出：{:?}",
            structural_reflection_ids.len(),
            structural_reflection_ids
        );
    }

    let validation_rows = decision_validations::list_validated_since_date(db, &since_date)
        .await
        .map_err(|e| format!("读取决策验证失败: {e}"))?;

    let mut validation_ids = Vec::new();
    let mut blocked_validation_ids = Vec::new();
    for v in &validation_rows {
        if validation_data_blocked(v) {
            blocked_validation_ids.push(v.id.clone());
        } else {
            validation_ids.push(v.id.clone());
        }
    }

    let mover_runs = mover_recall_runs::list_runs_since(db, since_ms)
        .await
        .map_err(|e| format!("读取漏检核查记录失败: {e}"))?;

    let mut mover_business_total = 0u64;
    let mut mover_blocked_total = 0u64;
    let mut mover_run_ids = Vec::new();
    let mut blocked_mover_run_ids = Vec::new();
    // A-2 起一次核查每链各落一行。本函数产的是**荐股链**的参数计划 ⇒ 智选链轮次在此跳过；
    // 未标链的旧轮次（混池口径）按现状继续计入 —— 把它们排掉会静默丢掉全部历史样本，
    // 比口径不纯更坏。跳过条数走日志留痕，不悄悄消失。
    let mut serenity_runs_skipped = 0usize;
    for run in &mover_runs {
        if run.chain.as_deref()
            == Some(axagent_analysis_engine::mover_recall::RecoChain::Serenity.as_str())
        {
            serenity_runs_skipped += 1;
            continue;
        }
        let (business, blocked) = classify_mover_misses(run.misses_json.as_deref());
        if business > 0 {
            mover_run_ids.push(run.id.clone());
        }
        if blocked > 0 {
            blocked_mover_run_ids.push(run.id.clone());
        }
        mover_business_total += business;
        mover_blocked_total += blocked;
    }

    if serenity_runs_skipped > 0 {
        tracing::info!(
            "[evolution-proposal] 跳过 {} 个趋势智选链核查轮次（本计划只产荐股链参数，A-2 分链）",
            serenity_runs_skipped
        );
    }

    let counts = ProposalSampleCounts {
        reflections: reflection_ids.len() as u64,
        validations: validation_ids.len() as u64,
        mover_misses: mover_business_total,
        data_blocked: blocked_reflection_ids.len() as u64
            + blocked_validation_ids.len() as u64
            + mover_blocked_total,
    };

    // 2. 无业务样本 ⇒ skip（数据失败再多也不产进化计划）
    if counts.total() == 0 {
        let message = if structural_reflection_ids.is_empty() {
            if counts.data_blocked > 0 {
                format!(
                    "窗口内 {} 条样本全部为数据可达性失败（行情缺失/判据不可用/候选池降级），\
                     只入数据质量看板，不触发进化，跳过",
                    counts.data_blocked
                )
            } else {
                "窗口内无新增反思/验证/漏检样本，跳过".to_string()
            }
        } else {
            // 结构层单独成句：说「无新增反思」是假的（有新反思，只是被判在 L2/L3），
            // 而「有反思却不产计划」如果没有这一句，读起来就像闭环坏了。
            format!(
                "窗口内 {} 条反思判为结构层（L2 改公式 / L3 改提示词），不驱动参数进化，\
                 需在待裁清单处置；数据可达性失败 {} 条（只入数据质量看板）⇒ 无可驱动\
                 参数进化的业务样本，跳过",
                structural_reflection_ids.len(),
                counts.data_blocked
            )
        };
        return Ok(ProposalSummary { skipped: true, plan_id: None, counts, message });
    }

    // 3. 窗口内已有待审批计划 ⇒ skip（审批未处理不重复堆计划）
    let pending_count = stock_evolution_history::count_pending_since(db, since_ms)
        .await
        .map_err(|e| format!("统计待审批计划失败: {e}"))?;
    if pending_count > 0 {
        return Ok(ProposalSummary {
            skipped: true,
            plan_id: None,
            counts,
            message: format!("窗口内已有 {pending_count} 条待审批进化计划，跳过"),
        });
    }

    // 3.5 执行可行性闸门（修「审批即失败」）：上面数的业务样本（反思/验证/漏检）只回答
    //     「有没有学习素材」；但参数进化执行器（`evolve_parameters → run_evolution`）真正
    //     消费的是 `strategy_performance` 表现窗口（近 PARAM_EVOLUTION_LOOKBACK_DAYS 天且
    //     `template_version >= HORIZON_BRANCH_GENERATION_FLOOR`），与业务样本并不同源。
    //     若该窗口为空，审批接受时必然在 `run_evolution` 报「无表现数据可供进化」并把计划
    //     打成 rejected —— 这种「注定失败的待审批计划」不应被产生，故此处用与执行器**完全
    //     相同的函数 + 相同的常量**（None = 以今天为 as_of）预判，空则 skip。
    let performable = load_performance_window(db, PARAM_EVOLUTION_LOOKBACK_DAYS, None)
        .await
        .map_err(|e| format!("读取策略表现窗口失败: {e}"))?;
    if performable.is_empty() {
        // 先绑 message 再 move counts（struct 字段按序求值，内联 format! 会先 move 后借用）
        let message = format!(
            "窗口内有 {} 条业务样本（反思/验证/漏检），但参数进化所需的策略表现样本 \
             （strategy_performance：近 {} 天且算法代际≥{}）为 0，审批执行必然报 \
             「无表现数据可供进化」；这些表现行当前未带算法代际（被代际下限规则排除），\
             故跳过，不产生无法执行的计划",
            counts.total(),
            PARAM_EVOLUTION_LOOKBACK_DAYS,
            HORIZON_BRANCH_GENERATION_FLOOR
        );
        return Ok(ProposalSummary { skipped: true, plan_id: None, counts, message });
    }

    // 4. 产出计划（仅构造，不执行变异）。固定参数类：结构变异的开放闸口见模块头硬规则。
    let reason = format!(
        "定时聚合（窗口 {}h）：到期反思 {} 条、决策验证 {} 条、妖股漏检 {} 起\
         （数据可达性失败 {} 条已隔离，不驱动进化）{}",
        config.window_hours_or_default(),
        counts.reflections,
        counts.validations,
        counts.mover_misses,
        counts.data_blocked,
        // 审批人要能看出「本轮计划只覆盖参数层，另有 N 条结构层根因没人管」，
        // 否则他会以为接受它就等于把这一窗口的反思都处置了。
        if structural_reflection_ids.is_empty() {
            String::new()
        } else {
            format!(
                "；另有 {} 条反思判为结构层（L2/L3），不在本计划的参数动作范围内",
                structural_reflection_ids.len()
            )
        }
    );
    let report = StockReflectionReport {
        execution_id: format!("proposal-{}", uuid::Uuid::new_v4()),
        workflow_id: "stock-invest-default".to_string(),
        should_trigger_evolution: true,
        evolution_trigger_reason: Some(reason.clone()),
        ..Default::default()
    };
    let mut plan = engine.create_plan(
        &EvolutionTrigger::ManualTrigger { reason: reason.clone() },
        &report,
        None,
    );
    // 定时聚合只出参数类计划：权重/阈值/horizon_prior 调优；不碰工作流结构。
    plan.evolution_type = EvolutionType::ParameterEvolution;

    // 5. 触发样本留痕（业务 id + 数据失败 id 分桶 + 计数），供审批抽屉审计。
    let snapshot = TriggerSampleSnapshot {
        reflection_ids,
        validation_ids,
        mover_run_ids,
        data_blocked: DataBlockedSamples {
            reflection_ids: blocked_reflection_ids,
            validation_ids: blocked_validation_ids,
            mover_run_ids: blocked_mover_run_ids,
        },
        structural_reflection_ids,
        counts: SnapshotCounts {
            reflections: counts.reflections,
            validations: counts.validations,
            mover_misses: counts.mover_misses,
            data_blocked: counts.data_blocked,
        },
    };
    let trigger_sample_json = serde_json::to_string(&snapshot).unwrap_or_default();

    // 6. 以待审批状态落库（无参数/流程结果——计划尚未执行）。
    //    摘要用 reason 原文：create_plan 对 ManualTrigger 统一拼"手动触发："前缀，
    //    本行实为定时聚合产生，用原文避免在历史列表里误导审批人。
    let record = PendingProposalRecord {
        id: format!("evo-{}", plan.plan_id),
        plan_id: plan.plan_id.clone(),
        trigger_json: serde_json::to_string(&plan.trigger).unwrap_or_default(),
        improvement_summary: reason,
        trigger_sample_json,
        created_at_ms: now_ms,
    };
    stock_evolution_history::insert_pending_proposal(db, &record)
        .await
        .map_err(|e| format!("待审批进化计划落库失败: {e}"))?;

    Ok(ProposalSummary {
        skipped: false,
        plan_id: Some(plan.plan_id),
        counts,
        message: "已生成待审批进化计划（参数类，未执行任何变更）".to_string(),
    })
}

#[cfg(test)]
mod tests {
    //! 配置解析/分流纯函数测试放文件末尾（clippy::items_after_test_module 口径）。

    use super::*;
    // 测试直接构造实体行（分层门禁不扫 test 块）；生产代码走 dao re-export 的类型别名。
    use axagent_dao::axagent_entities::{decision_validations, stock_reflections};

    fn reflection_with(
        mover_label: Option<&str>,
        blackboard: Option<&str>,
    ) -> stock_reflections::Model {
        stock_reflections::Model {
            id: "r1".to_string(),
            stock_code: "000001".to_string(),
            stock_name: "测试".to_string(),
            original_analysis_id: "a1".to_string(),
            as_of_date: "2026-09-01".to_string(),
            hindsight_date: "2026-09-08".to_string(),
            min_confidence_threshold: 60,
            reflection_depth: "light".to_string(),
            horizon: None,
            actual_outcome: String::new(),
            raw_return: None,
            alpha_return: None,
            holding_days: None,
            mover_label: mover_label.map(str::to_string),
            benchmark_name: None,
            verdict: None,
            alpha_cited: None,
            lesson_summary: None,
            what_went_wrong: None,
            missed_signals: None,
            fix_for_future: None,
            parameter_suggestions_json: None,
            horizon_results_json: None,
            decision_json: None,
            blackboard_snapshot: blackboard.map(str::to_string),
            template_version: None,
            source: "due".to_string(),
            miss_layer: None,
            asof_reco_generated_at: None,
            asof_layer: None,
            reco_diff_json: None,
            failure_modes: None,
            // 阶段 3：反思产出的层级轴（测试夹具 = 未标注，读侧不得当成 L1）
            implementation_tier: None,
            code_diff_proposal: None,
            status: "completed".to_string(),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn validation_with(
        t_plus_n_price: Option<f64>,
        final_return_pct: Option<f64>,
    ) -> decision_validations::Model {
        decision_validations::Model {
            id: "v1".to_string(),
            pick_id: "p1".to_string(),
            stock_code: "000001".to_string(),
            stock_name: "测试".to_string(),
            style: "trend".to_string(),
            period: "short".to_string(),
            t_plus_n: 5,
            generated_at: String::new(),
            validated_at: "2026-09-08".to_string(),
            entry_price: 10.0,
            target_price: 12.0,
            stop_loss: 9.0,
            position_pct: 10.0,
            confidence: 70,
            inferred_action: "buy".to_string(),
            t_plus_n_price,
            max_price: None,
            min_price: None,
            max_return_pct: None,
            max_drawdown_pct: None,
            final_return_pct,
            hit_stop_loss: None,
            hit_target: None,
            hit_outcome: None,
            factor_snapshot: None,
            data_source: "akshare_daily".to_string(),
            as_of_date: None,
            created_at: String::new(),
        }
    }

    #[test]
    fn config_empty_json_uses_default_window() {
        let cfg = EvolutionProposalConfig::from_json("").unwrap();
        assert_eq!(cfg.window_hours_or_default(), DEFAULT_WINDOW_HOURS);
        let cfg = EvolutionProposalConfig::from_json("{}").unwrap();
        assert_eq!(cfg.window_hours_or_default(), DEFAULT_WINDOW_HOURS);
    }

    #[test]
    fn config_reads_camel_case_window_hours() {
        let cfg = EvolutionProposalConfig::from_json(r#"{"windowHours":6}"#).unwrap();
        assert_eq!(cfg.window_hours_or_default(), 6);
    }

    #[test]
    fn config_rejects_garbage_and_clamps_non_positive() {
        assert!(EvolutionProposalConfig::from_json("not-json").is_err());
        let cfg = EvolutionProposalConfig { window_hours: Some(0) };
        assert_eq!(cfg.window_hours_or_default(), 1);
    }

    #[test]
    fn sample_counts_total_excludes_data_blocked() {
        let c = ProposalSampleCounts {
            reflections: 2,
            validations: 3,
            mover_misses: 4,
            data_blocked: 9,
        };
        assert_eq!(c.total(), 9);
        // 全是数据失败时 total 为 0 ⇒ 不产计划
        let c = ProposalSampleCounts { data_blocked: 5, ..Default::default() };
        assert_eq!(c.total(), 0);
    }

    #[test]
    fn reflection_blocked_by_mover_label_and_untrusted_marker() {
        // 业务类：正常 mover 标签、无不可信标记
        assert!(!reflection_data_blocked(&reflection_with(Some("normal"), None)));
        assert!(!reflection_data_blocked(&reflection_with(Some("mover"), None)));
        // 数据可达性：no_market_data / rule_unavailable
        assert!(reflection_data_blocked(&reflection_with(Some("no_market_data"), None)));
        assert!(reflection_data_blocked(&reflection_with(Some("rule_unavailable"), None)));
        // window_incomplete 不算数据失败（持有期未满，是时序问题，下轮还会再反思）
        assert!(!reflection_data_blocked(&reflection_with(Some("window_incomplete"), None)));
        // blackboard 含 __untrusted 标记
        assert!(reflection_data_blocked(&reflection_with(
            None,
            Some(r#"{"metadata":{"__untrusted":["node-a"]}}"#)
        )));
    }

    #[test]
    fn validation_blocked_when_price_or_return_missing() {
        assert!(validation_data_blocked(&validation_with(None, None)));
        assert!(validation_data_blocked(&validation_with(Some(11.0), None)));
        assert!(validation_data_blocked(&validation_with(None, Some(3.2))));
        assert!(!validation_data_blocked(&validation_with(Some(11.0), Some(3.2))));
    }

    #[test]
    fn mover_misses_split_by_pool_degraded_layer() {
        let json = r#"[
            {"stockCode":"1","layer":"not_in_pool"},
            {"stockCode":"2","layer":"pool_degraded"},
            {"stockCode":"3","layer":"l3_scored_out"},
            {"stockCode":"4","layer":"pool_degraded"}
        ]"#;
        assert_eq!(classify_mover_misses(Some(json)), (2, 2));
        assert_eq!(classify_mover_misses(None), (0, 0));
        assert_eq!(classify_mover_misses(Some("")), (0, 0));
        // 解析失败保守归零（业务账不记）
        assert_eq!(classify_mover_misses(Some("not-json")), (0, 0));
    }
}
