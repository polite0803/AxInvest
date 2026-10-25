// SPDX-License-Identifier: AGPL-3.0-only

//! 交易成本口径的**唯一来源**。
//!
//! ## 为什么单独成模块
//!
//! 2026-10-25 取证（`AUDIT-stock-profitability-verdict-2026-10-25.md` §1.3）发现：
//! 同一 crate 内存在**两套互不相干**的成本口径 ——
//!
//! - `backtest.rs` 的回测**扣** 0.18%（佣金双边 + 印花税）；
//! - `hit_rate_backtest.rs` 的命中率链**完全不扣**（`compute_price_metrics` 只算
//!   `(T+N 价 − 入场价) / 入场价`），且 `compute_hit_outcome` 判 `ret > 0.0 → hit`
//!   ⇒ **+0.01% 也算赢**。
//!
//! 而喂给闭环权重、命中率面板、先验校准的恰恰是**不扣成本**的那一条。后果是
//! 「真实 +0.1% 的样本被记成命中，实盘是 −0.15% 的亏损」—— 胜率与均值同时虚高，
//! 且虚高幅度随持仓期变短而放大（超短档受害最深）。
//!
//! 本模块把各项费率收敛为**唯一常量来源**，供两条链共用；`backtest.rs` 的
//! 0.18% 改为由本模块推导，**行为不变**（无滑点口径），但不再手抄。
//!
//! ## 单位约定
//!
//! 本模块全部以 **百分数** 表示（`0.18` 即 0.18%），与 `final_return_pct` 同量纲。
//! 需要小数（如 `0.0018`）的调用方自行 `/ 100.0`，见 [`ROUND_TRIP_COST_NO_SLIPPAGE_PCT`]。

/// 佣金（**双边合计**，%）。
///
/// A 股主流券商万 2.5 ~ 万 8，双边合计约 0.05% ~ 0.16%；取中位偏保守 0.08%
/// （即单边万 4），与 `backtest.rs` 既有取值一致。
pub const COMMISSION_BOTH_SIDES_PCT: f64 = 0.08;

/// 卖出印花税（%）。2023-08 起由 0.1% 减半为 0.05%，但**取回原值 0.1%** 作保守估计
/// （政策可回调；且本项目历史成本口径一直按 0.1% 计，改小会让新旧读数不可比）。
pub const STAMP_DUTY_PCT: f64 = 0.10;

/// 滑点（**双边合计**，%）。
///
/// 散户实操经验值：流动性好的大盘股 0.03~0.05%，中小盘 0.1~0.3%。
/// 取 0.07% 作保守中位。回测里不含此项，见 [`ROUND_TRIP_COST_NO_SLIPPAGE_PCT`]。
pub const SLIPPAGE_PCT: f64 = 0.07;

/// **不含滑点**的双边成本 = 0.18%。
///
/// 这是 `crates/analysis-engine/src/backtest.rs` 的历史口径（原为手抄
/// `const COST_RATE: f64 = 0.0018`）。保留为独立常量而非直接并入
/// [`DEFAULT_ROUND_TRIP_COST_PCT`]，是因为**改它会改变既有回测的语义**
/// （已落库的 `accuracy` 会全部漂移）—— 那属于另一件事，须单独换版本号。
pub const ROUND_TRIP_COST_NO_SLIPPAGE_PCT: f64 = COMMISSION_BOTH_SIDES_PCT + STAMP_DUTY_PCT;

/// **默认**双边成本（含保守滑点）= 0.25%。
///
/// 命中率链、净胜率、净收益一律用这个值。它是记账口径，不是最优执行假设：
/// 实际成交价更好时省钱，更差时不够 —— 取保守侧，避免再次系统性高估。
pub const DEFAULT_ROUND_TRIP_COST_PCT: f64 =
    COMMISSION_BOTH_SIDES_PCT + STAMP_DUTY_PCT + SLIPPAGE_PCT;

/// 把毛收益换算为净收益（两者均为 %）。
///
/// 纯减法：不做任何 clamp —— 正负都可能，`-0.25` 合法（完全不动的票在实盘是亏 0.25%）。
#[inline]
pub fn net_of_cost(gross_return_pct: f64, cost_pct: f64) -> f64 {
    gross_return_pct - cost_pct
}

/// 用默认成本把毛收益换算为净收益。
#[inline]
pub fn net_return_pct(gross_return_pct: f64) -> f64 {
    net_of_cost(gross_return_pct, DEFAULT_ROUND_TRIP_COST_PCT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 各项费率之和必须与两个聚合常量严格自洽 —— 防止日后改单项却忘了聚合值。
    ///
    /// 断言的操作数**先绑到 let**：直接对常量表达式 `assert!` 会触发
    /// `clippy::assertions_on_constants`，在 `-D warnings` 下直接红。
    #[test]
    fn aggregates_match_components() {
        let no_slip = ROUND_TRIP_COST_NO_SLIPPAGE_PCT;
        let expect_no_slip = COMMISSION_BOTH_SIDES_PCT + STAMP_DUTY_PCT;
        assert!((no_slip - expect_no_slip).abs() < 1e-9, "{no_slip} vs {expect_no_slip}");

        let full = DEFAULT_ROUND_TRIP_COST_PCT;
        let expect_full = COMMISSION_BOTH_SIDES_PCT + STAMP_DUTY_PCT + SLIPPAGE_PCT;
        assert!((full - expect_full).abs() < 1e-9, "{full} vs {expect_full}");

        // 含滑点口径必须严格大于不含滑点口径 —— 否则「保守」是假的。
        assert!(full > no_slip, "含滑点 {full} 应严格大于无滑点 {no_slip}");
    }

    /// 与 `backtest.rs` 的历史小数口径必须严格等价（0.18% ↔ 0.0018）。
    #[test]
    fn no_slippage_matches_legacy_decimal_rate() {
        let as_decimal = ROUND_TRIP_COST_NO_SLIPPAGE_PCT / 100.0;
        assert!((as_decimal - 0.0018).abs() < 1e-12, "实际 {as_decimal}");
    }

    /// 净收益 = 毛 − 成本，且**不 clamp**：微利必须变成净亏，不能留在 0。
    #[test]
    fn net_is_plain_subtraction_without_clamping() {
        // +0.10% 毛收益 ⇒ 实盘净亏（这是本模块存在的理由）
        assert!((net_return_pct(0.10) - (0.10 - DEFAULT_ROUND_TRIP_COST_PCT)).abs() < 1e-9);
        assert!(net_return_pct(0.10) < 0.0);
        // 完全不动的票 ⇒ 净亏成本，不是 0
        assert!(net_return_pct(0.0) < 0.0);
        // 大赚不会被误伤
        assert!(net_return_pct(5.0) > 4.0);
    }

    /// 负控：若把成本当成 0，`+0.10%` 会保持为正 —— 用来证明上面那条断言真的有区分力。
    #[test]
    fn zero_cost_keeps_tiny_gain_positive() {
        assert!(net_of_cost(0.10, 0.0) > 0.0);
    }
}
