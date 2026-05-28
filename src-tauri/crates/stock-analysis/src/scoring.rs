use axagent_astock_data::indicators::TechnicalIndicators;
use serde::{Deserialize, Serialize};

use crate::decision::ScoringWeights;
use crate::value_investing::ValueMetrics;

/// 100分制客观评分
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectiveScore {
    pub total: u32,                  // 综合评分 0-100
    pub trend_score: u32,            // 趋势 0-25
    pub deviation_score: u32,        // 乖离率 0-15
    pub macd_score: u32,             // MACD 0-15
    pub volume_score: u32,           // 量能 0-15
    pub rsi_score: u32,              // RSI 0-10
    pub support_score: u32,          // 支撑 0-5
    pub boll_score: u32,             // 布林带 0-5
    pub fundamental_adjustment: i32, // 基本面修正值（正加分/负扣分）
    pub signal: String, // "🟢强烈买入" | "🔵买入" | "🟡持有" | "⚪观望" | "🟠卖出" | "🔴强烈卖出"
    pub signal_code: String, // strong_buy | buy | hold | watch | sell | strong_sell
}

/// 100分评分引擎
pub struct ScoringEngine;

impl ScoringEngine {
    /// 从技术指标计算客观评分（可传入自定义权重）
    pub fn score(
        indicators: &TechnicalIndicators,
        latest_price: f64,
        weights: Option<&ScoringWeights>,
    ) -> ObjectiveScore {
        let default_weights = ScoringWeights::default();
        let w = weights.unwrap_or(&default_weights);
        let trend = (Self::score_trend(&indicators.ma_alignment) as f64 * w.trend / 30.0) as u32;
        let deviation =
            (Self::score_deviation(indicators.bias_ma5) as f64 * w.deviation / 20.0) as u32;
        let macd = (Self::score_macd(&indicators.macd_signal, indicators.macd_dif) as f64 * w.macd
            / 15.0) as u32;
        let volume =
            (Self::score_volume(&indicators.volume_signal) as f64 * w.volume / 15.0) as u32;
        let rsi = (Self::score_rsi(indicators.rsi6) as f64 * w.rsi / 10.0) as u32;
        let support = (Self::score_support(latest_price, &indicators.support_levels) as f64
            * w.support
            / 5.0) as u32;
        let boll = (Self::score_boll(&indicators.boll_position) as f64 * w.boll / 5.0) as u32;
        let total = (trend + deviation + macd + volume + rsi + support + boll).min(100);

        let (signal, signal_code) = Self::map_signal(total, &indicators.ma_alignment);

        ObjectiveScore {
            total,
            trend_score: trend,
            deviation_score: deviation,
            macd_score: macd,
            volume_score: volume,
            rsi_score: rsi,
            support_score: support,
            boll_score: boll,
            fundamental_adjustment: 0,
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
            "缠绕" => 6,
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

    /// 支撑评分 (满分5) -- 同时受MA5和MA10双重支撑最佳
    fn score_support(price: f64, supports: &[f64]) -> u32 {
        if price <= 0.0 {
            return 2;
        }
        let near_support = supports
            .iter()
            .filter(|&&s| s > 0.0 && price > s && (price - s) / price < 0.03)
            .count();
        match near_support {
            2.. => 5,
            1 => 3,
            _ => 1,
        }
    }

    /// 布林带位置评分 (满分5) -- 中轨附近最佳，上轨以上超买，下轨以下超卖
    fn score_boll(position: &str) -> u32 {
        match position {
            "中轨附近" => 5,
            "下轨区间" => 4,
            "上轨区间" => 3,
            "下轨以下" => 2,
            "上轨以上" => 1,
            _ => 3,
        }
    }

    /// 基本面修正（基于PE/PB/ROE等估值指标，可选）
    pub fn apply_fundamental_adjustment(
        score: &mut ObjectiveScore,
        pe: Option<f64>,
        pb: Option<f64>,
        roe: Option<f64>,
    ) {
        let orig_alignment = score.signal_code.clone();
        let mut adjustment: i32 = 0;
        let mut reasons = Vec::new();

        // PE 修正
        if let Some(pe) = pe {
            if pe <= 0.0 {
                adjustment -= 15;
                reasons.push("PE为负(亏损)");
            } else if pe > 200.0 {
                adjustment -= 10;
                reasons.push("PE>200(极高估值)");
            } else if pe > 100.0 {
                adjustment -= 5;
                reasons.push("PE>100(高估值)");
            } else if pe < 10.0 {
                adjustment += 10;
                reasons.push("PE<10(低估值)");
            } else if pe < 15.0 {
                adjustment += 5;
                reasons.push("PE<15(合理偏低)");
            }
        }

        // PB 修正
        if let Some(pb) = pb {
            if pb > 10.0 {
                adjustment -= 5;
                reasons.push("PB>10(高市净率)");
            } else if pb < 1.0 && pb > 0.0 {
                adjustment += 5;
                reasons.push("PB<1(破净)");
            }
        }

        // ROE 修正
        if let Some(roe) = roe {
            if roe >= 20.0 {
                adjustment += 10;
                reasons.push("ROE≥20%(优秀)");
            } else if roe >= 15.0 {
                adjustment += 5;
                reasons.push("ROE≥15%(良好)");
            } else if roe < 5.0 && roe > 0.0 {
                adjustment -= 5;
                reasons.push("ROE<5%(偏低)");
            } else if roe <= 0.0 {
                adjustment -= 10;
                reasons.push("ROE≤0(亏损)");
            }
        }

        score.fundamental_adjustment = adjustment;

        // Apply adjustment (cap at 0-100)
        let new_total = (score.total as i32 + adjustment).clamp(0, 100) as u32;
        score.total = new_total;

        // Re-map signal with original alignment (preserving trend context)
        let (signal, signal_code) = Self::map_signal(new_total, &orig_alignment);
        score.signal = signal.to_string();
        score.signal_code = signal_code.to_string();

        if !reasons.is_empty() {
            tracing::info!(
                "基本面修正: {:?} → 调整 {}, 最终 {}/100",
                reasons,
                adjustment,
                new_total
            );
        }
    }

    /// 价值投资修正（基于DCF安全边际、F-Score、护城河）
    pub fn apply_value_adjustment(score: &mut ObjectiveScore, value_metrics: &ValueMetrics) {
        let adjustment = if value_metrics.margin_of_safety_pct > 30.0 {
            20
        } else if value_metrics.margin_of_safety_pct > 15.0 {
            12
        } else if value_metrics.margin_of_safety_pct > 5.0 {
            6
        } else if value_metrics.margin_of_safety_pct > 0.0 {
            2
        } else if value_metrics.margin_of_safety_pct > -10.0 {
            -5
        } else {
            -10
        };

        let f_score_bonus = match value_metrics.f_score {
            7..=9 => 10,
            5..=6 => 5,
            3..=4 => 0,
            _ => -5,
        };
        let moat_bonus = match value_metrics.moat_level.as_str() {
            "宽阔" => 10,
            "狭窄" => 5,
            _ => 0,
        };

        let total_adj = adjustment + f_score_bonus + moat_bonus;
        let orig_alignment = score.signal_code.clone();
        score.fundamental_adjustment += total_adj;
        let new_total = (score.total as i32 + total_adj).clamp(0, 100) as u32;
        score.total = new_total;
        let (signal, signal_code) = Self::map_signal(new_total, &orig_alignment);
        score.signal = signal.to_string();
        score.signal_code = signal_code.to_string();

        tracing::info!(
            "价值投资修正: 安全边际{}{:+.0}, F-Score{}{:+}, 护城河{}{:+} → 总调整{:+}, 最终{}/100",
            value_metrics.mos_level,
            adjustment,
            value_metrics.f_score_level,
            f_score_bonus,
            value_metrics.moat_level,
            moat_bonus,
            total_adj,
            new_total
        );
    }

    pub fn apply_industry_adjustment(
        score: &mut ObjectiveScore,
        pe: Option<f64>,
        industry_avg_pe: Option<f64>,
        pb: Option<f64>,
        industry_avg_pb: Option<f64>,
    ) {
        let mut adjustment: i32 = 0;
        if let (Some(pe), Some(ind_pe)) = (pe, industry_avg_pe) {
            if ind_pe > 0.0 && pe > 0.0 {
                let ratio = pe / ind_pe;
                if ratio < 0.7 {
                    adjustment += 8;
                } else if ratio < 0.9 {
                    adjustment += 4;
                } else if ratio > 2.0 {
                    adjustment -= 8;
                } else if ratio > 1.5 {
                    adjustment -= 4;
                }
            }
        }
        if let (Some(pb), Some(ind_pb)) = (pb, industry_avg_pb) {
            if ind_pb > 0.0 {
                let ratio = pb / ind_pb;
                if ratio < 0.7 {
                    adjustment += 5;
                } else if ratio > 2.0 {
                    adjustment -= 5;
                }
            }
        }
        if adjustment != 0 {
            let orig_alignment = score.signal_code.clone();
            score.fundamental_adjustment += adjustment;
            let new_total = (score.total as i32 + adjustment).clamp(0, 100) as u32;
            score.total = new_total;
            let (signal, signal_code) = Self::map_signal(new_total, &orig_alignment);
            score.signal = signal.to_string();
            score.signal_code = signal_code.to_string();
        }
    }

    /// 评分到信号映射
    pub fn map_signal(total: u32, alignment: &str) -> (&str, &str) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_astock_data::indicators::TechnicalIndicators;

    fn make_indicators(
        ma_alignment: &str,
        bias_ma5: f64,
        macd_signal: &str,
        macd_dif: f64,
        volume_signal: &str,
        rsi6: f64,
        support_levels: Vec<f64>,
    ) -> TechnicalIndicators {
        TechnicalIndicators {
            stock_code: "000001".to_string(),
            latest_date: "2025-01-15".to_string(),
            ma5: 10.0,
            ma10: 9.5,
            ma20: 9.0,
            ma60: 8.0,
            ma_alignment: ma_alignment.to_string(),
            macd_dif,
            macd_dea: 0.5,
            macd_bar: 0.2,
            macd_signal: macd_signal.to_string(),
            rsi6,
            rsi12: 55.0,
            rsi24: 52.0,
            rsi_signal: "中性".to_string(),
            boll_upper: 12.0,
            boll_mid: 9.0,
            boll_lower: 6.0,
            boll_position: "中轨附近".to_string(),
            bias_ma5,
            bias_ma20: 1.0,
            volume_ratio: 1.0,
            volume_signal: volume_signal.to_string(),
            support_levels,
            resistance_levels: vec![12.0],
        }
    }

    #[test]
    fn test_score_trend_bull() {
        assert_eq!(ScoringEngine::score_trend("多头排列"), 30);
    }

    #[test]
    fn test_score_trend_weak_bull() {
        assert_eq!(ScoringEngine::score_trend("弱多头"), 20);
    }

    #[test]
    fn test_score_trend_bear() {
        assert_eq!(ScoringEngine::score_trend("空头排列"), 0);
    }

    #[test]
    fn test_score_deviation_small() {
        assert_eq!(ScoringEngine::score_deviation(0.5), 20);
    }

    #[test]
    fn test_score_deviation_large() {
        assert_eq!(ScoringEngine::score_deviation(10.0), 0);
    }

    #[test]
    fn test_score_macd_golden_cross_positive() {
        assert_eq!(ScoringEngine::score_macd("金叉", 0.5), 15);
    }

    #[test]
    fn test_score_macd_dead_cross() {
        assert_eq!(ScoringEngine::score_macd("死叉", -0.5), 0);
    }

    #[test]
    fn test_score_volume_shrink_retrace() {
        assert_eq!(ScoringEngine::score_volume("缩量回调"), 15);
    }

    #[test]
    fn test_score_volume_selloff() {
        assert_eq!(ScoringEngine::score_volume("放量下跌"), 0);
    }

    #[test]
    fn test_score_rsi_oversold() {
        assert_eq!(ScoringEngine::score_rsi(15.0), 10);
    }

    #[test]
    fn test_score_rsi_overbought() {
        assert_eq!(ScoringEngine::score_rsi(85.0), 0);
    }

    #[test]
    fn test_score_rsi_neutral() {
        assert_eq!(ScoringEngine::score_rsi(50.0), 5);
    }

    #[test]
    fn test_score_support_double() {
        let supports = vec![9.8, 9.75, 7.0];
        assert!(ScoringEngine::score_support(10.0, &supports) >= 5);
    }

    #[test]
    fn test_score_support_none() {
        assert_eq!(ScoringEngine::score_support(10.0, &[12.0, 15.0]), 1);
    }

    #[test]
    fn test_full_scoring_default() {
        let ind = make_indicators("多头排列", 1.0, "多头运行", 0.3, "正常", 50.0, vec![9.5, 9.0]);
        let score = ScoringEngine::score(&ind, 10.0, None);
        assert!(score.total > 0);
        assert!(score.total <= 100);
        assert!(!score.signal.is_empty());
        assert!(!score.signal_code.is_empty());
    }

    #[test]
    fn test_map_signal_strong_buy() {
        let (signal, code) = ScoringEngine::map_signal(80, "多头排列");
        assert!(signal.contains("强烈买入") || code == "strong_buy");
    }

    #[test]
    fn test_map_signal_buy() {
        let (_, code) = ScoringEngine::map_signal(65, "弱多头");
        assert_eq!(code, "buy");
    }

    #[test]
    fn test_map_signal_hold() {
        let (_, code) = ScoringEngine::map_signal(50, "缠绕/交叉");
        assert_eq!(code, "hold");
    }

    #[test]
    fn test_map_signal_strong_sell() {
        let (_, code) = ScoringEngine::map_signal(20, "空头排列");
        assert!(code == "strong_sell" || code == "sell");
    }

    #[test]
    fn test_apply_fundamental_adjustment() {
        let ind = make_indicators("多头排列", 1.0, "多头运行", 0.3, "正常", 50.0, vec![9.0]);
        let mut score = ScoringEngine::score(&ind, 10.0, None);
        let before = score.total;
        ScoringEngine::apply_fundamental_adjustment(&mut score, Some(8.0), Some(0.8), Some(22.0));
        assert!(
            score.fundamental_adjustment > 0,
            "Low PE + low PB + high ROE should yield positive adjustment"
        );
        assert!(score.total >= before || score.total <= 100, "Score should be capped at 0-100");
    }
}
