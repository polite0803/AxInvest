use crate::types::KLine;
use serde::{Deserialize, Serialize};

/// 技术指标计算结果
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// RSI (6/12/24)
    pub rsi6: f64,
    pub rsi12: f64,
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
}

/// Compute SMA (Simple Moving Average)
fn sma(data: &[f64], period: usize) -> Option<f64> {
    if data.len() < period {
        return None;
    }
    Some(data.iter().take(period).sum::<f64>() / period as f64)
}

/// Compute EMA for MACD
fn ema(data: &[f64], period: usize) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let multiplier = 2.0 / (period as f64 + 1.0);
    let mut result = data[0];
    for &val in &data[1..] {
        result = (val - result) * multiplier + result;
    }
    result
}

/// Compute RSI (Wilder's method)
fn rsi(closes: &[f64], period: usize) -> f64 {
    if closes.len() < period + 1 {
        return 50.0;
    }
    let mut gains = 0.0;
    let mut losses = 0.0;
    for i in 1..=period {
        let diff = closes[i] - closes[i - 1];
        if diff > 0.0 {
            gains += diff;
        } else {
            losses += -diff;
        }
    }
    let avg_gain = gains / period as f64;
    let avg_loss = losses / period as f64;
    if avg_loss == 0.0 {
        return 100.0;
    }
    let rs = avg_gain / avg_loss;
    100.0 - (100.0 / (1.0 + rs))
}

/// Compute standard deviation for Bollinger Bands
fn stddev(data: &[f64], mean: f64) -> f64 {
    let n = data.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    let variance = data.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
    variance.sqrt()
}

/// Compute all technical indicators from K-line data
pub fn compute_indicators(stock_code: &str, klines: &[KLine]) -> TechnicalIndicators {
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
        };
    }
    let closes: Vec<f64> = klines.iter().map(|k| k.close).collect();
    let volumes: Vec<f64> = klines.iter().map(|k| k.volume).collect();
    let latest = klines.last();
    let latest_date = latest.map(|k| k.date.clone()).unwrap_or_default();
    let latest_close = latest.map(|k| k.close).unwrap_or(0.0);
    let latest_volume = latest.map(|k| k.volume).unwrap_or(0.0);
    let prev_close = klines
        .get(klines.len().saturating_sub(2))
        .map(|k| k.close)
        .unwrap_or(0.0);
    let price_change = latest_close - prev_close;

    // MA
    let ma5 = sma(&closes, 5).unwrap_or(latest_close);
    let ma10 = sma(&closes, 10).unwrap_or(latest_close);
    let ma20 = sma(&closes, 20).unwrap_or(latest_close);
    let ma60 = sma(&closes, 60).unwrap_or(latest_close);

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

    // MACD
    let ema12 = ema(&closes, 12);
    let ema26 = ema(&closes, 26);
    let dif = ema12 - ema26;
    // DEA 是对 DIF 序列做 EMA(9) 的简化：用当前 DIF 值做近似
    let dea_slice = vec![dif];
    let dea = ema(&dea_slice, 9);
    let bar = (dif - dea) * 2.0;

    // MACD signal
    let macd_signal = if dif > dea && dif > 0.0 {
        "多头运行".to_string()
    } else if dif > dea {
        "金叉".to_string()
    } else if dif < dea && dif < 0.0 {
        "空头运行".to_string()
    } else {
        "死叉".to_string()
    };

    // RSI
    let rsi6 = rsi(&closes, 6);
    let rsi12 = rsi(&closes, 12);
    let rsi24 = rsi(&closes, 24);

    let rsi_signal = if rsi6 > 80.0 {
        "超买".to_string()
    } else if rsi6 < 20.0 {
        "超卖".to_string()
    } else if rsi6 > 60.0 {
        "强势".to_string()
    } else if rsi6 < 40.0 {
        "弱势".to_string()
    } else {
        "中性".to_string()
    };

    // Bollinger Bands (20,2)
    let boll_mid = ma20;
    let boll_std = stddev(&closes.iter().take(20).copied().collect::<Vec<_>>(), boll_mid);
    let boll_upper = boll_mid + 2.0 * boll_std;
    let boll_lower = boll_mid - 2.0 * boll_std;

    let boll_position = if latest_close > boll_upper {
        "上轨以上".to_string()
    } else if latest_close > boll_mid {
        "上轨区间".to_string()
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

    // Volume ratio
    let avg_vol_5 = if volumes.len() >= 5 {
        volumes.iter().take(5).sum::<f64>() / 5.0
    } else {
        volumes.iter().sum::<f64>() / volumes.len().max(1) as f64
    };
    let volume_ratio = if avg_vol_5 > 0.0 {
        latest_volume / avg_vol_5
    } else {
        1.0
    };

    let volume_signal = if volume_ratio > 1.5 && price_change > 0.0 {
        "放量上涨".to_string()
    } else if volume_ratio < 0.7 && price_change < 0.0 {
        "缩量回调".to_string()
    } else if volume_ratio > 1.5 && price_change < 0.0 {
        "放量下跌".to_string()
    } else if volume_ratio < 0.7 && price_change > 0.0 {
        "缩量上涨".to_string()
    } else {
        "正常".to_string()
    };

    // Support/Resistance from MAs and Bollinger
    let support_levels = vec![ma5.min(ma10).min(ma20), ma20.min(ma60)];
    let resistance_levels = vec![ma5.max(ma10).max(ma20), boll_upper];

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
    }
}
