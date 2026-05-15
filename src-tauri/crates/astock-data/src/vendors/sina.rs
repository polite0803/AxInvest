use crate::error::DataError;
use crate::types::*;
use crate::vendors::StockVendor;
use async_trait::async_trait;

pub struct SinaVendor {
    pub http: reqwest::Client,
}

#[async_trait]
impl StockVendor for SinaVendor {
    async fn get_quote(&self, _: &str) -> Result<StockQuote, DataError> {
        Err(DataError::VendorError {
            vendor: "sina".into(),
            message: "quote handled by tencent".into(),
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
}
