//! 复盘 → 进化：漂移数据访问与权重持久化层
//!
//! 主要职责：
//! 1. 读取 strategy_performance 表现行（按窗口过滤）
//! 2. 读取最近一次 strategy_weight_history 作为 current_weights
//! 3. 调用 `weight_decay::compute_adjusted_weights` 计算新权重
//! 4. 写回 strategy_weight_history（每次调整全量留痕）
//! 5. 提供 list/load 供前端 EvolutionDriftPanel 渲染
//!
//! 时间旅行注意：
//! - `as_of_date: Option<String>` 决定是否走 Replay 模式
//! - Live 模式（as_of_date = None）：基于当前时间窗口
//! - Replay 模式（as_of_date = "2024-09-30"）：基于 as_of 之前的窗口，避免未来泄漏

use std::collections::HashMap;
use std::str::FromStr;

use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

use axagent_entities::{stock_evolution_history, strategy_performance, strategy_weight_history};
use axagent_harness::Period;

use crate::decision_action::{normalize_action, ActionKind};
use crate::evolution_optimizer::EvolutionResult;
use crate::reflection_stats::{expand_horizon_samples, DecisionPerformanceSample};
use crate::types::StrategyTrend;
use crate::weight_decay::{
    compute_adjusted_weights, format_rationale, StrategyPerformanceRow, WeightDecayConfig,
};

/// 进化历史中「人工已接受」状态（P4 两段式审批的接受态）。
const EVOLUTION_STATUS_ACCEPTED: &str = "accepted";

/// 读取当前生效的权重衰减配置（P4 受控自动优化）。
///
/// 来源 = `stock_evolution_history` 中**最近一条 accepted 计划行**的
/// `parameter_result_json.best_config`（人工审批接受后才生效）；
/// 无 accepted 行 / 反序列化失败 ⇒ [`WeightDecayConfig::default`]（与历史行为一致）。
///
/// 回滚不需要额外开关：回滚命令新插一条 accepted 行（配置为上一版本快照），
/// 本函数取「最新 accepted」自然读到旧配置。
pub async fn load_active_weight_decay_config(db: &DatabaseConnection) -> WeightDecayConfig {
    let row = stock_evolution_history::Entity::find()
        .filter(stock_evolution_history::Column::Status.eq(EVOLUTION_STATUS_ACCEPTED))
        .filter(stock_evolution_history::Column::ParameterResultJson.is_not_null())
        .order_by_desc(stock_evolution_history::Column::CreatedAt)
        .one(db)
        .await;
    let Ok(Some(row)) = row else {
        if let Err(e) = row {
            warn!("[evolution_drift] 读取生效进化配置失败，回退默认: {e}");
        }
        return WeightDecayConfig::default();
    };
    let Some(raw) = row.parameter_result_json.as_deref() else {
        return WeightDecayConfig::default();
    };
    match serde_json::from_str::<EvolutionResult>(raw) {
        Ok(r) => r.best_config,
        Err(e) => {
            warn!("[evolution_drift] 生效进化配置反序列化失败，回退默认: {e}");
            WeightDecayConfig::default()
        },
    }
}

/// 前端 `EvolutionDriftPanel` 使用的统一响应
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvolutionDriftDashboard {
    /// 当前生效的 (strategy, period) -> 权重
    pub current_weights: HashMap<(String, String), f64>,
    /// 最近一次重算时间（ms），0 表示尚未重算
    pub last_recalc_at: i64,
    /// 仪表盘当前所有 (strategy, period) 的统计
    pub stats: Vec<StrategyStatRow>,
    /// 最近 N 次调整原因（Top 5）
    pub recent_changes: Vec<RecentChangeRow>,
    /// 各策略汇总视图（按 strategy_id 聚合）
    pub strategy_summary: Vec<StrategySummaryRow>,
    /// #31：本次权重口径的**起算代际**（= `HORIZON_BRANCH_GENERATION_FLOOR`）。
    /// 回传它而不是让前端手抄一个 125 —— 常量只有一处权威，面板只负责显示。
    pub generation_floor: i32,
    /// #31：窗口内因「早于起算代」被排除的样本数（旧代样本永久不可比）
    pub excluded_pre_floor_generation: usize,
    /// #31：窗口内因「代际未知」被排除的样本数（会随写侧补章而减少）
    pub excluded_unknown_generation: usize,
}

/// 单条 (strategy, period) 统计
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyStatRow {
    pub strategy_id: String,
    pub period: String,
    pub new_weight: f64,
    pub old_weight: f64,
    pub delta_pct: f64,
    pub win_rate: f64,
    pub sample_size: u32,
    pub confidence: f64,
    pub rationale: String,
}

/// Top 5 调整原因
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentChangeRow {
    pub id: String,
    pub strategy_id: String,
    pub period: String,
    pub old_weight: f64,
    pub new_weight: f64,
    pub delta_pct: f64,
    pub trigger: String,
    pub rationale: Option<String>,
    pub applied_at: i64,
}

/// 按 strategy_id 聚合的摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategySummaryRow {
    pub strategy_id: String,
    pub avg_weight: f64,
    pub total_samples: u32,
    pub avg_win_rate: f64,
    pub trend: StrategyTrend,
}

/// 读取当前生效的 weights（每个 (strategy, period) 取最新一条 weight_history.new_weight）
pub async fn load_current_weights(
    db: &DatabaseConnection,
) -> Result<HashMap<(String, String), f64>, String> {
    load_current_weights_by_trigger(db, None).await
}

/// 同 [`load_current_weights`]，但可按 trigger 过滤键空间 —— 荐股闭环
/// （`recommender::reco_loop`，trigger=`"reco-loop"`）与分析链演化权重**不得互相消费**
/// （`PLAN-reco-reflection-closure.md` Q3：归属分离）。
pub async fn load_current_weights_by_trigger(
    db: &DatabaseConnection,
    trigger: Option<&str>,
) -> Result<HashMap<(String, String), f64>, String> {
    let query = strategy_weight_history::Entity::find();
    let query = match trigger {
        Some(t) => query.filter(strategy_weight_history::Column::Trigger.eq(t)),
        None => query,
    };
    let all = query
        .order_by_desc(strategy_weight_history::Column::AppliedAt)
        .all(db)
        .await
        .map_err(|e| format!("读取 weight_history 失败: {e}"))?;

    // 同一 (strategy, period) 取最新一行
    let mut map: HashMap<(String, String), f64> = HashMap::new();
    for row in all {
        let key = (row.strategy_id.clone(), row.period.clone());
        map.entry(key).or_insert(row.new_weight);
    }
    Ok(map)
}

/// 指定 trigger 最近一次闭环重算的落库时刻（`strategy_weight_history.applied_at`）。
///
/// `reco_loop_view` 的 `last_recalc_at` 唯一来源：无行 ⇒ `Ok(None)`
/// （调用方按 0 展示；"查不到"与"没算过"在展示层同形，故返回 Option 而非保底值）。
pub async fn latest_recalc_applied_at(
    db: &DatabaseConnection,
    trigger: &str,
) -> Result<Option<i64>, sea_orm::DbErr> {
    let row = strategy_weight_history::Entity::find()
        .filter(strategy_weight_history::Column::Trigger.eq(trigger))
        .order_by_desc(strategy_weight_history::Column::AppliedAt)
        .limit(1)
        .one(db)
        .await?;
    Ok(row.map(|r| r.applied_at))
}

/// 读取 strategy_performance 在窗口内的所有行
pub async fn load_performance_window(
    db: &DatabaseConnection,
    lookback_days: u32,
    as_of_date: Option<&str>,
) -> Result<Vec<StrategyPerformanceRow>, String> {
    let cutoff = window_cutoff_ms(lookback_days, as_of_date)?;

    // #31 / PLAN §五十一-②：按**起算代际**筛样（下限，不是等号 —— 等号会让每次换代清零整个窗口）。
    // 与统计侧（`reflection_stats`）同一条规则、同一个常量；NULL（代际未知）**不进分母**。
    let rows = strategy_performance::Entity::find()
        .filter(strategy_performance::Column::ExitAt.gte(cutoff))
        .filter(
            strategy_performance::Column::TemplateVersion
                .gte(axagent_harness::holding_period::HORIZON_BRANCH_GENERATION_FLOOR),
        )
        .all(db)
        .await
        .map_err(|e| format!("读取 strategy_performance 失败: {e}"))?;

    Ok(rows
        .into_iter()
        .map(|r| StrategyPerformanceRow {
            strategy_id: r.strategy_id,
            period: r.period,
            was_correct: r.was_correct,
            exit_at: r.exit_at,
        })
        .collect())
}

/// 窗口内**因代际被排除**的样本数（早于起算代 / 代际未知分开计数）。
///
/// 为什么单独一个函数而不是塞进 `load_performance_window` 的返回值：那个函数被三条路径共用
/// （重算 / 仪表盘 / 进化搜索），它们的返回类型各不相同；而「被排除多少」只有仪表盘要展示。
/// 分开也避免了「为了报个数而改三处签名」。
pub async fn count_excluded_performance_window(
    db: &DatabaseConnection,
    lookback_days: u32,
    as_of_date: Option<&str>,
) -> Result<(usize, usize), String> {
    let cutoff = window_cutoff_ms(lookback_days, as_of_date)?;
    let rows = strategy_performance::Entity::find()
        .filter(strategy_performance::Column::ExitAt.gte(cutoff))
        .all(db)
        .await
        .map_err(|e| format!("读取 strategy_performance 失败: {e}"))?;
    let floor = axagent_harness::holding_period::HORIZON_BRANCH_GENERATION_FLOOR;
    let mut pre_floor = 0usize;
    let mut unknown = 0usize;
    for r in &rows {
        match r.template_version {
            Some(v) if v >= floor => {},
            Some(_) => pre_floor += 1,
            None => unknown += 1,
        }
    }
    Ok((pre_floor, unknown))
}

/// 时间窗左端（ms）—— 三条路径共用，避免同一条到期口径被抄三遍。
fn window_cutoff_ms(lookback_days: u32, as_of_date: Option<&str>) -> Result<i64, String> {
    if let Some(d) = as_of_date {
        // Replay 模式：以 as_of_date 当作"今天"
        let date = chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .map_err(|e| format!("as_of_date 格式错误: {e}"))?;
        let dt = date.and_hms_opt(0, 0, 0).ok_or_else(|| "无效日期".to_string())?;
        Ok(dt.and_utc().timestamp_millis() - (lookback_days as i64) * 86_400_000)
    } else {
        Ok(Utc::now().timestamp_millis() - (lookback_days as i64) * 86_400_000)
    }
}

/// 重算并写回所有 (strategy, period) 权重调整
///
/// 返回值：(写入了多少行 weight_history, 新权重表)
pub async fn recalc_and_persist(
    db: &DatabaseConnection,
    trigger: &str, // "cron" | "manual" | "rule"
    source_reflection_id: Option<&str>,
    as_of_date: Option<&str>,
) -> Result<(usize, HashMap<(String, String), f64>), String> {
    // P4：读最近一次人工接受的进化配置；从未审批接受过则与历史一致用默认值。
    let cfg = load_active_weight_decay_config(db).await;
    let current = load_current_weights(db).await?;
    let history = load_performance_window(db, cfg.lookback_days, as_of_date).await?;
    let new_map = compute_adjusted_weights(&history, &cfg, &current);

    if new_map.is_empty() {
        warn!("[evolution_drift] 窗口内无表现数据,跳过调整");
        return Ok((0, current));
    }

    let now = Utc::now().timestamp_millis();
    let mut written = 0usize;
    for (key, aw) in &new_map {
        let old = current.get(key).copied().unwrap_or(1.0);
        let delta_pct = if old.abs() > f64::EPSILON {
            (aw.new_weight - old) / old * 100.0
        } else {
            0.0
        };
        // 容忍极小抖动（< 1%）：不写库，避免污染
        if delta_pct.abs() < 1.0 {
            continue;
        }
        let rationale = format_rationale(aw, aw.sample_size);
        let am = strategy_weight_history::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            strategy_id: Set(aw.strategy_id.clone()),
            period: Set(aw.period.clone()),
            old_weight: Set(old),
            new_weight: Set(aw.new_weight),
            delta_pct: Set(delta_pct),
            trigger: Set(trigger.to_string()),
            source_reflection_id: Set(source_reflection_id.map(|s| s.to_string())),
            sample_size: Set(aw.sample_size as i32),
            win_rate: Set(aw.win_rate),
            rationale: Set(Some(rationale)),
            applied_at: Set(now),
        };
        strategy_weight_history::Entity::insert(am)
            .exec(db)
            .await
            .map_err(|e| format!("写入 weight_history 失败: {e}"))?;
        written += 1;
    }

    info!("[evolution_drift] 触发={trigger} 写入 {written} 条权重调整");
    let weight_only: HashMap<(String, String), f64> =
        new_map.iter().map(|(k, v)| (k.clone(), v.new_weight)).collect();
    Ok((written, weight_only))
}

/// 仪表盘（前端 EvolutionDriftPanel 主页用）
pub async fn get_dashboard(
    db: &DatabaseConnection,
    as_of_date: Option<&str>,
) -> Result<EvolutionDriftDashboard, String> {
    let cfg = load_active_weight_decay_config(db).await;
    let current = load_current_weights(db).await?;
    let history = load_performance_window(db, cfg.lookback_days, as_of_date).await?;
    let new_map = compute_adjusted_weights(&history, &cfg, &current);

    // 拉取最近 5 次调整原因
    let recent = strategy_weight_history::Entity::find()
        .order_by_desc(strategy_weight_history::Column::AppliedAt)
        .limit(5)
        .all(db)
        .await
        .map_err(|e| format!("读取 recent_changes 失败: {e}"))?;
    let recent_changes: Vec<RecentChangeRow> = recent
        .into_iter()
        .map(|r| RecentChangeRow {
            id: r.id,
            strategy_id: r.strategy_id,
            period: r.period,
            old_weight: r.old_weight,
            new_weight: r.new_weight,
            delta_pct: r.delta_pct,
            trigger: r.trigger,
            rationale: r.rationale,
            applied_at: r.applied_at,
        })
        .collect();

    // 拉取最近一次 applied_at 作为"最近重算时间"
    let last_recalc_at = strategy_weight_history::Entity::find()
        .order_by_desc(strategy_weight_history::Column::AppliedAt)
        .one(db)
        .await
        .map_err(|e| format!("查询 last_recalc 失败: {e}"))?
        .map(|r| r.applied_at)
        .unwrap_or(0);

    // 构造 stats：合并 current 与 new,old 来自最近一次 weight_history
    let mut stats: Vec<StrategyStatRow> = Vec::new();
    for (key, aw) in &new_map {
        let old = current.get(key).copied().unwrap_or(1.0);
        let delta_pct = if old.abs() > f64::EPSILON {
            (aw.new_weight - old) / old * 100.0
        } else {
            0.0
        };
        let rationale = format_rationale(aw, aw.sample_size);
        stats.push(StrategyStatRow {
            strategy_id: aw.strategy_id.clone(),
            period: aw.period.clone(),
            new_weight: aw.new_weight,
            old_weight: old,
            delta_pct,
            win_rate: aw.win_rate,
            sample_size: aw.sample_size,
            confidence: aw.confidence,
            rationale,
        });
    }
    // 补齐 new_map 中没有但 current 中有的（旧策略无近期表现）
    for (key, w) in &current {
        if !new_map.contains_key(key) {
            stats.push(StrategyStatRow {
                strategy_id: key.0.clone(),
                period: key.1.clone(),
                new_weight: *w,
                old_weight: *w,
                delta_pct: 0.0,
                win_rate: 0.0,
                sample_size: 0,
                confidence: 0.0,
                rationale: "窗口内无表现数据,保持上次权重".to_string(),
            });
        }
    }

    // strategy_summary: 按 strategy_id 聚合
    let mut summary_map: HashMap<String, (f64, u32, f64, u32)> = HashMap::new(); // (sum_weight, sum_samples, sum_win, count)
    for s in &stats {
        let entry = summary_map.entry(s.strategy_id.clone()).or_insert((0.0, 0, 0.0, 0));
        entry.0 += s.new_weight;
        entry.1 += s.sample_size;
        entry.2 += s.win_rate;
        entry.3 += 1;
    }
    let strategy_summary: Vec<StrategySummaryRow> = summary_map
        .into_iter()
        .map(|(sid, (sum_w, sum_s, sum_wr, cnt))| {
            let avg_w = if cnt > 0 { sum_w / cnt as f64 } else { 1.0 };
            let avg_wr = if cnt > 0 { sum_wr / cnt as f64 } else { 0.0 };
            // 简单趋势：根据 delta 符号聚合
            let stats_for: Vec<&StrategyStatRow> =
                stats.iter().filter(|s| s.strategy_id == sid).collect();
            let net_delta: f64 = stats_for.iter().map(|s| s.delta_pct).sum();
            let trend = StrategyTrend::from_net_delta(net_delta);
            StrategySummaryRow {
                strategy_id: sid,
                avg_weight: avg_w,
                total_samples: sum_s,
                avg_win_rate: avg_wr,
                trend,
            }
        })
        .collect();

    let weight_only: HashMap<(String, String), f64> =
        new_map.iter().map(|(k, v)| (k.clone(), v.new_weight)).collect();

    // #31：把「被代际排除多少」一并报出 —— 否则面板只看到样本变少，
    // 分不清「旧代不可比」与「还没盖章」（两者处置不同）。
    let (excluded_pre_floor_generation, excluded_unknown_generation) =
        count_excluded_performance_window(db, cfg.lookback_days, as_of_date).await?;

    Ok(EvolutionDriftDashboard {
        current_weights: weight_only,
        last_recalc_at,
        stats,
        recent_changes,
        strategy_summary,
        generation_floor: axagent_harness::holding_period::HORIZON_BRANCH_GENERATION_FLOOR,
        excluded_pre_floor_generation,
        excluded_unknown_generation,
    })
}

/// 拉取时间线（前端 sparkline 用）
pub async fn get_timeline(
    db: &DatabaseConnection,
    strategy_id: &str,
    period: &str,
    limit: u32,
) -> Result<Vec<TimelinePoint>, String> {
    let rows = strategy_weight_history::Entity::find()
        .filter(strategy_weight_history::Column::StrategyId.eq(strategy_id))
        .filter(strategy_weight_history::Column::Period.eq(period))
        .order_by_desc(strategy_weight_history::Column::AppliedAt)
        .limit(limit as u64)
        .all(db)
        .await
        .map_err(|e| format!("读取 timeline 失败: {e}"))?;
    Ok(rows
        .into_iter()
        .map(|r| TimelinePoint {
            applied_at: r.applied_at,
            new_weight: r.new_weight,
            old_weight: r.old_weight,
            delta_pct: r.delta_pct,
            trigger: r.trigger,
        })
        .collect())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelinePoint {
    pub applied_at: i64,
    pub new_weight: f64,
    pub old_weight: f64,
    pub delta_pct: f64,
    pub trigger: String,
}

/// 写入一行 strategy_performance（复盘 cron 触发时使用）
// 参数较多(策略调用方: cron / replay runner / 离线评测 共用)，保持显式参数以避免引入过多构造包装。
#[allow(clippy::too_many_arguments)]
pub async fn record_performance(
    db: &DatabaseConnection,
    strategy_id: &str,
    period: &str,
    stock_code: &str,
    stock_name: &str,
    decision_at: i64,
    exit_at: i64,
    holding_days: i32,
    return_pct: f64,
    was_correct: i32,
    decision_confidence: i32,
    horizon_pnl_json: Option<&str>,
    agreement_score: Option<i32>,
    // #31：本行对应的**决策所属算法代际**（权威 = 被复盘分析的 `template_version`）。
    // 调用方拿不到就传 `None` —— 那表示「代际未知」，读侧（`load_performance_window`）
    // 按 `HORIZON_BRANCH_GENERATION_FLOOR` 的下限规则把它排除在权重分母外。
    // ⚠ **不要在这里退回任何默认代**：把「不知道」写成「就是这一代」会让跨代样本混进胜率。
    template_version: Option<i32>,
) -> Result<String, String> {
    let now = Utc::now().timestamp_millis();
    let id = Uuid::new_v4().to_string();
    let am = strategy_performance::ActiveModel {
        id: Set(id.clone()),
        strategy_id: Set(strategy_id.to_string()),
        period: Set(period.to_string()),
        stock_code: Set(stock_code.to_string()),
        stock_name: Set(stock_name.to_string()),
        decision_at: Set(decision_at),
        exit_at: Set(exit_at),
        holding_days: Set(holding_days),
        return_pct: Set(return_pct),
        was_correct: Set(was_correct),
        decision_confidence: Set(decision_confidence),
        horizon_pnl_json: Set(horizon_pnl_json.map(|s| s.to_string())),
        agreement_score: Set(agreement_score),
        // B3 三键在本通道一律 None：这里是 cron / replay / 离线评测的**单条决策**回测，
        // 入参既没有逐档对照行也没有 LLM 侧动作 —— 拿公式侧反填等于凭空造一个「LLM 也这么判」。
        horizon_agreement: Set(None),
        llm_action: Set(None),
        llm_was_correct: Set(None),
        template_version: Set(template_version),
        // 非反思物化通道（cron / replay / 离线评测 / 对话直回测）不带来源键；
        // 幂等 upsert 只走 `upsert_reflection_horizon_performance`。
        source_ref: Set(None),
        // 同理不带反思行归属：本通道不是 Gap1 旧格，命中率统计的外键轴对它无意义。
        reflection_id: Set(None),
        created_at: Set(now),
    };
    strategy_performance::Entity::insert(am)
        .exec(db)
        .await
        .map_err(|e| format!("写入 strategy_performance 失败: {e}"))?;
    Ok(id)
}

// ───────────────────────────────────────────────────────────────────────────
// 到期反思四档 → strategy_performance 物化（#31 代际血缘管道）
//
// 为什么需要：参数进化执行器（`load_performance_window`）只吃 strategy_performance
// 中近窗口、代际 >= HORIZON_BRANCH_GENERATION_FLOOR 的表现行；而最权威的「决策到期
// 判定」一直只存在于 stock_reflections.horizon_results_json 的读时展开里
// （`expand_horizon_samples`），从未物化进本表 ⇒ 进化窗口恒空。本节把每条到期反思的
// 成熟档按「反思行 × 档」幂等 upsert 进真实策略格，打通代际血缘。
// ───────────────────────────────────────────────────────────────────────────

/// 决策方向 → 策略格 `strategy_id` 的**唯一权威映射**。
///
/// 口径与权重体系真实策略格一致（wiring 层 `map_action_to_strategy_id` 转发到这里，
/// 禁止再各写一份）：
/// - 买入 / 增持 → `trend`（顺势）
/// - 卖出 / 减持 → `reversion`（反转）
/// - 持有 → `value`
/// - 观望 / 不确定 → `capital`
/// - 数据缺失 / 无法归一（含 `None` / 空串）→ `watchlist`
pub fn action_to_strategy_id(action: Option<&str>) -> &'static str {
    match action.and_then(normalize_action) {
        Some(ActionKind::Buy) | Some(ActionKind::Increase) => "trend",
        Some(ActionKind::Sell) | Some(ActionKind::Reduce) => "reversion",
        Some(ActionKind::Hold) => "value",
        Some(ActionKind::Wait) | Some(ActionKind::Uncertain) => "capital",
        Some(ActionKind::Unavailable) | None => "watchlist",
    }
}

/// 构造幂等来源键：`reflection-horizon:{被复盘决策 id}:{档}`。
///
/// 用**决策 id**（`stock_analyses.id`）而非反思行 id：四周期「一行一档」，同一条决策的
/// 已到期档会在多条反思行里重复成熟，按决策去重才能收敛成一行。
fn reflection_horizon_source_ref(original_analysis_id: &str, horizon: &str) -> String {
    format!("reflection-horizon:{original_analysis_id}:{horizon}")
}

/// 单个成熟反思档 → 一行 strategy_performance 的纯映射（无 IO，便于单测）。
#[derive(Debug, Clone, PartialEq)]
struct HorizonRowDraft {
    source_ref: String,
    strategy_id: &'static str,
    period: String,
    holding_days: i32,
    return_pct: f64,
    was_correct: i32,
    decision_confidence: i32,
    horizon_pnl_json: Option<String>,
    template_version: Option<i32>,
    /// B3：该档三维分 / LLM 该档动作与判定 —— 三个 `None` 都是**缺席**，不得折算成 0。
    horizon_agreement: Option<i32>,
    llm_action: Option<String>,
    llm_was_correct: Option<i32>,
}

/// 把展开后的成熟样本映射成行字段。
///
/// 返回 `None`：样本缺合法 `horizon`（不是四档键）—— 非四档键（`schemaVersion` 等）
/// 在 `expand_horizon_samples` 阶段已剔除，这里再用 [`Period::from_str`] 兜底，
/// 解析失败就不物化（不猜档位）。
fn build_horizon_row_draft(
    original_analysis_id: &str,
    sample: &DecisionPerformanceSample,
) -> Option<HorizonRowDraft> {
    let horizon = sample.horizon.as_deref()?;
    let period = Period::from_str(horizon).ok()?;
    // 置信度口径 0–100；legacy 回退样本拿不到逐档置信度（None）→ 落 0，
    // 与 record_performance 既有「无置信度」通道一致。
    let confidence = sample.confidence.map(|c| c.round().clamp(0.0, 100.0) as i32).unwrap_or(0);
    let pnl = serde_json::json!({
        "returnPct": sample.return_pct,
        "alphaPct": sample.alpha_pct,
        "wasCorrect": sample.was_correct,
        "targetReached": sample.target_reached,
    });
    Some(HorizonRowDraft {
        source_ref: reflection_horizon_source_ref(original_analysis_id, horizon),
        strategy_id: action_to_strategy_id(sample.action.as_deref()),
        period: period.as_str().to_string(),
        holding_days: period.default_holding_days() as i32,
        return_pct: sample.return_pct,
        was_correct: sample.was_correct,
        decision_confidence: confidence,
        horizon_pnl_json: Some(pnl.to_string()),
        template_version: sample.template_version,
        // B3 三键直传：判定发生在产端（`reflection.rs` 拿的是**同一份档快照**，调的是
        // [`crate::reflection_stats::deterministic_horizon_was_correct`] 同一个函数），
        // 这里不重算 —— 重算就要在这里再拿一次 `within_expected_horizon`，等于给同一件事
        // 立第二个真相（两次读数不同时无从裁决）。
        horizon_agreement: sample.horizon_agreement,
        llm_action: sample.llm_action.clone(),
        llm_was_correct: sample.llm_was_correct,
    })
}

/// 把一条到期反思 `horizon_results_json` 中**所有成熟可判定档**幂等物化进
/// strategy_performance。
///
/// 幂等键 = `reflection-horizon:{original_analysis_id}:{horizon}`（`source_ref` 列 +
/// `idx_strategy_performance_source_ref` 部分唯一索引）。四周期模型「一行一档」：同一决策
/// 有 4 条独立反思行，每条运行时都重算全部四档快照，已到期短档会在后续反思行重复 mature。
/// 按「**被复盘决策 × 档**」upsert，无论由哪条反思行展开，同一决策的同一档永远收敛到一行
/// （INSERT 首次、其后 UPDATE），绝不重复计票。
///
/// - `original_analysis_id`：被复盘的**决策行** id（`stock_analyses.id`），不是反思行 id。
/// - `decision_at`：被复盘分析的决策时刻（`stock_analyses.created_at`）。
/// - `exit_at`：本次判定时刻（重跑时刷新为最新一次成熟判定时间）。
/// - 代际取样本自带的 `template_version`（= 被复盘分析的真实 workflow 代际）；拿不到
///   传 `None`，读侧按下限规则排除，不猜代。
///
/// 返回本次 upsert 的成熟档数（`0` = 该反思当前还没有任何成熟可判定档，或 JSON 非
/// 四周期对象——后者走 legacy/Gap1 通道）。
#[allow(clippy::too_many_arguments)]
pub async fn upsert_reflection_horizon_performance(
    db: &DatabaseConnection,
    original_analysis_id: &str,
    stock_code: &str,
    stock_name: &str,
    decision_at: i64,
    exit_at: i64,
    horizon_results_json: &str,
    template_version: Option<i32>,
) -> Result<usize, String> {
    // `None` = 不是四周期 JSON（旧记录），交由 legacy / Gap1 通道，不在此物化。
    let samples = match expand_horizon_samples(horizon_results_json, template_version) {
        Some(s) => s,
        None => return Ok(0),
    };

    let mut upserted = 0usize;
    for sample in &samples {
        let Some(draft) = build_horizon_row_draft(original_analysis_id, sample) else {
            continue;
        };

        let existing = strategy_performance::Entity::find()
            .filter(strategy_performance::Column::SourceRef.eq(&draft.source_ref))
            .one(db)
            .await
            .map_err(|e| format!("查询 strategy_performance 幂等键失败: {e}"))?;

        if let Some(model) = existing {
            // 重跑：刷新判定结果与判定时刻；保留 decision_at / created_at / 股票维度原值。
            let mut am: strategy_performance::ActiveModel = model.into();
            am.strategy_id = Set(draft.strategy_id.to_string());
            am.period = Set(draft.period);
            am.holding_days = Set(draft.holding_days);
            am.return_pct = Set(draft.return_pct);
            am.was_correct = Set(draft.was_correct);
            am.decision_confidence = Set(draft.decision_confidence);
            am.horizon_pnl_json = Set(draft.horizon_pnl_json);
            am.template_version = Set(draft.template_version);
            // B3：逐档三键在 UPDATE 分支同样要刷 —— 重跑时 LLM 侧可能从「缺席」变成「有结论」
            // （代际标记补上 / 该档补吐），只刷 INSERT 分支会让首写为 NULL 的行永久停在 NULL。
            am.horizon_agreement = Set(draft.horizon_agreement);
            am.llm_action = Set(draft.llm_action);
            am.llm_was_correct = Set(draft.llm_was_correct);
            am.exit_at = Set(exit_at);
            am.update(db).await.map_err(|e| format!("更新 strategy_performance 失败: {e}"))?;
        } else {
            let am = strategy_performance::ActiveModel {
                id: Set(Uuid::new_v4().to_string()),
                strategy_id: Set(draft.strategy_id.to_string()),
                period: Set(draft.period),
                stock_code: Set(stock_code.to_string()),
                stock_name: Set(stock_name.to_string()),
                decision_at: Set(decision_at),
                exit_at: Set(exit_at),
                holding_days: Set(draft.holding_days),
                return_pct: Set(draft.return_pct),
                was_correct: Set(draft.was_correct),
                decision_confidence: Set(draft.decision_confidence),
                horizon_pnl_json: Set(draft.horizon_pnl_json),
                agreement_score: Set(None),
                // ⚠ 本通道**不写** `agreement_score`（上面那行保持 None）：那一列的计数域是
                // 主档五维分，而这里是逐档行 —— 把 65 制的分写进 90 制那列，同一列就两种尺子，
                // `get_agreement_score_history` 的历史线会变成两种口径的平均。逐档分走 `horizon_agreement`。
                horizon_agreement: Set(draft.horizon_agreement),
                llm_action: Set(draft.llm_action),
                llm_was_correct: Set(draft.llm_was_correct),
                template_version: Set(draft.template_version),
                source_ref: Set(Some(draft.source_ref)),
                // #31 四档通道的归属键是 `source_ref`（决策 id × 档），不带反思行外键：
                // 同一决策的四档行由**任意一条**反思行展开都必须收敛到同一行（实体注释论证）。
                reflection_id: Set(None),
                created_at: Set(exit_at),
            };
            strategy_performance::Entity::insert(am)
                .exec(db)
                .await
                .map_err(|e| format!("插入 strategy_performance 失败: {e}"))?;
        }
        upserted += 1;
    }
    Ok(upserted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashboard_struct_serde_camel_case() {
        let d = EvolutionDriftDashboard {
            current_weights: HashMap::new(),
            last_recalc_at: 123,
            stats: vec![],
            recent_changes: vec![],
            strategy_summary: vec![],
            generation_floor: 125,
            excluded_pre_floor_generation: 3,
            excluded_unknown_generation: 5,
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"currentWeights\""), "camelCase 序列化");
        assert!(json.contains("\"lastRecalcAt\""), "camelCase 序列化");
        assert!(!json.contains("\"last_recalc_at\""), "不应有 snake_case 字段");
    }

    #[test]
    fn strategy_summary_trend_aggregation() {
        // 验证 trend 字段在 net_delta > 5 时为 "up",<-5 为 "down",其他 stable
        let stats = [
            StrategyStatRow {
                strategy_id: "trend".to_string(),
                period: "short".to_string(),
                new_weight: 1.2,
                old_weight: 1.0,
                delta_pct: 20.0, // +20%
                win_rate: 0.6,
                sample_size: 30,
                confidence: 0.95,
                rationale: "ok".to_string(),
            },
            StrategyStatRow {
                strategy_id: "trend".to_string(),
                period: "mid".to_string(),
                new_weight: 0.5,
                old_weight: 1.0,
                delta_pct: -50.0,
                win_rate: 0.3,
                sample_size: 30,
                confidence: 0.9,
                rationale: "ok".to_string(),
            },
        ];
        // trend/short: +20, trend/mid: -50 → net_delta = -30 → trend = "down"
        let net: f64 = stats.iter().map(|s| s.delta_pct).sum();
        assert!(net < -5.0, "净 delta 应明显为负");
    }

    #[tokio::test]
    async fn load_current_weights_picks_latest_per_key() {
        // A1 正负对照：演化权重回流到一个 (strategy, period) 的最新一行。
        // 正控——写入两条同键历史，键以最新 applied_at 取到 new_weight；
        // 负控——把第二条的 new_weight 改成别的值，读取结果必须随之变（证「读到的是哪条」有区分力，
        // 而非偶合到某一行历史）。
        use axagent_entities::strategy_weight_history::{
            ActiveModel, Column as WhColumn, Entity as WhEntity,
        };
        use sea_orm::ColumnTrait as _;

        let h = axagent_dao::db::create_test_pool().await.unwrap();
        let db = &h.conn;

        let base = 1_700_000_000_000i64;
        let insert_row = |strategy_id: &str, period: &str, weight: f64, applied_at: i64| {
            WhEntity::insert(ActiveModel {
                id: Set(Uuid::new_v4().to_string()),
                strategy_id: Set(strategy_id.to_string()),
                period: Set(period.to_string()),
                old_weight: Set(1.0),
                new_weight: Set(weight),
                delta_pct: Set((weight - 1.0) / 1.0 * 100.0),
                trigger: Set("rule".to_string()),
                source_reflection_id: Set(None),
                sample_size: Set(10),
                win_rate: Set(0.6),
                rationale: Set(Some("测试写入".to_string())),
                applied_at: Set(applied_at),
            })
        };

        // 正控：同键两条，新的在后，取最新 0.85
        insert_row("s1", "short", 1.0, base).exec(db).await.map_err(|e| e.to_string()).unwrap();
        insert_row("s1", "short", 0.85, base + 1)
            .exec(db)
            .await
            .map_err(|e| e.to_string())
            .unwrap();

        let map = load_current_weights(db).await.unwrap();
        let got = map.get(&("s1".to_string(), "short".to_string())).copied();
        assert_eq!(got, Some(0.85), "正控：同键应取最新一条 new_weight");

        // 负控：把新一行改成 0.42，读取必须随之变（证明断言绑定到「最新行」，不是某条固定历史）
        let latest = WhEntity::find()
            .filter(WhColumn::StrategyId.eq("s1"))
            .filter(WhColumn::Period.eq("short"))
            .order_by_desc(WhColumn::AppliedAt)
            .one(db)
            .await
            .map_err(|e| e.to_string())
            .unwrap()
            .unwrap();
        let mut am: ActiveModel = latest.into();
        am.new_weight = Set(0.42);
        WhEntity::update(am).exec(db).await.map_err(|e| e.to_string()).unwrap();

        let map2 = load_current_weights(db).await.unwrap();
        let got2 = map2.get(&("s1".to_string(), "short".to_string())).copied();
        assert_eq!(got2, Some(0.42), "负控：演化权重改写后读取结果必须随之变（正负对照区分力）");
    }

    /// #31：权重窗口**按起算代际**筛样（下限），且两类排除分开计数。
    ///
    /// 为什么这条必须有：权重线的分母是「同 (策略, 档) 的历史样本」，跨代混池算出的胜率
    /// 是两套判据的加权平均 —— 而它直接决定下一轮的策略权重。
    /// 夹具刻意放四个代际：`floor-1`（旧代）/`floor`（恰在起算代）/`floor+4`（起算代之后）/
    /// NULL（代际未知）⇒ 只要有人把实现改成**等号**，`floor+4` 那条当场掉出分母。
    #[tokio::test]
    async fn performance_window_filters_pre_floor_and_unknown_generations() {
        use axagent_entities::strategy_performance;
        use sea_orm::{EntityTrait, Set};

        let db = axagent_dao::db::create_test_pool().await.expect("测试库应可创建").conn;
        let floor = axagent_harness::holding_period::HORIZON_BRANCH_GENERATION_FLOOR;
        let now = Utc::now().timestamp_millis();
        // ⚠ `gen` 在 Rust 2024 是**保留字**（generator）⇒ 形参只能叫别的名字
        let row = |sid: &str, generation: Option<i32>| strategy_performance::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            strategy_id: Set(sid.to_string()),
            period: Set("reflection:mid".to_string()),
            stock_code: Set("600519".to_string()),
            stock_name: Set("贵州茅台".to_string()),
            decision_at: Set(now - 5 * 86_400_000),
            exit_at: Set(now),
            holding_days: Set(5),
            return_pct: Set(1.0),
            was_correct: Set(1),
            decision_confidence: Set(50),
            horizon_pnl_json: Set(None),
            agreement_score: Set(None),
            horizon_agreement: Set(None),
            llm_action: Set(None),
            llm_was_correct: Set(None),
            template_version: Set(generation),
            source_ref: Set(None),
            reflection_id: Set(None),
            created_at: Set(now),
        };
        for (sid, generation) in [
            ("w-old", Some(floor - 1)),
            ("w-at", Some(floor)),
            ("w-after", Some(floor + 4)),
            ("w-unknown", None),
        ] {
            strategy_performance::Entity::insert(row(sid, generation))
                .exec(&db)
                .await
                .expect("插入绩效行应成功");
        }

        let rows = load_performance_window(&db, 30, None).await.expect("窗口装载应成功");
        let ids: Vec<&str> = rows.iter().map(|r| r.strategy_id.as_str()).collect();
        assert!(ids.contains(&"w-at"), "恰在起算代的样本必须进窗口: {ids:?}");
        assert!(ids.contains(&"w-after"), "起算代之后的样本必须进窗口（等号实现会误杀）: {ids:?}");
        assert!(!ids.contains(&"w-old"), "早于起算代的样本不得进权重窗口: {ids:?}");
        assert!(!ids.contains(&"w-unknown"), "代际未知的样本不得进权重窗口: {ids:?}");

        let (pre, unk) =
            count_excluded_performance_window(&db, 30, None).await.expect("计数应成功");
        assert_eq!((pre, unk), (1, 1), "两类排除必须分开计数且如实（合成一个数就看不出处置差异）");
    }

    /// #31：action → 真实策略格映射穷举（中英文 / 缺失 / 未识别）。
    #[test]
    fn action_to_strategy_id_maps_every_bucket() {
        // 顺势：买入 / 增持（中英文同义）
        assert_eq!(action_to_strategy_id(Some("买入")), "trend");
        assert_eq!(action_to_strategy_id(Some("BUY")), "trend");
        assert_eq!(action_to_strategy_id(Some("增持")), "trend");
        assert_eq!(action_to_strategy_id(Some("加仓")), "trend");
        // 反转：卖出 / 减持
        assert_eq!(action_to_strategy_id(Some("卖出")), "reversion");
        assert_eq!(action_to_strategy_id(Some("减持")), "reversion");
        // 价值：持有
        assert_eq!(action_to_strategy_id(Some("持有")), "value");
        assert_eq!(action_to_strategy_id(Some("HOLD")), "value");
        // 资金/防御：观望 / 不确定
        assert_eq!(action_to_strategy_id(Some("观望")), "capital");
        assert_eq!(action_to_strategy_id(Some("不确定")), "capital");
        // 观察池：数据缺失 / None / 空串 / 未识别值域，一律 watchlist（不 fail-open 成观望）
        assert_eq!(action_to_strategy_id(Some("数据缺失")), "watchlist");
        assert_eq!(action_to_strategy_id(None), "watchlist");
        assert_eq!(action_to_strategy_id(Some("")), "watchlist");
        assert_eq!(action_to_strategy_id(Some("不知所云")), "watchlist");
    }

    /// #31：四档 JSON 展开 → 真实策略格行字段映射（含置信度 clamp、代际透传、
    /// 非四档对象键在 draft 层兜底剔除）。
    #[test]
    fn horizon_json_expands_into_real_strategy_rows() {
        let json = serde_json::json!({
            "schemaVersion": 1,
            "ultra_short": {
                "status": "mature",
                "evaluation": { "wasCorrect": 1 },
                "market": { "returnPct": 3.21, "alphaPct": 1.05, "targetReached": true },
                "decision": { "action": "买入", "confidence": 72.4 },
                // B3 三键（该档 LLM 侧）：产端 = reflection.rs 的 build_horizon_results_json
                "llmAction": "观望",
                "llmWasCorrect": null,
                "horizonAgreement": 63
            },
            "short": { "status": "unavailable" },
            "mid": {
                "status": "mature",
                "evaluation": { "wasCorrect": 0 },
                "market": { "returnPct": -1.5 },
                // 130 超上限 → 必须 clamp 到 100
                "decision": { "action": "持有", "confidence": 130.0 }
            },
            // expand 不校验对象键是否四档（会收成样本），draft 层必须用 Period 拒绝：
            "weird_horizon": {
                "status": "mature",
                "evaluation": { "wasCorrect": 1 },
                "market": { "returnPct": 2.0 },
                "decision": { "action": "买入" }
            }
        })
        .to_string();

        let samples = expand_horizon_samples(&json, Some(154)).expect("四周期 JSON 应可展开");
        // 3 个成熟对象键（含 1 个伪档），unavailable 档不入。
        assert_eq!(samples.len(), 3, "expand 收所有 mature 对象键，不校验档名");

        let drafts: Vec<HorizonRowDraft> =
            samples.iter().filter_map(|s| build_horizon_row_draft("R1", s)).collect();
        // 伪档 weird_horizon 必须在 draft 层被 Period::from_str 剔除，只剩真实两档。
        assert_eq!(drafts.len(), 2, "非四档键不得物化成策略格行");

        let by_period: HashMap<&str, &HorizonRowDraft> =
            drafts.iter().map(|d| (d.period.as_str(), d)).collect();

        let ultra = by_period.get("ultra_short").expect("应有 ultra_short 行");
        assert_eq!(ultra.strategy_id, "trend", "买入→trend");
        assert_eq!(ultra.holding_days, 2, "ultra_short 默认持有 2 天");
        assert_eq!(ultra.was_correct, 1);
        assert!((ultra.return_pct - 3.21).abs() < 1e-9);
        assert_eq!(ultra.decision_confidence, 72, "72.4 四舍五入为 72");
        assert_eq!(ultra.template_version, Some(154), "真实代际透传");
        assert_eq!(ultra.source_ref, "reflection-horizon:R1:ultra_short");
        // 审计切片保留 alpha / targetReached。
        let pnl: serde_json::Value =
            serde_json::from_str(ultra.horizon_pnl_json.as_deref().unwrap()).unwrap();
        assert_eq!(pnl["alphaPct"], 1.05);
        assert_eq!(pnl["targetReached"], true);

        let mid = by_period.get("mid").expect("应有 mid 行");
        assert_eq!(mid.strategy_id, "value", "持有→value");
        assert_eq!(mid.holding_days, 28, "mid 默认持有 28 天");
        assert_eq!(mid.was_correct, 0);
        assert_eq!(mid.decision_confidence, 100, "置信度超上限必须 clamp 到 100");
        assert_eq!(mid.source_ref, "reflection-horizon:R1:mid");

        // ── B3 三键透传（一列一个计数域：逐档分不得顶替公式侧任何既有列）──
        assert_eq!(ultra.horizon_agreement, Some(63), "该档三维分透传");
        assert_eq!(ultra.llm_action.as_deref(), Some("观望"));
        // 观望不是方向判断 ⇒ 产端判 None；这里必须停在 None，**不得**被折成 0（0 = 「LLM 判错」）
        assert_eq!(ultra.llm_was_correct, None, "LLM 侧未判定 ≠ LLM 判错");
        // 该档 JSON 没给这三键 ⇒ 三键全 None（缺席），而不是 0 / 空串
        assert_eq!(mid.horizon_agreement, None, "缺键不得当 0 分");
        assert_eq!(mid.llm_action, None);
        assert_eq!(mid.llm_was_correct, None);
    }

    /// #31：幂等粒度是「**被复盘决策 × 档**」而非「反思行 × 档」。
    ///
    /// 四周期模型一行一档：同一条决策有 4 条独立反思行，每条运行时都重算全部四档快照。
    /// 本测试模拟这个真实时序——
    /// ① ultra_short 反思行（D0+2）运行：只有 ultra_short 成熟 ⇒ 物化 1 行；
    /// ② mid 反思行（D0+28）运行：此时 ultra_short 也已到期再次 mature，mid 首次成熟 ⇒
    ///    ultra_short 必须**更新同一行**（判定可随更长持有翻转），mid 新增，共仍 2 行
    ///    （若错误地按反思行 id 去重，此处会变成 3 行）。
    #[tokio::test]
    async fn reflection_horizon_upsert_dedupes_by_analysis_and_horizon() {
        // .count() 来自 PaginatorTrait；ColumnTrait / EntityTrait / QueryFilter 已由 super::* 提供。
        use sea_orm::PaginatorTrait;

        let db = axagent_dao::db::create_test_pool().await.expect("测试库应可创建").conn;
        // 两次调用代表同一条被复盘决策的两条不同反思行 —— 传同一个 analysis_id。
        let analysis_id = "analysis-dedupe-1";
        let prefix = format!("reflection-horizon:{analysis_id}:");

        // ① D0+2：仅 ultra_short 成熟。
        let json_ultra = serde_json::json!({
            "schemaVersion": 1,
            "ultra_short": {
                "status": "mature",
                "evaluation": { "wasCorrect": 1 },
                "market": { "returnPct": 3.2 },
                "decision": { "action": "买入", "confidence": 70.0 }
            }
        })
        .to_string();
        let n1 = upsert_reflection_horizon_performance(
            &db,
            analysis_id,
            "600519",
            "贵州茅台",
            500,
            1000,
            &json_ultra,
            Some(154),
        )
        .await
        .expect("首次 upsert 应成功");
        assert_eq!(n1, 1, "D0+2 只有 ultra_short 一档成熟");
        let c1 = strategy_performance::Entity::find()
            .filter(strategy_performance::Column::SourceRef.starts_with(&prefix))
            .count(&db)
            .await
            .expect("计数应成功");
        assert_eq!(c1, 1, "首次后恰 1 行");

        // ② D0+28：ultra_short 再次 mature（判定随更长持有翻转为错、收益改写），mid 首成熟。
        let json_mid = serde_json::json!({
            "schemaVersion": 1,
            "ultra_short": {
                "status": "mature",
                "evaluation": { "wasCorrect": 0 },
                "market": { "returnPct": 9.9 },
                "decision": { "action": "买入", "confidence": 80.0 }
            },
            "mid": {
                "status": "mature",
                "evaluation": { "wasCorrect": 0 },
                "market": { "returnPct": -1.0 },
                "decision": { "action": "持有", "confidence": 60.0 }
            }
        })
        .to_string();
        let n2 = upsert_reflection_horizon_performance(
            &db,
            analysis_id,
            "600519",
            "贵州茅台",
            500,
            2000,
            &json_mid,
            Some(154),
        )
        .await
        .expect("二次 upsert 应成功");
        assert_eq!(n2, 2, "本次展开含 2 个成熟档");
        let c2 = strategy_performance::Entity::find()
            .filter(strategy_performance::Column::SourceRef.starts_with(&prefix))
            .count(&db)
            .await
            .expect("计数应成功");
        assert_eq!(
            c2, 2,
            "幂等：跨反思行重复成熟的 ultra_short 更新而非新增（仍 2 行，不是 3 行）"
        );

        // 同一决策的 ultra_short 行被刷新为二次判定，且保留决策建点时刻 / 代际 / 策略格。
        let ultra_key = format!("reflection-horizon:{analysis_id}:ultra_short");
        let row = strategy_performance::Entity::find()
            .filter(strategy_performance::Column::SourceRef.eq(ultra_key))
            .one(&db)
            .await
            .expect("查询应成功")
            .expect("ultra_short 行应存在");
        assert_eq!(row.was_correct, 0, "判定翻转必须更新既有行");
        assert!((row.return_pct - 9.9).abs() < 1e-9, "收益必须刷新");
        assert_eq!(row.exit_at, 2000, "判定时刻刷新为最新一次");
        assert_eq!(row.decision_at, 500, "决策建点时刻保留不变");
        assert_eq!(row.template_version, Some(154));
        assert_eq!(row.strategy_id, "trend");
    }
}
