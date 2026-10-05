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

use axagent_entities::{stock_analyses, stock_reflections, strategy_performance};
use axagent_harness::holding_period::HORIZON_BRANCH_GENERATION_FLOOR;
use axagent_harness::Period;
use sea_orm::{DatabaseConnection, EntityTrait};
use serde::{Deserialize, Serialize};

/// 命中率的最低样本数：低于此值不产出命中率数值（诚实性铁律）
pub const MIN_SAMPLE: usize = 5;

/// rank IC 的最低样本数。
///
/// 比 [`MIN_SAMPLE`] 高一档是**刻意的**：命中率是两个计数的比，5 条已能给出可读区间；
/// 相关系数是四阶矩量，n=5 的抽样噪声就能把 ρ 推到 ±0.8，报出来比不报更误导。
/// 阈值宁可提供「样本不足」这一诚实缺席，也不产出无法与噪声区分的数字。
pub const IC_MIN_SAMPLE: usize = 8;

/// 判定口径水印的锚定天数（与 `portfolio-mgr.rhai` 的 `SNR_ANCHOR_DAYS` 同值）。
/// 引擎侧只把它当「有没有」的水印用，不参与任何数值计算。
pub const SNR_ANCHOR: i64 = 28;

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
    /// **判定口径水印**（Phase D-2 起才有，值 = 该次决策 SNR 折算的锚定天数）。
    ///
    /// 为什么必须一路带进反思 JSON：v104 之前逐档 `confidence` = 生效后验，v104 起
    /// = `0.5+(生效后验−0.5)·√(h/28)`。两者**跨版本混在同一档里会改变排序**（同一档内
    /// 各自单调，但两代的刻度不同），rank IC 因此被污染。有水印才允许进 IC 分母，
    /// 没水印就是「上一代口径的记录」，如实标出来而不是默默混算。
    #[serde(alias = "snr_anchor_days", default)]
    pub snr_anchor_days: Option<i64>,
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
    /// 该样本所属的**判定口径水印**（见 [`HorizonDecision::snr_anchor_days`]）。
    /// `None` = 上一代口径（v104 前）的记录 ⇒ 其 confidence 与本档其余样本不同刻度，
    /// **不进 IC 分母**，并被计入 [`HitrateGroup::ic_regime_excluded`] 如实报出。
    #[serde(default)]
    pub snr_anchor_days: Option<i64>,
    /// 该样本所属的**算法代际**（= 被复盘那条分析的 `template_version`，§五十一-②/§七十三）。
    /// 筛选规则是**下限** [`HORIZON_BRANCH_GENERATION_FLOOR`]：`>= floor` 进分母；
    /// 早于 floor 的（`generation_pre_floor`）与 `None`（代际未知）都**排除**并分别计数 ——
    /// 两者处置不同：旧代样本永久不可比，未知代的要等写侧补章/重跑。
    /// 与 `snr_anchor_days` 的区别：那个水印管的是**IC 的置信度刻度**，这个管的是
    /// **全部统计量的分母**（命中率/IC/先验/权重），两者不可互相顶替。
    #[serde(default)]
    pub template_version: Option<i32>,
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
    /// `"no_confidence"` = 该组一条置信度都没有（legacy 回退样本）；
    /// `"degenerate_variance"` = 样本够但秩无方差（ρ 无定义）；
    /// `"pre_snr_regime"` = 有置信度但**全部缺判定口径水印**（v104 前的旧记录），
    /// 与上一类「没有置信度」是两件事：那要补字段，这要等样本换代累积。
    pub ic_status: String,
    /// 因「缺判定口径水印」被排除出 IC 分母的样本数（换代进度可见：它归零即说明
    /// 该档样本已全部属于当前口径）。
    pub ic_regime_excluded: usize,
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
    /// 本次统计采用的**起算代际** = [`HORIZON_BRANCH_GENERATION_FLOOR`]（§五十一-②）。
    /// 回传它是为了让面板能说清「分母为什么变小」——这个数是判据参数，不是运行时状态。
    #[serde(default)]
    pub generation_floor: i32,
    /// 因「早于起算代」被排除出**全部统计分母**的样本数。
    /// 与 `ic_regime_excluded` 的区别：那个只管 IC 这一项，这个管命中率/收益/IC/先验全套。
    #[serde(default)]
    pub excluded_pre_floor_generation: usize,
    /// 因「代际未知(NULL)」被排除的样本数 —— 与上一项分开计数，因为处置不同：
    /// 旧代样本**永远不会**再进分母（算法不可比），代际未知的会随写侧补章而减少。
    #[serde(default)]
    pub excluded_unknown_generation: usize,
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
    /// ⚠ 还要求该样本**带判定口径水印**（见 [`DecisionPerformanceSample::snr_anchor_days`]）：
    ///   v104 前后的 confidence 不是同一把尺，混在一档里会把秩排错，进而把 IC 算歪。
    ic_pairs: Vec<(f64, f64)>,
    /// 有置信度但缺水印 ⇒ 因口径换代被排除的条数（计入 `ic_regime_excluded`）。
    ic_regime_excluded: usize,
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
            // 水印缺失 = 上一代口径的 confidence，与同档其他样本不同尺 ⇒ 不进 IC，
            // 但要计数报出去（「样本还不够新」和「根本没有预测值」是两件不同的事）。
            if s.snr_anchor_days.is_some() {
                entry.ic_pairs.push((conf, s.return_pct));
            } else {
                entry.ic_regime_excluded += 1;
            }
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
                // 有预测值但全缺水印 ⇒ 是「口径换代、新样本还没攒够」，不是「没有预测值」
                if a.ic_regime_excluded > 0 {
                    "pre_snr_regime"
                } else {
                    "no_confidence"
                }
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
                ic_regime_excluded: a.ic_regime_excluded,
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
        // 代际三项由取数侧（`build_hitrate_stats`）填 —— 纯函数不做筛样。
        generation_floor: 0,
        excluded_pre_floor_generation: 0,
        excluded_unknown_generation: 0,
    }
}

// ── 落库行 → 统计样本的取数 + 展开（DB 边界）─────────────────────────────
//
// 以下四项原本住在 `commands/stock_workflow/reflection_stats.rs`，2026-09-29 搬到
// 这里：`reflection_stats` 命令、`stock_workflow/hooks.rs` 的先验注入、
// `stock_analysis.rs` 的荐股先验三处共用同一实现。放命令层会导致后两处跨模块
// 调用命令（分层门禁规则 2 `commands-no-sibling-call`），放这里则三处都只是
// 「调 engine 纯函数」，依赖方向保持 consumer→implementor（见 AGENTS.md 分层表）。
//
// 跨三表聚合口径：
// - `strategy_performance`：主表（was_correct 确定性判定 + 复盘 win/loss 写回、return_pct）
// - `stock_reflections`：反思行（alpha_return、blackboard_snapshot 里递归找 target_reached）
// - `stock_analyses`：维度（decision_action / decision_time_horizon）
//
// join 口径：
// 1. **反思行**（strategy_id="reflection_verdict"）：(stock_code, created_at)
//    精确匹配 stock_reflections **同批记录** → 经 original_analysis_id 精确取
//    stock_analyses 维度；alpha / target_reached 取反思行。
// 2. **分析回测行**（复盘 cron 写回，strategy_id=trend/value/...）：stock_code +
//    decision_at 与 stock_analyses.created_at 时间窗（±3 天）匹配最近分析行取维度。
//
// 诚实性：target_reached 未独立落库（反思时仅在内存 MarketSnapshot），故从
// blackboard_snapshot 递归查找；找不到 → None（前端按「样本不足/暂无数据」展示，不伪报）。

/// 时间窗匹配阈值：decision_at 与分析 created_at 允许的最大偏差（3 天）
const JOIN_WINDOW_MS: i64 = 3 * 86_400_000;

/// 在 JSON 值中递归查找指定 key 的布尔值（blackboard_snapshot 结构不确定，
/// 兼容「一层嵌套 / 多层嵌套 / 数组内」三种形态；找不到 → None）。
fn find_bool_recursive(value: &serde_json::Value, key: &str) -> Option<bool> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(b) = map.get(key).and_then(|v| v.as_bool()) {
                return Some(b);
            }
            for v in map.values() {
                if let Some(b) = find_bool_recursive(v, key) {
                    return Some(b);
                }
            }
            None
        },
        serde_json::Value::Array(arr) => arr.iter().find_map(|v| find_bool_recursive(v, key)),
        _ => None,
    }
}

/// 从四周期反思 JSON（`stock_reflections.horizon_results_json`）展开统计样本。
///
/// 展开口径（PLAN-stock-reflection-four-horizon 批次 4 第 11 条）：
/// - 仅 `mature` / `legacy` 周期可成样本；`immature` / `unavailable` **不进分母**。
/// - 还要求 `evaluation.wasCorrect ∈ {0,1}`（中性档 = 不可判定）且 `market.returnPct` 为有限数，
///   否则该周期整体跳过 —— 不伪造 0% 收益，也不把「未到期」写成「判错」。
/// - 已成熟周期的 action / 收益 / alpha / 目标价全部取自该周期自身条目，不与其他周期串线。
///
/// 返回 `None` 表示该 JSON 不是四周期对象（无法解析 / 非对象），调用方回退 legacy 单周期字段。
fn expand_horizon_samples(
    json: &str,
    template_version: Option<i32>,
) -> Option<Vec<DecisionPerformanceSample>> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let map = value.as_object()?;
    let mut samples = Vec::new();
    for (horizon, entry) in map {
        let Some(entry) = entry.as_object() else {
            continue; // schemaVersion 等标量键
        };
        let status = entry.get("status").and_then(|v| v.as_str()).unwrap_or("");
        if status != "mature" && status != "legacy" {
            continue;
        }
        let was_correct = entry
            .get("evaluation")
            .and_then(|e| e.get("wasCorrect"))
            .and_then(|v| v.as_i64())
            .and_then(|v| i32::try_from(v).ok())
            .filter(|v| *v == 0 || *v == 1);
        let Some(was_correct) = was_correct else { continue };
        let market = entry.get("market");
        let return_pct = market
            .and_then(|m| m.get("returnPct"))
            .and_then(|v| v.as_f64())
            .filter(|v| v.is_finite());
        let Some(return_pct) = return_pct else { continue };

        samples.push(DecisionPerformanceSample {
            action: entry
                .get("decision")
                .and_then(|d| d.get("action"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            horizon: Some(horizon.clone()),
            was_correct,
            return_pct,
            alpha_pct: market
                .and_then(|m| m.get("alphaPct"))
                .and_then(|v| v.as_f64())
                .filter(|v| v.is_finite()),
            target_reached: market.and_then(|m| m.get("targetReached")).and_then(|v| v.as_bool()),
            // Phase E：IC 的预测侧。旧记录无 confidence 字段 → None（不进 IC 分母），
            // 不做「用 action 反推置信度」这类填充 —— 那是把缺失伪装成测量。
            confidence: entry
                .get("decision")
                .and_then(|d| d.get("confidence"))
                .and_then(|v| v.as_f64())
                .filter(|v| v.is_finite()),
            // 判定口径水印：v104 前逐档 confidence = 生效后验，v104 起 = SNR √h 折算值，
            // 两代混在一档里会打乱池化秩 ⇒ 引擎按此位决定是否进 IC 分母。
            snr_anchor_days: entry
                .get("decision")
                .and_then(|d| d.get("snrAnchorDays").or_else(|| d.get("snr_anchor_days")))
                .and_then(|v| v.as_i64()),
            template_version,
            data_source: if status == "legacy" {
                SampleDataSource::Legacy
            } else {
                SampleDataSource::Reflection
            },
        });
    }
    Some(samples)
}

/// 按 (stock_code, decision_at) 在分析行中找时间窗内最近的一条，返回其维度。
fn nearest_analysis_dimensions<'a>(
    analyses: &'a [stock_analyses::Model],
    stock_code: &str,
    decision_at: i64,
) -> Option<&'a stock_analyses::Model> {
    analyses
        .iter()
        .filter(|a| a.stock_code == stock_code)
        .filter(|a| (a.created_at - decision_at).abs() <= JOIN_WINDOW_MS)
        .min_by_key(|a| (a.created_at - decision_at).abs())
}

/// 取数 + 逐周期展开 + 聚合的**唯一实现**（三处调用方共用，禁区 12：不重复实现）。
///
/// 2026-09-29 从命令体抽出：四周期科学化 Phase C 要在**工作流启动时**读同一份统计
/// （`hooks.rs` 据此注入 `horizon_prior_json` 逐档先验）。若留在命令体里，hooks 只能
/// 再抄一遍取数，且 `stock_analysis.rs` 的荐股先验也要调 —— 两处跨模块调命令 =
/// 分层门禁规则 2 违规。落到 engine 后三处都只是调本函数。
pub async fn build_hitrate_stats(db: &DatabaseConnection) -> Result<HitrateStats, String> {
    let sp_rows = strategy_performance::Entity::find()
        .all(db)
        .await
        .map_err(|e| format!("读取 strategy_performance 失败: {e}"))?;
    let ref_rows = stock_reflections::Entity::find()
        .all(db)
        .await
        .map_err(|e| format!("读取 stock_reflections 失败: {e}"))?;
    let ana_rows = stock_analyses::Entity::find()
        .all(db)
        .await
        .map_err(|e| format!("读取 stock_analyses 失败: {e}"))?;

    // 索引 1：stock_analyses.id → Model
    let ana_by_id: HashMap<&str, &stock_analyses::Model> =
        ana_rows.iter().map(|a| (a.id.as_str(), a)).collect();
    // 索引 2：反思行按 (stock_code, created_at, 复盘档) 批匹配（同批写入的时间戳相同）。
    //
    // 为什么键里必须带档：〇-B v2 起**一行反思 = 一个周期档**，同一条分析产出的四个档行
    // 由建点用**同一个 now_ms** 写入 `created_at` ⇒ 只用 (stock_code, created_at) 做键时
    // 四行同键，`.or_insert` 会**随机丢掉三档**（命中率统计的档间差异被抹平）。
    // sp 侧的档来自 `period` 的 `reflection:{档}` 后缀（写入点：
    // `commands/stock_workflow/reflection.rs` 的 strategy_performance 构造）。
    // 老数据两侧同构：sp.period 无后缀 ⇔ 反思行 horizon NULL ⇒ 仍按 None 相配。
    let mut ref_by_batch: HashMap<(String, i64, Option<String>), &stock_reflections::Model> =
        HashMap::new();
    for r in &ref_rows {
        ref_by_batch.entry((r.stock_code.clone(), r.created_at, r.horizon.clone())).or_insert(r);
    }

    let mut samples = Vec::with_capacity(sp_rows.len());
    for sp in &sp_rows {
        let sp_horizon = sp.period.strip_prefix("reflection:").map(str::to_string);
        let matched = ref_by_batch.get(&(sp.stock_code.clone(), sp.created_at, sp_horizon.clone()));
        // §七十三 代际归属：优先取**反思行的建点盖章**（#5 落的列），配不上反思行的 sp 行
        // 退到分析行的时间窗匹配（`nearest_analysis_dimensions` 与上面 action/horizon 同源）。
        // 两条都拿不到 ⇒ None = 代际未知，**不退回编译期版本**（那会把「不知道」写成「就是这一代」）。
        let sp_template_version: Option<i32> =
            matched.and_then(|r| r.template_version).or_else(|| {
                nearest_analysis_dimensions(&ana_rows, &sp.stock_code, sp.decision_at)
                    .and_then(|a| a.template_version)
            });

        // 路径 1：反思行有四周期结果 JSON → 按周期独立展开（每周期一条样本）
        if let Some(expanded) = matched
            .and_then(|r| r.horizon_results_json.as_deref())
            .and_then(|j| expand_horizon_samples(j, sp_template_version))
        {
            samples.extend(expanded);
            continue;
        }

        // 路径 2：旧记录无四周期 JSON → 回退 legacy 主周期字段，并带 legacy 数据源标识
        let mut action = None;
        let mut horizon = None;
        let mut alpha_pct = None;
        let mut target_reached = None;

        if let Some(r) = matched {
            alpha_pct = r.alpha_return;
            target_reached = r
                .blackboard_snapshot
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .and_then(|v| find_bool_recursive(&v, "target_reached"));
            // `original_analysis_id` 是**非空 String**（entity `stock_reflections.rs:14`），
            // 不是 Option ⇒ 直接取 `&str` 查索引，没有 Option 链可 `.and_then`。
            if let Some(a) = ana_by_id.get(r.original_analysis_id.as_str()) {
                action = a.decision_action.clone();
                horizon = a.decision_time_horizon.clone();
            }
        }

        // 时间窗匹配（分析回测行 / 反思批匹配失败兜底）
        if action.is_none() {
            if let Some(a) = nearest_analysis_dimensions(&ana_rows, &sp.stock_code, sp.decision_at)
            {
                action = a.decision_action.clone();
                horizon = a.decision_time_horizon.clone();
            }
        }

        samples.push(DecisionPerformanceSample {
            action,
            horizon,
            was_correct: sp.was_correct,
            return_pct: sp.return_pct,
            alpha_pct,
            target_reached,
            // legacy 回退路径没有逐档决策，置信度只能是 None —— 该样本进命中率
            // 分母，但**不进 IC 分母**（IC 缺席由 `ic_status="no_confidence"` 点名）。
            confidence: None,
            snr_anchor_days: None,
            template_version: sp_template_version,
            data_source: SampleDataSource::Legacy,
        });
    }

    // §七十三 按代筛样：把**全部统计量的分母**收窄到当前代。
    // 为什么必须筛：跨代混池算出来的命中率/IC/先验是「两套判据的加权平均」，
    // 既不是旧代的结论也不是新代的结论（本仓已为此付过代价：换算法后指标的含义悄悄变了）。
    // 为什么两类排除要**分开计数**：异代样本是永久性剔除，代际未知的会随存量补章而减少 ——
    // 合成一个数字就看不出「样本为什么变少」到底是哪一种。
    let (kept, pre_floor, unknown) =
        filter_by_generation_floor(samples, HORIZON_BRANCH_GENERATION_FLOOR);
    let mut stats = compute_reflection_stats(&kept);
    stats.generation_floor = HORIZON_BRANCH_GENERATION_FLOOR;
    stats.excluded_pre_floor_generation = pre_floor;
    stats.excluded_unknown_generation = unknown;
    Ok(stats)
}

/// 按**起算代际**筛样 + 两类排除计数（§五十一-②/§七十三）。纯函数，便于打表直接锁行为。
///
/// 规则是下限 `>= floor`（不是等号，理由见常量注释）。两类排除分开计数而不是合成一个数：
/// 旧代样本永久性剔除（不会回到分母），代际未知的会随写侧补章而减少 ——
/// 合成一个数就看不出「分母为什么变小」是哪一种。
fn filter_by_generation_floor(
    samples: Vec<DecisionPerformanceSample>,
    floor: i32,
) -> (Vec<DecisionPerformanceSample>, usize, usize) {
    let mut kept = Vec::with_capacity(samples.len());
    let mut pre_floor = 0usize;
    let mut unknown = 0usize;
    for s in samples {
        match s.template_version {
            Some(v) if v >= floor => kept.push(s),
            Some(_) => pre_floor += 1,
            None => unknown += 1,
        }
    }
    (kept, pre_floor, unknown)
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
        sample_gen(action, horizon, was_correct, return_pct, None)
    }

    /// 同 `sample`，但指定样本所属代际（§五十一-② 的按代筛样用）。
    fn sample_gen(
        action: Option<&str>,
        horizon: Option<&str>,
        was_correct: i32,
        return_pct: f64,
        template_version: Option<i32>,
    ) -> DecisionPerformanceSample {
        DecisionPerformanceSample {
            action: action.map(str::to_string),
            horizon: horizon.map(str::to_string),
            was_correct,
            return_pct,
            alpha_pct: None,
            target_reached: None,
            confidence: None,
            snr_anchor_days: None,
            template_version,
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

    /// 当代（v104+）带水印样本：IC 分母只收这种。
    fn conf_sample(horizon: &str, conf: f64, ret: f64) -> DecisionPerformanceSample {
        let mut s = sample(Some("买入"), Some(horizon), 1, ret);
        s.confidence = Some(conf);
        s.snr_anchor_days = Some(SNR_ANCHOR);
        s
    }

    /// 上一代记录：有 confidence 但**没有口径水印** ⇒ 不得进 IC 分母。
    fn legacy_conf_sample(horizon: &str, conf: f64, ret: f64) -> DecisionPerformanceSample {
        let mut s = conf_sample(horizon, conf, ret);
        s.snr_anchor_days = None;
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

    /// 判定口径换代（v104 给逐档 confidence 加了 SNR √h 折算）不能让上一代记录混进 IC。
    ///
    /// 我第一版在这里写错过一句论证：「同一档内换算是单调 ⇒ 秩不变 ⇒ IC 不变」。
    /// **那是错的**：单调性只在同一代刻度内成立，跨代混在一档里时，两代的相对顺序本身
    /// 会被各自不同的系数打乱 ⇒ 池化秩被改，IC 也随之偏。故按水印筛，而不是靠性质免疫。
    #[test]
    fn pre_watermark_records_are_excluded_and_named_separately() {
        let find = |stats: &HitrateStats, key: &str| -> HitrateGroup {
            stats.by_horizon.iter().find(|g| g.key == key).unwrap().clone()
        };

        // ① 全上一代：有 confidence 但无水印 ⇒ 不产 IC，状态必须说「换代」而非「没置信度」
        let stale: Vec<_> = (0..8).map(|i| legacy_conf_sample("mid", i as f64, i as f64)).collect();
        let g = find(&compute_reflection_stats(&stale), "mid");
        assert_eq!(g.rank_ic, None, "缺水印的样本不得进 IC");
        assert_eq!(g.ic_samples, 0);
        assert_eq!(g.ic_regime_excluded, 8, "排除条数必须报出来（换代进度可见）");
        assert_eq!(g.ic_status, "pre_snr_regime");

        // ② 与「根本没有置信度」区分开：那条状态仍是 no_confidence
        let noconf: Vec<_> =
            (0..8).map(|i| sample(Some("买入"), Some("mid"), 1, i as f64)).collect();
        let g2 = find(&compute_reflection_stats(&noconf), "mid");
        assert_eq!(g2.ic_status, "no_confidence");
        assert_eq!(g2.ic_regime_excluded, 0);

        // ③ 混代：只数当代的 3 条 ⇒ insufficient_ic_samples（门槛 8），排除数 5
        let mut mixed: Vec<_> =
            (0..5).map(|i| legacy_conf_sample("mid", i as f64, i as f64)).collect();
        mixed.extend((5..8).map(|i| conf_sample("mid", i as f64, i as f64)));
        let g3 = find(&compute_reflection_stats(&mixed), "mid");
        assert_eq!(g3.ic_samples, 3);
        assert_eq!(g3.ic_regime_excluded, 5);
        assert_eq!(g3.ic_status, "insufficient_ic_samples");
        assert_eq!(g3.rank_ic, None);

        // ④ 换代攒够 8 条后同一形态就该出数（证明排除按水印而非按数量）
        let mut fresh: Vec<_> =
            mixed.iter().filter(|s| s.snr_anchor_days.is_some()).cloned().collect();
        fresh.extend((8..13).map(|i| conf_sample("mid", i as f64, i as f64)));
        let g4 = find(&compute_reflection_stats(&fresh), "mid");
        assert_eq!(g4.ic_samples, 8);
        assert_eq!(g4.ic_regime_excluded, 0);
        assert_eq!(g4.ic_status, "ok");
        assert_eq!(g4.rank_ic, Some(1.0));
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
                s.snr_anchor_days = Some(SNR_ANCHOR);
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
        let single: Vec<_> = (0..8).map(|i| conf_sample("short", i as f64, i as f64)).collect();
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
            // §五十一-②/§七十三 三字段：同样进 IPC ⇒ 必须一起锁 camelCase 键（见下方断言）
            generation_floor: 125,
            excluded_pre_floor_generation: 4,
            excluded_unknown_generation: 7,
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
            "generationFloor",
            "excludedPreFloorGeneration",
            "excludedUnknownGeneration",
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
            "icRegimeExcluded",
            "holdingDays",
        ] {
            assert!(group.contains_key(key), "HitrateGroup 缺 camelCase 键 {key}：{group:?}");
        }
        // 反向锁：snake_case 旧键名不得回来（那正是前端读不到的形态）
        assert!(!top.contains_key("direction_hit_rate"), "顶层又出现 snake_case 键");
        assert!(!group.contains_key("rank_ic"), "分组又出现 snake_case 键");
    }

    // ── 取数展开 helpers 的纯逻辑测试（2026-09-29 随实现一起从 commands 搬入）──
    #[test]
    fn find_bool_recursive_walks_nested_and_arrays() {
        let v: serde_json::Value = serde_json::json!({
            "report": {
                "items": [
                    { "name": "a" },
                    { "snapshot": { "target_reached": true } }
                ]
            }
        });
        assert_eq!(find_bool_recursive(&v, "target_reached"), Some(true));
        assert_eq!(find_bool_recursive(&v, "target_price"), None);
        let flat: serde_json::Value = serde_json::json!({ "target_reached": false });
        assert_eq!(find_bool_recursive(&flat, "target_reached"), Some(false));
        let non_bool: serde_json::Value = serde_json::json!({ "target_reached": "yes" });
        assert_eq!(find_bool_recursive(&non_bool, "target_reached"), None);
    }

    /// 四周期 JSON：ultra_short 成熟、short 未到期、mid 无行情、long 缺失（null）、
    /// 以及一个标量 `schemaVersion` 键（不是周期条目）。
    fn four_horizon_json() -> String {
        serde_json::json!({
            "ultra_short": {
                "status": "mature",
                "decision": { "action": "买入", "confidence": 62.0, "snrAnchorDays": 28 },
                "market": { "returnPct": 3.1, "alphaPct": 1.4, "targetReached": true },
                "evaluation": { "wasCorrect": 1 }
            },
            "short": {
                "status": "immature",
                "decision": { "action": "买入" },
                "market": { "returnPct": 0.4, "alphaPct": 0.1, "targetReached": false },
                "evaluation": { "wasCorrect": null }
            },
            "mid": {
                "status": "unavailable",
                "decision": { "action": "卖出" },
                "market": null,
                "evaluation": null
            },
            "long": null,
            "schemaVersion": 1
        })
        .to_string()
    }

    #[test]
    fn expand_only_mature_and_legacy_horizons() {
        let samples = expand_horizon_samples(&four_horizon_json(), None).unwrap();
        // 未到期 / 无行情 / 缺失周期一律不进样本，也不进命中率分母
        assert_eq!(samples.len(), 1);
        let s = &samples[0];
        assert_eq!(s.horizon.as_deref(), Some("ultra_short"));
        assert_eq!(s.action.as_deref(), Some("买入"));
        assert_eq!(s.was_correct, 1);
        assert_eq!(s.return_pct, 3.1);
        assert_eq!(s.alpha_pct, Some(1.4));
        assert_eq!(s.target_reached, Some(true));
        // IC 的预测侧：键名与产出方（`reflection.rs::build_horizon_results_json` 的
        // `"confidence": decision.confidence`）逐字对齐，口径是 0–100 不是 0–1。
        assert_eq!(s.confidence, Some(62.0));
        // 判定口径水印必须一路带到统计层（v104 前后 confidence 不同尺，IC 靠它筛样本）
        assert_eq!(s.snr_anchor_days, Some(28));
        assert_eq!(s.data_source, SampleDataSource::Reflection);
    }

    /// 旧记录 / 产出方漏写 confidence 时必须是 None ⇒ 该样本进命中率但**不进 IC 分母**，
    /// 由 `ic_status="no_confidence"` 点名。若这里兜底成 0，IC 会被一批假 0 拉成负相关 ——
    /// 把「拿不到」伪装成「测到 0」正是本轮要避免的形态。
    #[test]
    fn missing_confidence_stays_none_instead_of_zero_fill() {
        let json = serde_json::json!({
            "short": {
                "status": "mature",
                "decision": { "action": "买入" },
                "market": { "returnPct": 2.0 },
                "evaluation": { "wasCorrect": 1 }
            }
        })
        .to_string();
        let samples = expand_horizon_samples(&json, None).unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].confidence, None);
        assert_eq!(samples[0].was_correct, 1);

        // 非有限值同样归 None（产出方写出 NaN/INF 时不进 IC）
        let nan = serde_json::json!({
            "short": {
                "status": "mature",
                "decision": { "action": "买入", "confidence": null },
                "market": { "returnPct": 2.0 },
                "evaluation": { "wasCorrect": 1 }
            }
        })
        .to_string();
        assert_eq!(expand_horizon_samples(&nan, None).unwrap()[0].confidence, None);
    }

    /// 水印的两种键名形态都要认（现网 camelCase、旧快照/手拼 snake_case），
    /// 而**缺水印**必须原样落 None —— 那是「上一代口径」的判据本身，兜底成 28 等于
    /// 把所有旧记录都说成当代样本，正是本轮要挡住的混算。
    #[test]
    fn snr_watermark_is_read_verbatim_in_both_key_forms_and_never_invented() {
        let camel = serde_json::json!({
            "mid": {
                "status": "mature",
                "decision": { "action": "买入", "confidence": 55.0, "snrAnchorDays": 28 },
                "market": { "returnPct": 1.5 },
                "evaluation": { "wasCorrect": 1 }
            }
        })
        .to_string();
        assert_eq!(expand_horizon_samples(&camel, None).unwrap()[0].snr_anchor_days, Some(28));

        let snake = camel.replace("snrAnchorDays", "snr_anchor_days");
        assert_eq!(expand_horizon_samples(&snake, None).unwrap()[0].snr_anchor_days, Some(28));

        let none = serde_json::json!({
            "mid": {
                "status": "mature",
                "decision": { "action": "买入", "confidence": 55.0 },
                "market": { "returnPct": 1.5 },
                "evaluation": { "wasCorrect": 1 }
            }
        })
        .to_string();
        let s = &expand_horizon_samples(&none, None).unwrap()[0];
        assert_eq!(s.confidence, Some(55.0), "没有水印不等于没有置信度，两者要分开表达");
        assert_eq!(s.snr_anchor_days, None);
    }

    #[test]
    fn expand_skips_neutral_action_and_missing_return() {
        let json = serde_json::json!({
            "ultra_short": {
                "status": "mature",
                "decision": { "action": "持有" },
                "market": { "returnPct": 1.0 },
                "evaluation": { "wasCorrect": null }
            },
            "short": {
                "status": "mature",
                "decision": { "action": "买入" },
                "market": { "returnPct": null },
                "evaluation": { "wasCorrect": 1 }
            }
        })
        .to_string();
        // 中性档不可判定、收益缺失不可伪报 0% → 两者都不成样本
        assert!(expand_horizon_samples(&json, None).unwrap().is_empty());
    }

    #[test]
    fn expand_marks_legacy_entries_and_rejects_non_object_json() {
        let json = serde_json::json!({
            "short": {
                "status": "legacy",
                "decision": { "action": "卖出" },
                "market": { "returnPct": -2.5, "targetReached": false },
                "evaluation": { "wasCorrect": 1 }
            }
        })
        .to_string();
        let samples = expand_horizon_samples(&json, None).unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].data_source, SampleDataSource::Legacy);
        assert_eq!(samples[0].alpha_pct, None);
        assert_eq!(samples[0].target_reached, Some(false));

        assert!(expand_horizon_samples("[]", None).is_none());
        assert!(expand_horizon_samples("not-json", None).is_none());
    }

    /// §五十一-② 统计分母按**起算代际**筛样：`>= floor` 进分母，旧代与代际未知**分开计数**。
    ///
    /// 为什么这条必须有：分母变小是**静默**的（面板只看到样本数少了）。丢掉「被排除多少」，
    /// 就会出现「面板样本 0、没人知道为什么」—— 而「样本早于起算代」与「存量样本还没盖章」
    /// 的处置完全不同（前者等新样本积累或抬 floor，后者要补章/重跑）。
    ///
    /// ⚠ 判据必须是**下限**：夹具里专门放了 `floor - 1`、`floor`、`floor + 4` 三个值，
    /// 若有人把实现改成等号，`floor + 4` 那条会掉出分母 ⇒ 本用例当场红。
    #[test]
    fn generation_floor_keeps_at_or_after_and_counts_the_rest_separately() {
        let f = HORIZON_BRANCH_GENERATION_FLOOR;
        let samples = vec![
            sample_gen(Some("买入"), Some("short"), 1, 5.0, Some(f)), // 恰在起算代
            sample_gen(Some("买入"), Some("short"), 1, 3.0, Some(f + 4)), // 起算代之后（等号实现会误杀）
            sample_gen(Some("卖出"), Some("mid"), 0, -2.0, Some(f - 1)),  // 旧代
            sample_gen(Some("卖出"), Some("mid"), 1, 1.0, None),          // 代际未知
        ];
        let (kept, pre_floor, unknown) = filter_by_generation_floor(samples, f);
        assert_eq!(kept.len(), 2, "floor 及其之后都必须留在分母里");
        assert_eq!((pre_floor, unknown), (1, 1));
        assert!(
            kept.iter().all(|s| s.template_version.is_some_and(|v| v >= f)),
            "留下的每个样本都必须 >= 起算代"
        );

        // 负控：全部旧代 ⇒ 分母归 0，但**排除计数必须等于原样本数**（「为什么是 0」的唯一解释来源）。
        let all_old = vec![
            sample_gen(Some("买入"), Some("short"), 1, 5.0, Some(f - 5)),
            sample_gen(Some("买入"), Some("short"), 1, 3.0, Some(f - 1)),
        ];
        let (kept, pre_floor, unknown) = filter_by_generation_floor(all_old, f);
        assert!(kept.is_empty(), "旧代样本不得进分母");
        assert_eq!((pre_floor, unknown), (2, 0), "排除量必须如实等于被剔掉的样本数");
    }
}
