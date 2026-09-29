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
}
