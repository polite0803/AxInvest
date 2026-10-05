//! 每日快照缓存(P5:本地 SQLite 缓存简化版)
//!
//! 为 NoHistoricalSemantic 方法(热门股/行业排名/概念板块/快讯等)
//! 提供"每日快照"缓存。这些数据本身没有历史语义——每日快照是在每天
//! 某个时间点调用 vendor 实时接口获取的"那一刻的今日数据"。
//!
//! 存储:**独立 DiskCache 实例 + 独立文件** `astock_daily_snapshot.json`（JSON 落盘 + LRU），
//! Key 格式 `daily:{method}:{date}`，个股级为 `daily:{method}:{code}:{date}`
//! （读取侧对应 `get_stock`；2026-09-25 之前它只写无读）。
//! `{date}` 在读写两侧都先归一到「当天或之前最近交易日」（见 `key_date`）——
//! 快照的键空间等于交易日，周末/假日锚点的回放因此能回填上一交易日那条。
//! 为什么强调"独立"：本文件过去写着「与 L2 隔离」，实现却是把 `with_l2_cache` 的
//! `Arc<DiskCache>` 复用一遍 —— 同一实例、同一 10_000 条容量、同一文件。
//! K 线缓存单条约 70 KB，几条就能把最旧快照按 LRU 挤掉，回放兜底于是"哪天有、哪天没"。
//!
//! 使用模式:
//! 1. 后台 cron 每小时 tick，采集**最近一个已完成交易日**的快照（归属日判据见
//!    `snapshot_target_date`；非交易日会话可回填上一交易日）
//!    （实现在 `src/init/services.rs::start_daily_snapshot_sweep` →
//!    `commands/stock_analysis.rs::run_daily_snapshot_sweep`，幂等执行）
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
    // #20①（2026-10-05）：一致预期补进白名单。它的 as-of 通道**只有**这一个 ——
    // eastmoney 已申报 `NoHistoricalSemantic`（P9-3，弃用了「板块常数估算」代理），
    // 而当日值只有采集侧入库、读取侧才可能命中；此前白名单漏了它 ⇒
    // `try_stock_daily_snapshot` 被 `contains` 挡掉、恒 miss ⇒ as-of 一致预期必然降级。
    // 补这一条与下面 `PER_STOCK_METHODS` 的登记必须**同批**，只补一边等于没接。
    "get_consensus_eps",
    // 仍未纳入（各自有独立理由，别顺手加）：
    //   · get_market_dragon_tiger / get_dragon_tiger：龙虎榜条目自带日期 ⇒ 走
    //     「vendor 全量 + lib.rs 按截止日截断」，不是 NoHistoricalSemantic ⇒ 归档对它无用；
    //   · get_board_fund_flow：全市场快照，待定语义后再说。
];

/// 需要遍历个股的 per-stock 快照方法（相对于全市场方法）
pub const PER_STOCK_METHODS: &[&str] = &[
    "get_money_flow",
    "get_north_bound_holding",
    "get_margin_data",
    "get_social_sentiment",
    "get_pledge_data",
    // #20①：一致预期是逐股取数（`get_consensus_eps(stock_code)`）⇒ 必须登记在这里，
    // 否则采集侧会把它当全市场方法、进不了遍历自选股那条臂。
    "get_consensus_eps",
];

/// 每日快照缓存
///
/// 零成本抽象:只有配置了 DiskCache 才创建实例;None 表示"未启用"。
#[derive(Clone)]
pub struct DailySnapshotCache {
    disk: Arc<DiskCache>,
}

/// 快照**归属日**与采集时机的单一判据（G2'，2026-09-27 粤海饲料回放实证）。
///
/// 返回 `Some(date)` = 此刻应采集归属日为 `date` 的快照；`None` = 还没到采集时机
/// （归属日就是今天、且尚未收盘 15:00 ⇒ 盘中只有半日数据，落盘会被回放当完整收盘值）。
///
/// 原实现在外层用 `is_trading_day(今天)` 做闸门 ⇒ 非交易日整轮跳过：
/// 09-24（周四）是最后交易日、09-25 中秋 + 周末的会话全部 skip ⇒ 09-24 的快照
/// **永远缺**（当日快照文件里只有 09-09 一条，回放行业排名因此 miss 去打已被
/// 本机阻断的 push2\* 合成链）。而非交易日 vendor 的「今日」接口返回的就是
/// 上一交易日完整收盘数据 ⇒ 归属日取 `previous_trading_day(今天)` 即可回填。
pub fn snapshot_target_date(
    now_local: &chrono::DateTime<chrono::Local>,
) -> Option<chrono::NaiveDate> {
    let today = now_local.date_naive();
    let target = crate::calendar::previous_trading_day(today);
    if target == today && chrono::Timelike::hour(now_local) < 15 {
        return None;
    }
    Some(target)
}

/// 快照的**日期键空间只有交易日**：读写两侧的日期一律先归一到
/// 「该日当天或之前最近的交易日」，再拼 key。
///
/// 为什么放在 key 构造处而不是各调用方（2026-09-27 001313 回放实证）：采集侧的归属日
/// 已由 `snapshot_target_date` 改成最近已完成交易日（周末会话回填 09-24），但读取侧
/// 仍拿 `as_of_date` **原值** 查 ⇒ 用户把回放锚点选在周日 09-27 时读的是
/// `daily:{method}:2026-09-27`，永远 miss，兜底形同不存在，面板继续红在被打断的合成链上。
/// 归一后与 `get_quote` 的休市回退同口径（09-27 → 上一交易日 09-24）；锚点本身是交易日时
/// `previous_trading_day` 返回原值，语义不变。读写共用同一函数 ⇒ 不可能再各走各的判据。
fn key_date(date: &str) -> String {
    match chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") {
        Ok(d) => crate::calendar::previous_trading_day(d).format("%Y-%m-%d").to_string(),
        // 非 YYYY-MM-DD 的调用方（理论上不应有）保持原样，不做二次猜测
        Err(_) => date.to_string(),
    }
}

/// 空结果不是快照：`"[]"` / `""` / `"null"` 一律按「这一天没有快照」处理。
///
/// 为什么必须在缓存层过滤而不是在采集侧逐个 method 判（2026-09-27 实测）：
/// 采集 09-24 的那一轮里 `daily:get_industry_ranking:2026-09-24` 与
/// `daily:get_hot_stocks:2026-09-24` 都落成了字面 `"[]"` —— 后台 tick 的采集任务
/// 没有 task-local 作用域，`current_as_of()` 走**全局栈**兜底，读到当时正在跑的
/// 回放上下文 ⇒ 走了 as-of 分支（探测 push2\* 失败 → `Ok(vec![])`）→ 采集把空列表
/// 当有效快照写盘。而闸门是 `has_daily_snapshot("get_index_quotes", date)`，
/// `"[]"` 也算「已有」⇒ 之后每一轮整轮 skip，**该日的快照永远修不好**。
/// 在读写与判重的唯一出口处过滤，旧文件里的中毒条目能自愈。
fn usable(json: Option<String>) -> Option<String> {
    json.filter(|v| {
        let t = v.trim();
        !t.is_empty() && t != "[]" && t != "null"
    })
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
        format!("{SNAPSHOT_PREFIX}:{method}:{}", key_date(date))
    }

    fn stock_cache_key(method: &str, stock_code: &str, date: &str) -> String {
        format!("{SNAPSHOT_PREFIX}:{method}:{stock_code}:{}", key_date(date))
    }

    /// 获取指定方法 + 日期的快照
    /// 返回 None 表示未命中缓存(或未启用)
    pub fn get(&self, method: &str, date: &str) -> Option<String> {
        let key = Self::cache_key(method, date);
        usable(self.disk.get(&key))
    }

    /// 获取个股级快照（key 含股票代码）
    ///
    /// **为什么必须有这个方法**：`set_stock_snapshot` 写入的 key 带股票代码，
    /// 而读取侧此前只有不带 code 的 `get` ⇒ 个股级快照**只写无读**，
    /// 采集回来的两融/资金流/北向数据在回放模式下一条也取不到（2026-09-25 补）。
    pub fn get_stock(&self, method: &str, stock_code: &str, date: &str) -> Option<String> {
        let key = Self::stock_cache_key(method, stock_code, date);
        usable(self.disk.get(&key))
    }

    /// 存入快照(TTL = 0 表示不过期,DiskCache 按 LRU 淘汰)
    pub fn set(&self, method: &str, date: &str, value: &str) {
        let key = Self::cache_key(method, date);
        self.disk.set(key, value.to_string(), 0i64);
    }

    /// 检查特定方法是否已缓存(避免反序列化大对象)
    pub fn has(&self, method: &str, date: &str) -> bool {
        let key = Self::cache_key(method, date);
        usable(self.disk.get(&key)).is_some()
    }

    // ── C5.3 修复：带 keyword 维度的快照 ──
    // search_stock 这类方法的结果与 keyword 强相关，原 cache_key
    // 仅含 method + date，导致不同 keyword 的搜索结果互相覆盖。
    // 新增 keyword 维度，cache_key = daily:{method}:{keyword}:{date}。

    fn cache_key_with_keyword(method: &str, keyword: &str, date: &str) -> String {
        format!("{SNAPSHOT_PREFIX}:{method}:{keyword}:{}", key_date(date))
    }

    /// 存入带 keyword 的快照（用于 search_stock 等方法）
    pub fn set_keyword_snapshot(&self, method: &str, keyword: &str, date: &str, json: &str) {
        let key = Self::cache_key_with_keyword(method, keyword, date);
        self.disk.set(key, json.to_string(), 0i64);
    }

    /// 获取带 keyword 的快照
    pub fn get_keyword(&self, method: &str, keyword: &str, date: &str) -> Option<String> {
        let key = Self::cache_key_with_keyword(method, keyword, date);
        usable(self.disk.get(&key))
    }

    /// 检查带 keyword 的快照是否已缓存
    pub fn has_keyword(&self, method: &str, keyword: &str, date: &str) -> bool {
        let key = Self::cache_key_with_keyword(method, keyword, date);
        usable(self.disk.get(&key)).is_some()
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

    /// G2'（2026-09-27 粤海饲料回放实证）：归属日必须是「最近一个已完成交易日」，
    /// 非交易日会话要能**回填**上一个交易日的快照，而不是整轮跳过。
    /// 场景日期逐字取自日历事实：09-24 周四为最后交易日、09-25 周五中秋休市、09-27 周日。
    #[test]
    fn snapshot_target_date_backfills_last_trading_day() {
        use chrono::TimeZone;
        let at = |y: i32, mo: u32, d: u32, h: u32| {
            chrono::Local.with_ymd_and_hms(y, mo, d, h, 0, 0).unwrap()
        };
        let day_str = |dt: Option<chrono::NaiveDate>| dt.map(|d| d.to_string());

        // 交易日盘中（14:00 < 15:00）⇒ 半日数据，不收
        assert_eq!(day_str(snapshot_target_date(&at(2026, 9, 24, 14))), None);
        // 交易日收盘后 ⇒ 收当天
        assert_eq!(day_str(snapshot_target_date(&at(2026, 9, 24, 15))), Some("2026-09-24".into()));
        // 中秋休市日（上午也不行？休市日 vendor 返回的就是 09-24 完整收盘）⇒ 回填 09-24
        assert_eq!(day_str(snapshot_target_date(&at(2026, 9, 25, 10))), Some("2026-09-24".into()));
        // 周日 19:00 —— 本次实证场景：原实现整轮 skip ⇒ 09-24 快照永远缺
        assert_eq!(day_str(snapshot_target_date(&at(2026, 9, 27, 19))), Some("2026-09-24".into()));
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

    /// G2''（2026-09-27 001313 回放实证）：读取侧的锚点日期必须与采集侧同判据。
    ///
    /// 缺陷形态：G2' 只把**归属日**改成最近已完成交易日，读取侧仍用 `as_of_date` 原值拼
    /// key ⇒ 周日 09-27 的回放读 `daily:{method}:2026-09-27`，永远 miss，
    /// 行业排名继续红在已被本机阻断的 push2\* 合成链上。
    #[test]
    fn non_trading_day_anchor_reads_last_trading_day_snapshot() {
        let cache = make_cache();
        cache.set_snapshot("get_industry_ranking", "2026-09-24", "ranking_0924");
        // 09-25 中秋（周五休市）、09-26 周六、09-27 周日 ⇒ 三个锚点都该回填 09-24 那一条
        for anchor in ["2026-09-25", "2026-09-26", "2026-09-27"] {
            assert_eq!(
                cache.get("get_industry_ranking", anchor).as_deref(),
                Some("ranking_0924"),
                "{anchor} 锚点必须回填上一交易日的快照"
            );
        }
        // 锚点本身是交易日 ⇒ 原值，不串到别的交易日
        assert_eq!(
            cache.get("get_industry_ranking", "2026-09-24").as_deref(),
            Some("ranking_0924")
        );
        assert!(
            cache.get("get_industry_ranking", "2026-09-23").is_none(),
            "归一只允许向后，不得把 09-24 的快照当成 09-23 的数据"
        );
    }

    /// 写入侧同判据：回放里合成结果的回写（锚点=周日）落到 09-24 的键上，
    /// 不留一条谁也读不到的周末孤儿键。
    #[test]
    fn weekend_write_lands_on_last_trading_day_key() {
        let cache = make_cache();
        cache.set("get_cls_flash", "2026-09-27", "flash_0924");
        assert_eq!(cache.get("get_cls_flash", "2026-09-24").as_deref(), Some("flash_0924"));
        assert!(cache.has("get_cls_flash", "2026-09-26"), "判重也必须按交易日归一");
        cache.set_stock_snapshot("get_announcements", "001313", "2026-09-27", "ann_0924");
        assert_eq!(
            cache.get_stock("get_announcements", "001313", "2026-09-25").as_deref(),
            Some("ann_0924"),
            "个股级快照同键空间"
        );
    }

    /// 采集被回放上下文污染时写下的 `"[]"` 不算快照：读取与判重都按 miss 处理，
    /// 否则闸门（`has_daily_snapshot`）会把这一日整轮 skip 掉、**永远修不好**。
    #[test]
    fn empty_result_is_not_a_snapshot() {
        let cache = make_cache();
        for poison in ["[]", "[] ", "", "null"] {
            cache.set("get_sector_info", "2026-10-09", poison);
            assert!(
                cache.get("get_sector_info", "2026-10-09").is_none(),
                "空结果 {poison:?} 必须按 miss 处理"
            );
            assert!(!cache.has("get_sector_info", "2026-10-09"), "判重不得被空快照锁死");
        }
        cache.set("get_sector_info", "2026-10-09", "[1]");
        assert_eq!(cache.get("get_sector_info", "2026-10-09").as_deref(), Some("[1]"));
        assert!(cache.has("get_sector_info", "2026-10-09"), "有内容才算已采到");
    }

    /// 个股级与 keyword 级同样过滤（采集侧的空臂不止全市场方法）。
    #[test]
    fn empty_stock_and_keyword_snapshots_are_misses() {
        let cache = make_cache();
        cache.set_stock_snapshot("get_pledge_data", "001313", "2026-10-09", "[]");
        assert!(cache.get_stock("get_pledge_data", "001313", "2026-10-09").is_none());
        cache.set_keyword_snapshot("search_stock", "粤海", "2026-10-09", "[]");
        assert!(cache.get_keyword("search_stock", "粤海", "2026-10-09").is_none());
        assert!(!cache.has_keyword("search_stock", "粤海", "2026-10-09"));
    }
}
