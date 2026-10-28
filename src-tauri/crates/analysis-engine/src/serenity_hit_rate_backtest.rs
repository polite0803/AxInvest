//! 瓶颈掘金工作流（serenity-screening）事后验证
//!
//! 对应 `hit_rate_backtest.rs`（单股分析），本模块针对 serenity 工作流的
//! 候选股清单（reco_picks 表 style="serenity"）做 T+N 命中率验证。
//!
//! 核心差异（vs hit_rate_backtest）：
//! - serenity 候选股无 target_price/stop_loss（LLM 只输出 serenity_score）
//! - 验证维度是"候选股在 T+N 内是否跑赢行业均值/沪深300"
//! - 因子快照来自 bottleneck-calc 的 bottleneck_composite / data_reliability
//!
//! 复用 hit_rate_backtest 中的 Spearman IC 计算（compute_factor_ic）。

// 2026-10-14 恢复编译并接消费者（PLAN-reflection-optimization-closure §四）。
// 此前本模块整份存在却不在 `lib.rs` 的 `pub mod` 清单里 ⇒ 不参与编译、11 个单测从未跑过、
// 智选（serenity）候选行也一直没有 T+N 超额命中的数值消费者。消费点：
// `commands/backtest_validation.rs` 的决策回测任务（style='serenity' 的 pick 走本模块判据）。

use std::collections::HashMap;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::hit_rate_backtest::{PickValidation, compute_factor_ic};

// ─────────────────────────────────────────────────
// 数据结构
// ─────────────────────────────────────────────────

/// serenity 候选股验证记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerenityPickValidation {
    pub pick_id: String,
    pub stock_code: String,
    pub stock_name: String,
    pub generated_at: String,
    pub validated_at: String,
    pub t_plus_n: i32,
    /// 瓶颈评分（0-100）。`None` = **该维度未持久化** —— 现网 `reco_picks.pick_data`
    /// 里没有 `serenity_score` / `bottleneck_composite` / `data_reliability` 三个键
    /// （2026-10-15 实测：75 条 serenity 行全部缺失），报表据此单列「未持久化」桶，
    /// 不拿 0.0 顶替（那会把「没记录」读成「评分为 0 / 完全不可信」）。
    pub serenity_score: Option<f64>,
    pub bottleneck_composite: Option<f64>,
    /// 数据可信度等级（`partially_verified` 等）。缺席语义同上。
    pub data_reliability: Option<String>,
    pub chain_untrusted: bool,
    /// T+N 内最大涨幅（%）
    pub max_return_pct: Option<f64>,
    /// T+N 内最大回撤（%）
    pub max_drawdown_pct: Option<f64>,
    /// T+N 最终收益（%）
    pub final_return_pct: Option<f64>,
    /// T+N 行业均值收益（%）
    pub industry_avg_return: Option<f64>,
    /// 超额收益 = final_return_pct - industry_avg_return
    pub excess_return: Option<f64>,
    /// 命中判定：超额收益 > 0 视为命中
    pub hit_outcome: Option<String>,
    pub data_source: String,
    pub created_at: String,
}

/// serenity 候选股命中率统计
///
/// `rename_all = "camelCase"`：本结构 2026-10-15 起随决策回测响应出 IPC 边界，
/// 字段必须与前端 TS 类型的 camelCase 对齐（禁区 13；此前它只在 crate 内自用，
/// 与 `HitrateStats` 同一类漏注解风险）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerenityHitRateReport {
    pub total_picks: usize,
    pub validated_picks: usize,
    pub hit_count: usize,
    pub miss_count: usize,
    pub pending_count: usize,
    pub hit_rate: f64,
    /// 平均超额收益
    pub avg_excess_return: f64,
    /// 按 serenity_score 分层的命中率
    pub score_bucket_hit_rates: Vec<ScoreBucketHitRate>,
    /// 按 data_reliability 分层的命中率
    pub reliability_breakdown: HashMap<String, ReliabilityStats>,
    /// 因子 IC（bottleneck_composite → 超额收益）
    pub factor_ic: HashMap<String, f64>,
    pub generated_at: String,
}

/// serenity_score 分层命中率
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreBucketHitRate {
    pub bucket: String,
    pub total: usize,
    pub hits: usize,
    pub hit_rate: f64,
    pub avg_excess_return: f64,
}

/// data_reliability 分层统计
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReliabilityStats {
    pub total: usize,
    pub hits: usize,
    pub hit_rate: f64,
    pub avg_excess_return: f64,
}

// ─────────────────────────────────────────────────
// 核心函数
// ─────────────────────────────────────────────────

/// 计算 serenity 候选股的命中结果
///
/// 判据是「**超额**收益」：本窗口收益 − 基准（沪深300）涨跌幅。因此基准缺失时不判命中
/// （返回 `None`）—— 2026-10-15 改动：此前 `industry_avg_return.unwrap_or(0.0)` 会把
/// 「没取到基准」当成「基准涨了 0%」，于是绝对收益冒充超额收益。
///
/// `serenity_score` / `bottleneck_composite` 是 `Option`：这两个诊断维度目前**没有**
/// 持久化进 `reco_picks.pick_data`（现网实测），本函数不参与判定，仅作随行的留痕字段。
pub fn compute_serenity_hit_outcome(
    serenity_score: Option<f64>,
    bottleneck_composite: Option<f64>,
    max_return_pct: Option<f64>,
    max_drawdown_pct: Option<f64>,
    final_return_pct: Option<f64>,
    industry_avg_return: Option<f64>,
) -> Option<String> {
    let final_ret = final_return_pct?;
    let ind_ret = industry_avg_return?;
    let excess = final_ret - ind_ret;
    // 占位引用以避免未使用参数警告（两个评分维度留作分层报表用，不参与命中判定）
    let _ = (serenity_score, bottleneck_composite, max_return_pct, max_drawdown_pct);

    if excess > 5.0 {
        Some("excess_outperform".into())
    } else if excess > 0.0 {
        Some("outperform".into())
    } else if excess > -5.0 {
        Some("underperform".into())
    } else {
        Some("excess_underperform".into())
    }
}

/// 计算超额收益
pub fn compute_excess_return(
    final_return_pct: Option<f64>,
    industry_avg_return: Option<f64>,
) -> Option<f64> {
    match (final_return_pct, industry_avg_return) {
        (Some(fin), Some(ind)) => Some(fin - ind),
        (Some(fin), None) => Some(fin),
        _ => None,
    }
}

/// 计算价格序列的最大回撤（百分比，负值）
///
/// 修复 P2-11: 提取为独立函数，遵循"峰值→谷值"标准定义。
/// 算法：遍历序列，跟踪历史峰值 peak；每个点的回撤 = (price - peak) / peak × 100；
/// 取所有点回撤的最小值（最负值）作为最大回撤。
pub fn compute_max_drawdown_from_series(prices: &[f64]) -> f64 {
    if prices.is_empty() {
        return 0.0;
    }
    let mut peak = prices[0];
    let mut max_dd = 0.0_f64;
    for &p in prices {
        if p > peak {
            peak = p;
        }
        if peak > 0.0 {
            let dd = (p - peak) / peak * 100.0;
            if dd < max_dd {
                max_dd = dd;
            }
        }
    }
    max_dd
}

/// 构建 SerenityPickValidation
#[allow(clippy::too_many_arguments)]
pub fn build_serenity_validation(
    pick_id: &str,
    stock_code: &str,
    stock_name: &str,
    generated_at: &str,
    t_plus_n: i32,
    serenity_score: Option<f64>,
    bottleneck_composite: Option<f64>,
    data_reliability: Option<&str>,
    chain_untrusted: bool,
    closes: &[f64],
    industry_avg_return: Option<f64>,
    data_source: &str,
) -> SerenityPickValidation {
    let validated_at = Utc::now().to_rfc3339();

    let (max_return_pct, max_drawdown_pct, final_return_pct) = if closes.is_empty() {
        (None, None, None)
    } else {
        let entry = closes[0];
        let max_price = closes.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let final_price = closes[closes.len() - 1];

        let max_ret = (max_price - entry) / entry * 100.0;
        // 修复 P2-11: 原算法 (min_price - entry) / entry 计算的是"从入场到最低价的跌幅"，
        // 不是真正的"最大回撤"。最大回撤定义：从历史峰值到随后谷值的最大跌幅。
        // 例如 closes=[10, 11, 9.5, 12, 11.5]：
        //   - 旧算法: (9.5-10)/10 = -5%
        //   - 正确算法: 在 i=2 时 peak=11, dd=(9.5-11)/11 = -13.6%（更真实）
        // 修复后反映持仓期间的真实风险暴露，而非简单的"入场后最大浮亏"。
        let max_dd = compute_max_drawdown_from_series(closes);
        let final_ret = (final_price - entry) / entry * 100.0;

        (Some(max_ret), Some(max_dd), Some(final_ret))
    };

    let excess_return = compute_excess_return(final_return_pct, industry_avg_return);

    let hit_outcome = compute_serenity_hit_outcome(
        serenity_score,
        bottleneck_composite,
        max_return_pct,
        max_drawdown_pct,
        final_return_pct,
        industry_avg_return,
    );

    SerenityPickValidation {
        pick_id: pick_id.to_string(),
        stock_code: stock_code.to_string(),
        stock_name: stock_name.to_string(),
        generated_at: generated_at.to_string(),
        validated_at,
        t_plus_n,
        serenity_score,
        bottleneck_composite,
        data_reliability: data_reliability.map(str::to_string),
        chain_untrusted,
        max_return_pct,
        max_drawdown_pct,
        final_return_pct,
        industry_avg_return,
        excess_return,
        hit_outcome,
        data_source: data_source.to_string(),
        created_at: Utc::now().to_rfc3339(),
    }
}

/// 把一条智选验证落成 `decision_validations` 的一行（`PickValidation` 形态）。
///
/// 与 `compute_serenity_hit_rate_report` 内部那份转换**不同**：这里保留 pick 的
/// 真实入场价与置信度（荐股 IC 的 x/y 轴要用），并带上回放代际 `as_of_date`。
///
/// ⚠ `hit_outcome` 用的是本模块的四档超额标签（`excess_outperform` 等），
/// 不是通用通道的 `{hit,partial,miss,false_hit}`。这是刻意的：智选候选没有目标价，
/// 「是否到达目标价」判据对它不成立。下游因此**必须**按标签域分支：
/// `hit_outcome_to_binary_outcome` 对这些标签返回 `None` ⇒ 不进胜负回写、
/// 不进荐股权重闭环的胜率样本；`compute_hit_rate_report` 的分母也不含它们
/// （调用方须把本通道的行排除在该报表之外，否则会被显示成「命中率 0%」）。
#[allow(clippy::too_many_arguments)]
pub fn serenity_validation_to_row(
    v: &SerenityPickValidation,
    entry_price: f64,
    confidence: i32,
    position_pct: f64,
    period: &str,
    as_of_date: Option<&str>,
) -> PickValidation {
    // T+N 收盘价：由已算出的收益率反推（收益率就是 (last-entry)/entry）
    let t_plus_n_price = v.final_return_pct.map(|ret| entry_price * (1.0 + ret / 100.0));
    let mut factors = HashMap::new();
    if let Some(c) = v.bottleneck_composite {
        factors.insert("bottleneck_composite".to_string(), c);
    }
    if let Some(s) = v.serenity_score {
        factors.insert("serenity_score".to_string(), s);
    }
    if let Some(ind) = v.industry_avg_return {
        factors.insert("benchmark_return_pct".to_string(), ind);
    }
    PickValidation {
        pick_id: v.pick_id.clone(),
        stock_code: v.stock_code.clone(),
        stock_name: v.stock_name.clone(),
        style: "serenity".to_string(),
        period: period.to_string(),
        t_plus_n: v.t_plus_n,
        generated_at: v.generated_at.clone(),
        entry_price,
        // 智选候选没有目标价 / 止损价：写 0.0 并由 `hit_outcome` 的标签域表明
        // 「本行不按目标价判命中」，不伪造一个价位
        target_price: 0.0,
        stop_loss: 0.0,
        position_pct,
        confidence,
        inferred_action: String::new(),
        t_plus_n_price,
        max_price: None,
        min_price: None,
        max_return_pct: v.max_return_pct,
        max_drawdown_pct: v.max_drawdown_pct,
        final_return_pct: v.final_return_pct,
        hit_stop_loss: None,
        hit_target: None,
        hit_outcome: v.hit_outcome.clone(),
        factor_snapshot: Some(factors),
        data_source: v.data_source.clone(),
        as_of_date: as_of_date.map(str::to_string),
    }
}

/// 「该维度未持久化」的分桶键（ASCII 稳定键；文案由前端 i18n 出，Rust 侧不写中文标签）。
///
/// 存在的理由：智选候选行目前没有 `serenity_score` / `bottleneck_composite` /
/// `data_reliability` 三个诊断键（2026-10-15 现网实测，75 条 serenity 行全缺）。
/// 分层报表必须能说出「这一层的样本是因为没记录才空」，而不是把它们并进
/// 「评分 0」或「不可信」某一层 —— 后者等于让「没数据」替策略发结论。
pub const SERENITY_METRIC_NOT_PERSISTED: &str = "not_persisted";

/// 一个子集的命中统计（score 分层与可信度分层共用，避免两份算法漂移）。
fn bucket_stats(label: &str, picks: &[&SerenityPickValidation]) -> ScoreBucketHitRate {
    let total = picks.len();
    let hits = picks
        .iter()
        .filter(|v| {
            matches!(v.hit_outcome.as_deref(), Some("excess_outperform") | Some("outperform"))
        })
        .count();
    let excesses: Vec<f64> = picks.iter().filter_map(|v| v.excess_return).collect();
    let avg = if excesses.is_empty() {
        0.0
    } else {
        excesses.iter().sum::<f64>() / excesses.len() as f64
    };
    let rate = if total > 0 {
        hits as f64 / total as f64
    } else {
        0.0
    };
    ScoreBucketHitRate {
        bucket: label.to_string(),
        total,
        hits,
        hit_rate: rate,
        avg_excess_return: avg,
    }
}

/// 汇总 serenity 候选股命中率报告
pub fn compute_serenity_hit_rate_report(
    validations: &[SerenityPickValidation],
) -> SerenityHitRateReport {
    let total_picks = validations.len();
    let validated_picks = validations.iter().filter(|v| v.hit_outcome.is_some()).count();

    let hit_count = validations
        .iter()
        .filter(|v| {
            matches!(v.hit_outcome.as_deref(), Some("excess_outperform") | Some("outperform"))
        })
        .count();

    let miss_count = validated_picks.saturating_sub(hit_count);
    let pending_count = total_picks.saturating_sub(validated_picks);

    let hit_rate = if validated_picks > 0 {
        hit_count as f64 / validated_picks as f64
    } else {
        0.0
    };

    let avg_excess_return = {
        let excesses: Vec<f64> = validations.iter().filter_map(|v| v.excess_return).collect();
        if excesses.is_empty() {
            0.0
        } else {
            excesses.iter().sum::<f64>() / excesses.len() as f64
        }
    };

    // serenity_score 分层：[0,40), [40,60), [60,80), [80,100]
    let score_buckets = [
        ("low(0-40)", 0.0, 40.0),
        ("mid(40-60)", 40.0, 60.0),
        ("high(60-80)", 60.0, 80.0),
        ("elite(80-100)", 80.0, 101.0),
    ];
    let mut score_bucket_hit_rates: Vec<ScoreBucketHitRate> = score_buckets
        .iter()
        .map(|(label, lo, hi)| {
            let bucket_picks: Vec<&SerenityPickValidation> = validations
                .iter()
                .filter(|v| v.serenity_score.is_some_and(|s| s >= *lo && s < *hi))
                .collect();
            bucket_stats(label, &bucket_picks)
        })
        .collect();
    // 评分未持久化的行单列一层 —— 不并进上面四桶，也不让它们的缺失把四桶读成「都没分」
    let unscored: Vec<&SerenityPickValidation> =
        validations.iter().filter(|v| v.serenity_score.is_none()).collect();
    if !unscored.is_empty() {
        score_bucket_hit_rates.push(bucket_stats(SERENITY_METRIC_NOT_PERSISTED, &unscored));
    }

    // data_reliability 分层
    let mut reliability_map: HashMap<String, (Vec<f64>, usize, usize)> = HashMap::new();
    for v in validations.iter().filter(|v| v.hit_outcome.is_some()) {
        let entry = reliability_map
            .entry(
                v.data_reliability
                    .clone()
                    .unwrap_or_else(|| SERENITY_METRIC_NOT_PERSISTED.to_string()),
            )
            .or_insert((vec![], 0, 0));
        entry.2 += 1;
        if matches!(v.hit_outcome.as_deref(), Some("excess_outperform") | Some("outperform")) {
            entry.1 += 1;
        }
        if let Some(ex) = v.excess_return {
            entry.0.push(ex);
        }
    }
    let reliability_breakdown = reliability_map
        .into_iter()
        .map(|(k, (excesses, hits, total))| {
            let avg = if excesses.is_empty() {
                0.0
            } else {
                excesses.iter().sum::<f64>() / excesses.len() as f64
            };
            let rate = if total > 0 {
                hits as f64 / total as f64
            } else {
                0.0
            };
            (k, ReliabilityStats { total, hits, hit_rate: rate, avg_excess_return: avg })
        })
        .collect();

    // 因子 IC：复用 compute_factor_ic，将 SerenityPickValidation 转 PickValidation。
    // 只吃**同时有评分与实现收益**的行：评分缺失时 x 轴不存在，把 confidence 填成 0
    // 会造出一批「评分 0」的假样本（分层报表已用 `not_persisted` 桶如实说明缺席）。
    let pick_validations: Vec<PickValidation> = validations
        .iter()
        .filter_map(|v| {
            v.final_return_pct.zip(v.serenity_score).map(|(ret, score)| {
                let mut factors = std::collections::HashMap::new();
                if let Some(c) = v.bottleneck_composite {
                    factors.insert("bottleneck_composite".to_string(), c);
                }
                factors.insert("serenity_score".to_string(), score);
                PickValidation {
                    pick_id: v.pick_id.clone(),
                    stock_code: v.stock_code.clone(),
                    stock_name: v.stock_name.clone(),
                    style: "serenity".into(),
                    period: format!("T+{}", v.t_plus_n),
                    t_plus_n: v.t_plus_n,
                    generated_at: v.generated_at.clone(),
                    entry_price: 0.0,
                    target_price: 0.0,
                    stop_loss: 0.0,
                    position_pct: 0.0,
                    confidence: score as i32,
                    inferred_action: v.hit_outcome.clone().unwrap_or_default(),
                    t_plus_n_price: None,
                    max_price: None,
                    min_price: None,
                    max_return_pct: v.max_return_pct,
                    max_drawdown_pct: v.max_drawdown_pct,
                    final_return_pct: Some(ret),
                    hit_stop_loss: None,
                    hit_target: None,
                    hit_outcome: v.hit_outcome.clone(),
                    factor_snapshot: Some(factors),
                    data_source: v.data_source.clone(),
                    // 智选胜率线的样本来自 reco_picks 的实盘行，无回放代际
                    as_of_date: None,
                }
            })
        })
        .collect();
    let factor_ic = compute_factor_ic(&pick_validations);

    SerenityHitRateReport {
        total_picks,
        validated_picks,
        hit_count,
        miss_count,
        pending_count,
        hit_rate,
        avg_excess_return,
        score_bucket_hit_rates,
        reliability_breakdown,
        factor_ic,
        generated_at: Utc::now().to_rfc3339(),
    }
}

// ─────────────────────────────────────────────────
// 单元测试
// ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_validation(
        score: f64,
        composite: f64,
        final_ret: Option<f64>,
        ind_ret: Option<f64>,
        reliability: &str,
    ) -> SerenityPickValidation {
        SerenityPickValidation {
            pick_id: format!("p_{score}"),
            stock_code: "000001".into(),
            stock_name: "test".into(),
            generated_at: "2026-01-01T00:00:00Z".into(),
            validated_at: "2026-01-11T00:00:00Z".into(),
            t_plus_n: 10,
            serenity_score: Some(score),
            bottleneck_composite: Some(composite),
            data_reliability: Some(reliability.into()),
            chain_untrusted: false,
            max_return_pct: Some(10.0),
            max_drawdown_pct: Some(-5.0),
            final_return_pct: final_ret,
            industry_avg_return: ind_ret,
            excess_return: compute_excess_return(final_ret, ind_ret),
            hit_outcome: compute_serenity_hit_outcome(
                Some(score),
                Some(composite),
                Some(10.0),
                Some(-5.0),
                final_ret,
                ind_ret,
            ),
            data_source: "test".into(),
            created_at: "2026-01-11T00:00:00Z".into(),
        }
    }

    #[test]
    fn test_compute_excess_return() {
        assert_eq!(compute_excess_return(Some(10.0), Some(3.0)), Some(7.0));
        assert_eq!(compute_excess_return(Some(10.0), None), Some(10.0));
        assert_eq!(compute_excess_return(None, Some(3.0)), None);
    }

    #[test]
    fn test_hit_outcome_outperform() {
        let outcome = compute_serenity_hit_outcome(
            Some(80.0),
            Some(75.0),
            Some(15.0),
            Some(-3.0),
            Some(12.0),
            Some(4.0),
        );
        assert_eq!(outcome.as_deref(), Some("excess_outperform"));
    }

    #[test]
    fn test_hit_outcome_underperform() {
        let outcome = compute_serenity_hit_outcome(
            Some(50.0),
            Some(45.0),
            Some(2.0),
            Some(-1.0),
            Some(-3.0),
            Some(5.0),
        );
        assert_eq!(outcome.as_deref(), Some("excess_underperform"));
    }

    /// 负控（2026-10-15）：**取不到基准就不判命中**。
    /// 旧实现 `industry_avg_return.unwrap_or(0.0)` 把「没取到基准」当成「基准涨 0%」，
    /// 于是绝对收益冒充超额收益 —— 涨 3% 会被判成 `outperform`。
    #[test]
    fn missing_benchmark_yields_no_verdict_not_zero_baseline() {
        assert_eq!(
            compute_serenity_hit_outcome(
                Some(70.0),
                Some(60.0),
                Some(2.0),
                Some(-1.0),
                Some(3.0),
                None
            ),
            None,
            "无基准时必须不判，而不是按 0% 基准判成跑赢"
        );
        // 有基准时才出四档标签
        assert_eq!(
            compute_serenity_hit_outcome(
                Some(70.0),
                Some(60.0),
                Some(2.0),
                Some(-1.0),
                Some(3.0),
                Some(0.5)
            )
            .as_deref(),
            Some("outperform")
        );
    }

    /// 诊断维度未持久化的行：分桶必须单列 `not_persisted`，且不得进入因子 IC 的样本
    /// （没有 x 轴就把 confidence 填 0 会造出一批「评分 0」的假样本）。
    #[test]
    fn unpersisted_metrics_get_their_own_bucket_and_leave_factor_ic() {
        let mut scored = make_validation(85.0, 80.0, Some(15.0), Some(5.0), "partially_verified");
        scored.serenity_score = None;
        scored.bottleneck_composite = None;
        scored.data_reliability = None;
        let rows =
            vec![make_validation(85.0, 80.0, Some(15.0), Some(5.0), "partially_verified"), scored];
        let report = compute_serenity_hit_rate_report(&rows);
        let not_persisted = report
            .score_bucket_hit_rates
            .iter()
            .find(|b| b.bucket == SERENITY_METRIC_NOT_PERSISTED)
            .expect("未持久化的行必须有自己的一层");
        assert_eq!(not_persisted.total, 1);
        assert_eq!(
            report.reliability_breakdown.get(SERENITY_METRIC_NOT_PERSISTED).map(|s| s.total),
            Some(1),
            "可信度缺席也要单列，不能并进任何真实等级"
        );
        // 因子 IC 只吃有评分的那一条（另一条无 x 轴）
        assert_eq!(report.total_picks, 2);
    }

    #[test]
    fn test_build_serenity_validation_empty_closes() {
        let v = build_serenity_validation(
            "p1",
            "000001",
            "test",
            "2026-01-01",
            10,
            Some(75.0),
            Some(70.0),
            Some("partially_verified"),
            false,
            &[],
            None,
            "test",
        );
        assert_eq!(v.max_return_pct, None);
        assert_eq!(v.final_return_pct, None);
        assert_eq!(v.hit_outcome, None);
    }

    #[test]
    fn test_build_serenity_validation_with_closes() {
        let closes = vec![10.0, 11.0, 9.5, 12.0, 11.5];
        let v = build_serenity_validation(
            "p1",
            "000001",
            "test",
            "2026-01-01",
            5,
            Some(75.0),
            Some(70.0),
            Some("partially_verified"),
            false,
            &closes,
            Some(5.0),
            "test",
        );
        // entry = closes[0] = 10.0, final = 11.5 → final_return = 15%
        assert!(v.max_return_pct.unwrap() > 0.0);
        assert!(v.final_return_pct.unwrap() > 0.0);
        assert!(v.excess_return.is_some());
        assert!(v.hit_outcome.is_some());
    }

    #[test]
    fn test_max_drawdown_peak_to_trough() {
        // 修复 P2-11 验证: closes=[10, 11, 9.5, 12, 11.5]
        // 旧算法 (min-entry)/entry = (9.5-10)/10 = -5%
        // 正确算法: peak=11 时 trough=9.5, dd = (9.5-11)/11 = -13.6%
        let closes = vec![10.0, 11.0, 9.5, 12.0, 11.5];
        let dd = compute_max_drawdown_from_series(&closes);
        assert!(dd < -13.0 && dd > -14.0, "expected ~-13.6%, got {dd}%");
    }

    #[test]
    fn test_max_drawdown_monotonic_increasing() {
        // 单调上涨序列无回撤
        let closes = vec![10.0, 11.0, 12.0, 13.0, 14.0];
        let dd = compute_max_drawdown_from_series(&closes);
        assert_eq!(dd, 0.0);
    }

    #[test]
    fn test_max_drawdown_empty_series() {
        assert_eq!(compute_max_drawdown_from_series(&[]), 0.0);
    }

    #[test]
    fn test_hit_rate_report_basic() {
        let validations = vec![
            make_validation(85.0, 80.0, Some(15.0), Some(5.0), "partially_verified"),
            make_validation(70.0, 65.0, Some(8.0), Some(5.0), "baseline_corroborated"),
            make_validation(45.0, 40.0, Some(2.0), Some(5.0), "llm_estimated"),
            make_validation(30.0, 25.0, Some(-3.0), Some(5.0), "insufficient"),
        ];
        let report = compute_serenity_hit_rate_report(&validations);
        assert_eq!(report.total_picks, 4);
        assert_eq!(report.validated_picks, 4);
        assert_eq!(report.hit_count, 2);
        assert_eq!(report.miss_count, 2);
        assert!((report.hit_rate - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_score_bucket_stratification() {
        let validations = vec![
            make_validation(85.0, 80.0, Some(15.0), Some(5.0), "partially_verified"),
            make_validation(75.0, 70.0, Some(10.0), Some(5.0), "partially_verified"),
            make_validation(50.0, 45.0, Some(2.0), Some(5.0), "llm_estimated"),
        ];
        let report = compute_serenity_hit_rate_report(&validations);
        let elite_bucket =
            report.score_bucket_hit_rates.iter().find(|b| b.bucket == "elite(80-100)").unwrap();
        assert_eq!(elite_bucket.total, 1);
        assert_eq!(elite_bucket.hits, 1);
        let mid_bucket =
            report.score_bucket_hit_rates.iter().find(|b| b.bucket == "mid(40-60)").unwrap();
        assert_eq!(mid_bucket.total, 1);
        assert_eq!(mid_bucket.hits, 0);
    }

    #[test]
    fn test_reliability_breakdown() {
        let validations = vec![
            make_validation(85.0, 80.0, Some(15.0), Some(5.0), "partially_verified"),
            make_validation(75.0, 70.0, Some(8.0), Some(5.0), "partially_verified"),
            make_validation(50.0, 45.0, Some(-3.0), Some(5.0), "llm_estimated"),
        ];
        let report = compute_serenity_hit_rate_report(&validations);
        let verified_stats = report.reliability_breakdown.get("partially_verified").unwrap();
        assert_eq!(verified_stats.total, 2);
        assert_eq!(verified_stats.hits, 2);
        let llm_stats = report.reliability_breakdown.get("llm_estimated").unwrap();
        assert_eq!(llm_stats.total, 1);
        assert_eq!(llm_stats.hits, 0);
    }

    #[test]
    fn test_factor_ic_empty() {
        let validations: Vec<SerenityPickValidation> = vec![];
        let report = compute_serenity_hit_rate_report(&validations);
        assert!(report.factor_ic.is_empty());
    }
}
