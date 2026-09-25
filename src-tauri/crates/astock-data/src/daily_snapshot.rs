//! 每日快照缓存(P5:本地 SQLite 缓存简化版)
//!
//! 为 NoHistoricalSemantic 方法(热门股/行业排名/概念板块/快讯等)
//! 提供"每日快照"缓存。这些数据本身没有历史语义——每日快照是在每天
//! 某个时间点调用 vendor 实时接口获取的"那一刻的今日数据"。
//!
//! 存储:**独立 DiskCache 实例 + 独立文件** `astock_daily_snapshot.json`（JSON 落盘 + LRU），
//! Key 格式 `daily:{method}:{date}`，个股级为 `daily:{method}:{code}:{date}`
//! （读取侧对应 `get_stock`；2026-09-25 之前它只写无读）。
//! 为什么强调"独立"：本文件过去写着「与 L2 隔离」，实现却是把 `with_l2_cache` 的
//! `Arc<DiskCache>` 复用一遍 —— 同一实例、同一 10_000 条容量、同一文件。
//! K 线缓存单条约 70 KB，几条就能把最旧快照按 LRU 挤掉，回放兜底于是"哪天有、哪天没"。
//!
//! 使用模式:
//! 1. 后台 cron 每天调用 sweep_daily() 一次,存入当日快照
//!    （实现在 `src/init/services.rs::start_daily_snapshot_sweep` →
//!    `commands/stock_analysis.rs::run_daily_snapshot_sweep`，交易日 15:00 后幂等执行）
//! 2. replay 模式遇到 NoHistoricalSemantic 方法,先查每日快照
//! 3. cache miss → 正常走 record_degradation + 返回空(不阻塞回测)
//!
//! 配置:通过 `AStockClient::with_daily_snapshot_cache(path)` 注入,默认关闭；
//! 它返回快照 DiskCache 句柄，调用方须为其起后台 flush 任务（`spawn_flush_loop`），
//! 否则当日快照只停在内存里，进程退出即丢。

use crate::disk_cache::DiskCache;
use std::sync::Arc;

/// 每日快照缓存 Key 前缀
const SNAPSHOT_PREFIX: &str = "daily";

/// 支持的 NoHistoricalSemantic 方法列表
pub const SNAPSHOT_METHODS: &[&str] = &[
    "get_hot_stocks",
    "get_industry_ranking",
    "get_cls_flash",
    "get_concept_blocks", // NoHistoricalSemantic: 概念板块，vendor asof_capability 用此名
    "search_stock",
    "get_sector_info",
    "get_money_flow",
    "get_north_bound_holding",
    "get_margin_data",
    "get_index_quotes",
    "get_stock_announcements",
    // 2026-09-25 补：舆情与质押都只有「当下」语义（SocialSentiment 只有 fetched_at、
    // PledgeData 连日期字段都没有），回放里的唯一历史通道就是每日快照。
    "get_social_sentiment",
    "get_pledge_data",
    // get_market_dragon_tiger / get_board_fund_flow 等全市场快照可后续补充
];

/// 需要遍历个股的 per-stock 快照方法（相对于全市场方法）
pub const PER_STOCK_METHODS: &[&str] = &[
    "get_money_flow",
    "get_north_bound_holding",
    "get_margin_data",
    "get_social_sentiment",
    "get_pledge_data",
];

/// 每日快照缓存
///
/// 零成本抽象:只有配置了 DiskCache 才创建实例;None 表示"未启用"。
#[derive(Clone)]
pub struct DailySnapshotCache {
    disk: Arc<DiskCache>,
}

impl DailySnapshotCache {
    /// 从已存在的 DiskCache 创建每日快照缓存
    pub fn from_disk(disk: Arc<DiskCache>) -> Self {
        Self { disk }
    }

    /// 执行一次完整的每日快照采集（由 Tauri command 调用）
    /// : 全市场方法的结果 json，key 为 method 名
    /// : 逐个股票采集的结果，key 为 "{method}:{stock_code}"
    /// Tue Jun 16 18:13:05     2026: 快照日期 YYYY-MM-DD
    /// 存入全市场快照（热门股、行业排名、快讯、概念板块等）
    /// 由 Tauri command sweep_daily_snapshots 采集后调用
    pub fn set_snapshot(&self, method: &str, date: &str, json: &str) {
        let key = Self::cache_key(method, date);
        self.disk.set(key, json.to_string(), 0i64);
    }

    /// 存入个股级快照（资金流向、北向持仓等），key 含股票代码
    /// 调用方遍历股票列表逐只采集后逐只存入
    pub fn set_stock_snapshot(&self, method: &str, stock_code: &str, date: &str, json: &str) {
        let key = Self::stock_cache_key(method, stock_code, date);
        self.disk.set(key, json.to_string(), 0i64);
    }

    fn cache_key(method: &str, date: &str) -> String {
        format!("{SNAPSHOT_PREFIX}:{method}:{date}")
    }

    fn stock_cache_key(method: &str, stock_code: &str, date: &str) -> String {
        format!("{SNAPSHOT_PREFIX}:{method}:{stock_code}:{date}")
    }

    /// 获取指定方法 + 日期的快照
    /// 返回 None 表示未命中缓存(或未启用)
    pub fn get(&self, method: &str, date: &str) -> Option<String> {
        let key = Self::cache_key(method, date);
        self.disk.get(&key)
    }

    /// 获取个股级快照（key 含股票代码）
    ///
    /// **为什么必须有这个方法**：`set_stock_snapshot` 写入的 key 带股票代码，
    /// 而读取侧此前只有不带 code 的 `get` ⇒ 个股级快照**只写无读**，
    /// 采集回来的两融/资金流/北向数据在回放模式下一条也取不到（2026-09-25 补）。
    pub fn get_stock(&self, method: &str, stock_code: &str, date: &str) -> Option<String> {
        let key = Self::stock_cache_key(method, stock_code, date);
        self.disk.get(&key)
    }

    /// 存入快照(TTL = 0 表示不过期,DiskCache 按 LRU 淘汰)
    pub fn set(&self, method: &str, date: &str, value: &str) {
        let key = Self::cache_key(method, date);
        self.disk.set(key, value.to_string(), 0i64);
    }

    /// 检查特定方法是否已缓存(避免反序列化大对象)
    pub fn has(&self, method: &str, date: &str) -> bool {
        let key = Self::cache_key(method, date);
        self.disk.get(&key).is_some()
    }

    // ── C5.3 修复：带 keyword 维度的快照 ──
    // search_stock 这类方法的结果与 keyword 强相关，原 cache_key
    // 仅含 method + date，导致不同 keyword 的搜索结果互相覆盖。
    // 新增 keyword 维度，cache_key = daily:{method}:{keyword}:{date}。

    fn cache_key_with_keyword(method: &str, keyword: &str, date: &str) -> String {
        format!("{SNAPSHOT_PREFIX}:{method}:{keyword}:{date}")
    }

    /// 存入带 keyword 的快照（用于 search_stock 等方法）
    pub fn set_keyword_snapshot(&self, method: &str, keyword: &str, date: &str, json: &str) {
        let key = Self::cache_key_with_keyword(method, keyword, date);
        self.disk.set(key, json.to_string(), 0i64);
    }

    /// 获取带 keyword 的快照
    pub fn get_keyword(&self, method: &str, keyword: &str, date: &str) -> Option<String> {
        let key = Self::cache_key_with_keyword(method, keyword, date);
        self.disk.get(&key)
    }

    /// 检查带 keyword 的快照是否已缓存
    pub fn has_keyword(&self, method: &str, keyword: &str, date: &str) -> bool {
        let key = Self::cache_key_with_keyword(method, keyword, date);
        self.disk.get(&key).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn make_cache() -> DailySnapshotCache {
        let dir = std::env::temp_dir().join("astock_daily_snapshot_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test_cache.json");
        let _ = std::fs::remove_file(&path);
        let disk = DiskCache::load_or_default(path);
        DailySnapshotCache::from_disk(disk)
    }

    #[test]
    fn test_cache_key_format() {
        let key = DailySnapshotCache::cache_key("get_hot_stocks", "2026-06-01");
        assert_eq!(key, "daily:get_hot_stocks:2026-06-01");
    }

    #[test]
    fn test_set_get_roundtrip() {
        let cache = make_cache();
        let data =
            r#"[{"stock_code":"000001","stockName":"平安银行","price":10.0,"changePct":1.0}]"#;
        cache.set("get_hot_stocks", "2026-06-01", data);
        let back = cache.get("get_hot_stocks", "2026-06-01");
        assert!(back.is_some());
        let back_str = back.unwrap();
        assert!(back_str.contains("000001"));
    }

    #[test]
    fn test_has_method() {
        let cache = make_cache();
        assert!(!cache.has("get_hot_stocks", "2026-06-01"));
        cache.set("get_hot_stocks", "2026-06-01", "test");
        assert!(cache.has("get_hot_stocks", "2026-06-01"));
    }

    #[test]
    fn test_miss_returns_none() {
        let cache = make_cache();
        let result = cache.get("get_hot_stocks", "2099-01-01");
        assert!(result.is_none());
    }

    #[test]
    fn test_multiple_dates() {
        let cache = make_cache();
        cache.set("get_hot_stocks", "2026-06-01", "data_jun1");
        cache.set("get_hot_stocks", "2026-06-02", "data_jun2");
        let d1 = cache.get("get_hot_stocks", "2026-06-01");
        let d2 = cache.get("get_hot_stocks", "2026-06-02");
        assert_eq!(d1, Some("data_jun1".to_string()));
        assert_eq!(d2, Some("data_jun2".to_string()));
    }

    #[test]
    fn test_different_methods_same_date() {
        let cache = make_cache();
        cache.set("get_hot_stocks", "2026-06-01", "hot");
        cache.set("get_industry_ranking", "2026-06-01", "industry");
        let hot = cache.get("get_hot_stocks", "2026-06-01");
        let ind = cache.get("get_industry_ranking", "2026-06-01");
        assert_eq!(hot, Some("hot".to_string()));
        assert_eq!(ind, Some("industry".to_string()));
    }

    #[test]
    fn test_snapshot_methods_list_contains_expected() {
        assert!(SNAPSHOT_METHODS.contains(&"get_hot_stocks"));
        assert!(SNAPSHOT_METHODS.contains(&"get_industry_ranking"));
        assert!(SNAPSHOT_METHODS.contains(&"get_cls_flash"));
        assert!(SNAPSHOT_METHODS.contains(&"get_concept_blocks"));
        assert!(SNAPSHOT_METHODS.contains(&"search_stock"));
        assert!(SNAPSHOT_METHODS.contains(&"get_sector_info"));
    }

    #[test]
    fn test_keyword_snapshot_isolation() {
        // C5.3: 不同 keyword 的快照应互相隔离
        let cache = make_cache();
        cache.set_keyword_snapshot("search_stock", "贵州茅台", "2026-06-01", "maotai_result");
        cache.set_keyword_snapshot("search_stock", "比亚迪", "2026-06-01", "byd_result");

        let maotai = cache.get_keyword("search_stock", "贵州茅台", "2026-06-01");
        let byd = cache.get_keyword("search_stock", "比亚迪", "2026-06-01");
        let miss = cache.get_keyword("search_stock", "未知股票", "2026-06-01");

        assert_eq!(maotai, Some("maotai_result".to_string()));
        assert_eq!(byd, Some("byd_result".to_string()));
        assert!(miss.is_none(), "未缓存的 keyword 应返回 None");
    }

    #[test]
    fn test_keyword_snapshot_key_format() {
        // 确认 keyword 版本的 key 含 keyword 维度
        let key =
            DailySnapshotCache::cache_key_with_keyword("search_stock", "贵州茅台", "2026-06-01");
        assert_eq!(key, "daily:search_stock:贵州茅台:2026-06-01");
    }

    /// 回归（2026-09-25）：个股级快照必须可读回。
    ///
    /// 缺陷形态：`set_stock_snapshot` 写入带股票代码的 key，读取侧却只有不带 code 的
    /// `get` ⇒ 回放模式一条也取不到（`sweep_daily_snapshots` 逐只采集看起来"有数据"，
    /// 实际是只写无读的死键）。
    #[test]
    fn test_stock_snapshot_roundtrip() {
        let cache = make_cache();
        cache.set_stock_snapshot(
            "get_margin_data",
            "600519",
            "2026-06-01",
            r#"{"date":"2026-06-01"}"#,
        );

        assert!(
            cache.get_stock("get_margin_data", "600519", "2026-06-01").is_some(),
            "个股级快照必须能按 code + date 读回"
        );
        assert!(
            cache.get_stock("get_margin_data", "000001", "2026-06-01").is_none(),
            "不同 code 不得串读"
        );
        assert!(
            cache.get("get_margin_data", "2026-06-01").is_none(),
            "不带 code 的全市场 key 读不到个股快照（这正是缺陷形态）"
        );
    }
}
