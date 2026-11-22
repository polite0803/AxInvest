use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SentimentSnapshot {
    pub date: String,
    pub post_count: u32,
    pub sentiment_score: f64,
    pub bull_ratio: f64,
    pub hot_rank: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SentimentTrend {
    Warming,
    Cooling,
    Stable,
    Volatile,
    Insufficient,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SentimentReport {
    pub stock_code: String,
    pub stock_name: String,
    pub avg_sentiment_7d: f64,
    pub sentiment_std_7d: f64,
    pub trend: SentimentTrend,
    pub has_sentiment_shock: bool,
    pub shock_direction: String,
    pub heat_change_pct: f64,
    pub bull_ratio_change: f64,
    pub bullish_keywords: Vec<String>,
    pub bearish_keywords: Vec<String>,
    pub verdict: String,
}

pub fn analyze_sentiment(
    stock_code: &str,
    stock_name: &str,
    history: &[SentimentSnapshot],
) -> SentimentReport {
    let n = history.len();
    if n < 2 {
        return SentimentReport {
            stock_code: stock_code.to_string(),
            stock_name: stock_name.to_string(),
            avg_sentiment_7d: history.first().map(|s| s.sentiment_score).unwrap_or(0.0),
            sentiment_std_7d: 0.0,
            trend: SentimentTrend::Insufficient,
            has_sentiment_shock: false,
            shock_direction: "none".into(),
            heat_change_pct: 0.0,
            bull_ratio_change: 0.0,
            bullish_keywords: vec![],
            bearish_keywords: vec![],
            verdict: "情绪数据不足".into(),
        };
    }

    let window = n.min(7);
    let scores: Vec<f64> = history[..window].iter().map(|s| s.sentiment_score).collect();
    let mean = scores.iter().sum::<f64>() / scores.len() as f64;
    let variance = scores.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / scores.len() as f64;
    let std = variance.sqrt();

    let latest = &history[0];
    let prev_window = if n >= 7 {
        &history[1..7]
    } else {
        &history[1..]
    };
    let prev_mean = prev_window.iter().map(|s| s.sentiment_score).sum::<f64>()
        / prev_window.len().max(1) as f64;
    let z_score = if std > 0.0 {
        (latest.sentiment_score - prev_mean) / std
    } else {
        0.0
    };
    let has_shock = z_score.abs() > 2.0;
    let shock_dir = if has_shock {
        if z_score > 0.0 {
            "positive"
        } else {
            "negative"
        }
    } else {
        "none"
    };

    let trend = if n >= 5 {
        let earliest: f64 =
            history[n.min(7) - 3..n.min(7)].iter().map(|s| s.sentiment_score).sum::<f64>() / 3.0;
        let latest_3: f64 = history[..3].iter().map(|s| s.sentiment_score).sum::<f64>() / 3.0;
        let slope = latest_3 - earliest; // positive = sentiment improving
        let volatile = std > 0.3;
        if volatile {
            SentimentTrend::Volatile
        } else if slope > 0.15 {
            SentimentTrend::Warming
        } else if slope < -0.15 {
            SentimentTrend::Cooling
        } else {
            SentimentTrend::Stable
        }
    } else {
        SentimentTrend::Insufficient
    };

    let heat_change = if n >= 14 {
        let recent_7: u32 = history[..7].iter().map(|s| s.post_count).sum();
        let prev_7: u32 = history[7..14].iter().map(|s| s.post_count).sum();
        if prev_7 > 0 {
            (recent_7 as f64 - prev_7 as f64) / prev_7 as f64 * 100.0
        } else {
            0.0
        }
    } else if n >= 2 {
        // R1-修复: 修正 heat_change 计算方向。
        //   history 为 newest-first（最新在前），history[..n/2] 是较新的一半，
        //   history[n/2..] 是较老的一半。
        //   原代码 (first_half - second_half) / second_half = (较老 - 较新) / 较新，
        //   热度上升时结果为负，与实际趋势相反。
        //   修正为 (较新 - 较老) / 较老，热度上升时结果为正。
        let recent_half: u32 = history[..n / 2].iter().map(|s| s.post_count).sum();
        let older_half: u32 = history[n / 2..].iter().map(|s| s.post_count).sum();
        if older_half > 0 {
            (recent_half as f64 - older_half as f64) / older_half as f64 * 100.0
        } else {
            0.0
        }
    } else {
        0.0
    };

    let bull_change = if n >= 2 {
        let recent_bull =
            history[..n.min(3)].iter().map(|s| s.bull_ratio).sum::<f64>() / n.min(3) as f64;
        let old_idx = history.len().saturating_sub(3);
        let old_bull = history[old_idx..].iter().map(|s| s.bull_ratio).sum::<f64>()
            / (history.len() - old_idx).max(1) as f64;
        if old_bull > 0.0 {
            (recent_bull - old_bull) / old_bull * 100.0
        } else {
            0.0
        }
    } else {
        0.0
    };

    let verdict = if has_shock && shock_dir == "negative" {
        format!("情绪骤降 (Z={:.1})", z_score)
    } else if has_shock && shock_dir == "positive" {
        format!("情绪飙升 (Z={:.1})", z_score)
    } else {
        format!("情绪{:?}, 均值{:.2}", trend, mean)
    };

    SentimentReport {
        stock_code: stock_code.to_string(),
        stock_name: stock_name.to_string(),
        avg_sentiment_7d: mean,
        sentiment_std_7d: std,
        trend,
        has_sentiment_shock: has_shock,
        shock_direction: shock_dir.into(),
        heat_change_pct: heat_change,
        bull_ratio_change: bull_change,
        bullish_keywords: vec![],
        bearish_keywords: vec![],
        verdict,
    }
}
