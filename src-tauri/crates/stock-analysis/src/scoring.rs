use axagent_astock_data::indicators::TechnicalIndicators;
use serde::{Deserialize, Serialize};

/// 100分制客观评分
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectiveScore {
    pub total: u32,           // 综合评分 0-100
    pub trend_score: u32,     // 趋势 0-30
    pub deviation_score: u32, // 乖离率 0-20
    pub macd_score: u32,      // MACD 0-15
    pub volume_score: u32,    // 量能 0-15
    pub rsi_score: u32,       // RSI 0-10
    pub support_score: u32,   // 支撑 0-10
    pub signal: String, // "🟢强烈买入" | "🔵买入" | "🟡持有" | "⚪观望" | "🟠卖出" | "🔴强烈卖出"
    pub signal_code: String, // strong_buy | buy | hold | watch | sell | strong_sell
}

/// 100分评分引擎
pub struct ScoringEngine;

impl ScoringEngine {
    /// 从技术指标计算客观评分
    pub fn score(indicators: &TechnicalIndicators, latest_price: f64) -> ObjectiveScore {
        let trend = Self::score_trend(&indicators.ma_alignment);
        let deviation = Self::score_deviation(indicators.bias_ma5);
        let macd = Self::score_macd(&indicators.macd_signal, indicators.macd_dif);
        let volume = Self::score_volume(&indicators.volume_signal);
        let rsi = Self::score_rsi(indicators.rsi6);
        let support = Self::score_support(latest_price, &indicators.support_levels);

        let total = trend + deviation + macd + volume + rsi + support;

        let (signal, signal_code) = Self::map_signal(total, &indicators.ma_alignment);

        ObjectiveScore {
            total,
            trend_score: trend,
            deviation_score: deviation,
            macd_score: macd,
            volume_score: volume,
            rsi_score: rsi,
            support_score: support,
            signal: signal.to_string(),
            signal_code: signal_code.to_string(),
        }
    }

    /// 趋势评分 (满分30)
    fn score_trend(alignment: &str) -> u32 {
        match alignment {
            "多头排列" => 30,
            "弱多头" => 20,
            "缠绕/交叉" => 12,
            "空头排列" => 0,
            _ => 12,
        }
    }

    /// 乖离率评分 (满分20) -- 略高于MA5最佳，远超MA5(>5%)最差
    fn score_deviation(bias_ma5: f64) -> u32 {
        let abs_bias = bias_ma5.abs();
        if bias_ma5 > 0.0 && abs_bias < 1.0 {
            return 20;
        }
        if abs_bias < 2.0 {
            return 18;
        }
        if abs_bias < 3.0 {
            return 15;
        }
        if abs_bias < 5.0 {
            return 10;
        }
        if abs_bias < 8.0 {
            return 5;
        }
        0
    }

    /// MACD评分 (满分15)
    fn score_macd(signal: &str, dif: f64) -> u32 {
        match signal {
            "金叉" if dif > 0.0 => 15,
            "金叉" => 10,
            "多头运行" => 12,
            "空头运行" => 3,
            "死叉" => 0,
            _ => 6,
        }
    }

    /// 量能评分 (满分15) -- 缩量回调最佳
    fn score_volume(signal: &str) -> u32 {
        match signal {
            "缩量回调" => 15,
            "缩量上涨" => 10,
            "放量上涨" => 8,
            "正常" => 7,
            "放量下跌" => 0,
            _ => 7,
        }
    }

    /// RSI评分 (满分10) -- 超卖反弹最佳，超买最差
    fn score_rsi(rsi6: f64) -> u32 {
        if rsi6 < 20.0 {
            return 10;
        }
        if rsi6 < 30.0 {
            return 8;
        }
        if rsi6 < 40.0 {
            return 6;
        }
        if rsi6 <= 60.0 {
            return 5;
        }
        if rsi6 < 70.0 {
            return 3;
        }
        if rsi6 < 80.0 {
            return 1;
        }
        0
    }

    /// 支撑评分 (满分10) -- 同时受MA5和MA10双重支撑最佳
    fn score_support(price: f64, supports: &[f64]) -> u32 {
        let near_support = supports
            .iter()
            .filter(|&&s| price > s && (price - s) / price < 0.03)
            .count();
        match near_support {
            2.. => 10,
            1 => 6,
            _ => 2,
        }
    }

    /// 评分到信号映射
    fn map_signal(total: u32, alignment: &str) -> (&str, &str) {
        if total >= 75 && alignment == "多头排列" {
            ("🟢强烈买入", "strong_buy")
        } else if total >= 60 {
            ("🔵买入", "buy")
        } else if total >= 45 {
            ("🟡持有", "hold")
        } else if total >= 30 {
            ("⚪观望", "watch")
        } else if alignment == "空头排列" && total < 30 {
            ("🔴强烈卖出", "strong_sell")
        } else {
            ("🟠卖出", "sell")
        }
    }
}
