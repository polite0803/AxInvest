use crate::error::DataError;
use crate::types::*;
use async_trait::async_trait;

#[async_trait]
#[allow(dead_code)]
pub trait StockVendor: Send + Sync {
    fn name(&self) -> &'static str;

    async fn get_quote(&self, stock_code: &str) -> Result<StockQuote, DataError>;

    async fn get_klines(
        &self,
        stock_code: &str,
        period: &str,
        limit: u32,
    ) -> Result<Vec<KLine>, DataError>;

    async fn get_financials(&self, stock_code: &str) -> Result<Vec<FinancialReport>, DataError>;

    async fn get_news(&self, stock_code: &str, limit: u32) -> Result<Vec<NewsItem>, DataError>;

    async fn get_money_flow(&self, stock_code: &str) -> Result<Option<MoneyFlow>, DataError>;

    async fn get_dragon_tiger(&self, stock_code: &str) -> Result<Vec<DragonTigerEntry>, DataError>;

    async fn get_lockup_schedule(&self, stock_code: &str)
        -> Result<Vec<LockupSchedule>, DataError>;

    async fn search_stock(&self, keyword: &str) -> Result<Vec<StockSearchResult>, DataError>;
}

pub mod eastmoney;
pub mod sina;
pub mod tencent;
