//! 荐股链的尺度层（Phase R-B）—— 档位 → 取数尺度的**唯一入口**
//!
//! ## 为什么单独一层
//!
//! 此前四个档位的策略**全部**写死 `get_klines(code, "daily", N)`，档间只差「取几根日线」。
//! 于是「长线视角」实际是 300 根日线、超短视角是 20 根日线 —— 同一尺度换根数，
//! 不是不同尺度（诊断 R4，见 `PLAN-reco-horizon-science-alignment.md`）。
//! 分析决策链在 Phase B 已把尺度升成一等概念（`astock_data::scale::ScaleProfile`，
//! 未知 period **直接报错**、季度由月线聚合、阈值按每根 bar 覆盖的交易日数归一），
//! 荐股链过去完全没接 —— 本层就是那条接缝，**不新建第二套尺度定义**（禁区 12）。
//!
//! 门 `check-reco-horizon-parity.mjs` 的 f 段锁「`recommender/strategies/` 里不得出现写死
//! `"daily"` 的 `get_klines(`」，e 段锁风格×档位矩阵完备。

use axagent_astock_data::scale::{aggregate_monthly_to_quarterly, Scale, ScaleProfile};
use axagent_harness::market_data::{KLine, MarketDataProvider};

use crate::recommender::types::Period;

/// 档位 → 尺度名。与目标架构 §五 一致：超短=60 分钟、短=日线、中=周线、长=季度。
pub fn scale_name(period: Period) -> &'static str {
    match period {
        Period::UltraShort => "hourly",
        Period::Short => "daily",
        Period::Mid => "weekly",
        Period::Long => "quarterly",
    }
}

/// 档位 → 尺度剖面。未知档位名会走 `ScaleProfile::resolve` 的显式失败分支（不静默归日线）。
pub fn profile_for(period: Period) -> Result<ScaleProfile, String> {
    ScaleProfile::resolve(scale_name(period))
}

/// 按档位取其**应使用的尺度**的 K 线（季度档由月线本地聚合，vendor 无稳定季线 klt）。
///
/// 返回空串错误时调用方必须显式失败（该档无输入），不得回退日线再产出一个看起来正常的 pick。
pub async fn fetch(
    client: &dyn MarketDataProvider,
    code: &str,
    period: Period,
) -> Result<(Vec<KLine>, ScaleProfile), String> {
    let profile = profile_for(period)?;
    let raw = client
        .get_klines(code, profile.vendor_period, profile.fetch_limit, None)
        .await
        .map_err(|e| format!("{e}"))?;
    let bars = if profile.scale == Scale::Quarterly {
        aggregate_monthly_to_quarterly(&raw)
    } else {
        raw
    };
    Ok((bars, profile))
}

/// 出分所需最少 bar 数（口径来自 `ScaleProfile::min_bars`，策略侧不得自带一份条数门槛）。
pub fn enough_bars(bars: &[KLine], profile: &ScaleProfile) -> bool {
    bars.len() >= profile.min_bars
}

/// 把**日历日**口径的窗口（如「5 日动量」「20 日均额」）换算成当前尺度的 bar 数。
///
/// 用途：`capital` / `value` / `reversion` 的窗口本意是「多少个交易日的信息量」，
/// 换成尺度 bar 后必须保持同一信息量 —— 否则「日线 20 日均额」到季线就变成 20 季 ≈ 10 年，
/// 语义漂移完全不可见。
pub fn bars_for_daily_span(days: usize, profile: &ScaleProfile) -> usize {
    let per_bar = profile.trading_days_per_bar.max(f64::EPSILON);
    ((days as f64) / per_bar).ceil() as usize
}

/// 把**日线口径**的偏离容差（如「收盘价不低于 MA20 × 0.99」的 0.99）归一到当前尺度。
///
/// 依据：偏离幅度是波动的一次矩，每根 bar 覆盖 `d` 个交易日时典型偏离按 `√d` 放大
/// （与 `ScoreBands::scaled_for` 同一口径）。因此
/// `tol_weekly = 1 − (1 − tol_daily)·√5`。
pub fn dev_tolerance(tol_daily: f64, profile: &ScaleProfile) -> f64 {
    1.0 - (1.0 - tol_daily) * profile.trading_days_per_bar.sqrt()
}

/// 把**日线口径**的止损/目标幅度（正数百分比，如 5.0 = 5%）归一到当前尺度。
pub fn dev_move(move_daily_pct: f64, profile: &ScaleProfile) -> f64 {
    move_daily_pct * profile.trading_days_per_bar.sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 四档映射到四个**不同**尺度（档间尺度必须真的换，不是换根数）。
    #[test]
    fn four_tiers_map_to_four_distinct_scales() {
        let names: Vec<&str> = Period::ALL.iter().map(|p| scale_name(*p)).collect();
        assert_eq!(names, vec!["hourly", "daily", "weekly", "quarterly"]);
        let days: Vec<f64> =
            Period::ALL.iter().map(|p| profile_for(*p).unwrap().trading_days_per_bar).collect();
        for w in days.windows(2) {
            assert!(w[0] < w[1], "尺度粗细必须随档位单调: {days:?}");
        }
    }

    /// 季度档向 vendor 取的是月线并本地聚合；超短档取 60 分钟。
    #[test]
    fn quarterly_fetches_monthly_and_hourly_maps_to_vendor_60() {
        assert_eq!(profile_for(Period::Long).unwrap().vendor_period, "monthly");
        assert_eq!(profile_for(Period::UltraShort).unwrap().vendor_period, "60");
    }

    /// 日历日窗口换算：日线不动、周线压缩、季线压到很少几根。
    #[test]
    fn bars_for_daily_span_converts_calendar_days_to_bars() {
        let daily = profile_for(Period::Short).unwrap();
        let weekly = profile_for(Period::Mid).unwrap();
        let quarterly = profile_for(Period::Long).unwrap();
        assert_eq!(bars_for_daily_span(20, &daily), 20, "日线尺度下 20 日 = 20 根");
        assert_eq!(bars_for_daily_span(20, &weekly), 4, "20 个交易日 = 4 根周线");
        assert_eq!(bars_for_daily_span(20, &quarterly), 1, "20 个交易日 = 1 根季线");
    }

    /// 阈值归一：日线基准不动，周线放宽、小时线收紧，且比例正是 √d。
    #[test]
    fn dev_tolerance_scales_by_sqrt_trading_days() {
        let daily = profile_for(Period::Short).unwrap();
        let weekly = profile_for(Period::Mid).unwrap();
        let hourly = profile_for(Period::UltraShort).unwrap();
        assert!((dev_tolerance(0.99, &daily) - 0.99).abs() < 1e-12, "日线基准不应被改动");
        let expect_weekly = 1.0 - 0.01 * 5.0_f64.sqrt();
        assert!((dev_tolerance(0.99, &weekly) - expect_weekly).abs() < 1e-9);
        assert!(dev_tolerance(0.99, &hourly) > 0.99, "小时线的容差应收紧（更接近 1）");
        assert!(dev_tolerance(0.99, &weekly) < 0.99, "周线的容差应放宽（更远离 1）");
    }
}
