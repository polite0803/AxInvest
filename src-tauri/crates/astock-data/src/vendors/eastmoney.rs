use async_trait::async_trait;
use crate::error::DataError;
use crate::vendors::StockVendor;
use crate::types::*;

pub struct EastMoneyVendor;

#[async_trait]
impl StockVendor for EastMoneyVendor {
    fn name(&self) -> &'static str {
        "eastmoney"
    }

    async fn get_quote(&self, _stock_code: &str) -> Result<StockQuote, DataError> {
        Err(DataError::VendorError {
            vendor: "eastmoney".into(),
            message: "not implemented".into(),
        })
    }

    async fn get_klines(
        &self,
        _stock_code: &str,
        _period: &str,
        _limit: u32,
    ) -> Result<Vec<KLine>, DataError> {
        Err(DataError::VendorError {
            vendor: "eastmoney".into(),
            message: "not implemented".into(),
        })
    }

    async fn get_financials(
        &self,
        _stock_code: &str,
    ) -> Result<Vec<FinancialReport>, DataError> {
        Err(DataError::VendorError {
            vendor: "eastmoney".into(),
            message: "not implemented".into(),
        })
    }

    async fn get_news(&self, _stock_code: &str, _limit: u32) -> Result<Vec<NewsItem>, DataError> {
        Err(DataError::VendorError {
            vendor: "eastmoney".into(),
            message: "not implemented".into(),
        })
    }

    async fn get_money_flow(&self, _stock_code: &str) -> Result<Option<MoneyFlow>, DataError> {
        Err(DataError::VendorError {
            vendor: "eastmoney".into(),
            message: "not implemented".into(),
        })
    }

    async fn get_dragon_tiger(
        &self,
        _stock_code: &str,
    ) -> Result<Vec<DragonTigerEntry>, DataError> {
        Err(DataError::VendorError {
            vendor: "eastmoney".into(),
            message: "not implemented".into(),
        })
    }

    async fn get_lockup_schedule(
        &self,
        _stock_code: &str,
    ) -> Result<Vec<LockupSchedule>, DataError> {
        Err(DataError::VendorError {
            vendor: "eastmoney".into(),
            message: "not implemented".into(),
        })
    }

    async fn search_stock(
        &self,
        _keyword: &str,
    ) -> Result<Vec<StockSearchResult>, DataError> {
        Err(DataError::VendorError {
            vendor: "eastmoney".into(),
            message: "not implemented".into(),
        })
    }
}
