use crate::types::KLine;
use serde::{Deserialize, Serialize};

/// 技术指标计算结果
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TechnicalIndicators {
    pub stock_code: String,
    pub latest_date: String,
    /// 均线 SMA
    pub ma5: f64,
    pub ma10: f64,
    pub ma20: f64,
    pub ma60: f64,
    /// MA排列状态: "多头排列", "空头排列", "弱多头", "缠绕/交叉"
    pub ma_alignment: String,
    /// MACD (12/26/9)
    pub macd_dif: f64,
    pub macd_dea: f64,
    pub macd_bar: f64,       // (DIF - DEA) × 2
    pub macd_signal: String, // "金叉", "死叉", "多头运行", "空头运行"
    /// RSI (6/12/14/24)
    pub rsi6: f64,
    pub rsi12: f64,
    pub rsi14: f64,
    pub rsi24: f64,
    pub rsi_signal: String, // "超买", "超卖", "强势", "弱势", "中性"
    /// 布林带 (20,2)
    pub boll_upper: f64,
    pub boll_mid: f64, // MA20
    pub boll_lower: f64,
    pub boll_position: String, // "上轨以上", "上轨区间", "中轨附近", "下轨区间", "下轨以下"
    /// 乖离率 (%)
    pub bias_ma5: f64, // (close - MA5) / MA5 × 100
    pub bias_ma20: f64,
    /// 量能
    pub volume_ratio: f64, // 当日量 / 5日均量
    pub volume_signal: String, // "放量上涨", "缩量回调", "放量下跌", "缩量上涨", "正常"
    /// 支撑/压力位 (基于近期高低点和均线)
    pub support_levels: Vec<f64>,
    pub resistance_levels: Vec<f64>,
    /// 本次计算**实际使用**的窗口（回显 `IndicatorConfig`，不是档位声称想要的那份）。
    ///
    /// 存在理由：四个 MA 命名槽与四个 RSI 命名槽是按**周期数值**认领的
    /// （`match period { 5 => …, _ => {} }`），于是「传进去的窗口」与「字段里剩下的值」
    /// 可以完全不同 —— 传 `[8,24]` 时四个 MA 槽全是收盘价，而字段名还写着 `ma60`。
    /// 本字段把实际窗口与数值命名槽解耦：消费端要按尺度读，就读这里 + `scale_trend`，
    /// 不要再猜 `maNN` 是几根 bar。判据与实证见 PLAN §九十七。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows: Option<IndicatorWindows>,
    /// 与尺度无关的**趋势形态事实**（快带 / 慢带 / 差幅 / 快带斜率）。
    ///
    /// 刻意**只出数字、不出形态标签**：`ma_alignment` 的「多头排列」是四条带比大小，
    /// 而档尺度按 `ceil(h/d)` 只有 2~5 根可分辨带 ⇒ 四槽比较在粗尺度上恒退化成「缠绕」
    /// （PLAN §九十六(1) 那条分歧的定量部分）。分类留给有尺度上下文的一侧
    /// （`ScoringEngine` 的 `ScoreBands::scaled_for` 已经按 `√d` 缩放阈值），
    /// 这里再造一个阈值就是无依据的第三套口径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_trend: Option<ScaleTrend>,
    /// 动量事实：`rsi_periods` 里**最快**那个周期的 RSI，连同周期一起给出。
    ///
    /// 为什么要有它：评分侧的 rsi 分量读的是 `rsi6` —— 那是「六周期 RSI」这个**命名槽**，
    /// 档位链传 `[8,24]` 这类周期时 `rsi6` 会被静默丢弃停在 50.0（§九十八(2) 实测：
    /// 七个评分分量里五个走命名槽）。本字段是它的尺度中立版本，角色与 `rsi6` 一一对应：
    /// 日线默认口径下 `period == 6` 且 `value == rsi6`（有测试逐值对账，不是自称一致）。
    /// 样本不够算该周期时给 `None`，评分侧按「中性 50」走既有路径 —— 那**不是**伪装的读数。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_momentum: Option<ScaleValue>,
}

/// 一个「带周期声明 + 该周期上的值」对（见 [`TechnicalIndicators::scale_momentum`]）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScaleValue {
    pub period: usize,
    pub value: f64,
}

/// `compute_indicators_with_config` 实际吃进去的窗口集合（见 [`TechnicalIndicators::windows`]）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndicatorWindows {
    pub ma_periods: Vec<usize>,
    pub macd_fast: usize,
    pub macd_slow: usize,
    pub macd_signal: usize,
    pub rsi_periods: Vec<usize>,
    pub boll_period: usize,
    pub volume_lookback: usize,
}

impl IndicatorWindows {
    fn from_cfg(cfg: &IndicatorConfig) -> Self {
        Self {
            ma_periods: cfg.ma_periods.clone(),
            macd_fast: cfg.macd_fast,
            macd_slow: cfg.macd_slow,
            macd_signal: cfg.macd_signal,
            rsi_periods: cfg.rsi_periods.clone(),
            boll_period: cfg.boll_period,
            volume_lookback: cfg.volume_lookback,
        }
    }
}

/// 两带趋势事实（见 [`TechnicalIndicators::scale_trend`]）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScaleTrend {
    /// 快带的 bar 数（= `ma_periods` 里倒数第二个）
    pub fast_bars: usize,
    /// 慢带的 bar 数（= `ma_periods` 里最后一个）
    pub slow_bars: usize,
    pub fast: f64,
    pub slow: f64,
    /// `(fast - slow) / slow × 100`，慢带非正或不可算时为 `None`（**不是 0**）
    pub diff_pct: Option<f64>,
    /// 快带相对上一根的位移（正=向上）；上一根快带不可算时为 `None`
    pub fast_slope: Option<f64>,
}

/// 指标计算参数配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndicatorConfig {
    pub ma_periods: Vec<usize>,
    pub macd_fast: usize,
    pub macd_slow: usize,
    pub macd_signal: usize,
    pub rsi_periods: Vec<usize>,
    pub boll_period: usize,
    pub boll_stddev: f64,
    pub volume_lookback: usize,
    pub volume_surge_ratio: f64,
    pub volume_shrink_ratio: f64,
    pub rsi_overbought: f64,
    pub rsi_oversold: f64,
}

impl Default for IndicatorConfig {
    fn default() -> Self {
        Self {
            ma_periods: vec![5, 10, 20, 60],
            macd_fast: 12,
            macd_slow: 26,
            macd_signal: 9,
            rsi_periods: vec![6, 12, 14, 24],
            boll_period: 20,
            boll_stddev: 2.0,
            volume_lookback: 5,
            volume_surge_ratio: 1.5,
            volume_shrink_ratio: 0.7,
            rsi_overbought: 80.0,
            rsi_oversold: 20.0,
        }
    }
}

// ── 档位窗口计划 → 指标窗口（#41 片 A，PLAN §一○二(4)）──
//
// 为什么要有这条映射：`ScaleWindowPlan` 已经按持有期把「几根 bar」全套算好了
// （中带/长带/MACD/RSI/BOLL/量能回看），而 `IndicatorConfig` 是
// `compute_indicators_with_config` 的唯一入口。中间再写一份数字就是第三处权威。
//
// ⚠ **只搬窗口，不搬阈值**：`boll_stddev` / `volume_surge_ratio` / `volume_shrink_ratio` /
// `rsi_overbought` / `rsi_oversold` 一律沿用日线默认 —— §九十六(2) 已裁定「阈值类不随尺度缩」
// （RSI 本身是 0-100 的有界统计量，与 bar 的日历跨度无关；再缩一次就是无依据的第四套口径）。
//
// ⚠ **`rsi_periods` 只给一个周期**：日线链的 `[6,12,14,24]` 是四个**命名槽**，
// 而命名槽按数值认领（§九十七），档位换算出来的周期不会恰好落进那几个数。
// 给一个周期 = 让 `windows.rsiPeriods` 与 `scaleMomentum{period,value}` 说的是同一件事，
// 而不是留三个停在 50.0 的假读数。
impl From<&crate::scale::ScaleWindowPlan> for IndicatorConfig {
    fn from(plan: &crate::scale::ScaleWindowPlan) -> Self {
        // 快带族的下限取 2 而不是 1：MA1 就是当根收盘价，把它算作「一条带」会让
        // `windows` 里出现一个不是均线的数（粗尺度上 short_bars=2 时 `2/4` 会取到 1）。
        let mut ma_periods = vec![
            (plan.short_bars / 4).max(2),
            (plan.short_bars / 2).max(2),
            plan.short_bars,
            plan.long_bars,
        ];
        ma_periods.sort_unstable();
        ma_periods.dedup();
        Self {
            ma_periods,
            macd_fast: plan.macd_fast,
            macd_slow: plan.macd_slow,
            macd_signal: plan.macd_signal,
            rsi_periods: vec![plan.rsi_period],
            boll_period: plan.boll_period,
            volume_lookback: plan.volume_lookback,
            ..Default::default()
        }
    }
}

// ── P2-C7: 技术指标统一收口到 harness foundation 层 ──
//
// 历史上 SMA/EMA/RSI/stddev 在 astock-data、quant、market-sim、stock-analysis
// 各有重复实现，存在算法漂移风险。现已统一到 `axagent_harness::indicators`，
// 本 crate 通过 `pub use` re-export 保持外部 API 兼容（`axagent_astock_data::indicators::sma` 等）。
//
// - `sma`            → harness::indicators::sma
// - `build_ema_series` → harness::indicators::build_ema_series
// - `rsi`            → harness::indicators::rsi_wilder (别名)
// - `stddev`         → harness::indicators::stddev_sample (crate 内部别名, 原 private)
// - `ema` (单值版)   → 删除, 测试改用 build_ema_series 或 ema_last

pub use axagent_harness::indicators::rsi_wilder as rsi;
pub use axagent_harness::indicators::{build_ema_series, sma};
// crate 内部便捷别名（原 private fn，外部不应依赖）
pub(crate) use axagent_harness::indicators::stddev_sample as stddev;

/// Compute all technical indicators from K-line data with configurable parameters.
/// Pass `None` for `config` to use default parameters.
pub fn compute_indicators_with_config(
    stock_code: &str,
    klines: &[KLine],
    config: Option<&IndicatorConfig>,
) -> TechnicalIndicators {
    let default_config = IndicatorConfig::default();
    let cfg = config.unwrap_or(&default_config);

    if klines.is_empty() {
        return TechnicalIndicators {
            stock_code: stock_code.to_string(),
            latest_date: String::new(),
            ma5: 0.0,
            ma10: 0.0,
            ma20: 0.0,
            ma60: 0.0,
            ma_alignment: "无数据".to_string(),
            macd_dif: 0.0,
            macd_dea: 0.0,
            macd_bar: 0.0,
            macd_signal: "无数据".to_string(),
            rsi6: 50.0,
            rsi12: 50.0,
            rsi14: 50.0,
            rsi24: 50.0,
            rsi_signal: "无数据".to_string(),
            boll_upper: 0.0,
            boll_mid: 0.0,
            boll_lower: 0.0,
            boll_position: "无数据".to_string(),
            bias_ma5: 0.0,
            bias_ma20: 0.0,
            volume_ratio: 1.0,
            volume_signal: "无数据".to_string(),
            support_levels: vec![],
            resistance_levels: vec![],
            windows: Some(IndicatorWindows::from_cfg(cfg)),
            scale_trend: None,
            scale_momentum: None,
        };
    }
    let closes: Vec<f64> = klines.iter().map(|k| k.close).collect();
    let volumes: Vec<f64> = klines.iter().map(|k| k.volume).collect();
    let latest = klines.last();
    let latest_date = latest.map(|k| k.date.clone()).unwrap_or_default();
    let latest_close = latest.map(|k| k.close).unwrap_or(0.0);
    let latest_volume = latest.map(|k| k.volume).unwrap_or(0.0);
    let prev_close = klines.get(klines.len().saturating_sub(2)).map(|k| k.close).unwrap_or(0.0);
    let price_change = latest_close - prev_close;

    // MA — 计算配置中所有周期，按 period 值映射到命名域
    let mut ma5 = latest_close;
    let mut ma10 = latest_close;
    let mut ma20 = latest_close;
    let mut ma60 = latest_close;
    for &period in &cfg.ma_periods {
        let val = sma(&closes, period).unwrap_or(latest_close);
        match period {
            5 => ma5 = val,
            10 => ma10 = val,
            20 => ma20 = val,
            60 => ma60 = val,
            _ => {},
        }
    }

    // MA alignment
    let ma_alignment = if ma5 > ma10 && ma10 > ma20 && ma20 > ma60 {
        "多头排列".to_string()
    } else if ma5 < ma10 && ma10 < ma20 && ma20 < ma60 {
        "空头排列".to_string()
    } else if ma5 > ma10 && ma10 > ma20 {
        "弱多头".to_string()
    } else {
        "缠绕/交叉".to_string()
    };

    // MACD: 计算完整 DIF 序列后再做 EMA(signal) 得到 DEA
    let dif_series: Vec<f64> = if closes.len() >= cfg.macd_slow {
        let ema_fast_series = build_ema_series(&closes, cfg.macd_fast);
        let ema_slow_series = build_ema_series(&closes, cfg.macd_slow);
        ema_fast_series
            .iter()
            .zip(ema_slow_series.iter())
            .map(|(&e_fast, &e_slow)| e_fast - e_slow)
            .collect()
    } else {
        vec![0.0]
    };
    let dea_series = build_ema_series(&dif_series, cfg.macd_signal);
    let dif = dif_series.last().copied().unwrap_or(0.0);
    let prev_dif = if dif_series.len() >= 2 {
        dif_series[dif_series.len() - 2]
    } else {
        dif
    };
    let dea = dea_series.last().copied().unwrap_or(0.0);
    let prev_dea = if dea_series.len() >= 2 {
        dea_series[dea_series.len() - 2]
    } else {
        dea
    };
    let bar = (dif - dea) * 2.0;

    // MACD signal
    let macd_signal = if prev_dif <= prev_dea && dif > dea {
        "金叉".to_string()
    } else if prev_dif >= prev_dea && dif < dea {
        "死叉".to_string()
    } else if dif > dea {
        "多头运行".to_string()
    } else if dif < dea {
        "空头运行".to_string()
    } else {
        "缠绕".to_string()
    };

    // RSI — 计算配置中所有周期，按 period 值映射到命名域
    let mut rsi6 = 50.0;
    let mut rsi12 = 50.0;
    let mut rsi14 = 50.0;
    let mut rsi24 = 50.0;
    for &period in &cfg.rsi_periods {
        let val = rsi(&closes, period).unwrap_or(50.0);
        match period {
            6 => rsi6 = val,
            12 => rsi12 = val,
            14 => rsi14 = val,
            24 => rsi24 = val,
            _ => {},
        }
    }

    let rsi_signal = if rsi6 > cfg.rsi_overbought {
        "超买".to_string()
    } else if rsi6 < cfg.rsi_oversold {
        "超卖".to_string()
    } else if rsi6 > 60.0 {
        "强势".to_string()
    } else if rsi6 < 40.0 {
        "弱势".to_string()
    } else {
        "中性".to_string()
    };

    // Bollinger Bands — 取最近 boll_period 根K线计算
    let boll_mid = sma(&closes, cfg.boll_period).unwrap_or(latest_close);
    let boll_std = if closes.len() >= cfg.boll_period {
        stddev(&closes[closes.len() - cfg.boll_period..], boll_mid)
    } else if !closes.is_empty() {
        stddev(&closes, boll_mid)
    } else {
        0.0
    };
    let boll_upper = boll_mid + cfg.boll_stddev * boll_std;
    let boll_lower = boll_mid - cfg.boll_stddev * boll_std;

    let half_std = boll_std * 0.5;
    let boll_position = if latest_close > boll_upper {
        "上轨以上".to_string()
    } else if latest_close > boll_mid + half_std {
        "上轨区间".to_string()
    } else if latest_close >= boll_mid - half_std {
        "中轨附近".to_string()
    } else if latest_close > boll_lower {
        "下轨区间".to_string()
    } else {
        "下轨以下".to_string()
    };

    // Bias (deviation rate)
    let bias_ma5 = if ma5 > 0.0 {
        ((latest_close - ma5) / ma5) * 100.0
    } else {
        0.0
    };
    let bias_ma20 = if ma20 > 0.0 {
        ((latest_close - ma20) / ma20) * 100.0
    } else {
        0.0
    };

    // Volume ratio — 取最近 volume_lookback 日均量
    let avg_vol = if volumes.len() > cfg.volume_lookback {
        volumes[volumes.len() - cfg.volume_lookback - 1..volumes.len() - 1].iter().sum::<f64>()
            / cfg.volume_lookback as f64
    } else if volumes.len() >= 2 {
        volumes[..volumes.len() - 1].iter().sum::<f64>() / (volumes.len() - 1) as f64
    } else {
        latest_volume
    };
    let volume_ratio = if avg_vol > 0.0 {
        latest_volume / avg_vol
    } else {
        1.0
    };

    let volume_signal = if volume_ratio > cfg.volume_surge_ratio && price_change > 0.0 {
        "放量上涨".to_string()
    } else if volume_ratio < cfg.volume_shrink_ratio && price_change < 0.0 {
        "缩量回调".to_string()
    } else if volume_ratio > cfg.volume_surge_ratio && price_change < 0.0 {
        "放量下跌".to_string()
    } else if volume_ratio < cfg.volume_shrink_ratio && price_change > 0.0 {
        "缩量上涨".to_string()
    } else {
        "正常".to_string()
    };

    // Support/Resistance from MAs and Bollinger
    let mut support_levels = vec![ma5.min(ma10).min(ma20), ma20.min(ma60)];
    support_levels.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    support_levels.dedup_by(|a, b| (*a - *b).abs() < 0.01);
    let mut resistance_levels = vec![ma5.max(ma10).max(ma20), boll_upper];
    resistance_levels.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    resistance_levels.dedup_by(|a, b| (*a - *b).abs() < 0.01);

    TechnicalIndicators {
        stock_code: stock_code.to_string(),
        latest_date,
        ma5,
        ma10,
        ma20,
        ma60,
        ma_alignment,
        macd_dif: dif,
        macd_dea: dea,
        macd_bar: bar,
        macd_signal,
        rsi6,
        rsi12,
        rsi14,
        rsi24,
        rsi_signal,
        boll_upper,
        boll_mid,
        boll_lower,
        boll_position,
        bias_ma5,
        bias_ma20,
        volume_ratio,
        volume_signal,
        support_levels,
        resistance_levels,
        windows: Some(IndicatorWindows::from_cfg(cfg)),
        scale_trend: scale_trend_of(&closes, &cfg.ma_periods),
        scale_momentum: scale_momentum_of(&closes, &cfg.rsi_periods),
    }
}

/// 从实际使用的 MA 周期里取**最后两根带**算趋势事实（见 [`ScaleTrend`]）。
///
/// 取法说明：`ma_periods` 排序去重后取倒数第二 / 倒数第一 —— 日线默认 `[5,10,20,60]` 得到
/// `(20, 60)`，正是既有「中带 / 长带」那一对；档位 `[8,24]` 得到 `(8, 24)`。
/// 这样两代口径共用同一条取法，不需要谁特化。
///
/// 三条缺席路径全部返回 `None`（不给 0、不给「缠绕」）：少于两根可用带、
/// 样本不够算出快带或慢带、上一根快带算不出来（`fast_slope` 单独为 `None`）。
fn scale_trend_of(closes: &[f64], ma_periods: &[usize]) -> Option<ScaleTrend> {
    let mut bands: Vec<usize> = ma_periods.iter().copied().filter(|p| *p >= 1).collect();
    bands.sort_unstable();
    bands.dedup();
    if bands.len() < 2 {
        return None;
    }
    let fast_bars = bands[bands.len() - 2];
    let slow_bars = bands[bands.len() - 1];
    let fast = sma(closes, fast_bars)?;
    let slow = sma(closes, slow_bars)?;
    let diff_pct = if slow > 0.0 {
        Some((fast - slow) / slow * 100.0)
    } else {
        None
    };
    let fast_slope = if closes.len() >= 2 {
        sma(&closes[..closes.len() - 1], fast_bars).map(|prev| fast - prev)
    } else {
        None
    };
    Some(ScaleTrend { fast_bars, slow_bars, fast, slow, diff_pct, fast_slope })
}

/// 取 `rsi_periods` 里**最快**的那个周期算 RSI（角色对应命名槽 `rsi6`，见
/// [`TechnicalIndicators::scale_momentum`]）。周期列表为空或样本不够 ⇒ `None`（不给 50 冒充）。
fn scale_momentum_of(closes: &[f64], rsi_periods: &[usize]) -> Option<ScaleValue> {
    let period = rsi_periods.iter().copied().filter(|p| *p >= 2).min()?;
    let value = rsi(closes, period)?;
    Some(ScaleValue { period, value })
}

/// Compute all technical indicators from K-line data (使用默认参数)
pub fn compute_indicators(stock_code: &str, klines: &[KLine]) -> TechnicalIndicators {
    compute_indicators_with_config(stock_code, klines, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_kline(date: &str, open: f64, high: f64, low: f64, close: f64, volume: f64) -> KLine {
        KLine {
            date: date.to_string(),
            open,
            high,
            low,
            close,
            volume,
            amount: volume * close,
            turnover_rate: None,
            adj_factor: None,
        }
    }

    /// 上升序列夹具：每根 +0.1、收盘从 10.0 起 ⇒ MA20 比收盘低约 0.95、MA60 低约 2.95。
    /// 断言用这两个差值时**留边界**（0.5 / 1.5），不写贴着边界的绝对阈值。
    fn rising_klines(n: usize) -> Vec<KLine> {
        (0..n)
            .map(|i| {
                let c = 10.0 + i as f64 * 0.1;
                make_kline(&format!("2025-{:02}-01", i / 12 + 1), c, c + 0.1, c - 0.1, c, 1000.0)
            })
            .collect()
    }

    /// **现状锁（不是判据）**：给 `IndicatorConfig` 传非命名槽周期的 MA/RSI ⇒ 值被**静默丢弃**。
    ///
    /// 为什么要锁：片 2 第二步要把 `ScaleWindowPlan` 换算出的周期（如超短档 `[8,24]`）接进指标。
    /// 直接接的后果是这里的写法决定的 —— 四个 MA 槽按**周期数值**填
    /// （`match period { 5 => …, _ => {} }`），`8/24` 一条都不命中 ⇒ 四槽全部停在初值 `latest_close`，
    /// `ma_alignment` 恒判「缠绕/交叉」；RSI 同理（`unwrap_or(50.0)` ⇒ 伪装成「中性」）。
    /// 这就是 PLAN §九十三(1) 读数 2「能配不等于配了有用」的实证。
    /// ⇒ 2b 的契约必须是**新增与尺度无关的中性槽**，命名槽继续只服务日线链；
    ///   本条届时改判为「中性槽被填 + 命名槽未被拿去冒充档位窗口」，**不要**为了让它绿而删掉它。
    #[test]
    fn off_ladder_periods_are_silently_dropped_by_named_slots() {
        let klines: Vec<KLine> = (0..70)
            .map(|i| {
                let c = 10.0 + i as f64 * 0.1;
                make_kline(&format!("2025-{:02}-01", i / 12 + 1), c, c + 0.1, c - 0.1, c, 1000.0)
            })
            .collect();
        let latest = klines.last().unwrap().close;

        let cfg = IndicatorConfig {
            ma_periods: vec![8, 24],
            rsi_periods: vec![10],
            ..Default::default()
        };
        let ind = compute_indicators_with_config("600000", &klines, Some(&cfg));
        assert_eq!(ind.ma5, latest);
        assert_eq!(ind.ma10, latest);
        assert_eq!(ind.ma20, latest);
        assert_eq!(ind.ma60, latest);
        assert_eq!(ind.ma_alignment, "缠绕/交叉", "四槽全等于收盘价 ⇒ 形态判据恒「缠绕」");
        for r in [ind.rsi6, ind.rsi12, ind.rsi14, ind.rsi24] {
            assert_eq!(r, 50.0, "非命名槽的 RSI 周期被丢弃 ⇒ 伪装「中性」");
        }

        // 对照组：同一序列用默认周期真的命中了命名槽 ⇒ 丢弃来自写法，不是样本不足。
        // 没有这组，上面的相等就只是「数据不够」的同义反复。
        // 判据形式：序列每根 +0.1 ⇒ MA20 恰比收盘低约 0.95、MA60 低约 2.95，
        // 所以断言「明显低于收盘价」用差值，不用 `< latest - 1.0` 这种贴着边界的写法。
        let base = compute_indicators_with_config("600000", &klines, None);
        assert!(
            latest - base.ma20 > 0.5 && latest - base.ma60 > 1.5,
            "对照组应命中命名槽（ma20={} ma60={} latest={latest}）",
            base.ma20,
            base.ma60
        );
        assert!(base.rsi14 > 50.0, "对照组 RSI14 不该停在默认 50");

        // 2b 的改判（同一份输入）：命名槽被丢弃**不再等于没有事实** ——
        // `windows` 说清实际用了哪些周期，`scaleTrend` 给出两带的数值。日线默认口径下
        // 两带 = (20, 60)，与 `ma20`/`ma60` 是同一对 ⇒ 两代口径同构，不是另造一套。
        let w = ind.windows.expect("windows 必须回显实际使用的周期");
        assert_eq!(w.ma_periods, vec![8usize, 24]);
        assert_eq!(w.rsi_periods, vec![10usize]);
        let t = ind.scale_trend.expect("两带以上 ⇒ scaleTrend 必须有值");
        assert_eq!((t.fast_bars, t.slow_bars), (8, 24));
        assert!(t.fast > 0.0 && t.slow > 0.0 && (t.fast - t.slow).abs() > 1e-9);
        let bt = base.scale_trend.expect("默认周期也是两带以上");
        assert_eq!((bt.fast_bars, bt.slow_bars), (20, 60));
    }

    /// `windows` 是**回显**而不是「想要的配置」：默认路径要逐字等于 `IndicatorConfig::default()`。
    #[test]
    fn windows_echoes_the_config_actually_used() {
        let klines = rising_klines(70);
        let dflt = IndicatorConfig::default();
        let w = compute_indicators_with_config("600000", &klines, None)
            .windows
            .expect("默认路径也要回显");
        assert_eq!(w.ma_periods, dflt.ma_periods);
        assert_eq!(w.rsi_periods, dflt.rsi_periods);
        assert_eq!((w.macd_fast, w.macd_slow, w.macd_signal), (12, 26, 9));
        assert_eq!(w.boll_period, dflt.boll_period);
        assert_eq!(w.volume_lookback, dflt.volume_lookback);
    }

    /// 缺席路径一律给 `None`，不给 0、也不给「缠绕」——
    /// 「拿不到」可以接受，伪装成一个看起来正常的数是本仓最禁止的形态。
    #[test]
    fn scale_trend_absent_instead_of_fake_when_bands_unusable() {
        let klines = rising_klines(70);
        // ① 只有一根带 ⇒ 无从比较
        let one = IndicatorConfig { ma_periods: vec![20], ..Default::default() };
        assert!(compute_indicators_with_config("600000", &klines, Some(&one))
            .scale_trend
            .is_none());
        // ② 带数重复（去重后只剩一根）
        let dup = IndicatorConfig { ma_periods: vec![20, 20], ..Default::default() };
        assert!(compute_indicators_with_config("600000", &klines, Some(&dup))
            .scale_trend
            .is_none());
        // ③ 样本不够算最慢带 ⇒ 整块缺席（不是把 slow 记成 0）
        let few: Vec<KLine> = klines.iter().take(10).cloned().collect();
        let slow_only = IndicatorConfig { ma_periods: vec![5, 60], ..Default::default() };
        assert!(compute_indicators_with_config("600000", &few, Some(&slow_only))
            .scale_trend
            .is_none());
        // ④ 空输入：windows 仍回显（那是真用过的参数），scaleTrend 缺席
        let empty = compute_indicators_with_config("600000", &[], None);
        assert!(empty.scale_trend.is_none());
        assert!(empty.windows.is_some());
    }

    /// 两个新字段是 **additive**：旧键一个都不能少，新键按 camelCase 出现。
    ///
    /// 这是消费面的保险 —— Rhai / TS 读的是 `indicators.ma20` 这类旧键，
    /// 少一个键就是「字段一直发、界面从未显示」的反向版本（读不到 ⇒ 静默 null）。
    #[test]
    fn new_fields_are_additive_and_camel_case() {
        let json = serde_json::to_value(compute_indicators_with_config(
            "600000",
            &rising_klines(70),
            None,
        ))
        .expect("序列化");
        for old in [
            "ma5",
            "ma10",
            "ma20",
            "ma60",
            "maAlignment",
            "macdDif",
            "rsi6",
            "rsi14",
            "bollPosition",
            "biasMa5",
            "volumeRatio",
            "supportLevels",
            "resistanceLevels",
        ] {
            assert!(json.get(old).is_some(), "旧键 {old} 不能消失");
        }
        assert!(json.get("windows").is_some(), "缺 windows ⇒ 尺度声明没发出去");
        assert!(json.get("scaleTrend").is_some(), "缺 scaleTrend ⇒ 两带事实没发出去");
        let w = json["windows"].as_object().expect("windows 是对象");
        assert!(w.contains_key("maPeriods") && w.contains_key("bollPeriod"));
    }

    #[test]
    fn test_sma_basic() {
        let data = vec![10.0, 20.0, 30.0, 40.0, 50.0];
        assert!((sma(&data, 5).unwrap() - 30.0).abs() < 1e-6);
    }

    #[test]
    fn test_sma_takes_latest() {
        let data = vec![10.0, 20.0, 30.0, 40.0, 50.0, 60.0];
        let result = sma(&data, 3).unwrap();
        assert!((result - 50.0).abs() < 1e-6, "SMA(3) of last 3 should be 50.0, got {result}");
    }

    #[test]
    fn test_sma_insufficient_data() {
        let data = vec![10.0, 20.0];
        assert!(sma(&data, 5).is_none());
    }

    #[test]
    fn test_ema_non_empty() {
        // P2-C7: 本地 ema(单值版) 已删除, 改用 harness::build_ema_series 末值
        let data = vec![10.0, 20.0, 30.0, 40.0, 50.0];
        let series = build_ema_series(&data, 5);
        let result = series.last().copied().unwrap_or(0.0);
        assert!(result > 0.0);
    }

    #[test]
    fn test_rsi_uniform() {
        let closes = vec![10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0];
        let result = rsi(&closes, 6).unwrap();
        // 修复 L5: 零波动（完全平盘）时 RSI=50（中性），不再误报 100 超买
        assert!((result - 50.0).abs() < 1e-6);
    }

    #[test]
    fn test_rsi_all_gains() {
        let closes: Vec<f64> = (0..8).map(|i| i as f64 * 10.0).collect();
        let result = rsi(&closes, 6).unwrap();
        assert!(result > 80.0);
    }

    #[test]
    fn test_stddev_calculation() {
        let data = vec![2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let mean = data.iter().sum::<f64>() / data.len() as f64;
        let sd = stddev(&data, mean);
        assert!(sd > 0.0);
    }

    #[test]
    fn test_compute_indicators_empty() {
        let result = compute_indicators("000001", &[]);
        assert_eq!(result.stock_code, "000001");
        assert_eq!(result.ma_alignment, "无数据");
        assert_eq!(result.macd_signal, "无数据");
    }

    #[test]
    fn test_compute_indicators_basic() {
        let klines: Vec<KLine> = (0..65)
            .map(|i| {
                make_kline(
                    &format!("2025-01-{:02}", i + 1),
                    10.0,
                    10.5,
                    9.5,
                    10.0 + i as f64 * 0.1,
                    10000.0,
                )
            })
            .collect();
        let result = compute_indicators("000001", &klines);
        assert!(result.ma5 > 0.0);
        assert!(result.ma10 > 0.0);
        assert!(result.ma20 > 0.0);
        assert!(!result.macd_signal.is_empty());
        assert!(result.rsi6 >= 0.0 && result.rsi6 <= 100.0);
    }

    #[test]
    fn test_compute_indicators_ma_alignment() {
        // SMA 取最近 N 个元素，递增价格 → 最新价格高 → MA5 > MA10 > MA20 > MA60 为多头排列
        let klines: Vec<KLine> = (0..65)
            .map(|i| {
                let price = 10.0 + i as f64 * 0.5;
                make_kline(
                    &format!("2025-01-{:02}", i + 1),
                    price,
                    price + 1.0,
                    price - 1.0,
                    price,
                    10000.0,
                )
            })
            .collect();
        let result = compute_indicators("000001", &klines);
        assert!(
            result.ma_alignment == "多头排列" || result.ma_alignment == "弱多头",
            "Expected bull alignment, got: {}",
            result.ma_alignment
        );
    }

    #[test]
    fn test_macd_dea_not_equal_dif() {
        let klines: Vec<KLine> = (0..65)
            .map(|i| {
                let price = 10.0 + (i as f64 * 0.3).sin() * 2.0;
                make_kline(
                    &format!("2025-01-{:02}", i + 1),
                    price,
                    price + 0.5,
                    price - 0.5,
                    price,
                    10000.0,
                )
            })
            .collect();
        let result = compute_indicators("000001", &klines);
        assert!(
            (result.macd_dea - result.macd_dif).abs() > 0.001 || result.macd_bar.abs() > 0.001,
            "DEA should differ from DIF with sufficient data"
        );
    }

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn test_compute_indicators_with_config_custom() {
        let mut config = IndicatorConfig::default();
        config.ma_periods = vec![5, 10];
        config.rsi_periods = vec![6, 12];
        let klines: Vec<KLine> = (0..65)
            .map(|i| {
                make_kline(
                    &format!("2025-01-{:02}", i + 1),
                    10.0,
                    10.5,
                    9.5,
                    10.0 + i as f64 * 0.1,
                    10000.0,
                )
            })
            .collect();
        let result = compute_indicators_with_config("000001", &klines, Some(&config));
        assert!(result.ma5 > 0.0);
        assert!(result.ma10 > 0.0);
        assert!(!result.macd_signal.is_empty());
        assert!(result.rsi6 >= 0.0 && result.rsi6 <= 100.0);
    }

    /// 片 A 的零回归锚：日线窗口计划映射出的 `IndicatorConfig`，除 RSI 周期家族外必须与
    /// 既有默认**逐字段相等** ⇒ 「按档换算」这条通道在日线上不产生任何新数字。
    ///
    /// 顺带钉住两条边界：① 只搬窗口、不搬阈值（§九十六(2) 裁定 RSI 是有界统计量，
    /// 再缩一次就是无依据的第四套口径）；② 日线唯一被刻意改掉的是 `rsi_periods` 的
    /// **家族**（四个命名槽 → 一个周期），数值仍是 14 ⇒ 日线口径没变。
    #[test]
    fn plan_derived_config_reproduces_daily_numbers() {
        let cfg = IndicatorConfig::from(&crate::scale::ScaleWindowPlan::daily_default());
        let d = IndicatorConfig::default();
        assert_eq!(cfg.ma_periods, d.ma_periods, "日线 MA 窗口被换算改动");
        assert_eq!(
            (cfg.macd_fast, cfg.macd_slow, cfg.macd_signal),
            (d.macd_fast, d.macd_slow, d.macd_signal),
            "日线 MACD 被换算改动"
        );
        assert_eq!(cfg.boll_period, d.boll_period);
        assert_eq!(cfg.volume_lookback, d.volume_lookback);
        assert_eq!(
            (cfg.boll_stddev, cfg.volume_surge_ratio, cfg.volume_shrink_ratio),
            (d.boll_stddev, d.volume_surge_ratio, d.volume_shrink_ratio),
            "阈值类被这条映射偷偷缩了"
        );
        assert_eq!((cfg.rsi_overbought, cfg.rsi_oversold), (d.rsi_overbought, d.rsi_oversold));
        assert_eq!(
            cfg.rsi_periods,
            vec![d.rsi_periods[2]],
            "日线 RSI 周期数值不该变（只该从四槽收成一个）"
        );
    }

    /// 片 A：四档各得自己的**（尺度, 窗口）组合**，且实际使用的窗口与回显一致。
    ///
    /// 三条子判据各挡一种历史缺陷：
    ///  · 「四档实际同一份输入」（§九十二(4) 的死参数后果）⇒ 断言 **(尺度名, ma_periods)** 二元组
    ///    两两不同。**不是**只断言 ma_periods 不同 —— 首版我就写成后者，当场被真实数据否证：
    ///    中档（28 交易日 / 月线 d=20）与长档（90 / 季线 d=60）按 `ceil(h/d)` 都算出 short=2、
    ///    long=6 ⇒ 窗口根数**天然相同**。这不是回到旧缺陷：旧缺陷是「两档吃同一份日线」，
    ///    而这里两根 bar 的日历跨度不同（月线 MA6 ≈ 6 个月，季线 MA6 ≈ 6 个季）。
    ///    因此这条判据要连尺度名一起比 —— 只比窗口会把「尺度也确实相同」这一真缺陷放过去。
    ///  · 「声称按档、实际按日线」⇒ `windows` 回显实际吃进去的配置，必须等于映射产物；
    ///  · 命名槽停在 50 的假读数（§九十七：七个分量里五个走命名槽）⇒ 档位链的
    ///    `scaleMomentum.period` 必须就是该档算出来的那个周期，而不是 `rsi6` 的初值。
    #[test]
    fn plan_derived_configs_are_per_tier_and_match_actual_windows() {
        use axagent_harness::holding_period::Period;
        let mut seen: Vec<(String, Vec<usize>)> = Vec::new();
        for period in Period::ALL {
            let plan = crate::scale::ScaleWindowPlan::for_period(period);
            let cfg = IndicatorConfig::from(&plan);
            assert!(
                cfg.ma_periods.contains(&plan.short_bars)
                    && cfg.ma_periods.contains(&plan.long_bars),
                "{period:?} 的 MA 窗口没带上该档两带 {cfg:?}"
            );
            assert_eq!(cfg.rsi_periods, vec![plan.rsi_period]);
            // 不出现 MA1（当根收盘价冒充均线）与任何 <2 的带
            assert!(
                cfg.ma_periods.iter().all(|p| *p >= 2),
                "{period:?} 的窗口里有 <2 的假带：{:?}",
                cfg.ma_periods
            );
            let key = (period.scale_key().to_string(), cfg.ma_periods.clone());
            assert!(
                !seen.contains(&key),
                "{period:?} 的（尺度,窗口）与别的档重复 ⇒ 两档评分输入恒等：{seen:?}"
            );
            seen.push(key);

            let bars = plan.long_bars * 3 + 12;
            let klines: Vec<KLine> = (0..bars)
                .map(|i| {
                    make_kline(
                        &format!("2020-01-{:02}", (i % 27) + 1),
                        10.0,
                        10.6,
                        9.6,
                        10.0 + i as f64 * 0.05,
                        1000.0,
                    )
                })
                .collect();
            let ind = compute_indicators_with_config("600000", &klines, Some(&cfg));
            let w = ind.windows.expect("windows 必须回显实际使用的窗口");
            assert_eq!(w.ma_periods, cfg.ma_periods, "{period:?} 的 windows 与实际配置分叉");
            let mom = ind
                .scale_momentum
                .unwrap_or_else(|| panic!("{period:?} 样本够却没产出 scaleMomentum"));
            assert_eq!(
                mom.period, plan.rsi_period,
                "{period:?} 的中立动量周期不是该档算出来的那个"
            );
        }
        // 中/长档窗口根数相同这件事本身要**可见**（上面论证它成立的前提是尺度跨度不同）：
        // 哪天有人把某一档的尺度改了而没动窗口，这里会先红，而不是留到面板上数字分叉才发现。
        let mid = crate::scale::ScaleWindowPlan::for_period(Period::Mid);
        let long = crate::scale::ScaleWindowPlan::for_period(Period::Long);
        assert_eq!(
            IndicatorConfig::from(&mid).ma_periods,
            IndicatorConfig::from(&long).ma_periods,
            "中/长档窗口不再相同 —— 本测试的前提变了，判据与注释要一起重写"
        );
        assert_ne!(mid.scale_key, long.scale_key, "中/长档尺度相同 ⇒ 才是真正的两档恒等");
    }
}
