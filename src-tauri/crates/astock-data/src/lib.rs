pub mod calendar;
mod error;
pub mod indicators;
pub mod mcp_tools;
mod types;
mod vendors;

use std::collections::HashMap;
use tokio::sync::RwLock;

pub use error::DataError;
pub use types::*;
use vendors::akshare::AkshareVendor;
use vendors::baidu_stock::BaiduStockVendor;
use vendors::cninfo::CninfoVendor;
use vendors::eastmoney::EastMoneyVendor;
use vendors::iwencai::IwencaiVendor;
use vendors::mootdx::MootdxVendor;
use vendors::sina::SinaVendor;
use vendors::ths::ThsVendor;
use vendors::tencent::TencentVendor;
use vendors::StockVendor;

type VendorRef = (String, Box<dyn StockVendor>);

struct VendorRouting {
    quote: Vec<String>,
    klines: Vec<String>,
    financials: Vec<String>,
    news: Vec<String>,
    money_flow: Vec<String>,
    dragon_tiger: Vec<String>,
    lockup: Vec<String>,
    search: Vec<String>,
    margin: Vec<String>,
    north_bound: Vec<String>,
    sector: Vec<String>,
    shareholder_trades: Vec<String>,
    dividend: Vec<String>,
    research_reports: Vec<String>,
    consensus_eps: Vec<String>,
    concept_blocks: Vec<String>,
    announcements: Vec<String>,
    market_dragon_tiger: Vec<String>,
    hot_stocks: Vec<String>,
    industry_ranking: Vec<String>,
    cls_flash: Vec<String>,
    north_bound_flow: Vec<String>,
}

impl VendorRouting {
    fn default_routing() -> Self {
        Self {
            quote: vec!["tencent".into(), "mootdx".into(), "eastmoney".into()],
            klines: vec!["eastmoney".into(), "mootdx".into()],
            financials: vec!["eastmoney".into(), "akshare".into()],
            news: vec!["sina".into(), "akshare".into()],
            money_flow: vec!["eastmoney".into(), "baidu_stock".into()],
            dragon_tiger: vec!["eastmoney".into(), "baidu_stock".into()],
            lockup: vec!["eastmoney".into(), "baidu_stock".into()],
            search: vec!["eastmoney".into(), "iwencai".into(), "baidu_stock".into()],
            margin: vec!["eastmoney".into(), "baidu_stock".into()],
            north_bound: vec!["eastmoney".into(), "baidu_stock".into()],
            sector: vec!["eastmoney".into(), "ths".into(), "baidu_stock".into(), "iwencai".into()],
            shareholder_trades: vec!["eastmoney".into(), "baidu_stock".into()],
            dividend: vec!["eastmoney".into(), "baidu_stock".into()],
            research_reports: vec!["eastmoney".into(), "baidu_stock".into()],
            consensus_eps: vec!["ths".into(), "akshare".into(), "iwencai".into()],
            concept_blocks: vec!["ths".into(), "baidu_stock".into(), "iwencai".into()],
            announcements: vec!["cninfo".into()],
            market_dragon_tiger: vec!["eastmoney".into()],
            hot_stocks: vec!["ths".into(), "baidu_stock".into(), "iwencai".into()],
            industry_ranking: vec!["ths".into(), "baidu_stock".into()],
            cls_flash: vec!["eastmoney".into(), "akshare".into()],
            north_bound_flow: vec!["ths".into(), "baidu_stock".into()],
        }
    }
}

pub struct AStockClient {
    vendors: Vec<VendorRef>,
    routing: VendorRouting,
    http: reqwest::Client,
    cache: RwLock<HashMap<String, (i64, String)>>,
}

impl AStockClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        let mut client = Self {
            vendors: Vec::new(),
            routing: VendorRouting::default_routing(),
            http: http.clone(),
            cache: RwLock::new(HashMap::new()),
        };

        client.register_vendor("tencent", Box::new(TencentVendor { http: http.clone() }));
        client.register_vendor("eastmoney", Box::new(EastMoneyVendor { http: http.clone() }));
        client.register_vendor("sina", Box::new(SinaVendor { http: http.clone() }));
        client.register_vendor("ths", Box::new(ThsVendor { http: http.clone() }));
        client.register_vendor("cninfo", Box::new(CninfoVendor { http: http.clone() }));
        client.register_vendor("baidu_stock", Box::new(BaiduStockVendor { http: http.clone() }));
        client.register_vendor("iwencai", Box::new(IwencaiVendor { http: http.clone(), api_key: String::new() }));
        client.register_vendor("akshare", Box::new(AkshareVendor { http: http.clone() }));
        client.register_vendor("mootdx", Box::new(MootdxVendor::new()));

        client
    }

    pub fn register_vendor(&mut self, name: &str, vendor: Box<dyn StockVendor>) {
        self.vendors.push((name.to_string(), vendor));
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    async fn cache_get(&self, key: &str) -> Option<String> {
        let cache = self.cache.read().await;
        cache.get(key).and_then(|(expiry, val)| {
            if *expiry > chrono::Utc::now().timestamp() {
                Some(val.clone())
            } else {
                None
            }
        })
    }

    const MAX_CACHE_SIZE: usize = 1000;

    async fn cache_set(&self, key: String, value: String, ttl_secs: i64) {
        let mut cache = self.cache.write().await;
        // 容量检查：超出上限时清理过期条目
        if cache.len() >= Self::MAX_CACHE_SIZE {
            let now = chrono::Utc::now().timestamp();
            cache.retain(|_, (expiry, _)| *expiry > now);
        }
        let expiry = chrono::Utc::now().timestamp() + ttl_secs;
        cache.insert(key, (expiry, value));
    }

    fn find_vendor(&self, name: &str) -> Option<&dyn StockVendor> {
        self.vendors
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_ref())
    }

    pub async fn get_quote(&self, stock_code: &str) -> Result<StockQuote, DataError> {
        let cache_key = format!("quote:{stock_code}");
        if let Some(cached) = self.cache_get(&cache_key).await {
            if let Ok(quote) = serde_json::from_str::<StockQuote>(&cached) {
                return Ok(quote);
            }
        }

        let mut last_err = None;
        for name in &self.routing.quote {
            if let Some(vendor) = self.find_vendor(name) {
                match vendor.get_quote(stock_code).await {
                    Ok(result) => {
                        let json = serde_json::to_string(&result).unwrap_or_default();
                        self.cache_set(cache_key, json, 30).await;
                        return Ok(result);
                    },
                    Err(e) => {
                        tracing::warn!("[降级] {} 行情失败: {}", name, e);
                        last_err = Some(e);
                    },
                }
            }
        }
        Err(last_err.unwrap_or_else(|| DataError::VendorError {
            vendor: "all".into(),
            message: "所有行情数据源均不可用".into(),
        }))
    }

    pub async fn get_klines(
        &self,
        stock_code: &str,
        period: &str,
        limit: u32,
    ) -> Result<Vec<KLine>, DataError> {
        let cache_key = format!("klines:{stock_code}:{period}:{limit}");
        if let Some(cached) = self.cache_get(&cache_key).await {
            if let Ok(klines) = serde_json::from_str(&cached) {
                return Ok(klines);
            }
        }
        let mut last_err = None;
        for name in &self.routing.klines {
            if let Some(vendor) = self.find_vendor(name) {
                match vendor.get_klines(stock_code, period, limit).await {
                    Ok(result) => {
                        let json = serde_json::to_string(&result).unwrap_or_default();
                        self.cache_set(cache_key, json, 300).await;
                        return Ok(result);
                    },
                    Err(e) => {
                        tracing::warn!("[降级] {} K线失败: {}", name, e);
                        last_err = Some(e);
                    },
                }
            }
        }
        Err(last_err.unwrap_or_else(|| DataError::VendorError {
            vendor: "all".into(),
            message: "所有K线数据源均不可用".into(),
        }))
    }

    pub async fn get_financials(
        &self,
        stock_code: &str,
    ) -> Result<Vec<FinancialReport>, DataError> {
        for name in &self.routing.financials {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_financials(stock_code).await {
                    return Ok(result);
                }
            }
        }
        Ok(vec![])
    }

    pub async fn get_news(&self, stock_code: &str, limit: u32) -> Result<Vec<NewsItem>, DataError> {
        for name in &self.routing.news {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_news(stock_code, limit).await {
                    return Ok(result);
                }
            }
        }
        Ok(vec![])
    }

    pub async fn get_money_flow(&self, stock_code: &str) -> Result<Option<MoneyFlow>, DataError> {
        for name in &self.routing.money_flow {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_money_flow(stock_code).await {
                    return Ok(result);
                }
            }
        }
        Ok(None)
    }

    pub async fn get_dragon_tiger(
        &self,
        stock_code: &str,
    ) -> Result<Vec<DragonTigerEntry>, DataError> {
        for name in &self.routing.dragon_tiger {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_dragon_tiger(stock_code).await {
                    return Ok(result);
                }
            }
        }
        Ok(vec![])
    }

    pub async fn get_lockup_schedule(
        &self,
        stock_code: &str,
    ) -> Result<Vec<LockupSchedule>, DataError> {
        for name in &self.routing.lockup {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_lockup_schedule(stock_code).await {
                    return Ok(result);
                }
            }
        }
        Ok(vec![])
    }

    pub async fn search_stock(&self, keyword: &str) -> Result<Vec<StockSearchResult>, DataError> {
        for name in &self.routing.search {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.search_stock(keyword).await {
                    return Ok(result);
                }
            }
        }
        Ok(vec![])
    }

    pub async fn get_margin_data(&self, stock_code: &str) -> Result<Option<MarginData>, DataError> {
        for name in &self.routing.margin {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_margin_data(stock_code).await {
                    return Ok(result);
                }
            }
        }
        Ok(None)
    }

    pub async fn get_north_bound_holding(
        &self,
        stock_code: &str,
    ) -> Result<Option<NorthBoundHolding>, DataError> {
        for name in &self.routing.north_bound {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_north_bound_holding(stock_code).await {
                    return Ok(result);
                }
            }
        }
        Ok(None)
    }

    pub async fn get_sector_info(&self, stock_code: &str) -> Result<Option<SectorInfo>, DataError> {
        for name in &self.routing.sector {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_sector_info(stock_code).await {
                    return Ok(result);
                }
            }
        }
        Ok(None)
    }

    pub async fn get_shareholder_trades(
        &self,
        stock_code: &str,
    ) -> Result<Vec<ShareholderTrade>, DataError> {
        for name in &self.routing.shareholder_trades {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_shareholder_trades(stock_code).await {
                    return Ok(result);
                }
            }
        }
        Ok(vec![])
    }

    pub async fn get_dividend_records(
        &self,
        stock_code: &str,
    ) -> Result<Vec<DividendRecord>, DataError> {
        for name in &self.routing.dividend {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_dividend_records(stock_code).await {
                    return Ok(result);
                }
            }
        }
        Ok(vec![])
    }

    pub async fn get_research_reports(
        &self,
        stock_code: &str,
    ) -> Result<Vec<ResearchReport>, DataError> {
        for name in &self.routing.research_reports {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_research_reports(stock_code).await {
                    return Ok(result);
                }
            }
        }
        Ok(vec![])
    }

    pub async fn get_consensus_eps(
        &self,
        stock_code: &str,
    ) -> Result<Option<ConsensusEPS>, DataError> {
        for name in &self.routing.consensus_eps {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_consensus_eps(stock_code).await {
                    return Ok(result);
                }
            }
        }
        Ok(None)
    }

    pub async fn get_concept_blocks(
        &self,
        stock_code: &str,
    ) -> Result<Option<ConceptBlocks>, DataError> {
        for name in &self.routing.concept_blocks {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_concept_blocks(stock_code).await {
                    return Ok(result);
                }
            }
        }
        Ok(None)
    }

    pub async fn get_announcements(
        &self,
        stock_code: &str,
    ) -> Result<Vec<Announcement>, DataError> {
        for name in &self.routing.announcements {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_announcements(stock_code).await {
                    return Ok(result);
                }
            }
        }
        Ok(vec![])
    }

    pub async fn get_market_dragon_tiger(&self) -> Result<Vec<MarketDragonTiger>, DataError> {
        for name in &self.routing.market_dragon_tiger {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_market_dragon_tiger().await {
                    return Ok(result);
                }
            }
        }
        Ok(vec![])
    }

    pub async fn get_hot_stocks(&self) -> Result<Vec<HotStock>, DataError> {
        for name in &self.routing.hot_stocks {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_hot_stocks().await {
                    return Ok(result);
                }
            }
        }
        Ok(vec![])
    }

    pub async fn get_industry_ranking(&self) -> Result<Vec<IndustryRank>, DataError> {
        for name in &self.routing.industry_ranking {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_industry_ranking().await {
                    return Ok(result);
                }
            }
        }
        Ok(vec![])
    }

    pub async fn get_cls_flash(&self) -> Result<Vec<ClsFlashItem>, DataError> {
        for name in &self.routing.cls_flash {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_cls_flash().await {
                    return Ok(result);
                }
            }
        }
        Ok(vec![])
    }

    pub async fn get_north_bound_flow(&self) -> Result<Option<NorthBoundFlow>, DataError> {
        for name in &self.routing.north_bound_flow {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_north_bound_flow().await {
                    return Ok(result);
                }
            }
        }
        Ok(None)
    }

    pub async fn fetch_market_data(&self) -> Result<MarketRawData, DataError> {
        let (hot_r, industry_r, cls_r, mdt_r, nbf_r) = tokio::join!(
            self.get_hot_stocks(),
            self.get_industry_ranking(),
            self.get_cls_flash(),
            self.get_market_dragon_tiger(),
            self.get_north_bound_flow(),
        );

        Ok(MarketRawData {
            hot_stocks: hot_r.unwrap_or_else(|e| {
                tracing::warn!("hot_stocks failed: {e}");
                vec![]
            }),
            industry_ranking: industry_r.unwrap_or_else(|e| {
                tracing::warn!("industry_ranking failed: {e}");
                vec![]
            }),
            cls_flash: cls_r.unwrap_or_else(|e| {
                tracing::warn!("cls_flash failed: {e}");
                vec![]
            }),
            market_dragon_tiger: mdt_r.unwrap_or_else(|e| {
                tracing::warn!("market_dragon_tiger failed: {e}");
                vec![]
            }),
            north_bound_flow: nbf_r.unwrap_or_else(|e| {
                tracing::warn!("north_bound_flow failed: {e}");
                None
            }),
        })
    }

    pub async fn fetch_all(
        &self,
        stock_code: &str,
        kline_period: &str,
        kline_limit: u32,
        news_limit: u32,
    ) -> Result<StockRawData, DataError> {
        let (
            quote_r,
            klines_r,
            financials_r,
            news_r,
            money_flow_r,
            dragon_tiger_r,
            lockup_r,
            margin_r,
            north_bound_r,
            sector_r,
            shareholder_r,
            dividend_r,
            research_r,
            consensus_r,
            concept_r,
            announcements_r,
        ) = tokio::join!(
            self.get_quote(stock_code),
            self.get_klines(stock_code, kline_period, kline_limit),
            self.get_financials(stock_code),
            self.get_news(stock_code, news_limit),
            self.get_money_flow(stock_code),
            self.get_dragon_tiger(stock_code),
            self.get_lockup_schedule(stock_code),
            self.get_margin_data(stock_code),
            self.get_north_bound_holding(stock_code),
            self.get_sector_info(stock_code),
            self.get_shareholder_trades(stock_code),
            self.get_dividend_records(stock_code),
            self.get_research_reports(stock_code),
            self.get_consensus_eps(stock_code),
            self.get_concept_blocks(stock_code),
            self.get_announcements(stock_code),
        );

        let quote = quote_r.map_err(|e| {
            tracing::warn!("quote failed: {e}");
            e
        })?;
        let klines = klines_r.unwrap_or_else(|e| {
            tracing::warn!("klines failed: {e}");
            vec![]
        });
        let financials = financials_r.unwrap_or_else(|e| {
            tracing::warn!("financials failed: {e}");
            vec![]
        });
        let news = news_r.unwrap_or_else(|e| {
            tracing::warn!("news failed: {e}");
            vec![]
        });
        let money_flow = money_flow_r.unwrap_or_else(|e| {
            tracing::warn!("money_flow failed: {e}");
            None
        });
        let dragon_tiger = dragon_tiger_r.unwrap_or_else(|e| {
            tracing::warn!("dragon_tiger failed: {e}");
            vec![]
        });
        let lockup = lockup_r.unwrap_or_else(|e| {
            tracing::warn!("lockup failed: {e}");
            vec![]
        });
        let margin_data = margin_r.unwrap_or_else(|e| {
            tracing::warn!("margin_data failed: {e}");
            None
        });
        let north_bound = north_bound_r.unwrap_or_else(|e| {
            tracing::warn!("north_bound failed: {e}");
            None
        });
        let sector_info = sector_r.unwrap_or_else(|e| {
            tracing::warn!("sector_info failed: {e}");
            None
        });
        let shareholder_trades = shareholder_r.unwrap_or_else(|e| {
            tracing::warn!("shareholder_trades failed: {e}");
            vec![]
        });
        let dividend_records = dividend_r.unwrap_or_else(|e| {
            tracing::warn!("dividend_records failed: {e}");
            vec![]
        });
        let research_reports = research_r.unwrap_or_else(|e| {
            tracing::warn!("research_reports failed: {e}");
            vec![]
        });
        let consensus_eps = consensus_r.unwrap_or_else(|e| {
            tracing::warn!("consensus_eps failed: {e}");
            None
        });
        let concept_blocks = concept_r.unwrap_or_else(|e| {
            tracing::warn!("concept_blocks failed: {e}");
            None
        });
        let announcements = announcements_r.unwrap_or_else(|e| {
            tracing::warn!("announcements failed: {e}");
            vec![]
        });

        Ok(StockRawData {
            quote,
            klines,
            financials,
            news,
            money_flow,
            dragon_tiger,
            lockup,
            margin_data,
            north_bound,
            sector_info,
            shareholder_trades,
            dividend_records,
            research_reports,
            consensus_eps,
            concept_blocks,
            announcements,
        })
    }
}

impl Default for AStockClient {
    fn default() -> Self {
        Self::new()
    }
}
