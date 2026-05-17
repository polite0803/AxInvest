use std::collections::HashMap;

use axagent_astock_data::calendar;
use axagent_astock_data::AStockClient;

/// 收盘复盘报告
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyReview {
    pub date: String,
    /// "交易中" | "已收盘" | "非交易日"
    pub market_status: String,
    pub watchlist_summary: Vec<StockDaySummary>,
    pub generated_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockDaySummary {
    pub stock_code: String,
    pub stock_name: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub change_pct: f64,
    /// 当日量/5日均量
    pub volume_ratio: Option<f64>,
    pub key_events: Vec<String>,
    /// 当日触发的告警描述（来自 price_alerts 表）
    pub alert_triggers: Vec<String>,
}

/// 收盘复盘工作流
pub struct PostCloseReview;

impl PostCloseReview {
    /// 生成每日复盘报告
    ///
    /// `triggered_alerts` 为 stock_code -> alert descriptions 的映射，
    /// 用于将当日触发的告警沉淀到复盘摘要中。
    pub async fn generate(
        client: &AStockClient,
        watchlist: &[(String, String)],
        triggered_alerts: &HashMap<String, Vec<String>>,
    ) -> Result<DailyReview, String> {
        let now = chrono::Utc::now();
        let today = now.format("%Y-%m-%d").to_string();
        let today_date = chrono::NaiveDate::parse_from_str(&today, "%Y-%m-%d").unwrap_or_default();

        let market_status = if calendar::is_trading_day(&today_date) {
            if calendar::is_trading_time() {
                "交易中".to_string()
            } else {
                "已收盘".to_string()
            }
        } else {
            "非交易日".to_string()
        };

        let mut summaries = Vec::new();
        for (code, name) in watchlist {
            let quote = match client.get_quote(code).await {
                Ok(q) => q,
                Err(_) => continue,
            };

            let klines = client
                .get_klines(code, "daily", 6)
                .await
                .ok()
                .unwrap_or_default();
            let vol_ratio = if klines.len() >= 6 {
                let avg_vol_5 = klines.iter().rev().take(5).map(|k| k.volume).sum::<f64>() / 5.0;
                Some(klines.last().map(|k| k.volume).unwrap_or(0.0) / avg_vol_5)
            } else {
                None
            };

            let mut key_events = Vec::new();
            if quote.change_pct.abs() > 5.0 {
                key_events.push(format!("异常波动 {:.2}%", quote.change_pct));
            }
            if let Some(vr) = vol_ratio {
                if vr > 2.0 {
                    key_events.push(format!("放量 {:.1}x", vr));
                }
                if vr < 0.5 {
                    key_events.push("极度缩量".to_string());
                }
            }
            if quote.is_st {
                key_events.push("ST股票".to_string());
            }

            // 合并当日触发的告警
            let stock_alerts = triggered_alerts.get(code).cloned().unwrap_or_default();

            summaries.push(StockDaySummary {
                stock_code: code.clone(),
                stock_name: name.clone(),
                open: quote.open,
                high: quote.high,
                low: quote.low,
                close: quote.price,
                change_pct: quote.change_pct,
                volume_ratio: vol_ratio,
                key_events,
                alert_triggers: stock_alerts,
            });
        }

        Ok(DailyReview {
            date: today,
            market_status,
            watchlist_summary: summaries,
            generated_at: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        })
    }
}
