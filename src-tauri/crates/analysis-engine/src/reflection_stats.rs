// SPDX-License-Identifier: AGPL-3.0-only

//! 反思命中率聚合（PLAN-stock-decision-hitrate-validation M2）——
//! 把已落库的决策验证数据聚合成可读的命中率基线。
//!
//! ## 数据源分工（避免与 hit_rate_backtest.rs 重复）
//! - [`crate::hit_rate_backtest`]（V55）：消费 `reco_picks` + 实时行情 API，对**未落库**的决策做回溯验证。
//! - 本模块：消费**已落库**的 `strategy_performance`（M1 确定性判定 + 复盘 win/loss 写回），
//!   按决策方向（action）与时间维度（horizon）聚合，产出方向命中率 / 价位命中率 / 平均收益 / alpha。
//!
//! ## 诚实性铁律
//! 可判样本 < [`MIN_SAMPLE`] 时 `direction_hit_rate` 返回 `None`（前端显示「样本不足」），不产出伪结论。
//! 无法判定的记录（无行情 / 中性档 / 期中观察）在落库层**不写行**，天然不进命中率分子。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// 命中率的最低样本数：低于此值不产出命中率数值（诚实性铁律）
pub const MIN_SAMPLE: usize = 5;

/// 四周期反思状态。`Immature` 与 `Unavailable` 不得产生确定性胜负结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HorizonStatus {
    Mature,
    Immature,
    Unavailable,
    Legacy,
}

/// 从 `stock_analyses.horizon_decisions` 解析出的单周期决策。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HorizonDecision {
    pub action: String,
    #[serde(alias = "position_pct")]
    pub position_pct: Option<f64>,
    pub confidence: Option<f64>,
    #[serde(alias = "expected_holding_days")]
    pub expected_holding_days: Option<i64>,
    #[serde(alias = "target_price")]
    pub target_price: Option<f64>,
    #[serde(alias = "stop_loss")]
    pub stop_loss: Option<f64>,
    #[serde(alias = "conf_lower_bound")]
    pub conf_lower_bound: Option<f64>,
}

/// 兼容主周期所需的最小反思结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HorizonResult {
    pub status: HorizonStatus,
    pub horizon: String,
    pub expected_holding_days: i64,
    pub action: String,
}

/// 单周期客观评价的最小结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HorizonEvaluation {
    pub status: HorizonStatus,
    pub was_correct: Option<i32>,
    pub return_pct: Option<f64>,
}

/// 解析四周期决策 JSON 对象。
pub fn parse_horizon_decisions(
    source: &serde_json::Map<String, serde_json::Value>,
) -> Result<HashMap<String, HorizonDecision>, String> {
    source
        .iter()
        .map(|(horizon, value)| {
            let decision = serde_json::from_value::<HorizonDecision>(value.clone())
                .map_err(|error| format!("解析 horizon_decisions.{horizon} 失败: {error}"))?;
            Ok((horizon.clone(), decision))
        })
        .collect()
}

/// 为没有四周期输出的历史记录构造 legacy 主周期结果。
pub fn legacy_horizon_result(
    action: Option<&str>,
    horizon: Option<&str>,
    expected_holding_days: Option<i64>,
) -> Option<HorizonResult> {
    let action = action?.to_string();
    let horizon = horizon?.to_string();
    let expected_holding_days = expected_holding_days?;
    Some(HorizonResult { status: HorizonStatus::Legacy, horizon, expected_holding_days, action })
}

/// 四周期缺省期望持有期（交易日）。
pub fn default_expected_holding_days(horizon: &str) -> i64 {
    match horizon {
        "ultra_short" => 2,
        "short" => 5,
        "mid" => 28,
        "long" => 90,
        _ => 28,
    }
}

/// 构造行情不可用或未成熟等状态的评价，禁止用 0% 收益冒充事实。
pub fn build_horizon_evaluation(
    status: HorizonStatus,
    was_correct: Option<i64>,
    return_pct: Option<f64>,
) -> HorizonEvaluation {
    let is_mature = status == HorizonStatus::Mature || status == HorizonStatus::Legacy;
    let normalized_was_correct = if is_mature {
        was_correct.and_then(|value| i32::try_from(value).ok())
    } else {
        None
    };
    let normalized_return = if is_mature { return_pct } else { None };
    HorizonEvaluation { status, was_correct: normalized_was_correct, return_pct: normalized_return }
}

/// 按已成熟周期的 action 与净收益符号确定方向命中情况。
pub fn deterministic_horizon_was_correct(
    action: &str,
    return_pct: f64,
    within_expected_horizon: bool,
) -> Option<i32> {
    if within_expected_horizon || !return_pct.is_finite() {
        return None;
    }
    match action {
        "买入" | "增持" | "BUY" | "INCREASE" => Some(i32::from(return_pct > 0.0)),
        "卖出" | "减持" | "SELL" | "REDUCE" => Some(i32::from(return_pct < 0.0)),
        _ => None,
    }
}

/// 样本数据源标识 —— 命令层标注该样本来自四周期 JSON 还是旧单周期字段回退。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleDataSource {
    /// 四周期 `horizon_results_json` 中已成熟周期展开而来
    #[default]
    Reflection,
    /// 旧记录无四周期 JSON，经 `original_analysis_id` 回退主周期字段
    Legacy,
}

/// 单条决策表现样本 —— 命令层跨三表（strategy_performance / stock_reflections /
/// stock_analyses）查询并关联好维度后传入，本模块只做纯函数聚合（方便单元测试）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionPerformanceSample {
    /// 决策方向（stock_analyses.decision_action；未识别归 None）
    pub action: Option<String>,
    /// 时间维度（stock_analyses.decision_time_horizon；未识别归 None）
    pub horizon: Option<String>,
    /// 决策是否正确：1 对 / 0 错（M1 确定性判定或复盘 win/loss 写回）
    pub was_correct: i32,
    /// 实际净收益（%）
    pub return_pct: f64,
    /// 相对基准超额收益（%）；仅反思行有
    pub alpha_pct: Option<f64>,
    /// 是否达目标价；仅反思行有（解析自 stock_reflections.blackboard_snapshot）
    pub target_reached: Option<bool>,
    /// 数据源标识（四周期展开 / legacy 回退）
    pub data_source: SampleDataSource,
}

/// 按维度分组的方向命中率与分周期指标
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitrateGroup {
    /// 分组键：action / horizon 的原始值，缺失归 "unknown"
    pub key: String,
    /// 已判定（成熟）样本数（was_correct ∈ {0,1}）
    pub samples: usize,
    /// 方向命中率；样本 < [`MIN_SAMPLE`] → None（样本不足）
    pub direction_hit_rate: Option<f64>,
    /// 目标价命中率 = target_reached=true / 有目标价判定样本；样本不足 → None
    pub target_hit_rate: Option<f64>,
    /// 该组平均净收益（%）；无样本 → None
    pub avg_raw_return_pct: Option<f64>,
    /// 该组平均超额收益（%）；无 alpha 样本 → None
    pub avg_alpha_pct: Option<f64>,
}

/// 命中率聚合结果（DTO；前端按 camelCase 消费，见 src/types/stock-analysis.ts）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HitrateStats {
    /// 参与统计的已结算样本总数
    pub total_samples: usize,
    /// 方向命中率 = was_correct=1 / 已判定样本；样本 < [`MIN_SAMPLE`] → None
    pub direction_hit_rate: Option<f64>,
    /// 价位命中率 = target_reached=true / 有目标价判定样本；样本 < [`MIN_SAMPLE`] → None
    pub target_hit_rate: Option<f64>,
    /// 平均净收益（%）
    pub avg_raw_return_pct: Option<f64>,
    /// 平均超额收益（%）
    pub avg_alpha_pct: Option<f64>,
    /// 样本中来自旧单周期字段回退（`legacy`）的条数
    pub legacy_samples: usize,
    /// 按决策方向分组（含 "unknown"）
    pub by_action: Vec<HitrateGroup>,
    /// 按时间维度分组（含 "unknown"）
    pub by_horizon: Vec<HitrateGroup>,
}

/// 命中率：分母为 0 或样本不足 → None
fn hit_rate_or_none(good: usize, total: usize) -> Option<f64> {
    if total == 0 || total < MIN_SAMPLE {
        None
    } else {
        Some(good as f64 / total as f64)
    }
}

/// 均值：空序列 → None
fn mean(xs: &[f64]) -> Option<f64> {
    if xs.is_empty() {
        None
    } else {
        Some(xs.iter().sum::<f64>() / xs.len() as f64)
    }
}

/// 分组聚合中间态（一次遍历同时累加方向 / 目标价 / 收益 / alpha 四类指标）
#[derive(Default)]
struct GroupAggregate {
    /// 已判定样本（was_correct ∈ {0,1}）
    judged: usize,
    /// 方向命中样本（was_correct = 1）
    correct: usize,
    /// 有目标价判定的样本
    target_judged: usize,
    /// 目标价命中样本
    target_hit: usize,
    /// 净收益序列（与顶层同口径，组内全样本）
    returns: Vec<f64>,
    /// 超额收益序列（仅含 alpha_pct 有值的样本）
    alphas: Vec<f64>,
}

/// 按 key 分组统计方向命中率与分周期指标（保留首次出现的顺序，避免 HashMap 无序）。
/// 只统计已判定样本（was_correct ∈ {0,1}）；未判定样本不进任何分组。
fn group_hit_rates<F>(samples: &[DecisionPerformanceSample], key_of: F) -> Vec<HitrateGroup>
where
    F: Fn(&DecisionPerformanceSample) -> String,
{
    let mut order: Vec<String> = Vec::new();
    let mut agg: HashMap<String, GroupAggregate> = HashMap::new();
    for s in samples {
        if s.was_correct != 0 && s.was_correct != 1 {
            continue;
        }
        let key = key_of(s);
        let entry = agg.entry(key.clone()).or_default();
        entry.judged += 1;
        if s.was_correct == 1 {
            entry.correct += 1;
        }
        if let Some(reached) = s.target_reached {
            entry.target_judged += 1;
            if reached {
                entry.target_hit += 1;
            }
        }
        entry.returns.push(s.return_pct);
        if let Some(alpha) = s.alpha_pct {
            entry.alphas.push(alpha);
        }
        if !order.contains(&key) {
            order.push(key);
        }
    }
    order
        .into_iter()
        .map(|key| {
            let a = agg.remove(&key).unwrap_or_default();
            HitrateGroup {
                key,
                samples: a.judged,
                direction_hit_rate: hit_rate_or_none(a.correct, a.judged),
                target_hit_rate: hit_rate_or_none(a.target_hit, a.target_judged),
                avg_raw_return_pct: mean(&a.returns),
                avg_alpha_pct: mean(&a.alphas),
            }
        })
        .collect()
}

/// 聚合命中率（纯函数，无 I/O）。
///
/// 判定口径（与落库层一致）：
/// - 方向命中率分子 = was_correct=1；分母 = was_correct ∈ {0,1} 的已判定样本。
/// - 价位命中率分子 = target_reached=true；分母 = 有 target_reached 判定的样本。
/// - 平均收益只统计已结算样本（pending / 未判定不在落库层写行，天然排除）。
pub fn compute_reflection_stats(samples: &[DecisionPerformanceSample]) -> HitrateStats {
    let judged_total = samples.iter().filter(|s| s.was_correct == 0 || s.was_correct == 1).count();
    let correct_total = samples.iter().filter(|s| s.was_correct == 1).count();

    let target_judged: Vec<bool> = samples.iter().filter_map(|s| s.target_reached).collect();
    let target_hit = target_judged.iter().filter(|&&t| t).count();

    let returns: Vec<f64> = samples.iter().map(|s| s.return_pct).collect();
    let alphas: Vec<f64> = samples.iter().filter_map(|s| s.alpha_pct).collect();
    let legacy_samples =
        samples.iter().filter(|s| s.data_source == SampleDataSource::Legacy).count();

    HitrateStats {
        total_samples: samples.len(),
        direction_hit_rate: hit_rate_or_none(correct_total, judged_total),
        target_hit_rate: hit_rate_or_none(target_hit, target_judged.len()),
        avg_raw_return_pct: mean(&returns),
        avg_alpha_pct: mean(&alphas),
        legacy_samples,
        by_action: group_hit_rates(samples, |s| {
            s.action.clone().unwrap_or_else(|| "unknown".to_string())
        }),
        by_horizon: group_hit_rates(samples, |s| {
            s.horizon.clone().unwrap_or_else(|| "unknown".to_string())
        }),
    }
}

// ── 单元测试 ── 追加在文件末尾（防 clippy::items_after_test_module）
#[cfg(test)]
mod reflection_stats_tests {
    use super::*;

    const FOUR_HORIZON_FIXTURE: &str =
        include_str!("../tests/fixtures/stock_reflection_four_horizon_complete.json");
    const LEGACY_FIXTURE: &str =
        include_str!("../tests/fixtures/stock_reflection_legacy_single_horizon.json");
    const UNAVAILABLE_FIXTURE: &str =
        include_str!("../tests/fixtures/stock_reflection_market_unavailable.json");

    fn sample(
        action: Option<&str>,
        horizon: Option<&str>,
        was_correct: i32,
        return_pct: f64,
    ) -> DecisionPerformanceSample {
        DecisionPerformanceSample {
            action: action.map(str::to_string),
            horizon: horizon.map(str::to_string),
            was_correct,
            return_pct,
            alpha_pct: None,
            target_reached: None,
            data_source: SampleDataSource::Reflection,
        }
    }

    #[test]
    fn five_plus_samples_produce_hit_rate() {
        let samples = vec![
            sample(Some("买入"), Some("short"), 1, 5.0),
            sample(Some("买入"), Some("short"), 1, 3.0),
            sample(Some("买入"), Some("short"), 1, 1.0),
            sample(Some("买入"), Some("mid"), 0, -2.0),
            sample(Some("卖出"), Some("mid"), 1, 4.0),
        ];
        let stats = compute_reflection_stats(&samples);
        assert_eq!(stats.total_samples, 5);
        // 4 对 / 5 已判定 = 0.8
        assert_eq!(stats.direction_hit_rate, Some(0.8));
        // 全部样本都有 return_pct → 均值 (5+3+1-2+4)/5 = 2.2
        assert_eq!(stats.avg_raw_return_pct, Some(2.2));
        // 分组：买入 3/4、卖出 1/1
        let buy = stats.by_action.iter().find(|g| g.key == "买入").unwrap();
        assert_eq!(buy.samples, 4);
        assert_eq!(buy.direction_hit_rate, None); // 4 < MIN_SAMPLE
        let sell = stats.by_action.iter().find(|g| g.key == "卖出").unwrap();
        assert_eq!(sell.samples, 1);
        assert_eq!(sell.direction_hit_rate, None); // 1 < MIN_SAMPLE
    }

    #[test]
    fn below_min_sample_returns_none() {
        let samples = vec![sample(Some("买入"), Some("short"), 1, 1.0)];
        let stats = compute_reflection_stats(&samples);
        assert_eq!(stats.direction_hit_rate, None);
        assert_eq!(stats.avg_raw_return_pct, Some(1.0));
    }

    #[test]
    fn empty_input_returns_defaults() {
        let stats = compute_reflection_stats(&[]);
        assert_eq!(stats.total_samples, 0);
        assert_eq!(stats.direction_hit_rate, None);
        assert_eq!(stats.target_hit_rate, None);
        assert!(stats.by_action.is_empty());
        assert!(stats.by_horizon.is_empty());
    }

    #[test]
    fn target_hit_rate_counts_only_judged_samples() {
        let mut s1 = sample(Some("买入"), Some("short"), 1, 5.0);
        s1.target_reached = Some(true);
        let mut s2 = sample(Some("买入"), Some("short"), 0, -5.0);
        s2.target_reached = Some(false);
        let mut s3 = sample(Some("买入"), Some("short"), 1, 1.0);
        s3.target_reached = Some(true);
        let mut s4 = sample(Some("买入"), Some("short"), 1, 2.0);
        s4.target_reached = Some(true);
        let mut s5 = sample(Some("买入"), Some("short"), 1, 3.0);
        s5.target_reached = Some(false);
        let stats = compute_reflection_stats(&[s1, s2, s3, s4, s5]);
        // 3 达目标 / 5 判定 = 0.6
        assert_eq!(stats.target_hit_rate, Some(0.6));
    }

    #[test]
    fn alpha_mean_only_over_samples_with_alpha() {
        let mut s1 = sample(Some("买入"), Some("short"), 1, 5.0);
        s1.alpha_pct = Some(1.0);
        let mut s2 = sample(Some("买入"), Some("short"), 1, 3.0);
        s2.alpha_pct = Some(2.0);
        let s3 = sample(Some("买入"), Some("short"), 0, -2.0);
        let s4 = sample(Some("买入"), Some("short"), 1, 4.0);
        let s5 = sample(Some("买入"), Some("short"), 1, 6.0);
        let stats = compute_reflection_stats(&[s1, s2, s3, s4, s5]);
        // alpha 只取有值两项：(1+2)/2 = 1.5
        assert_eq!(stats.avg_alpha_pct, Some(1.5));
    }

    #[test]
    fn unknown_group_for_missing_dimension() {
        let samples = vec![
            sample(None, None, 1, 5.0),
            sample(None, None, 1, 3.0),
            sample(None, None, 1, 1.0),
            sample(None, None, 0, -2.0),
            sample(None, None, 1, 4.0),
        ];
        let stats = compute_reflection_stats(&samples);
        let act = stats.by_action.iter().find(|g| g.key == "unknown").unwrap();
        assert_eq!(act.samples, 5);
        assert_eq!(act.direction_hit_rate, Some(0.8));
        let hor = stats.by_horizon.iter().find(|g| g.key == "unknown").unwrap();
        assert_eq!(hor.samples, 5);
    }

    #[test]
    fn parses_four_horizon_decisions_from_fixture() {
        let fixture: serde_json::Value = serde_json::from_str(FOUR_HORIZON_FIXTURE).unwrap();
        let decisions = parse_horizon_decisions(
            fixture["stockAnalysis"]["horizonDecisions"].as_object().unwrap(),
        )
        .unwrap();

        assert_eq!(decisions.len(), 4);
        assert_eq!(decisions["ultra_short"].action, "买入");
        assert_eq!(decisions["short"].expected_holding_days, Some(5));
        assert_eq!(decisions["mid"].target_price, Some(1850.0));
        assert_eq!(decisions["long"].stop_loss, Some(1450.0));
    }

    #[test]
    fn legacy_fixture_falls_back_to_single_horizon() {
        let fixture: serde_json::Value = serde_json::from_str(LEGACY_FIXTURE).unwrap();
        let result = legacy_horizon_result(
            fixture["stockAnalysis"]["decisionAction"].as_str(),
            fixture["stockAnalysis"]["decisionTimeHorizon"].as_str(),
            fixture["stockAnalysis"]["decisionExpectedHoldingDays"].as_i64(),
        )
        .unwrap();

        assert_eq!(result.status, HorizonStatus::Legacy);
        assert_eq!(result.horizon, "short");
        assert_eq!(result.expected_holding_days, 5);
        assert_eq!(result.action, "卖出");
    }

    #[test]
    fn unavailable_fixture_never_becomes_zero_return() {
        let fixture: serde_json::Value = serde_json::from_str(UNAVAILABLE_FIXTURE).unwrap();
        let result = build_horizon_evaluation(
            HorizonStatus::Unavailable,
            fixture["expected"]["wasCorrect"].as_i64(),
            fixture["expected"]["returnPct"].as_f64(),
        );

        assert_eq!(result.status, HorizonStatus::Unavailable);
        assert_eq!(result.was_correct, None);
        assert_eq!(result.return_pct, None);
    }

    #[test]
    fn mature_horizon_direction_uses_action_and_return_sign() {
        assert_eq!(deterministic_horizon_was_correct("买入", 3.2, false), Some(1));
        assert_eq!(deterministic_horizon_was_correct("增持", -0.1, false), Some(0));
        assert_eq!(deterministic_horizon_was_correct("卖出", -3.2, false), Some(1));
        assert_eq!(deterministic_horizon_was_correct("减持", 0.1, false), Some(0));
        assert_eq!(deterministic_horizon_was_correct("持有", 3.2, false), None);
        assert_eq!(deterministic_horizon_was_correct("买入", -3.2, true), None);
        assert_eq!(deterministic_horizon_was_correct("买入", f64::NAN, false), None);
    }

    #[test]
    fn parses_snake_case_decision_fields_and_uses_horizon_default() {
        let source = serde_json::json!({
            "ultra_short": {
                "action": "BUY",
                "position_pct": 0.2,
                "expected_holding_days": null,
                "target_price": 102.0,
                "stop_loss": 98.0,
                "conf_lower_bound": 0.45
            }
        });
        let decisions = parse_horizon_decisions(source.as_object().unwrap()).unwrap();

        assert_eq!(decisions["ultra_short"].position_pct, Some(0.2));
        assert_eq!(decisions["ultra_short"].target_price, Some(102.0));
        assert_eq!(default_expected_holding_days("ultra_short"), 2);
        assert_eq!(default_expected_holding_days("short"), 5);
        assert_eq!(default_expected_holding_days("mid"), 28);
        assert_eq!(default_expected_holding_days("long"), 90);
    }

    #[test]
    fn horizon_group_carries_target_return_and_alpha() {
        // 5 条 horizon=short 样本：4 命中目标、净收益 1..5、alpha 仅 2 条有值
        let mut samples = Vec::new();
        for (idx, correct) in [1, 1, 1, 1, 0].into_iter().enumerate() {
            let mut s = sample(Some("买入"), Some("short"), correct, idx as f64 + 1.0);
            s.target_reached = Some(correct == 1);
            s.alpha_pct = if idx < 2 {
                Some(0.5 + idx as f64)
            } else {
                None
            };
            samples.push(s);
        }
        let stats = compute_reflection_stats(&samples);
        let group = stats.by_horizon.iter().find(|g| g.key == "short").unwrap();

        assert_eq!(group.samples, 5);
        assert_eq!(group.direction_hit_rate, Some(0.8));
        assert_eq!(group.target_hit_rate, Some(0.8));
        assert_eq!(group.avg_raw_return_pct, Some(3.0)); // (1+2+3+4+5)/5
        assert_eq!(group.avg_alpha_pct, Some(1.0)); // (0.5+1.5)/2
    }

    #[test]
    fn horizon_group_treats_missing_target_and_alpha_as_none() {
        // 全部样本无目标价判定 / 无 alpha → 两项指标必须是 None，不得伪报为 0
        let samples: Vec<_> =
            (0..5).map(|idx| sample(Some("买入"), Some("mid"), 1, idx as f64)).collect();
        let stats = compute_reflection_stats(&samples);
        let group = stats.by_horizon.iter().find(|g| g.key == "mid").unwrap();

        assert_eq!(group.samples, 5);
        assert_eq!(group.target_hit_rate, None);
        assert_eq!(group.avg_alpha_pct, None);
        assert_eq!(group.avg_raw_return_pct, Some(2.0));
    }

    #[test]
    fn horizon_min_sample_applied_per_horizon_independently() {
        // ultra_short 5 条、long 4 条：仅前者产出命中率，long 必须 None（按 horizon 独立门槛）
        let mut samples: Vec<_> =
            (0..5).map(|_| sample(Some("买入"), Some("ultra_short"), 1, 1.0)).collect();
        samples.extend((0..4).map(|_| sample(Some("买入"), Some("long"), 1, 1.0)));

        let stats = compute_reflection_stats(&samples);
        let ultra = stats.by_horizon.iter().find(|g| g.key == "ultra_short").unwrap();
        let long = stats.by_horizon.iter().find(|g| g.key == "long").unwrap();

        assert_eq!(ultra.direction_hit_rate, Some(1.0));
        assert_eq!(long.samples, 4);
        assert_eq!(long.direction_hit_rate, None);
    }

    #[test]
    fn legacy_samples_are_counted_without_changing_hit_rate() {
        let mut legacy = sample(Some("卖出"), Some("short"), 1, -3.0);
        legacy.data_source = SampleDataSource::Legacy;
        let samples = vec![
            legacy,
            sample(Some("买入"), Some("short"), 1, 1.0),
            sample(Some("买入"), Some("short"), 0, -1.0),
        ];
        let stats = compute_reflection_stats(&samples);

        assert_eq!(stats.total_samples, 3);
        assert_eq!(stats.legacy_samples, 1);
        // legacy 样本照常进分子分母：2 对 / 3 判定
        assert_eq!(stats.direction_hit_rate, None); // 3 < MIN_SAMPLE
    }
}
