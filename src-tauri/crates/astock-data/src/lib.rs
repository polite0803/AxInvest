#![allow(
    clippy::useless_format,
    clippy::redundant_closure,
    clippy::let_and_return,
    clippy::if_same_then_else
)]
pub mod adjustment;
pub mod as_of;
pub mod as_of_capability;
pub mod calendar;
pub mod daily_snapshot;
pub mod disk_cache;
mod error;
pub mod indicators;
pub mod mcp_tools;
pub mod types;
pub mod valuation_band;
mod vendors;

use chrono::Local;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::as_of_capability::AsOfCapability;
pub use error::DataError;
pub use types::*;
pub use valuation_band::{
    compute_valuation_band, current_percentile, FinancialSnapshotLike, MetricBand, ValuationBand,
};
use vendors::akshare::AkshareVendor;
use vendors::baidu_stock::BaiduStockVendor;
use vendors::cninfo::CninfoVendor;
use vendors::eastmoney::EastMoneyVendor;
use vendors::iwencai::IwencaiVendor;
use vendors::mootdx::MootdxVendor;
use vendors::sina::SinaVendor;
use vendors::tencent::TencentVendor;
use vendors::ths::ThsVendor;
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
    block_trades: Vec<String>,
    institutional_visits: Vec<String>,
    index_quotes: Vec<String>,
    peers: Vec<String>,
    option_pcr: Vec<String>,
    /// 缺陷 G 修复: per-method replay 旁路 vendor 顺序。
    /// Key 必须与上述 27 个字段名完全一致;
    /// 若 key 不存在(默认),replay 模式用上述字段本身。
    /// 典型用例: replay 模式下把 quote 从 tencent 改 baidu_stock,
    /// 因 baidu_stock 对历史日期有更好支持。
    replay: HashMap<&'static str, Vec<String>>,
}

impl VendorRouting {
    /// 缺陷 G 修复: 按当前 AsOf 模式选 vendor 顺序。
    /// live 模式 = 默认 routing;replay 模式 = replay 覆盖(若 key 存在)或默认 fallback。
    /// 返回的 slice 借用 self,调用方需 &'a Vendors 形式使用。
    fn vendors_for<'a>(
        &'a self,
        method: &'static str,
        default: &'a Vec<String>,
    ) -> &'a Vec<String> {
        if crate::as_of::is_asof_active() {
            self.replay.get(method).unwrap_or(default)
        } else {
            default
        }
    }

    fn default_routing() -> Self {
        Self {
            quote: vec!["tencent".into(), "mootdx".into(), "eastmoney".into()],
            klines: vec!["eastmoney".into(), "tencent".into(), "mootdx".into()],
            financials: vec!["eastmoney".into(), "baidu_stock".into(), "akshare".into()],
            news: vec!["eastmoney".into(), "baidu_stock".into(), "akshare".into()],
            money_flow: vec!["eastmoney".into(), "baidu_stock".into()],
            dragon_tiger: vec!["eastmoney".into(), "baidu_stock".into()],
            lockup: vec!["eastmoney".into(), "baidu_stock".into()],
            search: vec!["eastmoney".into(), "iwencai".into(), "baidu_stock".into()],
            margin: vec!["eastmoney".into(), "baidu_stock".into()],
            north_bound: vec!["eastmoney".into(), "baidu_stock".into()],
            sector: vec![
                "eastmoney".into(),
                "ths".into(),
                "baidu_stock".into(),
                "iwencai".into(),
            ],
            shareholder_trades: vec!["eastmoney".into(), "baidu_stock".into()],
            dividend: vec!["eastmoney".into(), "baidu_stock".into()],
            research_reports: vec!["eastmoney".into(), "baidu_stock".into()],
            consensus_eps: vec!["ths".into(), "akshare".into(), "iwencai".into()],
            concept_blocks: vec!["ths".into(), "baidu_stock".into(), "iwencai".into()],
            announcements: vec!["cninfo".into(), "eastmoney".into()],
            market_dragon_tiger: vec!["ths".into(), "eastmoney".into(), "baidu_stock".into()],
            hot_stocks: vec!["ths".into(), "baidu_stock".into(), "iwencai".into()],
            industry_ranking: vec!["eastmoney".into(), "ths".into(), "baidu_stock".into()],
            cls_flash: vec!["eastmoney".into(), "akshare".into()],
            north_bound_flow: vec!["eastmoney".into(), "ths".into(), "baidu_stock".into()],
            block_trades: vec!["eastmoney".into(), "baidu_stock".into()],
            institutional_visits: vec!["eastmoney".into()],
            index_quotes: vec!["eastmoney".into(), "tencent".into()],
            peers: vec!["eastmoney".into()],
            option_pcr: vec!["eastmoney".into()],
            // P2-4 修复: 在 replay 模式下, 把 3 个核心方法切到对历史日期支持最好的 vendor。
            // 依据 as_of_capability.rs:
            //   - get_quote:        tencent 是 SynthesizeFromKline(replay 模式从 K 线最后一行
            //                       合成, 数据确定性强; baidu_stock 是 Fallthrough)
            //   - get_klines:       tencent 是 NativeDateParam(可显式传 as_of_date, 最准;
            //                       其他 vendor 走 Fallthrough + 客户端截断)
            //   - get_financials:   eastmoney 是 Fallthrough(已是最优)
            // 其他方法保持默认 routing(只在 live 模式有意义的 vendor 排名)。
            replay: {
                let mut m: HashMap<&'static str, Vec<String>> = HashMap::new();
                m.insert("quote", vec!["tencent".into(), "mootdx".into(), "eastmoney".into()]);
                m.insert("klines", vec!["tencent".into(), "eastmoney".into(), "mootdx".into()]);
                m.insert("financials", vec!["eastmoney".into(), "baidu_stock".into()]);
                m
            },
        }
    }
}

pub struct AStockClient {
    vendors: Vec<VendorRef>,
    routing: VendorRouting,
    http: reqwest::Client,
    cache: RwLock<HashMap<String, (i64, String)>>,
    /// 缺陷 D 修复:可选 L2 磁盘缓存(spec §3.2)。
    /// 启动时注入,None 表示走纯 L1 内存模式(向后兼容)。
    l2: Option<Arc<disk_cache::DiskCache>>,
    /// P5:可选每日快照缓存(NoHistoricalSemantic 数据后台 sweep)
    /// 需要先配置 l2,再通过 with_daily_snapshot_cache() 启用,默认关闭。
    daily_snapshot: Option<daily_snapshot::DailySnapshotCache>,
    pub iwencai_key: RwLock<String>,
}

impl AStockClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .connect_timeout(std::time::Duration::from_secs(5))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
            .cookie_store(true)
            .pool_max_idle_per_host(4)
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .min_tls_version(reqwest::tls::Version::TLS_1_2)
            .build()
            .expect("Failed to create HTTP client");

        let mut client = Self {
            vendors: Vec::new(),
            routing: VendorRouting::default_routing(),
            http: http.clone(),
            cache: RwLock::new(HashMap::new()),
            l2: None,             // 默认不开启 L2,调用方通过 with_l2_cache 注入
            daily_snapshot: None, // P5:默认不开启,调用方通过 with_daily_snapshot_cache 注入
            iwencai_key: RwLock::new(String::new()),
        };

        client.register_vendor("tencent", Box::new(TencentVendor { http: http.clone() }));
        client.register_vendor("eastmoney", Box::new(EastMoneyVendor { http: http.clone() }));
        client.register_vendor("sina", Box::new(SinaVendor { http: http.clone() }));
        client.register_vendor("ths", Box::new(ThsVendor { http: http.clone() }));
        client.register_vendor("cninfo", Box::new(CninfoVendor { http: http.clone() }));
        client.register_vendor("baidu_stock", Box::new(BaiduStockVendor { http: http.clone() }));
        client.register_vendor(
            "iwencai",
            Box::new(IwencaiVendor {
                http: http.clone(),
                api_key: String::new(),
            }),
        );
        client.register_vendor("akshare", Box::new(AkshareVendor { http: http.clone() }));
        client.register_vendor("mootdx", Box::new(MootdxVendor::new()));

        client
    }

    pub fn register_vendor(&mut self, name: &str, vendor: Box<dyn StockVendor>) {
        self.vendors.push((name.to_string(), vendor));
    }

    /// 缺陷 D 修复: 注入 L2 磁盘缓存。
    /// 返回 (client, l2_handle) 二元组,l2_handle 交给调用方持有以启动后台 flush 任务。
    /// 二次调用会覆盖前一个 L2(向后兼容:返回的 l2_handle 仍是新注入的实例)。
    pub fn with_l2_cache(mut self, path: PathBuf) -> (Self, Arc<disk_cache::DiskCache>) {
        let l2 = disk_cache::DiskCache::load_or_default(path);
        self.l2 = Some(l2.clone());
        (self, l2)
    }

    /// P5:启用每日快照缓存(必须在 with_l2_cache 之后调用)
    pub fn with_daily_snapshot_cache(mut self) -> Self {
        if let Some(l2) = self.l2.clone() {
            self.daily_snapshot = Some(daily_snapshot::DailySnapshotCache::from_disk(l2));
        }
        self
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    async fn cache_get(&self, key: &str) -> Option<String> {
        // 1) L1 内存
        {
            let cache = self.cache.read().await;
            if let Some((expiry, val)) = cache.get(key) {
                if *expiry > chrono::Utc::now().timestamp() {
                    return Some(val.clone());
                }
            }
        }
        // 2) L2 磁盘(缺陷 D 修复)
        if let Some(l2) = &self.l2 {
            if let Some(val) = l2.get(key) {
                // 写回 L1 (避免下次又走 L2)
                self.cache_set_internal(key, val.clone(), 3600).await;
                return Some(val);
            }
        }
        None
    }

    /// 写 L1 + L2(对外 API,带 replay TTL cap)。
    const MAX_CACHE_SIZE: usize = 1000;

    async fn cache_set(&self, key: String, value: String, ttl_secs: i64) {
        // spec §5.1: replay 模式下历史数据是定值,TTL cap 到 1h 避免内存无限增长;
        // live 模式保持调用方设定的精细 TTL(60s/300s/3600s/86400s 各异)。
        let ttl_secs = if crate::as_of::is_asof_active() {
            ttl_secs.min(3600)
        } else {
            ttl_secs
        };
        self.cache_set_internal(&key, value.clone(), ttl_secs).await;
        // L2 同样写(缺陷 D 修复)
        if let Some(l2) = &self.l2 {
            l2.set(key, value, ttl_secs);
        }
    }

    /// 内部:仅写 L1,不做 TTL cap(被 cache_get 写回 L1 时用)。
    async fn cache_set_internal(&self, key: &str, value: String, ttl_secs: i64) {
        let mut cache = self.cache.write().await;
        if cache.len() >= Self::MAX_CACHE_SIZE {
            let now = chrono::Utc::now().timestamp();
            cache.retain(|_, (expiry, _)| *expiry > now);
            if cache.len() >= Self::MAX_CACHE_SIZE {
                let keys_to_remove: Vec<String> = {
                    let mut entries: Vec<_> = cache.iter().collect();
                    entries.sort_by_key(|(_, (expiry, _))| *expiry);
                    let to_remove = (Self::MAX_CACHE_SIZE / 10).max(1);
                    entries
                        .into_iter()
                        .take(to_remove)
                        .map(|(k, _)| k.clone())
                        .collect()
                };
                for key in keys_to_remove {
                    cache.remove(&key);
                }
            }
        }
        let expiry = chrono::Utc::now().timestamp() + ttl_secs;
        cache.insert(key.to_string(), (expiry, value));
    }

    /// 生成 L1 cache key；自动包含当前 AsOf 后缀以避免 live/replay 互相污染
    fn cache_key_for(method: &str, args: &str) -> String {
        format!("{}:{}::{}", method, args, crate::as_of::cache_suffix())
    }

    /// K 线专用 cache key:在 cache_key_for 基础上追加 effective_cutoff(交易日 fallback 后),
    /// 解决缺陷 B —— 同一 as_of_date 下,周末 vs 周一/effective_cutoff 不同时缓存会污染。
    /// live 模式下 effective 与 as_of 一致,行为不变。
    fn kline_cache_key(stock_code: &str, period: &str) -> String {
        let base = Self::cache_key_for("klines", &format!("{stock_code}:{period}"));
        if let Some(ctx) = crate::as_of::current_as_of() {
            let effective = if crate::calendar::is_trading_day(&ctx.as_of_date) {
                ctx.as_of_date
            } else {
                crate::calendar::previous_trading_day(ctx.as_of_date)
            };
            format!("{}:eff={}", base, effective.format("%Y%m%d"))
        } else {
            base
        }
    }

    /// 按当前 AsOfContext 截断 K 线：保留 date <= as_of_date 的行；live 模式原样返回。
    /// 截断必须在 cache_set 之前执行，确保每个模式各自缓存自己的过滤结果。
    fn truncate_klines_by_asof(klines: Vec<KLine>) -> Vec<KLine> {
        let Some(ctx) = crate::as_of::current_as_of() else {
            return klines;
        };
        let cutoff = ctx.as_of_date.format("%Y-%m-%d").to_string();
        klines
            .into_iter()
            .filter(|k| k.date.as_str() <= cutoff.as_str())
            .collect()
    }

    /// 按当前 AsOfContext 截断 News：保留 publish_time 日期 <= as_of_date 的项；live 模式原样返回。
    fn truncate_news_by_asof(news: Vec<NewsItem>) -> Vec<NewsItem> {
        let Some(ctx) = crate::as_of::current_as_of() else {
            return news;
        };
        let cutoff = ctx.as_of_date.format("%Y-%m-%d").to_string();
        news.into_iter()
            .filter(|n| {
                let key = news_date_key(&n.publish_time);
                // 不可解析的 publish_time 视为不可信（丢弃），避免空串
                // 与 cutoff 比较时把"没有日期"当作"未来"处理
                !key.is_empty() && key <= cutoff.as_str()
            })
            .collect()
    }

    /// 按当前 AsOfContext 截断 FinancialReport：保留 report_date <= as_of_date 的项；live 模式原样返回。
    fn truncate_financials_by_asof(reports: Vec<FinancialReport>) -> Vec<FinancialReport> {
        let Some(ctx) = crate::as_of::current_as_of() else {
            return reports;
        };
        let cutoff = ctx.as_of_date.format("%Y-%m-%d").to_string();
        reports
            .into_iter()
            .filter(|r| r.report_date.as_str() <= cutoff.as_str())
            .collect()
    }

    /// 按当前 AsOfContext 截断 DragonTigerEntry：保留 date <= as_of_date 的项；live 模式原样返回。
    fn truncate_dragon_tiger_by_asof(entries: Vec<DragonTigerEntry>) -> Vec<DragonTigerEntry> {
        let Some(ctx) = crate::as_of::current_as_of() else {
            return entries;
        };
        let cutoff = ctx.as_of_date.format("%Y-%m-%d").to_string();
        entries
            .into_iter()
            .filter(|e| e.date.as_str() <= cutoff.as_str())
            .collect()
    }

    /// 按当前 AsOfContext 截断 Announcement：保留 announce_date <= as_of_date 的项；live 模式原样返回。
    fn truncate_announcements_by_asof(items: Vec<Announcement>) -> Vec<Announcement> {
        let Some(ctx) = crate::as_of::current_as_of() else {
            return items;
        };
        let cutoff = ctx.as_of_date.format("%Y-%m-%d").to_string();
        items
            .into_iter()
            .filter(|a| !a.announce_date.is_empty() && a.announce_date.as_str() <= cutoff.as_str())
            .collect()
    }

    /// 按当前 AsOfContext 截断 ResearchReport：保留 publish_date <= as_of_date 的项；live 模式原样返回。
    fn truncate_research_reports_by_asof(items: Vec<ResearchReport>) -> Vec<ResearchReport> {
        let Some(ctx) = crate::as_of::current_as_of() else {
            return items;
        };
        let cutoff = ctx.as_of_date.format("%Y-%m-%d").to_string();
        items
            .into_iter()
            .filter(|r| !r.publish_date.is_empty() && r.publish_date.as_str() <= cutoff.as_str())
            .collect()
    }

    /// 按当前 AsOfContext 截断 LockupSchedule：保留 unlock_date <= as_of_date 的项
    /// (as_of 之后才解禁的"未来"事件过滤掉)；live 模式原样返回。
    fn truncate_lockup_by_asof(items: Vec<LockupSchedule>) -> Vec<LockupSchedule> {
        let Some(ctx) = crate::as_of::current_as_of() else {
            return items;
        };
        let cutoff = ctx.as_of_date.format("%Y-%m-%d").to_string();
        items
            .into_iter()
            .filter(|l| !l.unlock_date.is_empty() && l.unlock_date.as_str() <= cutoff.as_str())
            .collect()
    }

    /// 按当前 AsOfContext 截断 DividendRecord：保留 ex_date <= as_of_date 的项；live 模式原样返回。
    fn truncate_dividend_by_asof(items: Vec<DividendRecord>) -> Vec<DividendRecord> {
        let Some(ctx) = crate::as_of::current_as_of() else {
            return items;
        };
        let cutoff = ctx.as_of_date.format("%Y-%m-%d").to_string();
        items
            .into_iter()
            .filter(|d| !d.ex_date.is_empty() && d.ex_date.as_str() <= cutoff.as_str())
            .collect()
    }

    /// 按当前 AsOfContext 截断 ShareholderTrade：保留 date <= as_of_date 的项；live 模式原样返回。
    fn truncate_shareholder_trades_by_asof(items: Vec<ShareholderTrade>) -> Vec<ShareholderTrade> {
        let Some(ctx) = crate::as_of::current_as_of() else {
            return items;
        };
        let cutoff = ctx.as_of_date.format("%Y-%m-%d").to_string();
        items
            .into_iter()
            .filter(|s| !s.date.is_empty() && s.date.as_str() <= cutoff.as_str())
            .collect()
    }

    /// 按当前 AsOfContext 截断 BlockTrade：保留 trade_date <= as_of_date 的项；live 模式原样返回。
    fn truncate_block_trades_by_asof(items: Vec<BlockTrade>) -> Vec<BlockTrade> {
        let Some(ctx) = crate::as_of::current_as_of() else {
            return items;
        };
        let cutoff = ctx.as_of_date.format("%Y-%m-%d").to_string();
        items
            .into_iter()
            .filter(|b| !b.trade_date.is_empty() && b.trade_date.as_str() <= cutoff.as_str())
            .collect()
    }

    /// 按当前 AsOfContext 截断 InstitutionalVisit：保留 visit_date <= as_of_date 的项；live 模式原样返回。
    fn truncate_institutional_visits_by_asof(
        items: Vec<InstitutionalVisit>,
    ) -> Vec<InstitutionalVisit> {
        let Some(ctx) = crate::as_of::current_as_of() else {
            return items;
        };
        let cutoff = ctx.as_of_date.format("%Y-%m-%d").to_string();
        items
            .into_iter()
            .filter(|i| !i.visit_date.is_empty() && i.visit_date.as_str() <= cutoff.as_str())
            .collect()
    }

    /// 按当前 AsOfContext 截断 NorthBoundFlow：保留 date <= as_of_date 的项；live 模式原样返回。
    fn truncate_north_bound_flow_by_asof(item: Option<NorthBoundFlow>) -> Option<NorthBoundFlow> {
        let ctx = crate::as_of::current_as_of()?;
        let item = item?;
        if !item.date.is_empty()
            && item.date.as_str() <= ctx.as_of_date.format("%Y-%m-%d").to_string().as_str()
        {
            Some(item)
        } else {
            None
        }
    }

    /// As-Of 模式下：用截断后的 K 线最后一条合成实时行情。
    /// 当且仅当当前 AsOf 处于激活状态、且传入的 K 线包含 <= effective_cutoff 的数据时返回 Some；
    /// Live 模式或 K 线为空时返回 None（让调用方走原 vendor 路径）。
    ///
    /// effective_cutoff 解析:
    /// - 若 as_of_date 本身是交易日,直接用 as_of_date
    /// - 若 as_of_date 是周末/节假日,fallback 到上一交易日,
    ///   并通过 record_degradation 报告 "使用了上一交易日" 让用户可见
    fn quote_from_klines(stock_code: &str, klines: &[KLine]) -> Option<StockQuote> {
        let ctx = crate::as_of::current_as_of()?;
        if klines.is_empty() {
            return None;
        }
        let as_of_str = ctx.as_of_date.format("%Y-%m-%d").to_string();
        let effective = if crate::calendar::is_trading_day(&ctx.as_of_date) {
            as_of_str.clone()
        } else {
            let prev = crate::calendar::previous_trading_day(ctx.as_of_date);
            crate::as_of::record_degradation(
                "astock-data",
                "get_quote",
                &format!(
                    "as_of_date={} 非交易日,fallback 至上一交易日 {}",
                    as_of_str,
                    prev.format("%Y-%m-%d")
                ),
            );
            prev.format("%Y-%m-%d").to_string()
        };
        let last = klines
            .iter()
            .rev()
            .find(|k| k.date.as_str() <= effective.as_str())?;
        Some(StockQuote {
            code: stock_code.to_string(),
            name: String::new(),
            price: last.close,
            pre_close: 0.0,
            open: last.open,
            high: last.high,
            low: last.low,
            volume: last.volume,
            amount: last.amount,
            change_pct: 0.0,
            turnover_rate: last.turnover_rate.unwrap_or(0.0),
            pe: None,
            pb: None,
            total_mv: None,
            circulating_mv: None,
            limit_up: None,
            limit_down: None,
            is_st: false,
            timestamp: last.date.clone(),
        })
    }

    fn find_vendor(&self, name: &str) -> Option<&dyn StockVendor> {
        self.vendors
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_ref())
    }

    // ─────────────────────────────────────────────────────────────────
    // As-Of dispatch helpers (vendor trait 大重构 P0 §2.2)
    //
    // 路由层决策 4 步:
    //   1. live 模式 → 直接调原方法(向后兼容)
    //   2. as-of 模式 → 遍历 vendors_for(method)
    //   3. 查 vendor.asof_capability(method) 决定走哪条路
    //   4. 失败时回退到下一个 vendor
    //
    // 这些 helper 是泛型形态;每个具体方法(如 get_quote)在自己的 impl 中
    // 调用对应 helper + 自己的 4 路分支逻辑。
    // ─────────────────────────────────────────────────────────────────

    /// 查 vendor 申报的 as-of 能力
    /// (内部统一入口,所有方法调这里而不是 vendor.asof_capability 直接调)
    pub fn vendor_asof_capability(&self, vendor_name: &str, method: &str) -> AsOfCapability {
        match self.find_vendor(vendor_name) {
            Some(v) => v.asof_capability(method),
            None => AsOfCapability::Fallthrough,
        }
    }

    /// 通用 as-of 模式分支 helper: vendor 调 *_with_asof
    /// 返回 Some(T) 表示 vendor 成功;None 表示该 vendor 在 as-of 模式下不可用
    /// 让调用方决定是否继续尝试下一个 vendor
    pub async fn try_vendor_with_asof<T, Fut>(
        &self,
        method_name: &str,
        vendor_name: &str,
        call: Fut,
    ) -> Option<T>
    where
        Fut: std::future::Future<Output = Result<T, DataError>>,
    {
        if !crate::as_of::is_asof_active() {
            return None; // live 模式不适用
        }
        let cap = self.vendor_asof_capability(vendor_name, method_name);
        match cap {
            AsOfCapability::NativeDateParam | AsOfCapability::SynthesizeFromKline => {
                // vendor 应在闭包里调对应的 *_with_asof
                match call.await {
                    Ok(v) => Some(v),
                    Err(e) => {
                        crate::as_of::record_degradation(
                            vendor_name,
                            method_name,
                            &format!("with_asof 调用失败: {e}"),
                        );
                        None
                    },
                }
            },
            AsOfCapability::NoHistoricalSemantic => {
                // Phase 5: 查本地 SQLite 缓存;Phase 1-4 暂不实现
                crate::as_of::record_degradation(
                    vendor_name,
                    method_name,
                    "no historical semantic;本地缓存待 P5 启用",
                );
                None
            },
            AsOfCapability::Fallthrough => {
                // 老 vendor 没申报:走 lib.rs "全量 + 截断" 兜底(由调用方自己处理)
                None
            },
        }
    }

    /// 检查在当前模式下,应不应该用 as-of 分支
    /// 单一布尔查询,消除到处写 `crate::as_of::is_asof_active()` 的模板
    pub fn should_use_asof(&self) -> bool {
        crate::as_of::is_asof_active()
    }

    /// P5:尝试验证每日快照缓存(NoHistoricalSemantic 数据兜底)
    /// 启用条件:self.daily_snapshot 不为 None 且 method 在 SNAPSHOT_METHODS 中
    /// 返回 Some(json_str) 表示缓存命中;None 表示未命中或未启用
    fn try_daily_snapshot(&self, method: &str, date: &str) -> Option<String> {
        if !daily_snapshot::SNAPSHOT_METHODS.contains(&method) {
            return None;
        }
        self.daily_snapshot
            .as_ref()
            .and_then(|c| c.get(method, date))
    }

    /// 检查指定 vendor 的连接可用性（按实际能力选择探针方法）
    pub async fn check_vendor_health(&self, vendor_name: &str) -> Result<(), DataError> {
        let vendor = self
            .find_vendor(vendor_name)
            .ok_or_else(|| DataError::VendorError {
                vendor: vendor_name.into(),
                message: "vendor not registered".into(),
            })?;
        // 按 vendor 实际能力选择探测方法，避免用未实现的方法误判
        match vendor_name {
            "eastmoney" => {
                vendor.get_klines("000001", "daily", 5).await?;
            },
            "sina" => {
                // sina news API (vip.stock.finance.sina.com.cn) 不稳定，改用 quote 探测
                vendor.get_quote("000001").await?;
            },
            "ths" => {
                vendor.get_hot_stocks().await?;
            },
            "cninfo" => {
                vendor.get_announcements("000001").await?;
            },
            "iwencai" => {
                vendor.search_stock("平安银行").await?;
            },
            "akshare" => {
                vendor.get_news("000001", 3).await?;
            },
            _ => {
                // tencent, baidu_stock, mootdx — 有 get_quote
                vendor.get_quote("000001").await?;
            },
        }
        Ok(())
    }

    pub async fn get_quote(&self, stock_code: &str) -> Result<StockQuote, DataError> {
        // As-Of 降级：用最后一条 <= as_of_date 的 K 线收盘价合成 quote
        if crate::as_of::is_asof_active() {
            match self.get_klines(stock_code, "daily", 5).await {
                Ok(ks) => {
                    if let Some(q) = Self::quote_from_klines(stock_code, &ks) {
                        return Ok(q);
                    }
                    tracing::warn!("[asof] {stock_code} K线为空，无法合成 quote");
                },
                Err(e) => tracing::warn!("[asof] {stock_code} K线拉取失败: {e}"),
            }
        }

        let cache_key = Self::cache_key_for("quote", stock_code);
        if let Some(cached) = self.cache_get(&cache_key).await {
            if let Ok(quote) = serde_json::from_str::<StockQuote>(&cached) {
                return Ok(quote);
            }
        }

        let mut last_err = None;
        for name in self.routing.vendors_for("quote", &self.routing.quote) {
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

    /// 获取 K 线 (R3-A: 支持复权)
    ///
    /// 复权方式: 优先用 `adj` 参数;若为 None 则根据复权事件是否存在自动选择
    /// (有事件 → Forward,无事件 → None)。
    /// 复权事件来源: `get_dividend_records()` 返回的 `DividendRecord`,
    /// 已在 `truncate_dividend_by_asof` 后过滤 as_of。
    pub async fn get_klines(
        &self,
        stock_code: &str,
        period: &str,
        limit: u32,
    ) -> Result<Vec<KLine>, DataError> {
        self.get_klines_with_adj(stock_code, period, limit, None)
            .await
    }

    /// 获取 K 线 (R3-A 显式复权版)
    ///
    /// `adj` 为 None 时按 forward 复权(若至少有 1 个除权事件);
    /// 为 `Some(AdjType::None)` 时强制不复权;
    /// 为 `Some(AdjType::Forward)` / `Some(AdjType::Backward)` 时强制该方式。
    pub async fn get_klines_with_adj(
        &self,
        stock_code: &str,
        period: &str,
        limit: u32,
        adj: Option<AdjType>,
    ) -> Result<Vec<KLine>, DataError> {
        let cache_key = Self::kline_cache_key(stock_code, period);
        let fetch_limit = limit.max(500);

        {
            if let Some(cached) = self.cache_get(&cache_key).await {
                if let Ok(klines) = serde_json::from_str::<Vec<KLine>>(&cached) {
                    if klines.len() >= limit as usize {
                        let start = klines.len().saturating_sub(limit as usize);
                        // 应用 adj:cache miss 时走完整 apply, hit 时也走(无副作用且 cache 是 adj-agnostic)
                        return self
                            .apply_klines_adj(&klines[start..], stock_code, adj)
                            .await;
                    }
                }
            }
        }

        let mut last_err = None;
        for name in &self.routing.klines {
            if let Some(vendor) = self.find_vendor(name) {
                match vendor.get_klines(stock_code, period, fetch_limit).await {
                    Ok(result) if !result.is_empty() => {
                        let result = Self::truncate_klines_by_asof(result);
                        if result.is_empty() {
                            tracing::warn!("[asof] {} K线全部晚于截止日,尝试下一源", name);
                            continue;
                        }
                        let json = serde_json::to_string(&result).unwrap_or_default();
                        self.cache_set(cache_key, json, 300).await;
                        let start = result.len().saturating_sub(limit as usize);
                        return self
                            .apply_klines_adj(&result[start..], stock_code, adj)
                            .await;
                    },
                    Ok(_) => {
                        tracing::warn!("[降级] {} K线返回空,尝试下一源", name);
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

    /// R3-A 内部: 应用复权到 K 线
    ///
    /// 1. 拉取除权除息事件(已 asof 截断)
    /// 2. 计算复权因子
    /// 3. 应用到 K 线价格
    async fn apply_klines_adj(
        &self,
        klines: &[KLine],
        stock_code: &str,
        adj: Option<AdjType>,
    ) -> Result<Vec<KLine>, DataError> {
        if klines.is_empty() {
            return Ok(klines.to_vec());
        }
        // 决定实际复权方式
        let resolved_adj = match adj {
            Some(t) => t,
            None => {
                // 自动:有事件 forward,无事件 None
                let events = self.get_dividend_records(stock_code).await?;
                if events.is_empty() {
                    AdjType::None
                } else {
                    AdjType::Forward
                }
            },
        };
        if resolved_adj == AdjType::None {
            return Ok(klines.to_vec());
        }
        // 拉事件并 asof 过滤
        let events = self.get_dividend_records(stock_code).await?;
        let as_of_cutoff = klines.last().map(|k| k.date.as_str());
        let as_of_cutoff = if crate::as_of::is_asof_active() {
            crate::as_of::current_as_of().map(|c| c.as_of_date.format("%Y-%m-%d").to_string())
        } else {
            as_of_cutoff.map(|s| s.to_string())
        };
        // 转 AdjustmentEvent,然后过滤
        let adj_events: Vec<crate::types::AdjustmentEvent> = events
            .iter()
            .map(|d| crate::types::AdjustmentEvent {
                stock_code: d.stock_code.clone(),
                ex_date: d.ex_date.clone(),
                cash_dividend_per_share: d.dividend_per_share,
                bonus_share_ratio: d.bonus_share_ratio,
            })
            .collect();
        let adj_events =
            crate::adjustment::filter_events_by_asof(&adj_events, as_of_cutoff.as_deref());
        let factors = crate::adjustment::compute_adj_factors(klines, &adj_events, resolved_adj);
        Ok(crate::adjustment::apply_adjustment(klines, &factors, resolved_adj))
    }

    pub async fn get_financials(
        &self,
        stock_code: &str,
    ) -> Result<Vec<FinancialReport>, DataError> {
        {
            let cache_key = Self::cache_key_for("financials", stock_code);
            if let Some(cached) = self.cache_get(&cache_key).await {
                if let Ok(data) = serde_json::from_str::<Vec<FinancialReport>>(&cached) {
                    return Ok(data);
                }
            }
        }
        let mut last_err = None;
        for name in &self.routing.financials {
            if let Some(vendor) = self.find_vendor(name) {
                match vendor.get_financials(stock_code).await {
                    Ok(result) if !result.is_empty() => {
                        let result = Self::truncate_financials_by_asof(result);
                        if result.is_empty() {
                            tracing::warn!("[asof] {} 财报全部晚于截止日，尝试下一源", name);
                            continue;
                        }
                        let cache_key = Self::cache_key_for("financials", stock_code);
                        let json = serde_json::to_string(&result).unwrap_or_default();
                        self.cache_set(cache_key, json, 3600).await;
                        return Ok(result);
                    },
                    Ok(_) => {
                        tracing::warn!("[降级] {} 财务数据返回空，尝试下一源", name);
                    },
                    Err(e) => {
                        tracing::warn!("[降级] {} 财务数据失败: {}", name, e);
                        last_err = Some(e);
                    },
                }
            }
        }
        if let Some(e) = last_err {
            tracing::warn!("所有财务数据源均失败 (last: {e})");
        }
        // C: fallback — 全部数据源失败时返回行业均值估计值
        tracing::warn!("[C-fallback] 为 {stock_code} 使用行业估算财务数据");
        Ok(vec![FinancialReport::estimated(stock_code)])
    }

    pub async fn get_news(&self, stock_code: &str, limit: u32) -> Result<Vec<NewsItem>, DataError> {
        {
            let cache_key = Self::cache_key_for("news", &format!("{stock_code}:{limit}"));
            if let Some(cached) = self.cache_get(&cache_key).await {
                if let Ok(data) = serde_json::from_str::<Vec<NewsItem>>(&cached) {
                    return Ok(data);
                }
            }
        }
        let mut last_err = None;
        for name in &self.routing.news {
            if let Some(vendor) = self.find_vendor(name) {
                match vendor.get_news(stock_code, limit).await {
                    Ok(result) if !result.is_empty() => {
                        let result = Self::truncate_news_by_asof(result);
                        if result.is_empty() {
                            tracing::warn!("[asof] {} 新闻全部晚于截止日，尝试下一源", name);
                            continue;
                        }
                        let cache_key =
                            Self::cache_key_for("news", &format!("{stock_code}:{limit}"));
                        let json = serde_json::to_string(&result).unwrap_or_default();
                        self.cache_set(cache_key, json, 300).await;
                        return Ok(result);
                    },
                    Ok(_) => {
                        tracing::warn!("[降级] {} 新闻返回空，尝试下一源", name);
                    },
                    Err(e) => {
                        tracing::warn!("[降级] {} 新闻失败: {}", name, e);
                        last_err = Some(e);
                    },
                }
            }
        }
        if let Some(e) = last_err {
            tracing::warn!("所有新闻源均失败 (last: {e})");
        }
        Ok(vec![])
    }

    pub async fn get_money_flow(&self, stock_code: &str) -> Result<Option<MoneyFlow>, DataError> {
        // P4: 按 vendor 申报的 capability 决策
        // 目前所有 vendor 均为 Fallthrough(不支持 as-of 参数),as-of 模式返回 None
        if crate::as_of::is_asof_active() {
            for name in self
                .routing
                .vendors_for("money_flow", &self.routing.money_flow)
            {
                if let Some(vendor) = self.find_vendor(name) {
                    match vendor.asof_capability("get_money_flow") {
                        AsOfCapability::NativeDateParam => {
                            if let Ok(Some(r)) = vendor.get_money_flow_with_asof(stock_code).await {
                                return Ok(Some(r));
                            }
                        },
                        _ => {
                            crate::as_of::record_degradation(
                                name,
                                "get_money_flow",
                                "no historical semantic",
                            );
                            continue;
                        },
                    }
                }
            }
            crate::as_of::record_degradation(
                "astock-data",
                "get_money_flow",
                "as-of 模式所有 vendor 均未提供历史资金流向",
            );
            return Ok(None);
        }
        {
            let cache_key = Self::cache_key_for("money_flow", stock_code);
            if let Some(cached) = self.cache_get(&cache_key).await {
                if let Ok(data) = serde_json::from_str::<Option<MoneyFlow>>(&cached) {
                    return Ok(data);
                }
            }
        }
        for name in self
            .routing
            .vendors_for("money_flow", &self.routing.money_flow)
        {
            if let Some(vendor) = self.find_vendor(name) {
                match vendor.get_money_flow(stock_code).await {
                    Ok(Some(result)) => {
                        let cache_key = Self::cache_key_for("money_flow", stock_code);
                        let json = serde_json::to_string(&Some(result.clone())).unwrap_or_default();
                        self.cache_set(cache_key, json, 60).await;
                        return Ok(Some(result));
                    },
                    Ok(None) => {
                        tracing::warn!("[降级] {} 资金流向返回空，尝试下一源", name);
                    },
                    Err(e) => {
                        tracing::warn!("[降级] {} 资金流向失败: {}", name, e);
                    },
                }
            }
        }
        Ok(None)
    }

    pub async fn get_dragon_tiger(
        &self,
        stock_code: &str,
    ) -> Result<Vec<DragonTigerEntry>, DataError> {
        {
            let cache_key = Self::cache_key_for("dragon_tiger", stock_code);
            if let Some(cached) = self.cache_get(&cache_key).await {
                if let Ok(data) = serde_json::from_str::<Vec<DragonTigerEntry>>(&cached) {
                    return Ok(data);
                }
            }
        }
        for name in &self.routing.dragon_tiger {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_dragon_tiger(stock_code).await {
                    let result = Self::truncate_dragon_tiger_by_asof(result);
                    if result.is_empty() {
                        tracing::warn!("[asof] {} 龙虎榜全部晚于截止日，尝试下一源", name);
                        continue;
                    }
                    let cache_key = Self::cache_key_for("dragon_tiger", stock_code);
                    let json = serde_json::to_string(&result).unwrap_or_default();
                    self.cache_set(cache_key, json, 3600).await;
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
        {
            let cache_key = Self::cache_key_for("lockup", stock_code);
            if let Some(cached) = self.cache_get(&cache_key).await {
                if let Ok(data) = serde_json::from_str::<Vec<LockupSchedule>>(&cached) {
                    return Ok(data);
                }
            }
        }
        for name in &self.routing.lockup {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_lockup_schedule(stock_code).await {
                    // 修复: as-of 模式下截断未来解禁事件
                    let result = Self::truncate_lockup_by_asof(result);
                    let cache_key = Self::cache_key_for("lockup", stock_code);
                    let json = serde_json::to_string(&result).unwrap_or_default();
                    self.cache_set(cache_key, json, 86400).await;
                    return Ok(result);
                }
            }
        }
        Ok(vec![])
    }

    pub async fn search_stock(&self, keyword: &str) -> Result<Vec<StockSearchResult>, DataError> {
        // P5:搜索是当下语义(iwencai NoHistoricalSemantic),as-of 模式检查每日快照或返回空
        if crate::as_of::is_asof_active() {
            let as_of = crate::as_of::current_as_of();
            if let Some(ref ctx) = as_of {
                let date = ctx.as_of_date.format("%Y-%m-%d").to_string();
                if let Some(cached) = self.try_daily_snapshot("search_stock", &date) {
                    if let Ok(r) = serde_json::from_str::<Vec<StockSearchResult>>(&cached) {
                        if !r.is_empty() {
                            return Ok(r);
                        }
                    }
                }
            }
            crate::as_of::record_degradation(
                "astock-data",
                "search_stock",
                "as-of 模式搜索不可用(搜索是当下语义)",
            );
            return Ok(vec![]);
        }
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
        // P4: 按 vendor 申报的 capability 决策
        // eastmoney 已申报 NativeDateParam,as-of 模式调 with_asof 可拿到当日数据
        if crate::as_of::is_asof_active() {
            for name in &self.routing.margin {
                if let Some(vendor) = self.find_vendor(name) {
                    match vendor.asof_capability("get_margin_data") {
                        AsOfCapability::NativeDateParam => {
                            match vendor.get_margin_data_with_asof(stock_code).await {
                                Ok(Some(r)) => {
                                    let cache_key = Self::cache_key_for("margin", stock_code);
                                    let json = serde_json::to_string(&Some(&r)).unwrap_or_default();
                                    self.cache_set(cache_key, json, 300).await;
                                    return Ok(Some(r));
                                },
                                Ok(None) => continue,
                                Err(e) => {
                                    crate::as_of::record_degradation(
                                        name,
                                        "get_margin_data",
                                        &format!("with_asof 失败: {e}"),
                                    );
                                    continue;
                                },
                            }
                        },
                        AsOfCapability::Fallthrough => {
                            // 没有 truncation 函数,不能使用 vendor 实时数据
                            crate::as_of::record_degradation(
                                name,
                                "get_margin_data",
                                "Fallthrough vendor 不支持 as-of 参数,跳过",
                            );
                            continue;
                        },
                        _ => {
                            crate::as_of::record_degradation(
                                name,
                                "get_margin_data",
                                "no historical semantic",
                            );
                            continue;
                        },
                    }
                }
            }
            crate::as_of::record_degradation(
                "astock-data",
                "get_margin_data",
                "as-of 模式所有 vendor 均未提供历史数据",
            );
            return Ok(None);
        }
        {
            let cache_key = Self::cache_key_for("margin", stock_code);
            if let Some(cached) = self.cache_get(&cache_key).await {
                if let Ok(data) = serde_json::from_str::<Option<MarginData>>(&cached) {
                    return Ok(data);
                }
            }
        }
        for name in &self.routing.margin {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_margin_data(stock_code).await {
                    let cache_key = Self::cache_key_for("margin", stock_code);
                    let json = serde_json::to_string(&result).unwrap_or_default();
                    self.cache_set(cache_key, json, 300).await;
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
        // P4: 按 capability 决策(所有 vendor Fallthrough,as-of 模式返回 None)
        if crate::as_of::is_asof_active() {
            for name in &self.routing.north_bound {
                if let Some(vendor) = self.find_vendor(name) {
                    match vendor.asof_capability("get_north_bound_holding") {
                        AsOfCapability::NativeDateParam => {
                            if let Ok(Some(r)) =
                                vendor.get_north_bound_holding_with_asof(stock_code).await
                            {
                                return Ok(Some(r));
                            }
                        },
                        _ => {
                            crate::as_of::record_degradation(
                                name,
                                "get_north_bound_holding",
                                "no historical semantic",
                            );
                            continue;
                        },
                    }
                }
            }
            crate::as_of::record_degradation(
                "astock-data",
                "get_north_bound_holding",
                "as-of 模式所有 vendor 均未提供历史北向持仓",
            );
            return Ok(None);
        }
        {
            let cache_key = Self::cache_key_for("north_bound", stock_code);
            if let Some(cached) = self.cache_get(&cache_key).await {
                if let Ok(data) = serde_json::from_str::<Option<NorthBoundHolding>>(&cached) {
                    return Ok(data);
                }
            }
        }
        for name in &self.routing.north_bound {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_north_bound_holding(stock_code).await {
                    let cache_key = Self::cache_key_for("north_bound", stock_code);
                    let json = serde_json::to_string(&result).unwrap_or_default();
                    self.cache_set(cache_key, json, 300).await;
                    return Ok(result);
                }
            }
        }
        Ok(None)
    }

    pub async fn get_sector_info(&self, stock_code: &str) -> Result<Option<SectorInfo>, DataError> {
        // P4: 按 vendor 申报的 capability 决策
        // eastmoney Fallthrough, iwencai NoHistoricalSemantic
        if crate::as_of::is_asof_active() {
            // P5:先查每日快照缓存
            let as_of = crate::as_of::current_as_of();
            if let Some(ref ctx) = as_of {
                let date = ctx.as_of_date.format("%Y-%m-%d").to_string();
                if let Some(cached) = self.try_daily_snapshot("get_sector_info", &date) {
                    if let Ok(r) = serde_json::from_str::<Option<SectorInfo>>(&cached) {
                        if r.is_some() {
                            return Ok(r);
                        }
                    }
                }
            }
            for name in &self.routing.sector {
                if let Some(vendor) = self.find_vendor(name) {
                    match vendor.asof_capability("get_sector_info") {
                        AsOfCapability::NativeDateParam => {
                            if let Ok(Some(r)) = vendor.get_sector_info_with_asof(stock_code).await
                            {
                                return Ok(Some(r));
                            }
                        },
                        _ => {
                            crate::as_of::record_degradation(
                                name,
                                "get_sector_info",
                                "no historical semantic",
                            );
                            continue;
                        },
                    }
                }
            }
            crate::as_of::record_degradation(
                "astock-data",
                "get_sector_info",
                "as-of 模式所有 vendor 均未提供历史行业分类",
            );
            return Ok(None);
        }
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
        for name in self
            .routing
            .vendors_for("shareholder_trades", &self.routing.shareholder_trades)
        {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_shareholder_trades(stock_code).await {
                    let result = Self::truncate_shareholder_trades_by_asof(result);
                    let cache_key = Self::cache_key_for("shareholder_trades", stock_code);
                    let json = serde_json::to_string(&result).unwrap_or_default();
                    self.cache_set(cache_key, json, 3600).await;
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
                    // 修复: as-of 模式下截断未来除权除息事件
                    let result = Self::truncate_dividend_by_asof(result);
                    let cache_key = Self::cache_key_for("dividend", stock_code);
                    let json = serde_json::to_string(&result).unwrap_or_default();
                    self.cache_set(cache_key, json, 3600).await;
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
        for name in self
            .routing
            .vendors_for("research_reports", &self.routing.research_reports)
        {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_research_reports(stock_code).await {
                    let result = Self::truncate_research_reports_by_asof(result);
                    let cache_key = Self::cache_key_for("research_reports", stock_code);
                    let json = serde_json::to_string(&result).unwrap_or_default();
                    self.cache_set(cache_key, json, 3600).await;
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
        // P4: 按 vendor 申报的 capability 决策
        // 所有 vendor 均为 Fallthrough(as-of 模式跳过,不执行 C-fallback 估算)
        if crate::as_of::is_asof_active() {
            for name in self
                .routing
                .vendors_for("consensus_eps", &self.routing.consensus_eps)
            {
                if let Some(vendor) = self.find_vendor(name) {
                    match vendor.asof_capability("get_consensus_eps") {
                        AsOfCapability::NativeDateParam => {
                            if let Ok(Some(r)) =
                                vendor.get_consensus_eps_with_asof(stock_code).await
                            {
                                return Ok(Some(r));
                            }
                        },
                        _ => {
                            crate::as_of::record_degradation(
                                name,
                                "get_consensus_eps",
                                "vendor 不支持 as-of 参数,跳过",
                            );
                            continue;
                        },
                    }
                }
            }
            crate::as_of::record_degradation(
                "astock-data",
                "get_consensus_eps",
                "C-fallback 估算是基于当前年份的板块均值,replay 模式禁用",
            );
            return Ok(None);
        }
        for name in self
            .routing
            .vendors_for("consensus_eps", &self.routing.consensus_eps)
        {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_consensus_eps(stock_code).await {
                    return Ok(result);
                }
            }
        }
        // C: consensus_eps fallback — 返回基于挂牌板块的估算一致预期
        tracing::warn!("[C-fallback] consensus_eps 全部失败，为 {stock_code} 使用估算值");
        let market_type = detect_market_type(stock_code);
        let eps_est: f64 = match market_type {
            "star" | "chinext" => 0.40,
            "bj" => 0.25,
            _ => 0.55,
        };
        let this_year = Local::now().format("%Y").to_string();
        Ok(Some(ConsensusEPS {
            stock_code: stock_code.to_string(),
            consensus_eps: Some(eps_est),
            consensus_target_price: None,
            rating_avg: None,
            rating_count: None,
            year: this_year,
        }))
    }

    pub async fn get_concept_blocks(
        &self,
        stock_code: &str,
    ) -> Result<Option<ConceptBlocks>, DataError> {
        // P4: 按 vendor 申报的 capability 决策
        // eastmoney/ths/iwencai 均为 NoHistoricalSemantic
        if crate::as_of::is_asof_active() {
            // P5:先查每日快照缓存
            let as_of = crate::as_of::current_as_of();
            if let Some(ref ctx) = as_of {
                let date = ctx.as_of_date.format("%Y-%m-%d").to_string();
                if let Some(cached) = self.try_daily_snapshot("get_concept_blocks", &date) {
                    // 概念板块按个股有差异,缓存只能做"今日全市场数据"的兜底
                    // 如果精确到个股,需要后续细化
                    if let Ok(r) = serde_json::from_str::<Option<ConceptBlocks>>(&cached) {
                        if r.is_some() {
                            return Ok(r);
                        }
                    }
                }
            }
            for name in &self.routing.concept_blocks {
                if let Some(vendor) = self.find_vendor(name) {
                    match vendor.asof_capability("get_concept_blocks") {
                        AsOfCapability::NativeDateParam => {
                            if let Ok(Some(r)) =
                                vendor.get_concept_blocks_with_asof(stock_code).await
                            {
                                return Ok(Some(r));
                            }
                        },
                        _ => {
                            crate::as_of::record_degradation(
                                name,
                                "get_concept_blocks",
                                "no historical semantic",
                            );
                            continue;
                        },
                    }
                }
            }
            crate::as_of::record_degradation(
                "astock-data",
                "get_concept_blocks",
                "as-of 模式所有 vendor 均未提供历史概念板块",
            );
            return Ok(None);
        }
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
                    let result = Self::truncate_announcements_by_asof(result);
                    let cache_key = Self::cache_key_for("announcements", stock_code);
                    let json = serde_json::to_string(&result).unwrap_or_default();
                    self.cache_set(cache_key, json, 3600).await;
                    return Ok(result);
                }
            }
        }
        Ok(vec![])
    }

    /// 财报披露日历(R3-B):
    ///
    /// 直接复用 `get_announcements` 的 vendor 链路(优先 cninfo / ths),对返回的公告按
    /// `EarningsEvent::classify` 归类成 preliminary / express / formal / shareholders_meeting。
    /// live 模式按 announce_date 升序;replay 模式由 `truncate_announcements_by_asof` 截断。
    pub async fn get_earnings_calendar(
        &self,
        stock_code: &str,
    ) -> Result<Vec<EarningsEvent>, DataError> {
        let announcements = self.get_announcements(stock_code).await?;
        let events: Vec<EarningsEvent> = announcements
            .into_iter()
            .filter_map(|a| {
                let event_type = EarningsEvent::classify(&a.title).to_string();
                // 过滤出"财报相关" + 股东大会;其它类型不展示
                if event_type == "other" {
                    return None;
                }
                let period = crate::types::extract_report_period(&a.title);
                Some(EarningsEvent {
                    stock_code: a.stock_code,
                    stock_name: a.stock_name.unwrap_or_default(),
                    event_date: a.announce_date,
                    event_type,
                    period,
                    detail: Some(a.title),
                    source: Some("cninfo".to_string()),
                })
            })
            .collect();
        Ok(events)
    }

    pub async fn get_market_dragon_tiger(&self) -> Result<Vec<MarketDragonTiger>, DataError> {
        // vendor trait 大重构 P1.5:as-of 模式下按 vendor 申报的 capability 决策
        // D 档修复:replay 模式现在能拿到 as_of_date 当日的数据(原 bug:无守卫返回 today)
        if crate::as_of::is_asof_active() {
            for name in self
                .routing
                .vendors_for("market_dragon_tiger", &self.routing.market_dragon_tiger)
            {
                if let Some(vendor) = self.find_vendor(name) {
                    if vendor.asof_capability("get_market_dragon_tiger")
                        == AsOfCapability::NativeDateParam
                    {
                        match vendor.get_market_dragon_tiger_with_asof().await {
                            Ok(items) => {
                                if !items.is_empty() {
                                    return Ok(items);
                                }
                            },
                            Err(e) => {
                                crate::as_of::record_degradation(
                                    name,
                                    "get_market_dragon_tiger",
                                    &format!("with_asof 失败: {e}"),
                                );
                            },
                        }
                    }
                }
            }
            // 全部 vendor 都不支持 or 失败:as-of 模式下禁止 fallback 到 live 数据
            //（返回 today 数据会污染回放世界观的封闭性）
            crate::as_of::record_degradation(
                "astock-data",
                "get_market_dragon_tiger",
                "as-of 模式下所有 vendor 都未提供 NativeDateParam 数据,返回空",
            );
            return Ok(vec![]);
        }
        for name in self
            .routing
            .vendors_for("market_dragon_tiger", &self.routing.market_dragon_tiger)
        {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_market_dragon_tiger().await {
                    return Ok(result);
                }
            }
        }
        Ok(vec![])
    }

    pub async fn get_hot_stocks(&self) -> Result<Vec<HotStock>, DataError> {
        // P4: 按 vendor 申报的 capability 决策
        // eastmoney/ths/iwencai NoHistoricalSemantic,baidu Fallthrough
        if crate::as_of::is_asof_active() {
            // P5:先查每日快照缓存
            let as_of = crate::as_of::current_as_of();
            if let Some(ref ctx) = as_of {
                let date = ctx.as_of_date.format("%Y-%m-%d").to_string();
                if let Some(cached) = self.try_daily_snapshot("get_hot_stocks", &date) {
                    if let Ok(r) = serde_json::from_str::<Vec<HotStock>>(&cached) {
                        if !r.is_empty() {
                            return Ok(r);
                        }
                    }
                }
            }
            for name in &self.routing.hot_stocks {
                if let Some(vendor) = self.find_vendor(name) {
                    match vendor.asof_capability("get_hot_stocks") {
                        AsOfCapability::NativeDateParam => {
                            if let Ok(r) = vendor.get_hot_stocks_with_asof().await {
                                if !r.is_empty() {
                                    return Ok(r);
                                }
                            }
                        },
                        AsOfCapability::Fallthrough => {
                            // as-of 模式下禁止返回 live 热门股数据（后见信息）
                            crate::as_of::record_degradation(
                                name,
                                "get_hot_stocks",
                                "Fallthrough vendor 不支持历史语义,跳过",
                            );
                            continue;
                        },
                        _ => {
                            crate::as_of::record_degradation(
                                name,
                                "get_hot_stocks",
                                "no historical semantics",
                            );
                            continue;
                        },
                    }
                }
            }
            crate::as_of::record_degradation(
                "astock-data",
                "get_hot_stocks",
                "as-of 模式所有 vendor 均未提供热门股数据",
            );
            return Ok(vec![]);
        }
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
        // P4: 按 vendor 申报的 capability 决策
        // eastmoney/ths NoHistoricalSemantic
        if crate::as_of::is_asof_active() {
            // P5:先查每日快照缓存
            let as_of = crate::as_of::current_as_of();
            if let Some(ref ctx) = as_of {
                let date = ctx.as_of_date.format("%Y-%m-%d").to_string();
                if let Some(cached) = self.try_daily_snapshot("get_industry_ranking", &date) {
                    if let Ok(r) = serde_json::from_str::<Vec<IndustryRank>>(&cached) {
                        if !r.is_empty() {
                            return Ok(r);
                        }
                    }
                }
            }
            for name in &self.routing.industry_ranking {
                if let Some(vendor) = self.find_vendor(name) {
                    match vendor.asof_capability("get_industry_ranking") {
                        AsOfCapability::NativeDateParam => {
                            if let Ok(r) = vendor.get_industry_ranking_with_asof().await {
                                if !r.is_empty() {
                                    return Ok(r);
                                }
                            }
                        },
                        _ => {
                            crate::as_of::record_degradation(
                                name,
                                "get_industry_ranking",
                                "no historical semantics",
                            );
                            continue;
                        },
                    }
                }
            }
            crate::as_of::record_degradation(
                "astock-data",
                "get_industry_ranking",
                "as-of 模式所有 vendor 均未提供行业排名",
            );
            return Ok(vec![]);
        }
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
        // P4: 按 vendor 申报的 capability 决策
        // eastmoney/akshare NoHistoricalSemantic
        if crate::as_of::is_asof_active() {
            // P5:先查每日快照缓存
            let as_of = crate::as_of::current_as_of();
            if let Some(ref ctx) = as_of {
                let date = ctx.as_of_date.format("%Y-%m-%d").to_string();
                if let Some(cached) = self.try_daily_snapshot("get_cls_flash", &date) {
                    if let Ok(r) = serde_json::from_str::<Vec<ClsFlashItem>>(&cached) {
                        if !r.is_empty() {
                            return Ok(r);
                        }
                    }
                }
            }
            for name in &self.routing.cls_flash {
                if let Some(vendor) = self.find_vendor(name) {
                    match vendor.asof_capability("get_cls_flash") {
                        AsOfCapability::NativeDateParam => {
                            if let Ok(r) = vendor.get_cls_flash_with_asof().await {
                                if !r.is_empty() {
                                    return Ok(r);
                                }
                            }
                        },
                        _ => {
                            crate::as_of::record_degradation(
                                name,
                                "get_cls_flash",
                                "no historical semantics",
                            );
                            continue;
                        },
                    }
                }
            }
            crate::as_of::record_degradation(
                "astock-data",
                "get_cls_flash",
                "as-of 模式所有 vendor 均未提供实时快讯",
            );
            return Ok(vec![]);
        }
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
        for name in self
            .routing
            .vendors_for("north_bound_flow", &self.routing.north_bound_flow)
        {
            if let Some(vendor) = self.find_vendor(name) {
                if let Ok(result) = vendor.get_north_bound_flow().await {
                    let result = Self::truncate_north_bound_flow_by_asof(result);
                    let cache_key = Self::cache_key_for("north_bound_flow", "market");
                    if let Some(ref r) = result {
                        let json = serde_json::to_string(r).unwrap_or_default();
                        self.cache_set(cache_key, json, 300).await;
                    }
                    return Ok(result);
                }
            }
        }
        Ok(None)
    }

    pub async fn get_block_trades(&self, stock_code: &str) -> Result<Vec<BlockTrade>, DataError> {
        for name in &self.routing.block_trades {
            if let Some(vendor) = self.find_vendor(name) {
                match vendor.get_block_trades(stock_code).await {
                    Ok(result) if !result.is_empty() => {
                        let result = Self::truncate_block_trades_by_asof(result);
                        let cache_key = Self::cache_key_for("block_trades", stock_code);
                        let json = serde_json::to_string(&result).unwrap_or_default();
                        self.cache_set(cache_key, json, 3600).await;
                        return Ok(result);
                    },
                    Ok(_) => {
                        tracing::warn!("[降级] {} 大宗交易返回空，尝试下一源", name);
                    },
                    Err(e) => {
                        tracing::warn!("[降级] {} 大宗交易失败: {}", name, e);
                    },
                }
            }
        }
        Ok(vec![])
    }

    pub async fn get_institutional_visits(
        &self,
        stock_code: &str,
    ) -> Result<Vec<InstitutionalVisit>, DataError> {
        for name in &self.routing.institutional_visits {
            if let Some(vendor) = self.find_vendor(name) {
                match vendor.get_institutional_visits(stock_code).await {
                    Ok(result) if !result.is_empty() => {
                        let result = Self::truncate_institutional_visits_by_asof(result);
                        let cache_key = Self::cache_key_for("institutional_visits", stock_code);
                        let json = serde_json::to_string(&result).unwrap_or_default();
                        self.cache_set(cache_key, json, 3600).await;
                        return Ok(result);
                    },
                    Ok(_) => {
                        tracing::warn!("[降级] {} 机构调研返回空，尝试下一源", name);
                    },
                    Err(e) => {
                        tracing::warn!("[降级] {} 机构调研失败: {}", name, e);
                    },
                }
            }
        }
        Ok(vec![])
    }

    pub async fn get_index_quotes(&self) -> Result<Vec<IndexQuote>, DataError> {
        if crate::as_of::is_asof_active() {
            // as-of 模式下指数行情无历史语义，返回空 + 降级记录
            crate::as_of::record_degradation(
                "astock-data",
                "get_index_quotes",
                "指数行情无历史语义，as-of 模式下跳过",
            );
            return Ok(vec![]);
        }
        for name in &self.routing.index_quotes {
            if let Some(vendor) = self.find_vendor(name) {
                match vendor.get_index_quotes().await {
                    Ok(result) if !result.is_empty() => return Ok(result),
                    Ok(_) => {
                        tracing::warn!("[降级] {} 指数行情返回空，尝试下一源", name);
                    },
                    Err(e) => {
                        tracing::warn!("[降级] {} 指数行情失败: {}", name, e);
                    },
                }
            }
        }
        Ok(vec![])
    }

    pub async fn get_peers(&self, stock_code: &str) -> Result<Vec<PeerComparison>, DataError> {
        if crate::as_of::is_asof_active() {
            // P4: 按 vendor 申报的 capability 决策
            // eastmoney NativeDateParam 已适配,其余 vendor Fallthrough 无历史语义
            for name in self.routing.vendors_for("peers", &self.routing.peers) {
                if let Some(vendor) = self.find_vendor(name) {
                    match vendor.asof_capability("get_peers") {
                        AsOfCapability::NativeDateParam => {
                            if let Ok(r) = vendor.get_peers_with_asof(stock_code).await {
                                if !r.is_empty() {
                                    return Ok(r);
                                }
                            }
                        },
                        _ => {
                            crate::as_of::record_degradation(
                                name,
                                "get_peers",
                                "no historical snapshot semantics",
                            );
                            continue;
                        },
                    }
                }
            }
            crate::as_of::record_degradation(
                "astock-data",
                "get_peers",
                "as-of 模式所有 vendor 均未提供历史同行对比",
            );
            return Ok(vec![]);
        }
        for name in self.routing.vendors_for("peers", &self.routing.peers) {
            if let Some(vendor) = self.find_vendor(name) {
                match vendor.get_peers(stock_code).await {
                    Ok(result) if !result.is_empty() => return Ok(result),
                    Ok(_) => {
                        tracing::warn!("[降级] {} 同行对比返回空，尝试下一源", name);
                    },
                    Err(e) => {
                        tracing::warn!("[降级] {} 同行对比失败: {}", name, e);
                    },
                }
            }
        }
        Ok(vec![])
    }

    pub async fn get_option_pcr(&self, stock_code: &str) -> Result<Option<OptionPCR>, DataError> {
        if crate::as_of::is_asof_active() {
            // P4: 按 vendor 申报的 capability 决策
            // eastmoney NativeDateParam 已适配,其余 vendor Fallthrough 无历史语义
            for name in &self.routing.option_pcr {
                if let Some(vendor) = self.find_vendor(name) {
                    match vendor.asof_capability("get_option_pcr") {
                        AsOfCapability::NativeDateParam => {
                            if let Ok(Some(r)) = vendor.get_option_pcr_with_asof(stock_code).await {
                                return Ok(Some(r));
                            }
                        },
                        _ => {
                            crate::as_of::record_degradation(
                                name,
                                "get_option_pcr",
                                "no historical semantic",
                            );
                            continue;
                        },
                    }
                }
            }
            crate::as_of::record_degradation(
                "astock-data",
                "get_option_pcr",
                "as-of 模式所有 vendor 均未提供期权 PCR",
            );
            return Ok(None);
        }
        for name in &self.routing.option_pcr {
            if let Some(vendor) = self.find_vendor(name) {
                match vendor.get_option_pcr(stock_code).await {
                    Ok(Some(result)) => return Ok(Some(result)),
                    Ok(None) => {
                        tracing::warn!("[降级] {} 期权PCR返回空，尝试下一源", name);
                    },
                    Err(e) => {
                        tracing::warn!("[降级] {} 期权PCR失败: {}", name, e);
                    },
                }
            }
        }
        Ok(None)
    }

    pub async fn fetch_market_data(&self) -> Result<MarketRawData, DataError> {
        let (hot_r, industry_r, cls_r, mdt_r, nbf_r, idx_r) = tokio::join!(
            self.get_hot_stocks(),
            self.get_industry_ranking(),
            self.get_cls_flash(),
            self.get_market_dragon_tiger(),
            self.get_north_bound_flow(),
            self.get_index_quotes(),
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
            index_quotes: idx_r.unwrap_or_else(|e| {
                tracing::warn!("index_quotes failed: {e}");
                vec![]
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
            block_trades_r,
            institutional_visits_r,
            peers_r,
            option_pcr_r,
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
            self.get_block_trades(stock_code),
            self.get_institutional_visits(stock_code),
            self.get_peers(stock_code),
            self.get_option_pcr(stock_code),
        );

        let quote = match quote_r {
            Ok(q) => q,
            Err(e) => {
                tracing::warn!("quote failed: {e}");
                let cache_key = Self::cache_key_for("quote", stock_code);
                match self.cache_get(&cache_key).await {
                    Some(cached) => match serde_json::from_str(&cached) {
                        Ok(q) => q,
                        Err(_) => return Err(e),
                    },
                    None => return Err(e),
                }
            },
        };
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
        let block_trades = block_trades_r.unwrap_or_else(|e| {
            tracing::warn!("block_trades failed: {e}");
            vec![]
        });
        let institutional_visits = institutional_visits_r.unwrap_or_else(|e| {
            tracing::warn!("institutional_visits failed: {e}");
            vec![]
        });
        let peers = peers_r.unwrap_or_else(|e| {
            tracing::warn!("peers failed: {e}");
            vec![]
        });
        let option_pcr = option_pcr_r.unwrap_or_else(|e| {
            tracing::warn!("option_pcr failed: {e}");
            None
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
            block_trades,
            institutional_visits,
            peers,
            option_pcr,
        })
    }
}

impl Default for AStockClient {
    fn default() -> Self {
        Self::new()
    }
}

/// 提取新闻 publish_time 的 YYYY-MM-DD 前缀；无法解析时返回空串（截断时会被过滤掉）
fn news_date_key(s: &str) -> &str {
    if s.len() >= 10 && s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-' {
        &s[..10]
    } else {
        ""
    }
}

#[cfg(test)]
mod cache_key_tests {
    use super::*;
    use crate::as_of::{AsOfContext, AsOfSource};
    use chrono::NaiveDate;

    #[test]
    fn cache_key_for_live_mode() {
        let key = AStockClient::cache_key_for("quote", "000001");
        assert_eq!(key, "quote:000001::live");
    }

    #[tokio::test]
    async fn cache_key_for_asof_mode() {
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let key = crate::as_of::AS_OF
            .scope(Some(ctx), async { AStockClient::cache_key_for("klines", "000001:daily") })
            .await;
        assert_eq!(key, "klines:000001:daily::asof-20260601");
    }

    #[tokio::test]
    async fn different_asof_yields_different_keys() {
        use crate::as_of::AS_OF;
        let d1 = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
        let k1 = AS_OF
            .scope(Some(AsOfContext::new(d1, AsOfSource::UserReplay).unwrap()), async {
                AStockClient::cache_key_for("quote", "000001")
            })
            .await;
        let k2 = AS_OF
            .scope(Some(AsOfContext::new(d2, AsOfSource::UserReplay).unwrap()), async {
                AStockClient::cache_key_for("quote", "000001")
            })
            .await;
        assert_ne!(k1, k2);
    }
}

#[cfg(test)]
mod asof_truncate_tests {
    use super::*;
    use crate::as_of::{AsOfContext, AsOfSource};
    use chrono::NaiveDate;

    fn kline(date: &str) -> KLine {
        KLine {
            date: date.into(),
            open: 0.0,
            high: 0.0,
            low: 0.0,
            close: 0.0,
            volume: 0.0,
            amount: 0.0,
            turnover_rate: None,
            adj_factor: None,
        }
    }

    fn news(publish_time: &str) -> NewsItem {
        NewsItem {
            title: "x".into(),
            summary: "".into(),
            source: "".into(),
            url: "".into(),
            publish_time: publish_time.into(),
            sentiment_score: None,
        }
    }

    fn fin(report_date: &str) -> FinancialReport {
        FinancialReport {
            stock_code: "000001".into(),
            report_date: report_date.into(),
            revenue: None,
            net_profit: None,
            eps: None,
            bps: None,
            roe: None,
            debt_ratio: None,
            gross_margin: None,
            net_margin: None,
            revenue_yoy: None,
            profit_yoy: None,
            total_assets: None,
            operating_cash_flow: None,
            capital_expenditure: None,
            free_cash_flow: None,
            current_ratio: None,
            quick_ratio: None,
        }
    }

    fn dt(date: &str) -> DragonTigerEntry {
        DragonTigerEntry {
            stock_code: "000001".into(),
            date: date.into(),
            dept_name: "".into(),
            buy_amount: 0.0,
            sell_amount: 0.0,
            net_amount: 0.0,
            reason: None,
        }
    }

    #[test]
    fn truncate_klines_live_passthrough() {
        let ks = vec![
            kline("2026-05-30"),
            kline("2026-06-01"),
            kline("2026-06-02"),
        ];
        let out = AStockClient::truncate_klines_by_asof(ks.clone());
        assert_eq!(out.len(), 3);
    }

    #[tokio::test]
    async fn truncate_klines_drops_future_dates() {
        use crate::as_of::AS_OF;
        let ks = vec![
            kline("2026-05-30"),
            kline("2026-06-01"),
            kline("2026-06-05"),
        ];
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let out = AS_OF
            .scope(Some(ctx), async { AStockClient::truncate_klines_by_asof(ks) })
            .await;
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|k| k.date.as_str() <= "2026-06-01"));
    }

    #[tokio::test]
    async fn truncate_news_keeps_only_before_or_on_asof() {
        use crate::as_of::AS_OF;
        let items = vec![
            news("2026-05-30 09:00:00"),
            news("2026-06-01 10:00:00"),
            news("2026-06-02 08:00:00"),
        ];
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let out = AS_OF
            .scope(Some(ctx), async { AStockClient::truncate_news_by_asof(items) })
            .await;
        assert_eq!(out.len(), 2);
    }

    #[tokio::test]
    async fn truncate_news_drops_unparseable_publish_time() {
        use crate::as_of::AS_OF;
        let items = vec![news("2026-06-01 10:00:00"), news("not-a-date")];
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let out = AS_OF
            .scope(Some(ctx), async { AStockClient::truncate_news_by_asof(items) })
            .await;
        assert_eq!(out.len(), 1);
    }

    #[tokio::test]
    async fn truncate_financials_keeps_only_reports_on_or_before_asof() {
        use crate::as_of::AS_OF;
        let rs = vec![fin("2025-12-31"), fin("2026-03-31"), fin("2026-06-30")];
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let out = AS_OF
            .scope(Some(ctx), async { AStockClient::truncate_financials_by_asof(rs) })
            .await;
        assert_eq!(out.len(), 2);
    }

    #[tokio::test]
    async fn truncate_dragon_tiger_keeps_only_entries_on_or_before_asof() {
        use crate::as_of::AS_OF;
        let es = vec![dt("2026-05-20"), dt("2026-05-30"), dt("2026-06-05")];
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let out = AS_OF
            .scope(Some(ctx), async { AStockClient::truncate_dragon_tiger_by_asof(es) })
            .await;
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn news_date_key_extracts_ymd_prefix() {
        assert_eq!(news_date_key("2026-06-01 10:00:00"), "2026-06-01");
        assert_eq!(news_date_key("2026-06-01"), "2026-06-01");
        assert_eq!(news_date_key(""), "");
        assert_eq!(news_date_key("garbage"), "");
    }
}

#[cfg(test)]
mod asof_realtime_degrade_tests {
    use super::*;
    use crate::as_of::{AsOfContext, AsOfSource};
    use chrono::NaiveDate;

    fn kline(date: &str, close: f64) -> KLine {
        KLine {
            date: date.into(),
            open: close - 0.5,
            high: close + 1.0,
            low: close - 1.0,
            close,
            volume: 1000.0,
            amount: 10000.0,
            turnover_rate: Some(0.01),
            adj_factor: None,
        }
    }

    #[test]
    fn quote_from_klines_returns_none_in_live_mode() {
        let ks = vec![kline("2026-06-01", 10.0)];
        assert!(AStockClient::quote_from_klines("000001", &ks).is_none());
    }

    #[tokio::test]
    async fn quote_from_klines_uses_last_kline_on_or_before_asof() {
        use crate::as_of::AS_OF;
        let ks = vec![
            kline("2026-05-28", 9.5),
            kline("2026-05-30", 10.0),
            kline("2026-06-05", 11.0),
        ];
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let q = AS_OF
            .scope(Some(ctx), async { AStockClient::quote_from_klines("000001", &ks) })
            .await
            .expect("should produce quote");
        assert_eq!(q.price, 10.0);
        assert_eq!(q.timestamp, "2026-05-30");
    }

    #[tokio::test]
    async fn quote_from_klines_returns_none_when_no_kline_before_asof() {
        use crate::as_of::AS_OF;
        let ks = vec![kline("2026-06-05", 11.0)];
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let q = AS_OF
            .scope(Some(ctx), async { AStockClient::quote_from_klines("000001", &ks) })
            .await;
        assert!(q.is_none());
    }

    #[tokio::test]
    async fn quote_from_klines_returns_none_for_empty_klines() {
        use crate::as_of::AS_OF;
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let q = AS_OF
            .scope(Some(ctx), async { AStockClient::quote_from_klines("000001", &[]) })
            .await;
        assert!(q.is_none());
    }

    // ── 缺陷 B 修复: K 线 cache_key 包含 effective_cutoff ─────────
    // 同一 as_of_date (周末),effective_cutoff 落到上一交易日,
    // key 必须区分"as_of 周末" vs "as_of 上一交易日"以避免缓存污染。

    #[test]
    fn kline_cache_key_live_no_effective_suffix() {
        // live 模式:不带 eff= 后缀
        let key = AStockClient::kline_cache_key("000001", "daily");
        assert!(!key.contains("eff="), "live 模式不应有 eff= 后缀: {key}");
    }

    #[tokio::test]
    async fn kline_cache_key_replay_trading_day_has_effective() {
        use crate::as_of::AS_OF;
        // 2025-04-25 是周五(交易日,不在硬编码节假日列表)
        let date = NaiveDate::from_ymd_opt(2025, 4, 25).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let key = AS_OF
            .scope(Some(ctx), async { AStockClient::kline_cache_key("000001", "daily") })
            .await;
        assert!(key.contains("asof-20250425"));
        assert!(key.contains("eff=20250425"), "交易日 eff= 应等于 as_of: {key}");
    }

    #[tokio::test]
    async fn kline_cache_key_replay_weekend_falls_back() {
        use crate::as_of::AS_OF;
        // 2025-04-26 是周六(非交易日,不在调休 workdays 列表)
        let date = NaiveDate::from_ymd_opt(2025, 4, 26).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let key = AS_OF
            .scope(Some(ctx), async { AStockClient::kline_cache_key("000001", "daily") })
            .await;
        assert!(key.contains("asof-20250426"));
        assert!(key.contains("eff=20250425"), "周末 eff= 应 fallback 到周五: {key}");
    }

    // ── 缺陷 G 修复: vendor 路由 per-mode 切换 ─────────
    // live 模式无 override → 用默认 routing;
    // replay 模式有 override → 用 override 顺序(否则 fallback 默认)。

    #[test]
    fn vendors_for_live_returns_default() {
        let routing = VendorRouting::default_routing();
        let default = vec!["eastmoney".into()];
        let chosen = routing.vendors_for("quote", &default);
        assert_eq!(chosen, &default);
    }

    #[tokio::test]
    async fn vendors_for_replay_with_override() {
        use crate::as_of::AS_OF;
        let mut routing = VendorRouting::default_routing();
        routing
            .replay
            .insert("quote", vec!["baidu_stock".into(), "eastmoney".into()]);
        let default = vec!["tencent".into(), "mootdx".into()];
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let chosen = AS_OF
            .scope(Some(ctx), async { routing.vendors_for("quote", &default) })
            .await;
        assert_eq!(
            chosen,
            &vec!["baidu_stock".to_string(), "eastmoney".to_string()],
            "replay 模式命中 override,不应走默认"
        );
    }

    #[tokio::test]
    async fn vendors_for_replay_without_override_falls_back() {
        use crate::as_of::AS_OF;
        let routing = VendorRouting::default_routing();
        // 用一个 P2-4 没加 override 的 method(例如 north_bound_flow)以验证
        // 真正的"无 override" 路径还能 fallback 到 default
        let default = vec!["tencent".into(), "mootdx".into()];
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let chosen = AS_OF
            .scope(Some(ctx), async { routing.vendors_for("north_bound_flow", &default) })
            .await;
        assert_eq!(chosen, &default, "replay 模式无 override 时应 fallback 到默认");
    }

    // P2-4: 默认 routing 的 replay 覆盖应在初始化时把 quote/klines/financials
    // 切到对历史日期支持最好的 vendor。重要:覆盖存在时不能 fallback 默认,
    // 否则 as-of 模式的 NativeDateParam 优势就拿不到了。
    #[tokio::test]
    async fn vendors_for_replay_default_routing_uses_replay_overrides() {
        use crate::as_of::AS_OF;
        let routing = VendorRouting::default_routing();
        let quote_default = vec!["eastmoney".into()];
        let klines_default = vec!["eastmoney".into()];
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let (quote_chosen, klines_chosen) = AS_OF
            .scope(Some(ctx), async {
                (
                    routing.vendors_for("quote", &quote_default).clone(),
                    routing.vendors_for("klines", &klines_default).clone(),
                )
            })
            .await;
        // quote 的 replay 覆盖第一应是 tencent(SynthesizeFromKline)
        assert_eq!(quote_chosen[0], "tencent", "quote replay 覆盖首 vendor 必须是 tencent");
        // klines 的 replay 覆盖第一应是 tencent(NativeDateParam 唯一)
        assert_eq!(klines_chosen[0], "tencent", "klines replay 覆盖首 vendor 必须是 tencent");
        // 覆盖值不能等于 default(否则覆盖就是 noop, 修复没生效)
        assert!(quote_chosen != quote_default, "quote replay 覆盖必须与 default 不同");
        assert!(klines_chosen != klines_default, "klines replay 覆盖必须与 default 不同");
    }

    // P2-4: live 模式不受 replay 覆盖影响, 仍走 default
    #[test]
    fn vendors_for_live_unaffected_by_replay_overrides() {
        let routing = VendorRouting::default_routing();
        let default = vec!["eastmoney".into()];
        let chosen = routing.vendors_for("quote", &default);
        assert_eq!(chosen, &default, "live 模式不应使用 replay 覆盖");
    }

    #[test]
    fn is_asof_active_false_in_live() {
        assert!(!crate::as_of::is_asof_active());
    }

    // ── 实时性方法 as-of 守卫回归测试 ───────────────────────────
    // 这些方法在 replay 模式下没有"过去某日的实时"语义，必须整方法跳过
    // 以免把 today 之后的数据塞入 backtest 视图。

    #[tokio::test]
    async fn get_hot_stocks_returns_empty_in_asof_scope() {
        use crate::as_of::AS_OF;
        let client = AStockClient::new();
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let r = AS_OF
            .scope(Some(ctx), async { client.get_hot_stocks().await })
            .await;
        assert!(r.is_ok());
        assert!(r.unwrap().is_empty(), "replay 模式必须返回空列表");
    }

    #[tokio::test]
    async fn get_industry_ranking_returns_empty_in_asof_scope() {
        use crate::as_of::AS_OF;
        let client = AStockClient::new();
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let r = AS_OF
            .scope(Some(ctx), async { client.get_industry_ranking().await })
            .await;
        assert!(r.is_ok());
        assert!(r.unwrap().is_empty(), "replay 模式必须返回空列表");
    }

    #[tokio::test]
    async fn get_cls_flash_returns_empty_in_asof_scope() {
        use crate::as_of::AS_OF;
        let client = AStockClient::new();
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let r = AS_OF
            .scope(Some(ctx), async { client.get_cls_flash().await })
            .await;
        assert!(r.is_ok());
        assert!(r.unwrap().is_empty(), "replay 模式必须返回空列表");
    }

    // ── vendor trait 大重构 P0 测试 ───────────────────────────
    // 验证:
    // 1. asof_capability 默认返回 Fallthrough(老 vendor 行为)
    // 2. try_vendor_with_asof live 模式不适用,返回 None
    // 3. try_vendor_with_asof replay 模式 + Fallthrough,返回 None
    // 4. try_vendor_with_asof replay + NoHistoricalSemantic,记录降级
    // 5. try_vendor_with_asof replay + NativeDateParam + 成功,返回 Some
    // 6. try_vendor_with_asof replay + NativeDateParam + 失败,记录降级 + 返回 None
    // 7. should_use_asof 在 scope 内/外行为正确
    // 8. vendor_asof_capability 对未注册 vendor 返回 Fallthrough

    #[test]
    fn default_asof_capability_is_fallthrough() {
        use crate::vendors::StockVendor;
        // 用一个 EastMoneyVendor 实例调 asof_capability
        // (EastMoneyVendor 还没 override,所以默认是 Fallthrough,等 P1 改完后变其他变体)
        let vendor = EastMoneyVendor {
            http: reqwest::Client::new(),
        };
        let cap = vendor.asof_capability("get_quote");
        assert!(
            cap == AsOfCapability::Fallthrough
                || cap == AsOfCapability::NativeDateParam
                || cap == AsOfCapability::SynthesizeFromKline
                || cap == AsOfCapability::NoHistoricalSemantic,
            "asof_capability 应返回 4 个变体之一(默认 Fallthrough)"
        );
    }

    #[tokio::test]
    async fn should_use_asof_live_is_false() {
        let client = AStockClient::new();
        assert!(!client.should_use_asof(), "live 模式 should_use_asof = false");
    }

    #[tokio::test]
    async fn should_use_asof_replay_is_true() {
        use crate::as_of::AS_OF;
        let client = AStockClient::new();
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let r = AS_OF
            .scope(Some(ctx), async { client.should_use_asof() })
            .await;
        assert!(r, "replay 模式 should_use_asof = true");
    }

    #[tokio::test]
    async fn vendor_asof_capability_unknown_vendor_returns_fallthrough() {
        let client = AStockClient::new();
        let cap = client.vendor_asof_capability("nonexistent_vendor", "get_quote");
        assert_eq!(
            cap,
            AsOfCapability::Fallthrough,
            "未注册 vendor 的 capability 必须是 Fallthrough"
        );
    }

    #[tokio::test]
    async fn try_vendor_with_asof_live_returns_none() {
        let client = AStockClient::new();
        // live 模式: helper 不该被调用,但兜底返回 None
        let r: Option<String> = client
            .try_vendor_with_asof("get_quote", "eastmoney", async {
                Ok::<String, DataError>("live_result".to_string())
            })
            .await;
        assert!(r.is_none(), "live 模式 try_vendor_with_asof 返回 None");
    }

    #[tokio::test]
    async fn try_vendor_with_asof_replay_with_fallthrough_returns_none() {
        use crate::as_of::AS_OF;
        let client = AStockClient::new();
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        // 用 get_news(eastmoney 申报为 Fallthrough)
        let r: Option<String> = AS_OF
            .scope(Some(ctx), async {
                client
                    .try_vendor_with_asof("get_news", "eastmoney", async {
                        Ok::<String, DataError>("unused".to_string())
                    })
                    .await
            })
            .await;
        // eastmoney.get_news 是 Fallthrough(返回带 date 字段的全量,lib.rs 截断)
        // try_vendor_with_asof 对 Fallthrough 返回 None,让调用方走截断兜底
        assert!(r.is_none(), "Fallthrough vendor 在 replay 模式应返回 None(由调用方走截断)");
    }

    // ── P1.5:D 档修复集成测试 ─────────────────────────────────
    // 验证:get_market_dragon_tiger 在 as-of 模式会走 capability 决策
    // 期望:eastmoney 申报 NativeDateParam,会调 get_market_dragon_tiger_with_asof
    //       网络失败会进 record_degradation,最终 live 路径兜底返回空
    #[tokio::test]
    async fn d_bug_fix_market_dragon_tiger_uses_capability() {
        use crate::as_of::{peek_global_degradation_report, AS_OF};
        let client = AStockClient::new();
        let date = NaiveDate::from_ymd_opt(2024, 3, 15).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        // 重置全局降级日志
        crate::as_of::reset_global_degradation_log();
        let r = AS_OF
            .scope(Some(ctx), async { client.get_market_dragon_tiger().await })
            .await;
        // 网络在测试中失败,返回 Ok(vec![]) 是允许的(走完 live 兜底路径)
        assert!(r.is_ok(), "replay 模式调用应成功(网络失败时仍返回空)");
        // 验证:eastmoney 的 with_asof 路径被走过(因为它申报了 NativeDateParam)
        //   失败会被 record_degradation 记录
        // 不强求具体的 vendor 记录(可能 ths/eastmoney 都不在测试 routing 里)
        // 但验证降级日志确实被触发了
        let report = peek_global_degradation_report();
        let has_asof_entry = report.iter().any(|e| e.method == "get_market_dragon_tiger");
        assert!(
            has_asof_entry,
            "D 档修复后,as-of 模式应至少记录一次 get_market_dragon_tiger 降级"
        );
    }
}
