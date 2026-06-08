use crate::as_of_capability::AsOfCapability;
use crate::error::DataError;
use crate::types::*;
use crate::vendors::StockVendor;
use async_trait::async_trait;

pub struct SinaVendor {
    pub http: reqwest::Client,
}

#[async_trait]
impl StockVendor for SinaVendor {
    async fn get_quote(&self, stock_code: &str) -> Result<StockQuote, DataError> {
        let prefix = if stock_code.starts_with('6') {
            "sh"
        } else if stock_code.starts_with('8') || stock_code.starts_with('4') {
            "bj"
        } else {
            "sz"
        };
        let url = format!("https://hq.sinajs.cn/list={prefix}{stock_code}");
        let resp = self
            .http
            .get(&url)
            .header("Referer", "https://finance.sina.com.cn/")
            .send()
            .await?;
        let body = resp.text().await?;
        // 格式: var hq_str_sz000001="平安银行,12.50,12.30,12.60,12.80,..."
        let start = body
            .find('"')
            .ok_or_else(|| DataError::ParseError("sina quote parse: no opening quote".into()))?;
        let end = body[start + 1..]
            .find('"')
            .ok_or_else(|| DataError::ParseError("sina quote parse: no closing quote".into()))?;
        let data = &body[start + 1..start + 1 + end];
        let fields: Vec<&str> = data.split(',').collect();
        if fields.len() < 32 {
            return Err(DataError::ParseError(format!(
                "sina quote: expected >=32 fields, got {}",
                fields.len()
            )));
        }
        let f = |i: usize| -> f64 { fields.get(i).and_then(|s| s.parse().ok()).unwrap_or(0.0) };
        Ok(StockQuote {
            code: stock_code.to_string(),
            name: fields.first().copied().unwrap_or("").to_string(),
            price: f(3),
            pre_close: f(2),
            open: f(1),
            high: f(4),
            low: f(5),
            volume: f(8),
            amount: f(9),
            change_pct: (f(3) - f(2)) / f(2) * 100.0,
            turnover_rate: 0.0,
            pe: None,
            pb: None,
            total_mv: None,
            circulating_mv: None,
            limit_up: None,
            limit_down: None,
            is_st: false,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })
    }

    async fn get_klines(&self, _: &str, _: &str, _: u32) -> Result<Vec<KLine>, DataError> {
        Ok(vec![])
    }

    async fn get_financials(&self, _: &str) -> Result<Vec<FinancialReport>, DataError> {
        Ok(vec![])
    }

    async fn get_news(&self, stock_code: &str, limit: u32) -> Result<Vec<NewsItem>, DataError> {
        let url = format!(
            "https://vip.stock.finance.sina.com.cn/corp/go.php/vCB_AllNewsStock/symbol/{stock_code}.json?page=1&num={}",
            limit.min(50)
        );

        let resp = self
            .http
            .get(&url)
            .header("Referer", "https://finance.sina.com.cn/")
            .send()
            .await?;

        let items: Vec<serde_json::Value> = resp.json().await?;

        Ok(items
            .iter()
            .map(|item| NewsItem {
                title: item["title"].as_str().unwrap_or("").to_string(),
                summary: String::new(),
                source: "新浪财经".to_string(),
                url: format!("https://finance.sina.com.cn{}", item["url"].as_str().unwrap_or("")),
                publish_time: item["ctime"].as_str().unwrap_or("").to_string(),
                sentiment_score: None,
            })
            .collect())
    }

    async fn get_money_flow(&self, _: &str) -> Result<Option<MoneyFlow>, DataError> {
        Ok(None)
    }

    async fn get_dragon_tiger(&self, _: &str) -> Result<Vec<DragonTigerEntry>, DataError> {
        Ok(vec![])
    }

    async fn get_lockup_schedule(&self, _: &str) -> Result<Vec<LockupSchedule>, DataError> {
        Ok(vec![])
    }

    async fn search_stock(&self, _: &str) -> Result<Vec<StockSearchResult>, DataError> {
        Ok(vec![])
    }

    // ── P3:sina 能力申报 ──
    // get_quote:实时快照 → SynthesizeFromKline
    // get_news:带 publish_date,lib.rs 截断正确 → Fallthrough
    // 其他 stub:Fallthrough
    fn asof_capability(&self, method: &str) -> AsOfCapability {
        match method {
            "get_quote" => AsOfCapability::SynthesizeFromKline,
            _ => AsOfCapability::Fallthrough,
        }
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;

    fn make_vendor() -> SinaVendor {
        SinaVendor {
            http: reqwest::Client::new(),
        }
    }

    #[test]
    fn sina_quote_is_synthesize() {
        let v = make_vendor();
        assert_eq!(v.asof_capability("get_quote"), AsOfCapability::SynthesizeFromKline);
    }

    #[test]
    fn sina_others_are_fallthrough() {
        let v = make_vendor();
        for m in &["get_news", "get_klines", "get_financials", "get_money_flow", "get_dragon_tiger", "get_lockup_schedule", "search_stock"] {
            assert_eq!(v.asof_capability(m), AsOfCapability::Fallthrough);
        }
    }
}
