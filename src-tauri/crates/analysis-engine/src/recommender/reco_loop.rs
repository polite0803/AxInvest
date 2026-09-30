//! 荐股链反思优化闭环 —— 样本层（Phase A，见 `PLAN-reco-reflection-closure.md`）
//!
//! ## 为什么荐股链要有自己的样本通路
//!
//! 既有「胜率线」（evolution_drift B1）的输入 `strategy_performance` 只有分析链写入：
//! `backtest_analysis` 用 `map_action_to_strategy_id` 把**决策 action** 映射成与荐股
//! Style 同名但语义无关的 strategy_id，再被 B1 覆盖进 `reco_strategy_weights` ——
//! 荐股消费到的权重实际是「分析链买卖观望的胜率」（归属错位 D1）。本层从荐股链
//! **自己**的实现结果 `decision_validations`（逐 pick 带真 style×period）构造样本，
//! 供 Phase B 的逐格权重合成消费；不落 `strategy_performance`，避免与分析链伪风格行同表混码。
//!
//! ## 口径（全部复用唯一判定源，禁区 12）
//!
//! - 胜负判定：[`crate::hit_rate_backtest::hit_outcome_to_binary_outcome`]
//!   （hit/partial ⇒ win；miss/false_hit ⇒ loss；insufficient/None ⇒ 剔除）——
//!   与命中率统计分母同口径，不另写一份「partial 算不算对」。
//! - 风格名目：[`crate::recommender::style_matrix`]（serenity ⇔ bottleneck 等别名经
//!   `db_style_aliases` 合并，一名两写不得各算一套）。
//! - 档位：`Period::from_str` 同时读 `ultra_short` 与历史存档 `ultrashort`。
//! - **一 pick 一样本**：同一 pick 在多个 T+N 窗口（默认 5/20/60）各有一行验证时，
//!   只取 `t_plus_n` 最接近该档 `default_holding_days` 的一行（Q5 按档对齐；并列取更短窗，
//!   保守）。否则同一决策会被重复计入胜率分母，且实现收益口径混档。

use std::collections::HashMap;

use sea_orm::{DatabaseConnection, EntityTrait, Set};
use serde::Serialize;

use axagent_entities::decision_validations;

use crate::hit_rate_backtest::hit_outcome_to_binary_outcome;
use crate::recommender::style_matrix::{db_style_aliases, style_keys};
use crate::recommender::types::Period;
use crate::weight_decay::StrategyPerformanceRow;

/// 一条荐股链自己的已验证决策（胜率行 + IC 配对的统一样本）。
#[derive(Debug, Clone, PartialEq)]
pub struct RecoLoopSample {
    /// 矩阵名目（`serenity`，不是落库写法 `bottleneck`）——逐格权重的行键
    pub style_key: &'static str,
    pub period: Period,
    /// 验证时刻（ms）——按档 lookback 的过滤轴
    pub exit_at_ms: i64,
    /// 胜负（hit/partial ⇒ true）
    pub win: bool,
    /// 预测时的置信度 0-100（IC 的 x 轴）
    pub confidence: f64,
    /// 实现收益（%），按所选验证窗口（IC 的 y 轴）
    pub realized_return_pct: f64,
    /// 实际使用的验证窗口（天），半衰期拟合横轴
    pub holding_days: u32,
}

/// 逐格闭环态（Phase B 计算、Phase D 呈现共用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopCellStatus {
    /// 样本不足（n < IC_MIN_SAMPLE）⇒ 基线 1.0，显式「未校准」
    InsufficientSamples,
    /// IC 不可测（置信一侧方差退化）⇒ 基线 1.0，显式「未校准」
    IcUnmeasurable,
    /// 按设计不出票（矩阵不成立格）⇒ 不参与闭环
    NotInMatrix,
    /// IC 非负 ⇒ 仅胜率线降权通路（无提权）
    IcNonNegative,
    /// IC < 0 ⇒ 胜率权重再乘负 IC 惩罚（只降不升）
    DemotedNegativeIc,
}

impl LoopCellStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            LoopCellStatus::InsufficientSamples => "insufficient_samples",
            LoopCellStatus::IcUnmeasurable => "ic_unmeasurable",
            LoopCellStatus::NotInMatrix => "not_in_matrix",
            LoopCellStatus::IcNonNegative => "ic_non_negative",
            LoopCellStatus::DemotedNegativeIc => "demoted_negative_ic",
        }
    }
}

/// 把 decision_validations 的 Model 行归一化成闭环样本（纯函数，供单测直接喂构造行）。
///
/// 丢弃规则（每条都会在此注释处对应一个测试）：
/// - 风格不在矩阵名目（含别名）⇒ 丢（策略链之外的写入方不进闭环）
/// - period 解析失败 ⇒ 丢
/// - hit_outcome 无 win/loss 判定 ⇒ 丢
/// - 同一 pick_id 多窗口 ⇒ 只留最贴近该档持有天数的一行
pub fn samples_from_validation_rows(rows: Vec<decision_validations::Model>) -> Vec<RecoLoopSample> {
    // (pick_id, 候选样本, 窗口与该档默认天数的距离) —— 同 pick 取距离最小、并列取短窗
    let mut best: std::collections::HashMap<String, (RecoLoopSample, i64)> =
        std::collections::HashMap::new();
    let keys = style_keys();

    for r in rows {
        let Some(style_key) =
            keys.iter().copied().find(|k| db_style_aliases(k).contains(&r.style.as_str()))
        else {
            continue;
        };
        let Ok(period) = r.period.parse::<Period>() else { continue };
        let Some(binary) = hit_outcome_to_binary_outcome(r.hit_outcome.as_deref()) else {
            continue;
        };
        let Some(after) = r.t_plus_n_price else { continue };
        if !after.is_finite() || !r.entry_price.is_finite() || r.entry_price <= 0.0 {
            continue;
        }
        let Some(exit_at_ms) = parse_ms(&r.validated_at).or_else(|| parse_ms(&r.created_at)) else {
            continue;
        };
        let t_plus_n = r.t_plus_n.max(0) as u32;
        let sample = RecoLoopSample {
            style_key,
            period,
            exit_at_ms,
            win: binary == "win",
            confidence: r.confidence as f64,
            realized_return_pct: (after / r.entry_price - 1.0) * 100.0,
            holding_days: t_plus_n,
        };
        let dist = (t_plus_n as i64 - period.default_holding_days() as i64).abs();
        match best.get(&r.pick_id) {
            Some((_, d)) if *d <= dist => {},
            _ => {
                best.insert(r.pick_id.clone(), (sample, dist));
            },
        }
    }
    best.into_values().map(|(s, _)| s).collect()
}

impl RecoLoopSample {
    /// 喂给 `weight_decay::compute_adjusted_weights` 的胜率行形态。
    pub fn to_performance_row(&self) -> StrategyPerformanceRow {
        StrategyPerformanceRow {
            strategy_id: self.style_key.to_string(),
            period: self.period.as_str().to_string(),
            was_correct: i32::from(self.win),
            exit_at: self.exit_at_ms,
        }
    }

    /// 喂给 `ic::aggregate_reco_ic` 的 IC 配对行形态（holding_days 即所选验证窗口）。
    pub fn to_ic_row(&self) -> crate::recommender::ic::RecoIcRow {
        crate::recommender::ic::RecoIcRow {
            style: self.style_key.to_string(),
            period: self.period.as_str().to_string(),
            confidence: self.confidence,
            realized_return_pct: self.realized_return_pct,
            holding_days: self.holding_days,
        }
    }
}

/// rfc3339 与「YYYY-MM-DDTHH:MM:SS.fff」（reco_picks.generated_at 形态）两种时间写法都读。
fn parse_ms(s: &str) -> Option<i64> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(|dt| dt.and_utc().timestamp_millis())
}

/// 从库中读取全部已验证的荐股样本（薄封装；归一化与判定全在纯函数里）。
pub async fn load_reco_loop_samples(
    db: &DatabaseConnection,
) -> Result<Vec<RecoLoopSample>, String> {
    let rows = decision_validations::Entity::find()
        .all(db)
        .await
        .map_err(|e| format!("读取 decision_validations 失败: {e}"))?;
    Ok(samples_from_validation_rows(rows))
}

// ── Phase B：胜率 × IC 合成逐格权重（PLAN-reco-reflection-closure）──

/// 闭环留痕的 trigger 标记（`strategy_weight_history.trigger`）。
/// 与分析链演化的 `"cron"|"manual"|"rule"` 分键空间消费（Q3 归属分离）。
pub const RECO_LOOP_TRIGGER: &str = "reco-loop";

/// 基线权重：闭环未校准格的取值，也是「只降不升」（Q1）的上限。
pub const BASELINE_WEIGHT: f64 = 1.0;

/// 负 IC 格的额外惩罚乘数（rank IC < 0 ⇒ 该格预测力为反向，胜率权重再乘此系数）。
/// 之所以是常数而不是可调变量：只降不升 + shadow 起步下它只影响「降多少」，
/// 转正判据（Phase E A/B）看的是相对次序而不是该系数。
pub const NEGATIVE_IC_PENALTY: f64 = 0.5;

/// 权重下限（与 `weight_decay` 的 clamp 下限同值，不另造常量）。
const WEIGHT_FLOOR: f64 = 0.05;

/// 一个 (风格, 档位) 格的闭环计算结果（Phase D 呈现层的行来源）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopCellResult {
    /// 矩阵名目（`serenity`，非落库写法）
    pub style: &'static str,
    pub period: &'static str,
    pub status: LoopCellStatus,
    pub rank_ic: Option<f64>,
    /// 该档验证窗口 lookback 内的成对样本数
    pub samples: usize,
    pub win_rate: Option<f64>,
    pub old_weight: f64,
    pub new_weight: f64,
}

/// 逐格合成（纯函数）：胜率线（`weight_decay` 基元）× 力度线（`ic` 基元）。
///
/// 规则（对齐已批方案 Q1/P2/Q5）：
/// - 矩阵不成立格 ⇒ `NotInMatrix`，不产权重；
/// - 该档窗口（`default_holding_days`，Q5 按档对齐）内样本 < `IC_MIN_SAMPLE` ⇒ 基线 1.0（`InsufficientSamples`）；
/// - IC 可测但一侧方差退化 ⇒ 基线 1.0（`IcUnmeasurable`），不在测不出力度的格上做校准；
/// - IC ≥ 0 ⇒ 只吃胜率降权（`min(aw, 1.0)`，上限即基线 ⇒ 只降不升）；
/// - IC < 0 ⇒ 胜率权重再乘 `NEGATIVE_IC_PENALTY`，下限 `WEIGHT_FLOOR`。
pub fn compute_loop_cell_weights(
    samples: &[RecoLoopSample],
    current: &HashMap<(String, String), f64>,
    now_ms: i64,
) -> Vec<LoopCellResult> {
    let mut out = Vec::new();
    for style_key in style_keys() {
        for period in Period::ALL.iter() {
            let reason = crate::recommender::style_matrix::reason_code_by_key(style_key, *period);
            if reason != "cell_is_active" {
                out.push(LoopCellResult {
                    style: style_key,
                    period: period.as_str(),
                    status: LoopCellStatus::NotInMatrix,
                    rank_ic: None,
                    samples: 0,
                    win_rate: None,
                    old_weight: BASELINE_WEIGHT,
                    new_weight: BASELINE_WEIGHT,
                });
                continue;
            }
            let days = period.default_holding_days();
            let cutoff = now_ms - (days as i64) * 86_400_000;
            let cell: Vec<&RecoLoopSample> = samples
                .iter()
                .filter(|s| {
                    s.style_key == style_key && s.period == *period && s.exit_at_ms >= cutoff
                })
                .collect();
            let key = (style_key.to_string(), period.as_str().to_string());
            let old_weight = current.get(&key).copied().unwrap_or(BASELINE_WEIGHT);

            if cell.len() < crate::reflection_stats::IC_MIN_SAMPLE {
                out.push(LoopCellResult {
                    style: style_key,
                    period: period.as_str(),
                    status: LoopCellStatus::InsufficientSamples,
                    rank_ic: None,
                    samples: cell.len(),
                    win_rate: None,
                    old_weight,
                    new_weight: BASELINE_WEIGHT,
                });
                continue;
            }

            let perf_rows: Vec<StrategyPerformanceRow> =
                cell.iter().map(|s| s.to_performance_row()).collect();
            let cfg = crate::weight_decay::WeightDecayConfig {
                lookback_days: days,
                ..Default::default()
            };
            let mut cur1 = HashMap::new();
            cur1.insert(key.clone(), old_weight);
            let aw =
                crate::weight_decay::compute_adjusted_weights_at(&perf_rows, &cfg, &cur1, now_ms)
                    .remove(&key);

            let ic_rows: Vec<crate::recommender::ic::RecoIcRow> =
                cell.iter().map(|s| s.to_ic_row()).collect();
            let stats = crate::recommender::ic::aggregate_reco_ic(&ic_rows);
            let cell_ic = stats
                .styles
                .iter()
                .flat_map(|st| st.cells.iter())
                .find(|c| c.period == period.as_str());
            let (ic_status, rank_ic) = match cell_ic {
                Some(c) => (c.ic_status, c.rank_ic),
                None => ("insufficient_ic_samples", None),
            };

            let win = aw.map(|a| (a.new_weight.min(BASELINE_WEIGHT), a.win_rate));
            let (win_w, win_rate) = match win {
                Some((w, wr)) => (Some(w), Some(wr)),
                None => (None, None),
            };
            let (status, new_weight) = if ic_status != "ok" {
                // 样本门闸三态（P2）：测不出力度就不校准
                (LoopCellStatus::IcUnmeasurable, BASELINE_WEIGHT)
            } else if rank_ic.unwrap_or(0.0) < 0.0 {
                (
                    LoopCellStatus::DemotedNegativeIc,
                    win_w.unwrap_or(BASELINE_WEIGHT) * NEGATIVE_IC_PENALTY,
                )
            } else {
                (LoopCellStatus::IcNonNegative, win_w.unwrap_or(BASELINE_WEIGHT))
            };
            out.push(LoopCellResult {
                style: style_key,
                period: period.as_str(),
                status,
                rank_ic,
                samples: cell.len(),
                win_rate,
                old_weight,
                new_weight: new_weight.clamp(WEIGHT_FLOOR, BASELINE_WEIGHT),
            });
        }
    }
    out
}

/// 闭环整体视图（随 `reco_ic_stats` 一起出，Phase D 呈现层的表头来源）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoLoopView {
    /// 生效闸（`reco_ic_gate`）：`off` | `shadow` | `on`
    pub gate: String,
    pub cells: Vec<LoopCellResult>,
    /// 最近一次闭环重算时刻（ms）；0 = 从未重算
    pub last_recalc_at: i64,
}

impl Default for RecoLoopView {
    fn default() -> Self {
        Self { gate: "shadow".to_string(), cells: Vec::new(), last_recalc_at: 0 }
    }
}

/// 重算荐股闭环权重并留痕（写入键空间 = 矩阵名目，trigger = `reco-loop`）。
///
/// `as_of_date` 为时间旅行右端：样本按 `exit_at_ms <= as_of` 截断、窗口以 as_of 为
/// 「今天」，用于 Phase E 的 A/B 回放；写库的 `applied_at` 仍是真实时刻（与分析链
/// `recalc_and_persist` 同形态）。极小抖动（|Δ| < 1%）不写库，与既有权重线一致。
pub async fn recalc_and_persist_reco_loop(
    db: &DatabaseConnection,
    as_of_date: Option<&str>,
) -> Result<(usize, Vec<LoopCellResult>), String> {
    use axagent_entities::strategy_weight_history;

    let now_ms = match as_of_date {
        Some(d) => chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .map_err(|e| format!("as_of_date 格式错误: {e}"))?
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| "无效日期".to_string())?
            .and_utc()
            .timestamp_millis(),
        None => chrono::Utc::now().timestamp_millis(),
    };

    let mut samples = load_reco_loop_samples(db).await?;
    samples.retain(|s| s.exit_at_ms <= now_ms);
    let current =
        crate::evolution_drift::load_current_weights_by_trigger(db, Some(RECO_LOOP_TRIGGER))
            .await?;
    let results = compute_loop_cell_weights(&samples, &current, now_ms);

    let applied_at = chrono::Utc::now().timestamp_millis();
    let mut written = 0usize;
    for r in results.iter().filter(|r| r.status != LoopCellStatus::NotInMatrix) {
        let delta_pct = if r.old_weight.abs() > f64::EPSILON {
            (r.new_weight - r.old_weight) / r.old_weight * 100.0
        } else {
            0.0
        };
        if delta_pct.abs() < 1.0 {
            continue;
        }
        let rationale = match r.status {
            LoopCellStatus::DemotedNegativeIc => format!(
                "闭环降权: 窗口内 {} 样本, 负 rankIC {:.2}, 胜率 {:.0}%, 惩罚×{} ⇒ 权重 {:.2}→{:.2}",
                r.samples,
                r.rank_ic.unwrap_or(0.0),
                r.win_rate.unwrap_or(0.0) * 100.0,
                NEGATIVE_IC_PENALTY,
                r.old_weight,
                r.new_weight
            ),
            LoopCellStatus::IcNonNegative => format!(
                "闭环校准: 窗口内 {} 样本, rankIC {:.2}≥0, 胜率 {:.0}% ⇒ 权重 {:.2}→{:.2}",
                r.samples,
                r.rank_ic.unwrap_or(0.0),
                r.win_rate.unwrap_or(0.0) * 100.0,
                r.old_weight,
                r.new_weight
            ),
            _ => format!(
                "闭环未校准({}): 窗口内 {} 样本, 回退基线 {:.2}",
                r.status.as_str(),
                r.samples,
                r.new_weight
            ),
        };
        let am = strategy_weight_history::ActiveModel {
            id: Set(uuid::Uuid::new_v4().to_string()),
            strategy_id: Set(r.style.to_string()),
            period: Set(r.period.to_string()),
            old_weight: Set(r.old_weight),
            new_weight: Set(r.new_weight),
            delta_pct: Set(delta_pct),
            trigger: Set(RECO_LOOP_TRIGGER.to_string()),
            source_reflection_id: Set(None),
            sample_size: Set(r.samples as i32),
            win_rate: Set(r.win_rate.unwrap_or(0.0)),
            rationale: Set(Some(rationale)),
            applied_at: Set(applied_at),
        };
        strategy_weight_history::Entity::insert(am)
            .exec(db)
            .await
            .map_err(|e| format!("写入闭环权重留痕失败: {e}"))?;
        written += 1;
    }
    Ok((written, results))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dv_row(
        pick_id: &str,
        style: &str,
        period: &str,
        t_plus_n: i32,
        outcome: &str,
    ) -> decision_validations::Model {
        decision_validations::Model {
            id: format!("v-{pick_id}-{t_plus_n}"),
            pick_id: pick_id.to_string(),
            stock_code: "600519".into(),
            stock_name: "贵州茅台".into(),
            style: style.into(),
            period: period.into(),
            t_plus_n,
            generated_at: "2026-09-01T10:00:00.000".into(),
            validated_at: "2026-09-10T10:00:00+00:00".into(),
            entry_price: 100.0,
            target_price: 110.0,
            stop_loss: 95.0,
            position_pct: 5.0,
            confidence: 70,
            inferred_action: "buy".into(),
            t_plus_n_price: Some(105.0),
            max_price: Some(108.0),
            min_price: Some(98.0),
            max_return_pct: Some(8.0),
            max_drawdown_pct: Some(2.0),
            final_return_pct: Some(5.0),
            hit_stop_loss: Some(0),
            hit_target: Some(0),
            hit_outcome: Some(outcome.into()),
            factor_snapshot: None,
            data_source: "astock_client".into(),
            created_at: "2026-09-10T10:00:00+00:00".into(),
        }
    }

    /// 别名合并：落库两写（serenity/bottleneck）必须归到同一矩阵名目行。
    #[test]
    fn db_style_aliases_merge_into_matrix_key() {
        let rows = vec![
            dv_row("p1", "bottleneck", "mid", 20, "hit"),
            dv_row("p2", "serenity", "mid", 20, "hit"),
        ];
        let samples = samples_from_validation_rows(rows);
        assert_eq!(samples.len(), 2);
        assert!(samples.iter().all(|s| s.style_key == "serenity"), "{samples:?}");
    }

    /// 矩阵外风格（分析链伪写入方/未知来源）不进闭环。
    #[test]
    fn unknown_style_is_dropped() {
        let samples =
            samples_from_validation_rows(vec![dv_row("p1", "momentum_x", "mid", 20, "hit")]);
        assert!(samples.is_empty());
    }

    /// 历史存档 period 写法 ultrashort 必须被读成 UltraShort（与 Period::from_str 契约一致）。
    #[test]
    fn legacy_period_spelling_is_parsed() {
        let samples =
            samples_from_validation_rows(vec![dv_row("p1", "trend", "ultrashort", 5, "hit")]);
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].period, Period::UltraShort);
    }

    /// partial 计 win（与命中率分母同口径）；insufficient 整行剔除。
    #[test]
    fn outcome_uses_single_binary_source() {
        let samples = samples_from_validation_rows(vec![
            dv_row("p1", "trend", "short", 5, "partial"),
            dv_row("p2", "trend", "short", 5, "insufficient"),
            dv_row("p3", "value", "short", 5, "false_hit"),
        ]);
        assert_eq!(samples.len(), 2, "insufficient 整行剔除: {samples:?}");
        assert!(samples.iter().any(|s| s.style_key == "trend" && s.win), "partial 必须计 win");
        assert!(samples.iter().any(|s| s.style_key == "value" && !s.win), "false_hit 必须计 loss");
    }

    /// 一 pick 一样本：mid 档（28 天）三窗口 5/20/60 中取 20（距离 8 < 32）。
    #[test]
    fn one_pick_yields_one_sample_at_nearest_window() {
        let rows = vec![
            dv_row("p1", "trend", "mid", 5, "hit"),
            dv_row("p1", "trend", "mid", 20, "hit"),
            dv_row("p1", "trend", "mid", 60, "miss"),
        ];
        let samples = samples_from_validation_rows(rows);
        assert_eq!(samples.len(), 1, "同 pick 多窗口必须收敛为一行");
        assert_eq!(samples[0].holding_days, 20);
        assert!(samples[0].win, "应取 T+20 那行的判定");
    }

    /// 并列距离取更短窗（保守）：short 档 5 天，窗口 0/10 距离都是 5 ⇒ 取 0 不行，实际取更小 t_plus_n。
    #[test]
    fn tie_break_prefers_shorter_window() {
        let rows = vec![
            dv_row("p1", "trend", "short", 0, "miss"),
            dv_row("p1", "trend", "short", 10, "hit"),
        ];
        let samples = samples_from_validation_rows(rows);
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].holding_days, 0, "并列时取更短窗（先入且距离相等不覆盖）");
        assert!(!samples[0].win);
    }

    /// 坏时间戳/坏价格的行不得混进样本（NaN 与不可解析时间都防御）。
    #[test]
    fn corrupt_rows_are_dropped() {
        let mut bad_time = dv_row("p1", "trend", "short", 5, "hit");
        bad_time.validated_at = "not-a-time".into();
        bad_time.created_at = "not-a-time".into();
        let mut bad_price = dv_row("p2", "trend", "short", 5, "hit");
        bad_price.entry_price = 0.0;
        let mut nan_price = dv_row("p3", "trend", "short", 5, "hit");
        nan_price.t_plus_n_price = Some(f64::NAN);
        let samples = samples_from_validation_rows(vec![bad_time, bad_price, nan_price]);
        assert!(samples.is_empty());
    }

    /// 胜率行形态：strategy_id 用矩阵名目、period 用规范键（与 reco_strategy_weights 的
    /// "{style}_{period}" 键空间往返一致）。
    #[test]
    fn performance_row_round_trips_matrix_naming() {
        let samples =
            samples_from_validation_rows(vec![dv_row("p1", "bottleneck", "mid", 20, "hit")]);
        let row = samples[0].to_performance_row();
        assert_eq!(row.strategy_id, "serenity");
        assert_eq!(row.period, "mid");
        assert_eq!(row.was_correct, 1);
    }

    // ── Phase B：逐格合成 ──

    fn mk(
        style: &'static str,
        period: Period,
        conf: f64,
        ret: f64,
        win: bool,
        exit_at_ms: i64,
    ) -> RecoLoopSample {
        RecoLoopSample {
            style_key: style,
            period,
            exit_at_ms,
            win,
            confidence: conf,
            realized_return_pct: ret,
            holding_days: period.default_holding_days(),
        }
    }

    fn cell<'a>(res: &'a [LoopCellResult], style: &str, period: &str) -> &'a LoopCellResult {
        res.iter().find(|r| r.style == style && r.period == period).expect("缺格")
    }

    /// 负 IC（conf 与收益反向）⇒ 触发惩罚降权，且低于同规模的无 IC 校准基线。
    #[test]
    fn negative_ic_demotes_below_baseline() {
        let now = 1_700_000_000_000i64;
        // 8 样本，conf 10..80 递增、ret 反向递减 ⇒ Spearman ρ = -1
        let samples: Vec<_> = (0..8)
            .map(|i| mk("trend", Period::Short, 10.0 + i as f64 * 10.0, -i as f64, i < 5, now))
            .collect();
        let res = compute_loop_cell_weights(&samples, &HashMap::new(), now);
        let c = cell(&res, "trend", "short");
        assert_eq!(c.status, LoopCellStatus::DemotedNegativeIc, "{c:?}");
        assert!(c.new_weight < BASELINE_WEIGHT, "负 IC 必须降权，实际 {}", c.new_weight);
        assert!(c.new_weight >= WEIGHT_FLOOR, "不得越过下限，实际 {}", c.new_weight);
    }

    /// 正 IC ⇒ 只走胜率线，权重封顶基线 1.0（只降不升，绝不因高胜率提权超过基线）。
    #[test]
    fn non_negative_ic_never_boosts_above_baseline() {
        let now = 1_700_000_000_000i64;
        // conf 与 ret 同向 ⇒ ρ = +1，且全 win（高胜率）
        let samples: Vec<_> = (0..8)
            .map(|i| mk("trend", Period::Short, 10.0 + i as f64 * 10.0, i as f64, true, now))
            .collect();
        let res = compute_loop_cell_weights(&samples, &HashMap::new(), now);
        let c = cell(&res, "trend", "short");
        assert_eq!(c.status, LoopCellStatus::IcNonNegative, "{c:?}");
        assert!(
            c.new_weight <= BASELINE_WEIGHT + 1e-9,
            "正 IC 高胜率也不得提权超基线，实际 {}",
            c.new_weight
        );
    }

    /// 样本 < IC_MIN_SAMPLE ⇒ 基线 1.0 且显式「未校准」（不冒充校准过）。
    #[test]
    fn insufficient_samples_stay_baseline_and_mark_uncalibrated() {
        let now = 1_700_000_000_000i64;
        let samples: Vec<_> = (0..3)
            .map(|i| mk("trend", Period::Mid, 60.0 + i as f64, i as f64, true, now))
            .collect();
        let res = compute_loop_cell_weights(&samples, &HashMap::new(), now);
        let c = cell(&res, "trend", "mid");
        assert_eq!(c.status, LoopCellStatus::InsufficientSamples, "{c:?}");
        assert_eq!(c.new_weight, BASELINE_WEIGHT);
    }

    /// conf 一侧恒等（方差退化）⇒ IC 测不出力度 ⇒ 回退基线，标 IcUnmeasurable。
    #[test]
    fn degenerate_variance_marks_ic_unmeasurable() {
        let now = 1_700_000_000_000i64;
        let samples: Vec<_> =
            (0..8).map(|i| mk("trend", Period::Mid, 60.0, i as f64, true, now)).collect();
        let res = compute_loop_cell_weights(&samples, &HashMap::new(), now);
        let c = cell(&res, "trend", "mid");
        assert_eq!(c.status, LoopCellStatus::IcUnmeasurable, "{c:?}");
        assert_eq!(c.new_weight, BASELINE_WEIGHT);
    }

    /// 超短档 lookback 只 2 天：3 天前的样本必须落在窗口外 ⇒ 视作样本不足。
    #[test]
    fn stale_samples_fall_outside_short_lookback() {
        let now = 1_700_000_000_000i64;
        let day = 86_400_000i64;
        let samples: Vec<_> = (0..8)
            .map(|i| {
                mk("trend", Period::UltraShort, 10.0 + i as f64, -i as f64, true, now - 3 * day)
            })
            .collect();
        let res = compute_loop_cell_weights(&samples, &HashMap::new(), now);
        let c = cell(&res, "trend", "ultra_short");
        assert_eq!(c.status, LoopCellStatus::InsufficientSamples, "3 天前样本应出超短 2 天窗");
        assert_eq!(c.new_weight, BASELINE_WEIGHT);
    }

    /// 矩阵不成立格（reversion×ultra_short）⇒ NotInMatrix，不产权重。
    #[test]
    fn inactive_cell_is_not_in_matrix() {
        let now = 1_700_000_000_000i64;
        let samples: Vec<_> = (0..8)
            .map(|i| mk("reversion", Period::UltraShort, 10.0 + i as f64, -i as f64, false, now))
            .collect();
        let res = compute_loop_cell_weights(&samples, &HashMap::new(), now);
        let c = cell(&res, "reversion", "ultra_short");
        assert_eq!(c.status, LoopCellStatus::NotInMatrix, "{c:?}");
        assert_eq!(c.new_weight, BASELINE_WEIGHT);
    }

    /// IPC 契约（禁区 13）：LoopCellResult 跨边界必须 camelCase，status 用 snake_case 词表。
    #[test]
    fn loop_cell_result_serializes_camel_case() {
        let now = 1_700_000_000_000i64;
        let samples: Vec<_> = (0..8)
            .map(|i| mk("trend", Period::Short, 10.0 + i as f64 * 10.0, -i as f64, true, now))
            .collect();
        let res = compute_loop_cell_weights(&samples, &HashMap::new(), now);
        let value = serde_json::to_value(cell(&res, "trend", "short")).unwrap();
        let obj = value.as_object().unwrap();
        for key in
            ["style", "period", "status", "rankIc", "samples", "winRate", "oldWeight", "newWeight"]
        {
            assert!(obj.contains_key(key), "LoopCellResult 缺 camelCase 键 {key}: {obj:?}");
        }
        assert!(!obj.contains_key("rank_ic"));
        assert_eq!(obj["status"].as_str(), Some("demoted_negative_ic"), "status 须 snake_case");
    }
}
