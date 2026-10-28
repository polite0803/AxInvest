//! 参数集的**收益回放**评分器：WFO 影子闸的判据层。
//!
//! ## 为什么需要它（2026-10-28）
//!
//! 现网的 `score_param_set` 比的是「候选参数与反思建议方向的契合度」，**完全不看收益**
//! （入参只有 `verdict` 与参数集）。于是 `bestParams` 可以拿着一个从未被收益验证过的
//! 参数集出现在面板上；而复盘数据表明决策本身有没有 alpha 都还没站住 ⇒ 契合度冒充
//! 收益的风险不是假想的。
//!
//! 本模块只做一件事：给定参数集与一批**已结算**的历史样本，算出「按这套参数会真的
//! 持仓的那部分样本」的收益表现，并与「全部样本的平均收益」比较。它不读写数据库、
//! 不调用 LLM，是纯函数 ⇒ 可单测、可复算。
//!
//! ## 已知口径（必须写在调用处，别让读者高估）
//!
//! - 样本分数现网取 `confidence / 100`（`decision_validations.confidence` 是 0–100）。
//!   真实链路里的分数是 portfolio-mgr 的贝叶斯后验（0–1），两者**不是同一个量**；
//!   用 confidence 当代理的后果是：绝对水平不可信，**但同一批样本上的相对比较仍然成立**
//!   （同一批样本、同一套阈值函数 ⇒ 候选与基线的差异只来自阈值本身）。
//! - 因此本模块的结论只用于**影子对比**（"这套参数比默认好还是差"），
//!   不得当作绝对收益预测，也不得直接写进收益报表。

use serde::Serialize;

use crate::portfolio_formula::PortfolioMgrParamSet;

/// 一条可回放的历史样本。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReplaySample {
    /// 决策时的分数（0–1）。现网取 `confidence / 100` 作代理，见模块头注。
    pub score: f64,
    /// 该决策的 T+N 已结算收益（百分比，带符号）。
    pub forward_return_pct: f64,
}

/// portfolio-mgr 的动作档（与 rhai 的阈值判据同序）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayAction {
    Buy,
    Increase,
    Hold,
    Watch,
    Reduce,
    Sell,
}

impl ReplayAction {
    pub fn as_str(self) -> &'static str {
        match self {
            ReplayAction::Buy => "buy",
            ReplayAction::Increase => "increase",
            ReplayAction::Hold => "hold",
            ReplayAction::Watch => "watch",
            ReplayAction::Reduce => "reduce",
            ReplayAction::Sell => "sell",
        }
    }

    /// 是否**真的持仓**。只有买入/加仓算；持有不算（没新增仓位），减仓/卖出/观察也不算。
    pub fn is_long(self) -> bool {
        matches!(self, ReplayAction::Buy | ReplayAction::Increase)
    }
}

/// 按参数集把分数映射成动作 —— 与 `portfolio-mgr.rhai` 的阈值链同序（高到低）。
pub fn decide(params: &PortfolioMgrParamSet, score: f64) -> ReplayAction {
    if score >= params.buy_threshold {
        ReplayAction::Buy
    } else if score >= params.increase_threshold {
        ReplayAction::Increase
    } else if score >= params.hold_threshold {
        ReplayAction::Hold
    } else if score >= params.watch_threshold {
        ReplayAction::Watch
    } else if score >= params.reduce_threshold {
        ReplayAction::Reduce
    } else {
        ReplayAction::Sell
    }
}

/// 一套参数在给定样本上的回放结果。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReplayReport {
    pub samples: usize,
    /// 会被判为「真的持仓」的样本数
    pub long_samples: usize,
    /// 持仓子集的平均收益（%）；无持仓样本时为 None
    pub long_avg_return_pct: Option<f64>,
    /// 持仓子集胜率（收益 > 0 的占比，0–100）
    pub long_win_rate_pct: Option<f64>,
    /// 全部样本的平均收益（%），作为「什么都买」的对照
    pub all_avg_return_pct: f64,
    /// 持仓子集相对全样本的收益差（百分点，正=选得比平均好）
    pub edge_vs_all_pct: f64,
}

/// 回放一套参数。空样本返回全零，`long_avg_return_pct` 为 None —— 不伪造 0。
pub fn evaluate(params: &PortfolioMgrParamSet, samples: &[ReplaySample]) -> ReplayReport {
    let all_avg = mean(samples.iter().map(|s| s.forward_return_pct)).unwrap_or(0.0);
    let longs: Vec<f64> = samples
        .iter()
        .filter(|s| decide(params, s.score).is_long())
        .map(|s| s.forward_return_pct)
        .collect();
    let long_avg = mean(longs.iter().copied());
    let win = if longs.is_empty() {
        None
    } else {
        let wins = longs.iter().filter(|r| **r > 0.0).count();
        Some(wins as f64 * 100.0 / longs.len() as f64)
    };
    ReplayReport {
        samples: samples.len(),
        long_samples: longs.len(),
        long_avg_return_pct: long_avg,
        long_win_rate_pct: win,
        all_avg_return_pct: all_avg,
        edge_vs_all_pct: long_avg.map(|v| v - all_avg).unwrap_or(0.0),
    }
}

fn mean(iter: impl Iterator<Item = f64>) -> Option<f64> {
    let (sum, n) = iter.fold((0.0_f64, 0_usize), |(s, n), v| (s + v, n + 1));
    (n > 0).then(|| sum / n as f64)
}

/// 影子闸的最少样本数：少于这个数，「候选更好」与「这一天恰好涨」无法区分。
pub const SHADOW_MIN_SAMPLES: usize = 30;

/// 影子闸的裁决。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ShadowVerdict {
    /// 候选参数集是否**允许**被当作「可应用」
    pub applicable: bool,
    /// 人话原因（面板直接展示）
    pub reason: String,
    pub baseline: ReplayReport,
    pub candidate: ReplayReport,
}

/// 候选 vs 基线的影子对比。
///
/// 判据（三条全过才算 applicable）：
/// 1. 样本数 ≥ [`SHADOW_MIN_SAMPLES`]；
/// 2. 候选的持仓子集样本数 ≥ 样本数的 10%（否则「只买 3 只都涨」也能通过）；
/// 3. 候选的持仓子集平均收益 **高于基线**，且 **> 0**。
///
/// 三条都不涉及绝对收益的可信度 —— 见模块头注：结论只用于相对比较。
pub fn shadow_compare(
    candidate: &PortfolioMgrParamSet,
    baseline: &PortfolioMgrParamSet,
    samples: &[ReplaySample],
) -> ShadowVerdict {
    let base = evaluate(baseline, samples);
    let cand = evaluate(candidate, samples);

    let reject = |reason: String| ShadowVerdict {
        applicable: false,
        reason,
        baseline: base.clone(),
        candidate: cand.clone(),
    };

    if base.samples < SHADOW_MIN_SAMPLES {
        return reject(format!(
            "样本不足：{} < {SHADOW_MIN_SAMPLES}，影子对比无法区分「参数更好」与「这段行情恰好涨」",
            base.samples
        ));
    }
    let min_long = (base.samples / 10).max(1);
    if cand.long_samples < min_long {
        return reject(format!(
            "候选只覆盖 {} 笔持仓（下限 {min_long}），样本太小，不足以支持应用",
            cand.long_samples
        ));
    }
    let Some(cand_avg) = cand.long_avg_return_pct else {
        return reject("候选在历史样本上从不触发买入/加仓 ⇒ 无从验证".to_string());
    };
    if cand_avg <= 0.0 {
        return reject(format!(
            "候选持仓子集平均收益 {cand_avg:.2}% ≤ 0，收益为负的参数集不得应用"
        ));
    }
    let base_avg = base.long_avg_return_pct.unwrap_or(base.all_avg_return_pct);
    if cand_avg <= base_avg {
        return reject(format!(
            "候选 {cand_avg:.2}% 不优于基线 {base_avg:.2}%（同一批 {} 笔样本）",
            base.samples
        ));
    }
    ShadowVerdict {
        applicable: true,
        reason: format!(
            "候选 {cand_avg:.2}% > 基线 {base_avg:.2}%（{} 笔持仓 / {} 笔样本，胜率 {:.1}%）",
            cand.long_samples,
            base.samples,
            cand.long_win_rate_pct.unwrap_or(0.0)
        ),
        baseline: base,
        candidate: cand,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(buy: f64) -> PortfolioMgrParamSet {
        PortfolioMgrParamSet { buy_threshold: buy, ..PortfolioMgrParamSet::v56_default() }
    }

    /// 阈值链必须与 rhai 同序，且边界取等号（`>=`）。
    #[test]
    fn decide_follows_the_threshold_chain() {
        let p = PortfolioMgrParamSet::v56_default();
        assert_eq!(decide(&p, p.buy_threshold), ReplayAction::Buy);
        assert_eq!(decide(&p, p.increase_threshold), ReplayAction::Increase);
        assert_eq!(decide(&p, p.hold_threshold), ReplayAction::Hold);
        assert_eq!(decide(&p, p.watch_threshold), ReplayAction::Watch);
        assert_eq!(decide(&p, p.reduce_threshold), ReplayAction::Reduce);
        assert_eq!(decide(&p, 0.0), ReplayAction::Sell);
    }

    /// 只有买入/加仓算持仓；持有不算新增仓位。
    #[test]
    fn only_buy_and_increase_are_long() {
        assert!(ReplayAction::Buy.is_long());
        assert!(ReplayAction::Increase.is_long());
        assert!(!ReplayAction::Hold.is_long());
        assert!(!ReplayAction::Watch.is_long());
    }

    /// 空样本不得伪造 0 收益，持仓均值必须是 None。
    #[test]
    fn empty_samples_report_unknown_not_zero() {
        let r = evaluate(&PortfolioMgrParamSet::v56_default(), &[]);
        assert_eq!(r.samples, 0);
        assert_eq!(r.long_avg_return_pct, None);
        assert_eq!(r.long_win_rate_pct, None);
        assert_eq!(r.all_avg_return_pct, 0.0);
    }

    /// 影子闸三条判据各有一条反向对照：样本不足 / 持仓太少 / 收益为负 / 不优于基线。
    #[test]
    fn shadow_gate_rejects_each_failure_mode() {
        let base_p = PortfolioMgrParamSet::v56_default();

        // ① 样本不足
        let few: Vec<ReplaySample> =
            (0..10).map(|i| ReplaySample { score: 0.9, forward_return_pct: i as f64 }).collect();
        assert!(!shadow_compare(&params(0.5), &base_p, &few).applicable);

        // ② 持仓太少：只买 0.95 以上，60 笔里仅 2 笔
        let mut sparse: Vec<ReplaySample> =
            (0..60).map(|_| ReplaySample { score: 0.5, forward_return_pct: 5.0 }).collect();
        sparse.push(ReplaySample { score: 0.99, forward_return_pct: 5.0 });
        sparse.push(ReplaySample { score: 0.97, forward_return_pct: 5.0 });
        let v = shadow_compare(&params(0.96), &base_p, &sparse);
        assert!(!v.applicable);
        assert!(v.reason.contains("覆盖"), "{}", v.reason);

        // ③ 收益为负：所有持仓样本都亏
        let losers: Vec<ReplaySample> =
            (0..60).map(|i| ReplaySample { score: 0.9, forward_return_pct: -(i as f64) }).collect();
        assert!(!shadow_compare(&params(0.5), &base_p, &losers).applicable);

        // ④ 不优于基线：候选只是把阈值调高，持仓子集更差
        let mixed: Vec<ReplaySample> = (0..60)
            .map(|i| ReplaySample {
                score: if i % 2 == 0 { 0.9 } else { 0.5 },
                forward_return_pct: if i % 2 == 0 { -1.0 } else { 3.0 },
            })
            .collect();
        assert!(!shadow_compare(&params(0.8), &base_p, &mixed).applicable);

        // 正向对照：候选只买高分区，而高分区确实更赚 ⇒ 必须放行
        let good: Vec<ReplaySample> = (0..60)
            .map(|i| ReplaySample {
                score: if i % 2 == 0 { 0.9 } else { 0.5 },
                forward_return_pct: if i % 2 == 0 { 4.0 } else { 0.5 },
            })
            .collect();
        let ok = shadow_compare(&params(0.8), &base_p, &good);
        assert!(ok.applicable, "{}", ok.reason);
        assert!(
            ok.candidate.long_avg_return_pct.unwrap() > ok.baseline.long_avg_return_pct.unwrap()
        );
    }
}
