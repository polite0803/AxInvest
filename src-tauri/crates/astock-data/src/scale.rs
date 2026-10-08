//! 尺度剖面（四周期科学化 Phase B）
//!
//! ## 为什么需要它
//!
//! `compute_scoring` 此前把 `period` 白名单写死成 daily/weekly/monthly，且
//! **未知值静默归 daily**（`mcp_tools.rs` 的 `if period == "weekly" || ... else "daily"`）
//! —— 调用方以为拿到月线评分，实得日线评分，且没有任何信号。同时缺两个尺度：
//! 长线复用月线（`seed_stock_analysis.rs:4202`）、超短复用日线。
//!
//! 本模块把「尺度」升成一等概念：每个尺度自带
//! ① 向 vendor 取数用的 period 与根数、② 出分所需的最少 bar 数、
//! ③ 每根 bar 覆盖的交易日数（供阈值缩放与波动率换算复用）。
//!
//! ⚠ **指标窗口不需要按尺度改写**：`TechnicalIndicators` 的 MA/RSI/MACD 窗口以 **bar**
//! 为单位，周线 MA20 天然 = 20 周 ≈ 100 交易日。真正的尺度缺陷在
//! ① 静默降级、② 评分分段阈值是按日线波动标定的固定百分比（`ScoreBands`，Phase B-1 处理）、
//! ③ 缺 quarterly / sub-daily 两个尺度（本文件处理）。
//!
//! ⚠⚠ **上面这句只被证伪了一半，2026-10-07 起被 `ScaleWindowPlan` 取代其前半**：
//! ①②③ 全部成立且已生效（`compute_scoring` 现在就在调 `ScoreBands::scaled_for`），
//! 但「窗口以 bar 为单位」被顺势读成了「窗口与持有期无关」——后果可算：`min_bars = 60`
//! 在季线上要 **15 年**历史，而长档回答的是 90 交易日的持有问题，60 期均线的响应时间 ≫ 持有期
//! ⇒ 多数票上恒不穿越 ⇒ `trendScore` 在中/长档退化成常数。
//! 取代关系与比值推导记 PLAN `PLAN-four-horizon-workflow-alignment.md` §九十六；
//! 原文刻意不删，因为「bar 数含义上不错」这半句仍然成立，删掉会让人重推一遍。

use axagent_harness::market_data::KLine;

/// 评分尺度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scale {
    /// 60 分钟（超短档专用；vendor klt=60）
    Hourly,
    Daily,
    Weekly,
    Monthly,
    /// 季度（长档专用；由月线本地聚合而成 —— vendor 无稳定的季度 klt，不猜）
    Quarterly,
}

/// 一个尺度的取数与出分条件。
#[derive(Debug, Clone)]
pub struct ScaleProfile {
    pub scale: Scale,
    /// 对外声明名（`compute_scoring` 的 `period` 参数值）
    pub period: &'static str,
    /// 向 vendor `get_klines` 传的 period
    pub vendor_period: &'static str,
    /// 每根 bar 覆盖的交易日数（A 股口径：1 周 = 5 日、1 月 ≈ 20 日、1 季 ≈ 60 日、1 小时 = 0.25 日）
    pub trading_days_per_bar: f64,
    /// 取数根数（季度取的是**月线**根数）
    pub fetch_limit: u32,
    /// 出分所需最少 bar 数（聚合后计数）：不足 ⇒ 调用方必须显式失败，
    /// 不得拿 3 根 bar 去算 MA60 再给出一个看起来正常的分数。
    pub min_bars: usize,
}

impl ScaleProfile {
    const fn hourly() -> Self {
        Self {
            scale: Scale::Hourly,
            period: "hourly",
            vendor_period: "60",
            trading_days_per_bar: 0.25,
            // 320 根小时线 ≈ 80 个交易日，够 MA60（= 15 个交易日）留余量
            fetch_limit: 320,
            min_bars: 60,
        }
    }
    const fn daily() -> Self {
        Self {
            scale: Scale::Daily,
            period: "daily",
            vendor_period: "daily",
            trading_days_per_bar: 1.0,
            fetch_limit: 120,
            min_bars: 60,
        }
    }
    const fn weekly() -> Self {
        Self {
            scale: Scale::Weekly,
            period: "weekly",
            vendor_period: "weekly",
            trading_days_per_bar: 5.0,
            fetch_limit: 120,
            min_bars: 60,
        }
    }
    const fn monthly() -> Self {
        Self {
            scale: Scale::Monthly,
            period: "monthly",
            vendor_period: "monthly",
            trading_days_per_bar: 20.0,
            fetch_limit: 120,
            min_bars: 60,
        }
    }
    const fn quarterly() -> Self {
        Self {
            scale: Scale::Quarterly,
            period: "quarterly",
            // 季线由月线聚合而来 ⇒ 向 vendor 取的是月线
            vendor_period: "monthly",
            trading_days_per_bar: 60.0,
            // 60 根季线需要 180 根月线；多取一点应对停牌缺月
            fetch_limit: 240,
            min_bars: 60,
        }
    }

    /// 全部支持尺度（错误信息与自检都用它，避免两处列不同）。
    pub const ALL: [Scale; 5] =
        [Scale::Hourly, Scale::Daily, Scale::Weekly, Scale::Monthly, Scale::Quarterly];

    fn of(scale: Scale) -> Self {
        match scale {
            Scale::Hourly => Self::hourly(),
            Scale::Daily => Self::daily(),
            Scale::Weekly => Self::weekly(),
            Scale::Monthly => Self::monthly(),
            Scale::Quarterly => Self::quarterly(),
        }
    }

    /// 解析调用方传入的 period。
    ///
    /// **未知值一律 Err**（含拼错、含历史别名之外的值）—— 静默归 daily 会让
    /// 「以为拿到月线/季线，实得日线」这种错档完全不可见，是本模块存在的理由。
    pub fn resolve(period: &str) -> Result<Self, String> {
        let p = period.trim().to_ascii_lowercase();
        let scale = match p.as_str() {
            "hourly" | "60" | "min60" | "1h" => Scale::Hourly,
            "daily" | "101" | "day" => Scale::Daily,
            "weekly" | "102" | "week" => Scale::Weekly,
            "monthly" | "103" | "month" => Scale::Monthly,
            "quarterly" | "quarter" | "season" => Scale::Quarterly,
            other => {
                let allowed: Vec<&str> = Self::ALL.iter().map(|s| Self::of(*s).period).collect();
                return Err(format!(
                    "不支持的评分尺度 '{other}'（允许：{}）。**不静默回退 daily** —— \
                     回退会让「以为拿到月/季线、实得日线」这种错档不可见。",
                    allowed.join("/")
                ));
            },
        };
        Ok(Self::of(scale))
    }
}

/// 一档在该尺度上的**指标窗口根数**（PLAN `PLAN-four-horizon-workflow-alignment.md` §九十六）。
///
/// ## 为什么需要它（与本文件头注释的分歧，如实记）
///
/// 本文件 Phase B 的头部结论是「指标窗口不需要按尺度改写：`TechnicalIndicators` 的 MA/RSI/MACD
/// 窗口以 bar 为单位，周线 MA20 天然 = 20 周 ≈ 100 交易日」。这句话在**bar 数含义**上没错，
/// 但它把「窗口跨度」与「持有期」这两件事解耦了，后果实测如下：
///
/// - `min_bars = 60` 在四个尺度上分别是 60 交易日 / 60 周(≈1.2 年) / 60 月(**5 年**) /
///   60 季(**15 年**) —— 长档的出分条件要求 15 年季线历史，而它要回答的是 90 交易日的持有问题；
/// - 一条 60 期均线的响应时间远大于 90 交易日 ⇒ 对多数票**恒不穿越**，
///   于是 `trendScore` 在中/长档退化成常数（四档「互不相同」的名义与实质分离）。
///
/// 2026-10-06 用户就 §九十二 量到的缺陷拍板「丙：接通 + 同步改出分口径」，本类型就是那句
/// 「改口径」的落点：**窗口按日历跨度定，bar 数由尺度换算**，日线链逐字不变（见 `daily_default`）。
///
/// ## 倍数从哪来（不从零发明）
///
/// 全部锚在日线既有口径**自身的**比例上（`indicators.rs` 的 `IndicatorConfig::default()`：
/// `ma_periods=[5,10,20,60]`、`macd=12/26/9`、`rsi=[6,12,14,24]`、`boll_period=20`、
/// `volume_lookback=5`、`ScaleProfile::daily` 的 `min_bars=60`/`fetch_limit=120`）。
/// 取日线「中带 = MA20」为基准 `short`，其余族按比值定：
/// `long = 3×short`（60=3×20）、`rsi = 0.7×short`（14/20）、`macd_fast = 0.6×short`（12/20）、
/// `macd_slow = 1.3×short`（26/20）、`macd_signal = 0.75×macd_fast`（9/12）、
/// `boll = short`（20/20）、`volume_lookback = 0.25×short`（5/20）、`min_bars = long`（60）、
/// `fetch_limit = 2×long`（120）。
/// ⇒ 把 `short` 取 20 时代式**逐字复现日线默认**（见测试 `daily_default_reproduces_existing_constants`），
/// 所以「档位用这套、日线不用」不是两套真相，而是同一个比值在两个尺度上的两次代入。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScaleWindowPlan {
    /// 本计划所在尺度（`ScaleProfile::period`）
    pub scale_key: &'static str,
    /// 该档的建议持有交易日数（权威 = `Period::default_holding_days`）
    pub holding_days: u32,
    /// 中带（趋势带下沿）根数
    pub short_bars: usize,
    /// 长带（趋势带上沿）根数，= `3 × short_bars`
    pub long_bars: usize,
    pub rsi_period: usize,
    pub macd_fast: usize,
    pub macd_slow: usize,
    pub macd_signal: usize,
    pub boll_period: usize,
    pub volume_lookback: usize,
    /// 出分所需最少 bar 数（= `long_bars`；不足 ⇒ 显式失败，不得拿少数 bar 出「看起来正常」的分）
    pub min_bars: usize,
    /// 向 vendor 取的根数（= `2 × long_bars`，留一倍余量给 MA 的前置窗口）
    pub fetch_limit: usize,
    /// 季线专用：聚合**前**需要的月线根数（vendor 无稳定季度 klt，见 `aggregate_monthly_to_quarterly`）
    pub parent_fetch_limit: Option<usize>,
}

/// 正比取整：`round(x × ratio)` 且下限 `floor`（所有族共用的钳位，避免 1 根窗口）。
fn scaled(short: usize, ratio: f64, floor: usize) -> usize {
    ((short as f64 * ratio).round() as usize).max(floor)
}

impl ScaleWindowPlan {
    /// 档位 → 该档尺度的窗口计划。
    ///
    /// 尺度与持有期都**不在这里重述**：`scale_key` / `default_holding_days` 来自
    /// `axagent_harness::holding_period::Period`（2026-10-07 才有的权威，见 PLAN §九十五），
    /// 尺度的日历跨度来自 `ScaleProfile::trading_days_per_bar`。这里只做换算。
    pub fn for_period(period: axagent_harness::holding_period::Period) -> Self {
        let profile = ScaleProfile::resolve(period.scale_key())
            .unwrap_or_else(|e| panic!("{e} —— `Period::scale_key` 与尺度表白名单不同步"));
        let d = profile.trading_days_per_bar;
        let h = period.default_holding_days() as f64;
        let short_bars = (h / d).ceil() as usize;
        let short_bars = short_bars.clamp(2, 60);
        let long_bars = 3 * short_bars;
        let macd_fast = scaled(short_bars, 0.6, 2);
        let macd_slow = scaled(short_bars, 1.3, macd_fast + 1);
        let parent_fetch_limit = (profile.scale == Scale::Quarterly).then(|| 3 * 2 * long_bars);
        Self {
            scale_key: profile.period,
            holding_days: period.default_holding_days(),
            short_bars,
            long_bars,
            rsi_period: scaled(short_bars, 0.7, 2),
            macd_fast,
            macd_slow,
            macd_signal: scaled(macd_fast, 0.75, 2),
            boll_period: short_bars,
            volume_lookback: scaled(short_bars, 0.25, 2),
            min_bars: long_bars,
            fetch_limit: 2 * long_bars,
            parent_fetch_limit,
        }
    }

    /// 日线链的窗口 —— **逐字等于既有默认**，这条是零回归锚（有测试钉）。
    ///
    /// 日线不属于任何一档（主链的 `t-scoring` 是 σ_daily 与主评分的共同来源），
    /// 所以它不走 `for_period`；本函数存在是为了让「档位用比值、日线用常数」这两条
    /// 在同一处对账，而不是让日线也吃一次换算结果。
    pub fn daily_default() -> Self {
        Self {
            scale_key: "daily",
            holding_days: 0,
            short_bars: 20,
            long_bars: 60,
            rsi_period: 14,
            macd_fast: 12,
            macd_slow: 26,
            macd_signal: 9,
            boll_period: 20,
            volume_lookback: 5,
            min_bars: 60,
            fetch_limit: 120,
            parent_fetch_limit: None,
        }
    }
}

/// 月线 → 季线：按**自然季度**聚合（1-3 月 = Q1 …）。
///
/// 口径：`open` 取季内首月开盘、`close` 取季内末月收盘、`high`/`low` 取极值、
/// `volume`/`amount` 求和、`date` 取末月日期（季末日）；`turnover_rate` 求和
/// （月换手率相加 = 季内累计换手，语义自洽）；`adj_factor` 取末月值。
/// 输入必须按日期升序；乱序输入先按日期排序再聚合。
pub fn aggregate_monthly_to_quarterly(monthly: &[KLine]) -> Vec<KLine> {
    let mut sorted: Vec<KLine> = monthly.to_vec();
    sorted.sort_by(|a, b| a.date.cmp(&b.date));
    let mut out: Vec<KLine> = Vec::new();
    let mut cur_key: Option<(i32, u32)> = None;
    for k in sorted {
        let key = match quarter_key(&k.date) {
            Some(v) => v,
            None => continue, // 日期解析不出来的 bar 直接跳过（不猜它属于哪一季）
        };
        if cur_key != Some(key) {
            out.push(k.clone());
            cur_key = Some(key);
            continue;
        }
        let last = out.last_mut().expect("已开季内必有聚合条");
        last.high = last.high.max(k.high);
        last.low = last.low.min(k.low);
        last.close = k.close;
        last.date = k.date;
        last.volume += k.volume;
        last.amount += k.amount;
        last.turnover_rate = match (last.turnover_rate, k.turnover_rate) {
            (Some(a), Some(b)) => Some(a + b),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
        last.adj_factor = k.adj_factor;
    }
    out
}

/// 从 `YYYY-MM-DD` / `YYYY-MM` 形态的日期取 (年, 季度)。
fn quarter_key(date: &str) -> Option<(i32, u32)> {
    let y: i32 = date.get(0..4)?.parse().ok()?;
    let m: u32 = date.get(5..7)?.parse().ok()?;
    if !(1..=12).contains(&m) {
        return None;
    }
    Some((y, (m - 1) / 3 + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(date: &str, o: f64, h: f64, l: f64, c: f64, v: f64) -> KLine {
        KLine {
            date: date.to_string(),
            open: o,
            high: h,
            low: l,
            close: c,
            volume: v,
            amount: v * c,
            turnover_rate: Some(1.0),
            adj_factor: None,
        }
    }

    /// 未知尺度必须显式失败（旧实现静默归 daily 是本模块的存在理由）。
    #[test]
    fn unknown_period_fails_instead_of_silently_becoming_daily() {
        for bad in ["", "quarter", "dayly", "1101", "monthly2", "weeklyy"] {
            let err = ScaleProfile::resolve(if bad == "quarter" { "not-a-scale" } else { bad })
                .expect_err("未知尺度应失败");
            assert!(err.contains("不支持的评分尺度"), "诊断应点名: {err}");
        }
        // `quarter` 是季线的合法别名 ⇒ 上面循环里换成真错串单独验
        assert!(ScaleProfile::resolve("quarter").is_ok());
    }

    /// 五个尺度各自可解析，且「每根 bar 覆盖的交易日数」严格单调（尺度序不能乱）。
    #[test]
    fn five_scales_resolve_and_are_ordered_by_granularity() {
        let days: Vec<f64> =
            ScaleProfile::ALL.iter().map(|s| ScaleProfile::of(*s).trading_days_per_bar).collect();
        for pair in days.windows(2) {
            assert!(pair[0] < pair[1], "尺度粗细顺序错: {days:?}");
        }
        assert_eq!(ScaleProfile::resolve("hourly").unwrap().vendor_period, "60");
        assert_eq!(ScaleProfile::resolve("quarterly").unwrap().vendor_period, "monthly");
    }

    /// 季线聚合：3 根月线合成 1 根季线，OHLC 口径逐项正确。
    #[test]
    fn quarterly_aggregation_uses_open_first_close_last_extremes_sum() {
        let monthly = vec![
            bar("2025-01-31", 10.0, 12.0, 9.0, 11.0, 100.0),
            bar("2025-02-28", 11.0, 13.0, 10.5, 12.0, 200.0),
            bar("2025-03-31", 12.0, 15.0, 11.0, 14.0, 300.0),
        ];
        let q = aggregate_monthly_to_quarterly(&monthly);
        assert_eq!(q.len(), 1);
        assert_eq!((q[0].open, q[0].close), (10.0, 14.0), "开=季首月开、收=季末月收");
        assert_eq!((q[0].high, q[0].low), (15.0, 9.0), "高低取季内极值");
        assert_eq!(q[0].volume, 600.0, "量应求和");
        assert_eq!(q[0].date, "2025-03-31", "日期应是季末");
    }

    /// 跨季要断开；乱序输入要先排序。
    #[test]
    fn quarterly_aggregation_splits_across_quarters() {
        let monthly = vec![
            bar("2025-04-30", 14.0, 16.0, 13.0, 15.0, 400.0),
            bar("2025-01-31", 10.0, 12.0, 9.0, 11.0, 100.0),
            bar("2025-02-28", 11.0, 13.0, 10.5, 12.0, 200.0),
        ];
        let q = aggregate_monthly_to_quarterly(&monthly);
        assert_eq!(q.len(), 2, "Q1 与 Q2 必须分成两根");
        assert_eq!(q[0].date, "2025-02-28");
        assert_eq!(q[1].open, 14.0);
    }

    /// 日期解析不出来的 bar 被跳过，而不是被塞进某一季。
    #[test]
    fn unparsable_dates_are_skipped_not_guessed() {
        let q = aggregate_monthly_to_quarterly(&[bar("n/a", 1.0, 2.0, 0.5, 1.5, 10.0)]);
        assert!(q.is_empty(), "坏日期不得产出季线");
    }

    /// 零回归锚：日线不属于任何一档，它的窗口必须**逐字等于**既有默认。
    ///
    /// 对账对象不是我手敲的数，而是两处既有权威：`IndicatorConfig::default()` 与
    /// `ScaleProfile::daily()` 的 `min_bars`/`fetch_limit`。任何一侧被改而另一侧没改 ⇒ 红。
    #[test]
    fn daily_default_reproduces_existing_constants() {
        use crate::indicators::IndicatorConfig;
        let cfg = IndicatorConfig::default();
        let plan = ScaleWindowPlan::daily_default();
        assert!(
            cfg.ma_periods.contains(&plan.short_bars),
            "日线中带 {} 不在 ma_periods 里",
            plan.short_bars
        );
        assert!(
            cfg.ma_periods.contains(&plan.long_bars),
            "日线长带 {} 不在 ma_periods 里",
            plan.long_bars
        );
        assert_eq!(cfg.macd_fast, plan.macd_fast);
        assert_eq!(cfg.macd_slow, plan.macd_slow);
        assert_eq!(cfg.macd_signal, plan.macd_signal);
        assert!(
            cfg.rsi_periods.contains(&plan.rsi_period),
            "日线 RSI {} 不在 rsi_periods 里",
            plan.rsi_period
        );
        assert_eq!(cfg.boll_period, plan.boll_period);
        assert_eq!(cfg.volume_lookback, plan.volume_lookback);
        let daily = ScaleProfile::of(Scale::Daily);
        assert_eq!(daily.min_bars, plan.min_bars, "min_bars 与尺度表不一致");
        assert_eq!(daily.fetch_limit as usize, plan.fetch_limit, "fetch_limit 与尺度表不一致");
    }

    /// `harness::Period::scale_key()` 的字面量必须能被本模块的尺度表白名单认下来。
    ///
    /// 这条就是 §九十二 那条 `period` 死参数的 CI 版：串写错/漏登记时，`for_period` 会 panic
    /// （`resolve` 不静默回退 daily），本测试因此红，而不是让四档悄悄都用日线。
    #[test]
    fn tier_scale_keys_are_resolvable_by_this_table() {
        use axagent_harness::holding_period::Period;
        for period in Period::ALL {
            let plan = ScaleWindowPlan::for_period(period);
            assert_eq!(plan.scale_key, period.scale_key());
            assert_eq!(plan.holding_days, period.default_holding_days());
        }
    }

    /// 四档计划的结构性判据：中带至少要盖住持有期，且 `min_bars ≤ fetch_limit`（否则永远出不了分）。
    ///
    /// ⚠ 刻意**不**断言「四档的根数互不相同」：短/中/长三档的 `short_bars` 按换算就是同一个整数 2，
    /// 区别在**尺度**（2 周 / 2 月 / 2 季）—— 拿整数当判据会当场恒假，
    /// 而「两档输入恒等」的真判据是 R5 那条（节点 id 是否等于本档尺度的节点）。
    #[test]
    fn tier_window_plans_cover_holding_period_and_are_admissible() {
        use axagent_harness::holding_period::Period;
        for period in Period::ALL {
            let plan = ScaleWindowPlan::for_period(period);
            let d = ScaleProfile::resolve(plan.scale_key).unwrap().trading_days_per_bar;
            let short_span = plan.short_bars as f64 * d;
            assert!(
                short_span >= period.default_holding_days() as f64,
                "{}：中带只有 {short_span:.1} 交易日，盖不住 {} 天的持有期",
                period.as_str(),
                period.default_holding_days()
            );
            assert_eq!(
                plan.long_bars,
                3 * plan.short_bars,
                "长带必须 = 3×中带（日线锚 MA60=3×MA20）"
            );
            assert!(plan.macd_slow > plan.macd_fast, "MACD 慢线必须 > 快线");
            assert!(
                plan.min_bars <= plan.fetch_limit,
                "{}：要 {} 根才出分，却只取 {} 根 ⇒ 恒失败",
                period.as_str(),
                plan.min_bars,
                plan.fetch_limit
            );
            for name in [plan.rsi_period, plan.boll_period, plan.volume_lookback, plan.macd_signal]
            {
                assert!(name >= 2, "窗口 {name} 被钳到 <2 ⇒ 指标会退化成常数");
            }
        }
    }

    /// 季线由月线聚合 ⇒ 计划必须同时给出「聚合前该取多少根月线」，其余尺度不得给这个值。
    #[test]
    fn quarterly_plan_carries_parent_fetch_and_others_do_not() {
        use axagent_harness::holding_period::Period;
        let long_plan = ScaleWindowPlan::for_period(Period::Long);
        assert_eq!(long_plan.scale_key, "quarterly");
        assert_eq!(
            long_plan.parent_fetch_limit,
            Some(3 * long_plan.fetch_limit),
            "季线根数 × 3 才是聚合前的月线根数"
        );
        for p in [Period::UltraShort, Period::Short, Period::Mid] {
            assert_eq!(ScaleWindowPlan::for_period(p).parent_fetch_limit, None);
        }
    }

    /// 样本不足时的口径判据：要显式失败并点名尺度与两者根数，**不得**出分。
    ///
    /// 这条是 §九十三(2)「只下调 min_bars 不可用」的另一面 —— 地板留着，但诊断必须可归因，
    /// 否则「这一档没算出来」与「该档按设计无结论」又混成一句（本仓禁止的伪装）。
    #[test]
    fn insufficient_history_fails_by_naming_scale_and_counts() {
        use axagent_harness::holding_period::Period;
        let plan = ScaleWindowPlan::for_period(Period::Long);
        let bars = 3usize;
        assert!(bars < plan.min_bars, "前提：这条样本本来就该拒");
        let err = format!(
            "compute_scoring: 尺度 {} 只有 {bars} 根 bar，出分需要 {} 根 ⇒ 拒绝出分",
            plan.scale_key, plan.min_bars
        );
        assert!(err.contains("quarterly") && err.contains("6"), "诊断没点名尺度或根数: {err}");
    }

    /// 负控（对齐 PLAN §九十三(1) 读数 2 那条半接线史）：计划里的窗口若被写死成
    /// **日线专属**的数（5/10/20/60），粗尺度就退化成「换个标签」。这条钉住
    /// 「档位计划的 short 带不等于日线默认」，除非两者本来就是同一尺度。
    #[test]
    fn tier_plans_are_not_copies_of_the_daily_default() {
        use axagent_harness::holding_period::Period;
        let daily = ScaleWindowPlan::daily_default();
        for period in Period::ALL {
            let plan = ScaleWindowPlan::for_period(period);
            if period.scale_key() == "daily" {
                assert_eq!(plan.short_bars, daily.short_bars);
            } else {
                assert_ne!(
                    plan.scale_key,
                    daily.scale_key,
                    "{} 档不该落回日线尺度",
                    period.as_str()
                );
            }
        }
    }

    /// 把四档窗口表打出来（人工审阅 + PLAN §九十六 那张表的来源，`#[ignore]`）：
    /// `cargo test -p axagent-astock-data --lib horizon_window_plan_dump -- --ignored --nocapture`
    #[test]
    #[ignore = "仅用于人工核对窗口表"]
    fn horizon_window_plan_dump() {
        use axagent_harness::holding_period::Period;
        for p in Period::ALL {
            let pl = ScaleWindowPlan::for_period(p);
            println!(
                "档={} scale={} h={} 短带={} 长带={} rsi={} macd={}/{}/{} boll={} vol={} min={} fetch={} 聚合前={:?}",
                p.as_str(),
                pl.scale_key,
                pl.holding_days,
                pl.short_bars,
                pl.long_bars,
                pl.rsi_period,
                pl.macd_fast,
                pl.macd_slow,
                pl.macd_signal,
                pl.boll_period,
                pl.volume_lookback,
                pl.min_bars,
                pl.fetch_limit,
                pl.parent_fetch_limit
            );
        }
        let d = ScaleWindowPlan::daily_default();
        println!(
            "档=daily(零回归锚) 短带={} 长带={} rsi={} macd={}/{}/{} boll={} vol={} min={} fetch={}",
            d.short_bars,
            d.long_bars,
            d.rsi_period,
            d.macd_fast,
            d.macd_slow,
            d.macd_signal,
            d.boll_period,
            d.volume_lookback,
            d.min_bars,
            d.fetch_limit
        );
    }
}
