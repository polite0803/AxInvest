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
}

/// 按维度分组的方向命中率
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitrateGroup {
    /// 分组键：action / horizon 的原始值，缺失归 "unknown"
    pub key: String,
    /// 已判定样本数（was_correct ∈ {0,1}）
    pub samples: usize,
    /// 方向命中率；样本 < [`MIN_SAMPLE`] → None（样本不足）
    pub direction_hit_rate: Option<f64>,
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

/// 按 key 分组统计方向命中率（保留首次出现的顺序，避免 HashMap 无序）。
/// 只统计已判定样本（was_correct ∈ {0,1}）；未判定样本不进任何分组。
fn group_hit_rates<F>(samples: &[DecisionPerformanceSample], key_of: F) -> Vec<HitrateGroup>
where
    F: Fn(&DecisionPerformanceSample) -> String,
{
    let mut order: Vec<String> = Vec::new();
    let mut agg: HashMap<String, (usize, usize)> = HashMap::new(); // (judged, correct)
    for s in samples {
        if s.was_correct != 0 && s.was_correct != 1 {
            continue;
        }
        let key = key_of(s);
        let entry = agg.entry(key.clone()).or_insert((0, 0));
        entry.0 += 1;
        if s.was_correct == 1 {
            entry.1 += 1;
        }
        if !order.contains(&key) {
            order.push(key);
        }
    }
    order
        .into_iter()
        .map(|key| {
            let (total, correct) = agg.remove(&key).unwrap_or((0, 0));
            HitrateGroup {
                key,
                samples: total,
                direction_hit_rate: hit_rate_or_none(correct, total),
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

    HitrateStats {
        total_samples: samples.len(),
        direction_hit_rate: hit_rate_or_none(correct_total, judged_total),
        target_hit_rate: hit_rate_or_none(target_hit, target_judged.len()),
        avg_raw_return_pct: mean(&returns),
        avg_alpha_pct: mean(&alphas),
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
}
