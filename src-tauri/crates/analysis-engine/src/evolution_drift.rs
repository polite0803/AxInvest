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

use chrono::Utc;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

use axagent_entities::{strategy_performance, strategy_weight_history};

use crate::types::StrategyTrend;
use crate::weight_decay::{
    compute_adjusted_weights, format_rationale, StrategyPerformanceRow, WeightDecayConfig,
};

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
    let cfg = WeightDecayConfig::default();
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
    let cfg = WeightDecayConfig::default();
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
        template_version: Set(template_version),
        created_at: Set(now),
    };
    strategy_performance::Entity::insert(am)
        .exec(db)
        .await
        .map_err(|e| format!("写入 strategy_performance 失败: {e}"))?;
    Ok(id)
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
            template_version: Set(generation),
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
}
