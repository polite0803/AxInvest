mod error;
mod types;
mod vendors;

use std::collections::HashMap;
use tokio::sync::RwLock;

pub use error::DataError;
pub use types::*;
use vendors::eastmoney::EastMoneyVendor;
use vendors::sina::SinaVendor;
use vendors::tencent::TencentVendor;
use vendors::StockVendor;

pub struct AStockClient {
    tencent: TencentVendor,
    eastmoney: EastMoneyVendor,
    sina: SinaVendor,
    http: reqwest::Client,
    /// 进程内简易缓存: key -> (过期时间戳_秒, json值)
    cache: RwLock<HashMap<String, (i64, String)>>,
}

impl AStockClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");
        Self {
            tencent: TencentVendor { http: http.clone() },
            eastmoney: EastMoneyVendor { http: http.clone() },
            sina: SinaVendor { http: http.clone() },
            http,
            cache: RwLock::new(HashMap::new()),
        }
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// 从缓存读取值（未过期时返回 Some）
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

    /// 写入缓存（key + 值 + TTL秒数）
    async fn cache_set(&self, key: String, value: String, ttl_secs: i64) {
        let mut cache = self.cache.write().await;
        let expiry = chrono::Utc::now().timestamp() + ttl_secs;
        cache.insert(key, (expiry, value));
    }

    /// 获取实时行情（腾讯财经）— 30s 缓存
    pub async fn get_quote(&self, stock_code: &str) -> Result<StockQuote, DataError> {
        let cache_key = format!("quote:{}", stock_code);
        if let Some(cached) = self.cache_get(&cache_key).await {
            if let Ok(quote) = serde_json::from_str(&cached) {
                return Ok(quote);
            }
        }
        let result = self.tencent.get_quote(stock_code).await?;
        let json = serde_json::to_string(&result).unwrap_or_default();
        self.cache_set(cache_key, json, 30).await;
        Ok(result)
    }

    /// 获取K线数据（东方财富）— 300s 缓存
    pub async fn get_klines(
        &self,
        stock_code: &str,
        period: &str,
        limit: u32,
    ) -> Result<Vec<KLine>, DataError> {
        let cache_key = format!("klines:{}:{}:{}", stock_code, period, limit);
        if let Some(cached) = self.cache_get(&cache_key).await {
            if let Ok(klines) = serde_json::from_str(&cached) {
                return Ok(klines);
            }
        }
        let result = self.eastmoney.get_klines(stock_code, period, limit).await?;
        let json = serde_json::to_string(&result).unwrap_or_default();
        self.cache_set(cache_key, json, 300).await;
        Ok(result)
    }

    /// 获取财务报表（东方财富）
    pub async fn get_financials(
        &self,
        stock_code: &str,
    ) -> Result<Vec<FinancialReport>, DataError> {
        self.eastmoney.get_financials(stock_code).await
    }

    /// 获取新闻（新浪财经）
    pub async fn get_news(&self, stock_code: &str, limit: u32) -> Result<Vec<NewsItem>, DataError> {
        self.sina.get_news(stock_code, limit).await
    }

    /// 获取资金流向（东方财富）
    pub async fn get_money_flow(&self, stock_code: &str) -> Result<Option<MoneyFlow>, DataError> {
        self.eastmoney.get_money_flow(stock_code).await
    }

    /// 获取龙虎榜（东方财富）
    pub async fn get_dragon_tiger(
        &self,
        stock_code: &str,
    ) -> Result<Vec<DragonTigerEntry>, DataError> {
        self.eastmoney.get_dragon_tiger(stock_code).await
    }

    /// 获取限售解禁（东方财富）
    pub async fn get_lockup_schedule(
        &self,
        stock_code: &str,
    ) -> Result<Vec<LockupSchedule>, DataError> {
        self.eastmoney.get_lockup_schedule(stock_code).await
    }

    /// 搜索股票（东方财富）
    pub async fn search_stock(&self, keyword: &str) -> Result<Vec<StockSearchResult>, DataError> {
        self.eastmoney.search_stock(keyword).await
    }

    /// 一次性获取所有原始数据。
    /// 各子请求独立容错：只有 quote 为必需；其余失败时记录 warn 日志并回退为空值。
    /// TODO: 为高频调用（如 get_quote）补充 retry 逻辑。
    pub async fn fetch_all(
        &self,
        stock_code: &str,
        kline_period: &str,
        kline_limit: u32,
        news_limit: u32,
    ) -> Result<StockRawData, DataError> {
        let (quote_r, klines_r, financials_r, news_r, money_flow_r, dragon_tiger_r, lockup_r) = tokio::join!(
            self.get_quote(stock_code),
            self.get_klines(stock_code, kline_period, kline_limit),
            self.get_financials(stock_code),
            self.get_news(stock_code, news_limit),
            self.get_money_flow(stock_code),
            self.get_dragon_tiger(stock_code),
            self.get_lockup_schedule(stock_code),
        );

        let quote = quote_r.map_err(|e| {
            tracing::warn!("quote failed: {}", e);
            e
        })?; // quote is required
        let klines = klines_r.unwrap_or_else(|e| {
            tracing::warn!("klines failed: {}", e);
            vec![]
        });
        let financials = financials_r.unwrap_or_else(|e| {
            tracing::warn!("financials failed: {}", e);
            vec![]
        });
        let news = news_r.unwrap_or_else(|e| {
            tracing::warn!("news failed: {}", e);
            vec![]
        });
        let money_flow = money_flow_r.unwrap_or_else(|e| {
            tracing::warn!("money_flow failed: {}", e);
            None
        });
        let dragon_tiger = dragon_tiger_r.unwrap_or_else(|e| {
            tracing::warn!("dragon_tiger failed: {}", e);
            vec![]
        });
        let lockup = lockup_r.unwrap_or_else(|e| {
            tracing::warn!("lockup failed: {}", e);
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
        })
    }
}

impl Default for AStockClient {
    fn default() -> Self {
        Self::new()
    }
}
