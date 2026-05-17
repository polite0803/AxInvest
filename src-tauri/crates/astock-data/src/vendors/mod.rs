use crate::error::DataError;
use crate::types::*;
use async_trait::async_trait;

#[async_trait]
pub trait StockVendor: Send + Sync {
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

    /// 获取融资融券数据
    async fn get_margin_data(&self, stock_code: &str) -> Result<Option<MarginData>, DataError> {
        let _ = stock_code;
        Ok(None)
    }

    /// 获取北向资金持仓
    async fn get_north_bound_holding(
        &self,
        stock_code: &str,
    ) -> Result<Option<NorthBoundHolding>, DataError> {
        let _ = stock_code;
        Ok(None)
    }

    /// 获取行业分类
    async fn get_sector_info(&self, stock_code: &str) -> Result<Option<SectorInfo>, DataError> {
        let _ = stock_code;
        Ok(None)
    }

    /// 获取股东增减持
    async fn get_shareholder_trades(
        &self,
        stock_code: &str,
    ) -> Result<Vec<ShareholderTrade>, DataError> {
        let _ = stock_code;
        Ok(vec![])
    }

    /// 获取除权除息记录
    async fn get_dividend_records(
        &self,
        stock_code: &str,
    ) -> Result<Vec<DividendRecord>, DataError> {
        let _ = stock_code;
        Ok(vec![])
    }

    async fn get_research_reports(
        &self,
        stock_code: &str,
    ) -> Result<Vec<ResearchReport>, DataError> {
        let _ = stock_code;
        Ok(vec![])
    }

    async fn get_consensus_eps(&self, stock_code: &str) -> Result<Option<ConsensusEPS>, DataError> {
        let _ = stock_code;
        Ok(None)
    }

    async fn get_concept_blocks(
        &self,
        stock_code: &str,
    ) -> Result<Option<ConceptBlocks>, DataError> {
        let _ = stock_code;
        Ok(None)
    }

    async fn get_announcements(&self, stock_code: &str) -> Result<Vec<Announcement>, DataError> {
        let _ = stock_code;
        Ok(vec![])
    }

    async fn get_market_dragon_tiger(&self) -> Result<Vec<MarketDragonTiger>, DataError> {
        Ok(vec![])
    }

    async fn get_hot_stocks(&self) -> Result<Vec<HotStock>, DataError> {
        Ok(vec![])
    }

    async fn get_industry_ranking(&self) -> Result<Vec<IndustryRank>, DataError> {
        Ok(vec![])
    }

    async fn get_cls_flash(&self) -> Result<Vec<ClsFlashItem>, DataError> {
        Ok(vec![])
    }

    async fn get_north_bound_flow(&self) -> Result<Option<NorthBoundFlow>, DataError> {
        Ok(None)
    }
}

pub mod akshare;
pub mod baidu_stock;
pub mod cninfo;
pub mod eastmoney;
pub mod iwencai;
pub mod mootdx;
pub mod sina;
pub mod tencent;
pub mod ths;
