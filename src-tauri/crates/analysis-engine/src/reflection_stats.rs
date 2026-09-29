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

use axagent_harness::Period;
use serde::{Deserialize, Serialize};

/// 命中率的最低样本数：低于此值不产出命中率数值（诚实性铁律）
pub const MIN_SAMPLE: usize = 5;

/// rank IC 的最低样本数。
///
/// 比 [`MIN_SAMPLE`] 高一档是**刻意的**：命中率是两个计数的比，5 条已能给出可读区间；
/// 相关系数是四阶矩量，n=5 的抽样噪声就能把 ρ 推到 ±0.8，报出来比不报更误导。
/// 阈值宁可提供「样本不足」这一诚实缺席，也不产出无法与噪声区分的数字。
pub const IC_MIN_SAMPLE: usize = 8;

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
///
/// 天数从 `Period::default_holding_days` 取（唯一权威源，见
/// `axagent_harness::holding_period` 模块头）；本函数只做「周期字符串 → 档位」的解析，
/// 不再手抄一份天数表。未知/缺失周期沿用中线档（与既有行为一致）。
pub fn default_expected_holding_days(horizon: &str) -> i64 {
    horizon
        .parse::<Period>()
        .map(|p| p.default_holding_days() as i64)
        .unwrap_or(Period::Mid.default_holding_days() as i64)
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
    /// 该档决策的置信度 —— Phase E rank IC 的**预测侧**。
    /// 口径与 `portfolio-mgr.rhai` 输出一致：**0–100**（`(hconf*100)` 保留一位小数），
    /// 不是 0–1 概率；IC 是秩相关，量纲只要全体一致就不影响结果。
    /// 只有四周期 JSON 展开的样本有（`horizon_results_json.decision.confidence`）；
    /// legacy 回退路径拿不到逐档置信度，归 None ⇒ 进不了 IC 分母，不冒充「0 置信」。
    #[serde(default)]
    pub confidence: Option<f64>,
    /// 数据源标识（四周期展开 / legacy 回退）
    pub data_source: SampleDataSource,
}

/// 按维度分组的方向命中率与分周期指标
///
/// ⚠ 跨 IPC 边界（`reflection_stats` 命令返回值），字段必须 camelCase 输出 —— 本结构
/// 定义在 `analysis-engine`，而 `check-serde-annotations` 原先只扫 `harness/`+`commands/`，
/// 缺注解不会被报红，前端按 camelCase 读**恒为 undefined**（表现为「暂无数据」而非报错）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    /// 该组 rank IC（Spearman ρ：决策置信度 vs 实际净收益）—— Phase E 观测面。
    /// 可判样本 < [`IC_MIN_SAMPLE`] → None（样本不足，不产出与噪声难分的系数）。
    pub rank_ic: Option<f64>,
    /// 参与 IC 计算的样本数（同时有置信度与有限收益者）；与 `samples` 不同口径，
    /// 单独报出是为了让「IC 为 None 是因为缺置信度」与「是因为缺样本」可分辨。
    pub ic_samples: usize,
    /// IC 计算的**口径标注**（Phase F 的结构性缺席原则：不可得必须说清为什么）。
    /// `"ok"` = 正常产出；`"insufficient_ic_samples"` = 有预测值的样本太少；
    /// `"no_confidence"` = 该组一条置信度都没有（legacy 回退样本）。
    pub ic_status: String,
    /// 该档的期望持有交易日（半衰期拟合的自变量）；未知档 → None
    pub holding_days: Option<i64>,
}

/// 命中率聚合结果（DTO；前端按 camelCase 消费，见 src/types/stock-analysis.ts）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
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
    /// 预测半衰期（交易日）—— 由逐档 rank IC 的指数衰减拟合导出（[`signal_half_life_days`]）。
    /// None 的三种真实原因由 `usable_ic_tiers` 判别：不足 3 档 / IC 非正 / 拟合不达标。
    /// **观测面**：Phase E 只报数，不回写任何权重。
    pub signal_half_life_days: Option<f64>,
    /// 参与半衰期拟合的档数（有持有期且 rank IC 有值的档）
    pub usable_ic_tiers: usize,
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

/// 分组聚合中间态（一次遍历同时累加方向 / 目标价 / 收益 / alpha / IC 五类指标）
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
    /// IC 样本对：(决策置信度, 实际净收益%)。缺任一不进此列 —— 宁缺毋伪。
    ic_pairs: Vec<(f64, f64)>,
}

/// rank IC 的唯一实现来自 [`axagent_harness::indicators::spearman_rank_ic`]（秩相关 =
/// `pearson(average_ranks(x), average_ranks(y))`）—— 本仓已有两份同族实现
/// （`hit_rate_backtest` 的 9 因子 IC、`portfolio_monitor` 的 Pearson），
/// 再抄第三份就是禁区 12。此处只 re-export，本模块负责的是**门槛与缺席标注**：
/// 样本数门槛见 [`IC_MIN_SAMPLE`]，缺席原因见 [`HitrateGroup::ic_status`]。
pub use axagent_harness::indicators::spearman_rank_ic;

/// 由逐档 (持有天数, rank IC) 序列拟合**预测半衰期**。
///
/// 模型：IC(h) ≈ IC₀ · e^(−λh) ⇒ 半衰期 t½ = ln2 / λ。对 IC>0 的档取 ln|IC| 对 h
/// 做最小二乘直线拟合（斜率 −λ）。
/// 诚实性（三重，任一不满足即 None，绝不外推）：
/// 1. 至少 3 个有效点 —— 两点总能连出一条线，把「样本噪声」卖成「衰减规律」；
/// 2. 拟合斜率必须 < 0 —— 没有衰减就不该报半衰期（报 None 而不是「无限长」）；
/// 3. 决定系数 R² 必须 ≥ [`HALF_LIFE_MIN_R2`] —— 拟合本身不可信时，给出的天数是
///    装饰而非信息。调用方按 None 显示「无法拟合」。
pub fn signal_half_life_days(points: &[(f64, f64)]) -> Option<f64> {
    let pts: Vec<(f64, f64)> = points
        .iter()
        .filter(|(h, ic)| h.is_finite() && ic.is_finite() && *h > 0.0 && *ic > 0.0)
        .map(|(h, ic)| (*h, ic.ln()))
        .collect();
    if pts.len() < 3 {
        return None;
    }
    let n = pts.len() as f64;
    let mx = pts.iter().map(|p| p.0).sum::<f64>() / n;
    let my = pts.iter().map(|p| p.1).sum::<f64>() / n;
    let mut sxx = 0.0;
    let mut sxy = 0.0;
    let mut syy = 0.0;
    for (x, y) in &pts {
        sxx += (x - mx) * (x - mx);
        sxy += (x - mx) * (y - my);
        syy += (y - my) * (y - my);
    }
    if sxx <= 0.0 {
        return None;
    }
    let slope = sxy / sxx;
    if slope >= 0.0 {
        return None;
    }
    // R² = 1 − SSE/SST；一元回归下 SSE = SST − slope²·SXX ⇒ R² = slope²·SXX/SST
    if syy <= 0.0 {
        return None;
    }
    let r2 = (slope * slope * sxx) / syy;
    if r2 < HALF_LIFE_MIN_R2 {
        return None;
    }
    Some(std::f64::consts::LN_2 / -slope)
}

/// 半衰期拟合的最低拟合优度：低于此值说明「天数 → lnIC」根本不成直线，
/// 报出来的半衰期是拟合噪声而非衰减。
pub const HALF_LIFE_MIN_R2: f64 = 0.5;

/// 按 key 分组统计方向命中率、分周期指标与 rank IC（保留首次出现的顺序，避免 HashMap 无序）。
/// 只统计已判定样本（was_correct ∈ {0,1}）；未判定样本不进任何分组。
///
/// `days_of` 只在 horizon 分组下有值（半衰期拟合的自变量）；action 分组传 `|_| None`。
fn group_hit_rates<F, G>(
    samples: &[DecisionPerformanceSample],
    key_of: F,
    days_of: G,
) -> Vec<HitrateGroup>
where
    F: Fn(&DecisionPerformanceSample) -> String,
    G: Fn(&str) -> Option<i64>,
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
        if let Some(conf) = s.confidence {
            entry.ic_pairs.push((conf, s.return_pct));
        }
        if !order.contains(&key) {
            order.push(key);
        }
    }
    order
        .into_iter()
        .map(|key| {
            let a = agg.remove(&key).unwrap_or_default();
            let ic_status = if a.ic_pairs.is_empty() {
                "no_confidence"
            } else if a.ic_pairs.len() < IC_MIN_SAMPLE {
                "insufficient_ic_samples"
            } else {
                "ok"
            };
            let rank_ic = if a.ic_pairs.len() >= IC_MIN_SAMPLE {
                spearman_rank_ic(&a.ic_pairs)
            } else {
                None
            };
            // 秩全同（置信度或收益一侧无方差）时 ρ 无定义 → None，状态跟着改口径，
            // 不能让前端把「样本够了却算不出」误读成「样本不足」。
            let ic_status = if rank_ic.is_none() && ic_status == "ok" {
                "degenerate_variance"
            } else {
                ic_status
            };
            HitrateGroup {
                key: key.clone(),
                samples: a.judged,
                direction_hit_rate: hit_rate_or_none(a.correct, a.judged),
                target_hit_rate: hit_rate_or_none(a.target_hit, a.target_judged),
                avg_raw_return_pct: mean(&a.returns),
                avg_alpha_pct: mean(&a.alphas),
                rank_ic,
                ic_samples: a.ic_pairs.len(),
                ic_status: ic_status.to_string(),
                holding_days: days_of(&key),
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

    let by_action = group_hit_rates(
        samples,
        |s| s.action.clone().unwrap_or_else(|| "unknown".to_string()),
        |_| None,
    );
    let by_horizon = group_hit_rates(
        samples,
        |s| s.horizon.clone().unwrap_or_else(|| "unknown".to_string()),
        |key| {
            // "unknown" 没有持有期可言；其余走权威天数表（不手抄）。
            if key == "unknown" {
                None
            } else {
                Some(default_expected_holding_days(key))
            }
        },
    );
    let ic_points: Vec<(f64, f64)> = by_horizon
        .iter()
        .filter(|g| g.key != "unknown")
        .filter_map(|g| match (g.holding_days, g.rank_ic) {
            (Some(d), Some(ic)) => Some((d as f64, ic)),
            _ => None,
        })
        .collect();
    let usable_ic_tiers = ic_points.len();
    let signal_half_life_days = signal_half_life_days(&ic_points);

    HitrateStats {
        total_samples: samples.len(),
        direction_hit_rate: hit_rate_or_none(correct_total, judged_total),
        target_hit_rate: hit_rate_or_none(target_hit, target_judged.len()),
        avg_raw_return_pct: mean(&returns),
        avg_alpha_pct: mean(&alphas),
        legacy_samples,
        by_action,
        by_horizon,
        signal_half_life_days,
        usable_ic_tiers,
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
            confidence: None,
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

    // ── Phase E：rank IC 与预测半衰期 ──

    fn conf_sample(horizon: &str, conf: f64, ret: f64) -> DecisionPerformanceSample {
        let mut s = sample(Some("买入"), Some(horizon), 1, ret);
        s.confidence = Some(conf);
        s
    }

    #[test]
    fn spearman_is_perfectly_monotone_regardless_of_scaling() {
        // 秩相关只看顺序：把 y 换成任意严格递增变换，ρ 必须仍是 1
        let pairs: Vec<(f64, f64)> =
            (0..10).map(|i| (i as f64, (i as f64).powi(3) + 7.0)).collect();
        assert_eq!(spearman_rank_ic(&pairs), Some(1.0));
        let rev: Vec<(f64, f64)> = pairs.iter().map(|(x, y)| (*x, -y)).collect();
        assert_eq!(spearman_rank_ic(&rev), Some(-1.0));
    }

    #[test]
    fn spearman_uses_mid_ranks_for_ties() {
        // x 有并列：(1,1,2,3) → 秩 (1.5,1.5,3,4)；y 严格递增 ⇒ ρ 是确定值而非 NaN
        let pairs = vec![(1.0, 1.0), (1.0, 2.0), (2.0, 3.0), (3.0, 4.0)];
        let ic = spearman_rank_ic(&pairs).unwrap();
        // 手算：rx=(1.5,1.5,3,4)、ry=(1,2,3,4) ⇒ cov=4.5, vx=4.5, vy=5.0
        // ⇒ ρ = 4.5/√22.5 = 0.9486833
        assert!((ic - 0.9486833).abs() < 1e-6, "并列未取平均秩：{ic}");
    }

    #[test]
    fn spearman_returns_none_when_undefined_not_fake_zero() {
        // 一侧无方差（全同分）⇒ ρ 无定义，必须 None（返回 0 会被读成「无相关」）
        assert_eq!(spearman_rank_ic(&[(0.5, 1.0), (0.5, 2.0), (0.5, 3.0)]), None);
        // 样本 < 2 / NaN / Inf 同样 None，不产出系数
        assert_eq!(spearman_rank_ic(&[(0.5, 1.0)]), None);
        assert_eq!(spearman_rank_ic(&[(f64::NAN, 1.0), (0.5, 2.0)]), None);
        assert_eq!(spearman_rank_ic(&[(0.5, f64::INFINITY), (0.6, 2.0)]), None);
    }

    #[test]
    fn ic_gate_is_eight_and_labels_why_it_is_missing() {
        // 7 条带置信度的样本：命中率照出（≥5），IC 必须 None 且状态点名「样本不足」
        let samples: Vec<_> =
            (0..7).map(|i| conf_sample("short", 0.5 + i as f64 * 0.01, i as f64)).collect();
        let stats = compute_reflection_stats(&samples);
        let g = stats.by_horizon.iter().find(|g| g.key == "short").unwrap();
        assert_eq!(g.direction_hit_rate, Some(1.0));
        assert_eq!(g.rank_ic, None);
        assert_eq!(g.ic_samples, 7);
        assert_eq!(g.ic_status, "insufficient_ic_samples");

        // 补到 8 条 ⇒ 同一批口径下 IC 出数
        let mut more = samples.clone();
        more.push(conf_sample("short", 0.58, 7.0));
        let stats = compute_reflection_stats(&more);
        let g = stats.by_horizon.iter().find(|g| g.key == "short").unwrap();
        assert_eq!(g.ic_status, "ok");
        assert_eq!(g.rank_ic, Some(1.0));
    }

    #[test]
    fn missing_confidence_is_labeled_apart_from_missing_samples() {
        // 全部无置信度（legacy 回退的真实形态）：缺席原因必须是 no_confidence，
        // 不能与「有置信度但太少」混成同一句话 —— 前者要补字段、后者要攒样本。
        let samples: Vec<_> =
            (0..6).map(|i| sample(Some("买入"), Some("long"), 1, i as f64)).collect();
        let stats = compute_reflection_stats(&samples);
        let g = stats.by_horizon.iter().find(|g| g.key == "long").unwrap();
        assert_eq!(g.rank_ic, None);
        assert_eq!(g.ic_samples, 0);
        assert_eq!(g.ic_status, "no_confidence");
    }

    #[test]
    fn horizon_groups_carry_holding_days_from_the_single_authority() {
        let samples: Vec<_> = ["ultra_short", "short", "mid", "long"]
            .iter()
            .flat_map(|h| (0..8).map(move |i| conf_sample(h, 0.5 + i as f64 * 0.01, i as f64)))
            .chain((0..8).map(|i| sample(None, None, 1, i as f64)))
            .collect();
        let stats = compute_reflection_stats(&samples);
        for (key, days) in [("ultra_short", 2_i64), ("short", 5), ("mid", 28), ("long", 90)] {
            let g = stats.by_horizon.iter().find(|g| g.key == key).unwrap();
            assert_eq!(g.holding_days, Some(days), "{key} 持有期未取自权威天数表");
        }
        // 维度缺失档：没有持有期可言，必须是 None 而不是中线兜底
        let unk = stats.by_horizon.iter().find(|g| g.key == "unknown").unwrap();
        assert_eq!(unk.holding_days, None);
    }

    #[test]
    fn half_life_fits_exponential_ic_decay() {
        // 构造 IC(h)=0.6·e^(−h/20)：h=2/5/28/90 ⇒ λ=0.05 ⇒ t½=ln2/0.05≈13.86
        let pts: Vec<(f64, f64)> = [2.0_f64, 5.0, 28.0, 90.0]
            .iter()
            .map(|h| (*h, 0.6_f64 * (-h / 20.0_f64).exp()))
            .collect();
        let t_half = signal_half_life_days(&pts).unwrap();
        assert!((t_half - 13.8629).abs() < 0.01, "拟合半衰期偏离解析值：{t_half}");
    }

    #[test]
    fn half_life_refuses_to_invent_a_number() {
        // ① 不足 3 个点：两点连线必然完美，但不能卖成规律
        assert_eq!(signal_half_life_days(&[(2.0, 0.4), (5.0, 0.3)]), None);
        // ② IC 随持有期**上升**（无衰减）⇒ None，不是「半衰期无限长」
        let rising = vec![(2.0, 0.1), (5.0, 0.2), (28.0, 0.3), (90.0, 0.4)];
        assert_eq!(signal_half_life_days(&rising), None);
        // ③ 非正 IC 不进拟合（ln 无定义），只剩两点 ⇒ None
        let mixed = vec![(2.0, 0.4), (5.0, -0.1), (28.0, 0.1), (90.0, -0.2)];
        assert_eq!(signal_half_life_days(&mixed), None);
        // ④ 有衰减但噪声压过趋势（R² < 0.5）⇒ None
        let noisy = vec![(2.0, 0.02), (5.0, 0.5), (28.0, 0.01), (90.0, 0.4)];
        assert_eq!(signal_half_life_days(&noisy), None);
    }

    #[test]
    fn half_life_is_reported_only_from_tier_ic_that_actually_exists() {
        // 三档各 8 条，逐档 IC 由「置换距离」构造（无并列 ⇒ ρ = 1 − 6Σd²/(n(n²−1))）：
        // ultra_short 全对 ⇒ 1.0；short 三处相邻互换 Σd²=6 ⇒ 0.928571；
        // mid 四处互换 Σd²=24 ⇒ 0.714286。IC 随持有期衰减 ⇒ 拟合应给出正半衰期。
        let perms: [(&str, [usize; 8]); 3] = [
            ("ultra_short", [0, 1, 2, 3, 4, 5, 6, 7]),
            ("short", [1, 0, 3, 2, 5, 4, 6, 7]),
            ("mid", [1, 0, 3, 2, 7, 6, 5, 4]),
        ];
        let mut samples = Vec::new();
        for (h, perm) in perms {
            for (i, &ret) in perm.iter().enumerate() {
                let mut s = sample(Some("买入"), Some(h), 1, ret as f64);
                s.confidence = Some(i as f64);
                samples.push(s);
            }
        }
        let stats = compute_reflection_stats(&samples);
        let want = [
            ("ultra_short", 1.0),
            ("short", 1.0 - 6.0 * 6.0 / 504.0),
            ("mid", 1.0 - 6.0 * 24.0 / 504.0),
        ];
        for (key, ic) in want {
            let g = stats.by_horizon.iter().find(|g| g.key == key).unwrap();
            let got = g.rank_ic.expect("该档 IC 应产出");
            assert!((got - ic).abs() < 1e-9, "{key} 档 IC 与闭式解不符：{got} vs {ic}");
        }
        assert_eq!(stats.usable_ic_tiers, 3);
        let t_half = stats.signal_half_life_days.expect("三档递减 IC 应拟合出半衰期");
        assert!(t_half > 0.0 && t_half < 200.0, "半衰期数量级不合理：{t_half}");

        // 只有一档够样本 ⇒ 拟合无从谈起，两个字段必须一起沉默
        let single: Vec<_> = (0..8)
            .map(|i| {
                let mut s = sample(Some("买入"), Some("short"), 1, i as f64);
                s.confidence = Some(i as f64);
                s
            })
            .collect();
        let stats = compute_reflection_stats(&single);
        assert_eq!(stats.usable_ic_tiers, 1);
        assert_eq!(stats.signal_half_life_days, None);
    }

    #[test]
    fn action_groups_never_carry_holding_days() {
        // 半衰期的自变量是「持有期」，action 分组没有这个维度 ⇒ 逐条 None，
        // 防止把 horizon 的天数串味到 action 表上。
        let samples: Vec<_> = (0..8)
            .map(|i| {
                let mut s = conf_sample("short", i as f64 * 0.01, i as f64);
                s.action = Some("买入".into());
                s
            })
            .collect();
        let stats = compute_reflection_stats(&samples);
        let g = stats.by_action.iter().find(|g| g.key == "买入").unwrap();
        assert_eq!(g.holding_days, None);
        assert_eq!(g.rank_ic, Some(1.0));
    }

    /// IPC 契约：命中率 DTO 跨边界必须输出 camelCase（禁区 13）。
    /// 这两个 DTO 定义在 `analysis-engine`，而 `check-serde-annotations` 只扫
    /// `harness/` 与 `commands/` ⇒ 注解缺失不会有门禁报红，前端按 camelCase 读**恒为
    /// undefined**（表现为命中率卡永远「暂无数据」，而不是报错）。因此把键名逐条锁在
    /// 这里 —— 先于修复本测试必红，它就是那个缺失的检法。
    #[test]
    fn hitrate_dto_serializes_camel_case_for_ipc() {
        let stats = HitrateStats {
            total_samples: 9,
            direction_hit_rate: Some(0.5),
            target_hit_rate: Some(0.4),
            avg_raw_return_pct: Some(1.2),
            avg_alpha_pct: Some(0.3),
            legacy_samples: 2,
            by_action: vec![HitrateGroup {
                key: "买入".into(),
                samples: 5,
                direction_hit_rate: Some(0.6),
                rank_ic: Some(0.2),
                ic_samples: 8,
                ic_status: "ok".into(),
                ..Default::default()
            }],
            by_horizon: vec![HitrateGroup {
                key: "short".into(),
                samples: 5,
                holding_days: Some(5),
                ..Default::default()
            }],
            signal_half_life_days: Some(13.86),
            usable_ic_tiers: 3,
        };
        let value = serde_json::to_value(&stats).unwrap();
        let top = value.as_object().unwrap();
        for key in [
            "totalSamples",
            "directionHitRate",
            "targetHitRate",
            "avgRawReturnPct",
            "avgAlphaPct",
            "legacySamples",
            "byAction",
            "byHorizon",
            "signalHalfLifeDays",
            "usableIcTiers",
        ] {
            assert!(top.contains_key(key), "HitrateStats 顶层缺 camelCase 键 {key}：{top:?}");
        }
        let group = value["byHorizon"][0].as_object().unwrap();
        for key in [
            "key",
            "samples",
            "directionHitRate",
            "targetHitRate",
            "avgRawReturnPct",
            "avgAlphaPct",
            "rankIc",
            "icSamples",
            "icStatus",
            "holdingDays",
        ] {
            assert!(group.contains_key(key), "HitrateGroup 缺 camelCase 键 {key}：{group:?}");
        }
        // 反向锁：snake_case 旧键名不得回来（那正是前端读不到的形态）
        assert!(!top.contains_key("direction_hit_rate"), "顶层又出现 snake_case 键");
        assert!(!group.contains_key("rank_ic"), "分组又出现 snake_case 键");
    }
}
