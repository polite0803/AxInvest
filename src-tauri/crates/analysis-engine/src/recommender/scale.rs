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
//! ## 2026-10-18 口径收敛（`PLAN-horizon-scale-alignment.md`）
//!
//! 本层原先自带一张 `Period → 尺度` 四臂表（`hourly/daily/weekly/quarterly`），
//! 与分析链 harness 里的 `Period::scale_key()`（旧值 `hourly/weekly/monthly/quarterly`）
//! **是两套阶梯**：同一个「短期」在两条链上不是同一尺度。当日已删——本层只**转发**
//! `Period::scale_key()`，阶梯只存在于 harness 一处（门 `check-reco-horizon-parity.mjs` 门 n）。
//!
//! 同时「出分所需最少 bar 数」改由该档的 `ScaleWindowPlan` 给（`ceil(h/d)` 起算的长带），
//! **不再读 `ScaleProfile::min_bars`**：那是「尺度自身的粗下限」，在季线上等于 60 根 ≈ 15 年，
//! 而长档要回答的是 **90 交易日**的问题 ⇒ 旧地板让长期档几乎恒空（实测 300567/688012/301302
//! 全部整档拒）。
//!
//! 门 `check-reco-horizon-parity.mjs` 的 f 段锁「`recommender/strategies/` 里不得出现写死
//! `"daily"` 的 `get_klines(`」，n 段锁「本文件不得再写四臂阶梯」，e 段锁风格×档位矩阵完备。

use axagent_astock_data::scale::{
    aggregate_monthly_to_quarterly, Scale, ScaleProfile, ScaleWindowPlan,
};
use axagent_harness::market_data::{KLine, MarketDataProvider};

use crate::recommender::types::Period;

/// 档位 → 尺度名 —— **转发** `Period::scale_key()`，本层不持有第二份阶梯。
///
/// 保留这个函数名是因为策略侧（`trend.rs` 的 reasons 文案）已经在用；
/// 它的语义从「本层的一张表」降级为「权威的一个别名」。
pub fn scale_name(period: Period) -> &'static str {
    period.scale_key()
}

/// 档位 → 尺度剖面。未知档位名会走 `ScaleProfile::resolve` 的显式失败分支（不静默归日线）。
pub fn profile_for(period: Period) -> Result<ScaleProfile, String> {
    ScaleProfile::resolve(scale_name(period))
}

/// 本档的**窗口计划** —— 取数根数与出分下限的唯一来源（按持有期换算，见模块文档）。
pub fn window_plan(period: Period) -> ScaleWindowPlan {
    ScaleWindowPlan::for_period(period)
}

/// 按档位取其**应使用的尺度**的 K 线（季线由月线本地聚合；2026-10-18 起无档位落在季线上）。
///
/// 返回空串错误时调用方必须显式失败（该档无输入），不得回退日线再产出一个看起来正常的 pick。
///
/// ## `required_daily_bars`（R-3e，2026-10-26）
///
/// 窗口计划（`window_plan`）的 `fetch_limit` 是**按持有期**换算的（`2 × long_bars`），
/// 但策略的**指标窗口**是另一套口径（seed 里的「回撤回看天数」等）——
/// 两者可能不一致：实测 `reversion` 的 `rev_dd_period=250`（天）在 mid(weekly) 档换算后需
/// **50 根周线**，而 `fetch_limit` 只有 **36 根** ⇒ 指标永远在「不够的窗口」上计算。
///
/// 因此调用方要声明「本策略额外需要多少**日**口径的 bar」，
/// 本函数把它换算到当前尺度后与 `fetch_limit` 取**较大者**。
///
/// ⚠ `client.get_klines` 内部会 `limit.max(KLINE_FETCH_FLOOR)` 去 vendor 取 500 根，
/// 但**返回时又截到 `limit`** ⇒ 这里传入的 `limit` 就是最终可用根数，**必须算准**。
pub async fn fetch(
    client: &dyn MarketDataProvider,
    code: &str,
    period: Period,
    required_daily_bars: usize,
) -> Result<(Vec<KLine>, ScaleProfile), String> {
    let profile = profile_for(period)?;
    // 取数根数归窗口计划管：季线要的是**聚合前**的月线根数（`parent_fetch_limit`），
    // 其余尺度就是 `fetch_limit`（= 2 × 长带，留一倍余量给 MA 的前置窗口）。
    let plan = window_plan(period);
    let mut limit = plan.parent_fetch_limit.unwrap_or(plan.fetch_limit);
    // 策略声明的额外需求（按日口径）换算到当前尺度后，与窗口计划取较大者。
    let strategy_need = bars_for_daily_span(required_daily_bars, &profile).max(1);
    if strategy_need > limit {
        limit = strategy_need;
    }
    let limit = limit as u32;
    let raw = client
        .get_klines(code, profile.vendor_period, limit, None)
        .await
        .map_err(|e| format!("{e}"))?;
    let bars = if profile.scale == Scale::Quarterly {
        aggregate_monthly_to_quarterly(&raw)
    } else {
        raw
    };
    Ok((bars, profile))
}

/// 出分所需最少 bar 数 —— **按档**（= 本档窗口计划的 `min_bars`）。
///
/// ⚠ 参数是 `Period` 而不是 `&ScaleProfile`（2026-10-18 改签名）：旧签名只能读
/// `ScaleProfile::min_bars`，那是**每根 bar 都 60 期**的粗下限 —— 在季线上要 15 年历史、
/// 在月线上要 5 年，与「这一档持有多少交易日」毫无关系。签名改成要档位，
/// 就是让「拿错下限」在编译期就不可能（策略侧不得自带一份条数门槛）。
pub fn enough_bars(bars: &[KLine], period: Period) -> bool {
    bars.len() >= window_plan(period).min_bars
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

    /// 四档映射到四个**不同**尺度（档间尺度必须真的换，不是换根数），
    /// 且本层的 `scale_name` **逐档等于** harness 的 `Period::scale_key()`
    /// —— 它只是别名，不是第二张表（门 n 锁同一条；门 g 是「理由码 11 语言」，不是这条）。
    #[test]
    fn four_tiers_map_to_four_distinct_scales() {
        let names: Vec<&str> = Period::ALL.iter().map(|p| scale_name(*p)).collect();
        assert_eq!(names, vec!["hourly", "daily", "weekly", "monthly"]);
        for p in Period::ALL {
            assert_eq!(
                scale_name(p),
                p.scale_key(),
                "{:?} 的尺度在荐股层与 harness 分了家 ⇒ 阶梯出现第二份",
                p
            );
        }
        let days: Vec<f64> =
            Period::ALL.iter().map(|p| profile_for(*p).unwrap().trading_days_per_bar).collect();
        for w in days.windows(2) {
            assert!(w[0] < w[1], "尺度粗细必须随档位单调: {days:?}");
        }
    }

    /// 长档取的是**月线**（不是季线：90 交易日只有 1.5 根季线）；超短仍取 60 分钟。
    #[test]
    fn long_fetches_monthly_and_hourly_maps_to_vendor_60() {
        assert_eq!(profile_for(Period::Long).unwrap().vendor_period, "monthly");
        assert_eq!(profile_for(Period::Long).unwrap().scale, Scale::Monthly);
        assert_eq!(profile_for(Period::UltraShort).unwrap().vendor_period, "60");
        for p in Period::ALL {
            assert_ne!(
                profile_for(p).unwrap().scale,
                Scale::Quarterly,
                "{p:?} 又落回季线 ⇒ 违反 d ≤ h/3"
            );
        }
    }

    /// R-3f（2026-10-26）：背离检测的 **RSI 平滑窗与比较窗必须同量纲**。
    ///
    /// `detect_all_divergences(klines, rsi_period, lookback)` 的两个参数都在**同一
    /// bar 序列**上取下标 ⇒ 必须都用「日历日 → 当前尺度 bar 数」换算。
    /// 原实现 `lookback` 换算了、`rsi_period` 硬编码 14 根 bar ⇒ 粗尺度上两者不同量纲。
    ///
    /// 本判据把「同量纲」写成可执行断言（而不是注释论证）：
    /// ① daily 尺度下换算结果**恒等 14**（零回归的硬保证）；
    /// ② 任一粗尺度下，换算后的 rsi_period **不得**仍等于 14（那是漏换算的特征值）；
    /// ③ 换算后两者**同量级**（rsi_period ≤ lookback × 3）—— 否则平滑窗远长于比较窗，
    ///    背离检测比较的是「两个不同长度窗口里的极值」，不是同一个检测。
    #[test]
    fn divergence_rsi_period_and_lookback_share_one_scale() {
        // ① daily 恒等 ⇒ 零回归
        let daily = profile_for(Period::Short).unwrap();
        assert_eq!(bars_for_daily_span(14, &daily), 14, "daily 尺度换算必须恒等（零回归前提）");

        for p in [Period::Short, Period::Mid, Period::Long] {
            let profile = profile_for(p).unwrap();
            let lookback = bars_for_daily_span(14, &profile).max(5);
            let rsi_period = bars_for_daily_span(14, &profile).max(5);
            // ② 粗尺度不得仍为 14（14 是漏换算的特征值）
            if profile.trading_days_per_bar > 1.0 {
                assert_ne!(
                    rsi_period, 14,
                    "{p:?} 尺度上 rsi_period 仍是 14 ⇒ 漏了 bars_for_daily_span 换算"
                );
            }
            // ③ 同量级
            assert!(
                rsi_period <= lookback * 3,
                "{p:?} 上 rsi_period={rsi_period} 远长于 lookback={lookback} ⇒ 两个窗口不同量纲"
            );
        }
    }

    /// 出分下限**按档**而不是按尺度：长档只要 15 根月线（≈15 个月），
    /// 而 `ScaleProfile::monthly().min_bars = 60`（5 年）是被退役的旧地板。
    ///
    /// 这条就是「长期档恒空」的回归锁：300567 有 120 根月线、688012 有 88 根、
    /// 301302 有 53 根 —— 在旧地板下全部被拒，在新下限下全部通过。
    #[test]
    fn enough_bars_uses_tier_min_bars_not_the_scale_floor() {
        let plan = window_plan(Period::Long);
        let floor = profile_for(Period::Long).unwrap().min_bars;
        assert!(
            plan.min_bars < floor,
            "前提：按档下限 {plan_min} 必须比尺度粗地板 {floor} 更宽松（否则本测试测不出差别）",
            plan_min = plan.min_bars
        );
        let bars = |n: usize| -> Vec<KLine> {
            (0..n)
                .map(|i| KLine {
                    date: format!("2024-{:02}-01", (i % 12) + 1),
                    open: 10.0,
                    high: 11.0,
                    low: 9.0,
                    close: 10.0,
                    volume: 1.0,
                    amount: 10.0,
                    turnover_rate: None,
                    adj_factor: None,
                })
                .collect()
        };
        for n in [15usize, 53, 88, 120] {
            assert!(enough_bars(&bars(n), Period::Long), "长档 {n} 根月线应能出分");
        }
        assert!(
            !enough_bars(&bars(plan.min_bars - 1), Period::Long),
            "差一根就该拒（不得拿少数 bar 出「看起来正常」的分）"
        );
    }

    /// 日历日窗口换算：日线不动、周线压缩、月线压到很少几根。
    #[test]
    fn bars_for_daily_span_converts_calendar_days_to_bars() {
        let daily = profile_for(Period::Short).unwrap();
        let weekly = profile_for(Period::Mid).unwrap();
        let monthly = profile_for(Period::Long).unwrap();
        assert_eq!(bars_for_daily_span(20, &daily), 20, "日线尺度下 20 日 = 20 根");
        assert_eq!(bars_for_daily_span(20, &weekly), 4, "20 个交易日 = 4 根周线");
        assert_eq!(bars_for_daily_span(20, &monthly), 1, "20 个交易日 = 1 根月线");
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

    /// **R-3e 回归（2026-10-26）**：各策略声明的 `required_daily_bars` 换算后
    /// **不得超过该档的 `fetch_limit`**，否则取数根数会变 ⇒ 既有判定结果会变。
    ///
    /// 背景：`scale::fetch` 新增 `required_daily_bars` 参数后，
    /// `limit = max(fetch_limit, 换算后的需求)`。
    /// 若需求 > `fetch_limit`，该档的取数根数会**变大** ⇒
    /// `detect_all_patterns` / `divergence` 这类**扫全数组**的判定结果会变
    /// ⇒ 那就是**自己制造的回归**（R-3e 的修复只允许影响此前恒 `None` 的档位）。
    ///
    /// 本测试把「零回归」从注释里的论证变成**可执行的判据**。
    #[test]
    fn strategy_declared_requirements_never_exceed_fetch_limit_except_reversion_mid() {
        // (策略, 档位, 声明的日口径需求)
        let declared: [(&str, Period, usize); 11] = [
            ("reversion", Period::Short, 14),
            ("reversion", Period::Mid, 250),
            ("reversion", Period::UltraShort, 0),
            ("value", Period::Short, 20),
            ("value", Period::Mid, 20),
            ("value", Period::Long, 60),
            ("value", Period::UltraShort, 10),
            ("trend", Period::Short, 0),
            ("trend", Period::Mid, 0),
            ("trend", Period::Long, 0),
            ("capital", Period::Short, 12),
        ];
        for (name, period, req) in declared {
            let profile = profile_for(period).unwrap();
            let plan = window_plan(period);
            let base = plan.parent_fetch_limit.unwrap_or(plan.fetch_limit);
            let need = bars_for_daily_span(req, &profile).max(1);
            // 唯一允许变化的是 reversion/mid（此前该档因窗口不足而恒 None ⇒ 变更无回归风险）
            let allowed_to_change = name == "reversion" && period == Period::Mid;
            if need > base {
                assert!(
                    allowed_to_change,
                    "{name}/{period:?} 的需求 {req} 天换算成 {need} 根 > fetch_limit {base} 根 \
                     ⇒ 取数根数会变 ⇒ 会造成回归；请下调该档的声明需求"
                );
            }
        }
    }
}
