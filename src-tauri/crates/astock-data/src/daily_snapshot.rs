//! 每日快照归档（P5 的存储升格版：DiskCache JSON 文件 → SQL 历史表，#20② / B-3）
//!
//! 为 NoHistoricalSemantic 方法(热门股/行业排名/概念板块/快讯等)
//! 提供"每日快照"归档。这些数据本身没有历史语义——每日快照是在每天
//! 某个时间点调用 vendor 实时接口获取的"那一刻的今日数据"。
//!
//! 存储：**SQL 历史表 `astock_daily_snapshot`**（实体 `axagent_entities::astock_daily_snapshot`，
//! 主键 `id = "{method}@{scope或'-'}@{date}"`，`(method, snapshot_date)` 建多列索引）。
//! `{date}` 在读写两侧都先归一到「当天或之前最近交易日」（见 `key_date`）——
//! 归档的键空间等于交易日，周末/假日锚点的回放因此能回填上一交易日那条。
//!
//! ## 为什么必须换掉 DiskCache（本批的立档理由，不是性能优化）
//!
//! 旧归档 = 独立 DiskCache 实例 + 单文件 `~/.axagent/astock_daily_snapshot.json`，
//! 全量重写 tmp+rename，上限 10_000 条 + 64 MB 字节预算、超预算按 `last_access` LRU 淘汰：
//!
//! 1. **LRU 会吃掉历史**。K 线条目单条约 70 KB，几条就能把最旧快照挤掉
//!    ⇒ 回放「哪天有、哪天没」，而现场只看得到「没数据」，看不出是被淘汰吃掉的。
//!    缓存的淘汰语义与「归档」的只增不减是**相反的**，共用一层必然互相打架。
//! 2. **结构上取不出跨日聚合**。DiskCache 只有点查 API，**没有 prefix-scan**
//!    ⇒「某方法跨多日的聚合」写不出来。本批判据里必须真产出一条跨日聚合
//!    （见 `DailySnapshotStore::count_by_date_range` 与其消费点 `snapshot_absence_clause`），
//!    否则升格只是换个存储，本批不算闭环。
//! 3. **全量重写单文件**：每次 flush 都整份序列化，归档越大启动越慢。
//!
//! 同类既有正门：`crates/entities/src/market_daily_close.rs`（同一理由建的表）。
//!
//! ## 分层：端口在下层声明、适配器在上层实现
//!
//! `astock-data` 的依赖只有 harness + redb + moka + parking_lot，**没有**
//! entities/dao/sea-orm；而 `analysis-engine → astock-data` 已存在，
//! 给 astock-data 加数据库依赖会把 implementor 层拧成环（AGENTS.md 禁区 1）。
//! 故本文件只声明 trait，实现在 wiring 层
//! `src-tauri/src/init/daily_snapshot_store.rs`（看得见 dao/entities）。
//! 一模一样的先例：`lib.rs` 的 `NewsArchiveSink` + `init/news_archive_sink.rs`。
//! 注入点是 `AStockClient::with_daily_snapshot_store(Arc<dyn DailySnapshotStore>)`。
//!
//! 使用模式:
//! 1. 后台 cron 每小时 tick，采集**最近一个已完成交易日**的快照（归属日判据见
//!    `snapshot_target_date`；非交易日会话可回填上一交易日）
//!    （实现在 `src/init/services.rs::start_daily_snapshot_sweep` →
//!    `commands/stock_analysis.rs::run_daily_snapshot_sweep`，幂等执行）
//! 2. replay 模式遇到 NoHistoricalSemantic 方法,先查每日快照
//! 3. cache miss → 正常走 record_degradation + 返回空(不阻塞回测)
//!
//! 配置:通过 `AStockClient::with_daily_snapshot_store(...)` 注入,默认关闭。
//! 归档即写即落库（SQL upsert），**不再需要后台 flush 任务** —— 旧实现里调用方必须
//! 为快照 DiskCache 起 `spawn_flush_loop`，否则当日快照只停在内存里、进程退出即丢。

use async_trait::async_trait;

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

/// 全市场快照在主键里的 `scope` 占位值。
///
/// 与实体 `astock_daily_snapshot::NULL_SCOPE` 逐字相同 —— 两处各留一份是因为
/// `astock-data` 不依赖 `entities`（分层禁令），端口层需要一个自己的字面量来拼主键。
/// ⚠ 改这里必须同时改实体侧，并由 `init/daily_snapshot_store.rs` 的一致性测试锁住。
pub const FULL_MARKET_SCOPE: &str = "-";

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

/// 归档的**日期键空间只有交易日**：读写两侧的日期一律先归一到
/// 「该日当天或之前最近的交易日」，再拼主键。
///
/// 为什么放在主键构造处而不是各调用方（2026-09-27 001313 回放实证）：采集侧的归属日
/// 已由 `snapshot_target_date` 改成最近已完成交易日（周末会话回填 09-24），但读取侧
/// 仍拿 `as_of_date` **原值** 查 ⇒ 用户把回放锚点选在周日 09-27 时读的是
/// `{method}@-@2026-09-27`，永远 miss，兜底形同不存在，面板继续红在被打断的合成链上。
/// 归一后与 `get_quote` 的休市回退同口径（09-27 → 上一交易日 09-24）；锚点本身是交易日时
/// `previous_trading_day` 返回原值，语义不变。读写共用同一函数 ⇒ 不可能再各走各的判据。
///
/// ⚠ 升格后它仍是**单一函数**，且只在 `row_id` / `count_by_date_range` 的入参归一处出现
/// （适配器侧），不散到调用方。
pub fn key_date(date: &str) -> String {
    match chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") {
        Ok(d) => crate::calendar::previous_trading_day(d).format("%Y-%m-%d").to_string(),
        // 非 YYYY-MM-DD 的调用方（理论上不应有）保持原样，不做二次猜测
        Err(_) => date.to_string(),
    }
}

/// 跨日窗口的起点：从 `to_date`（含）往前数 `n` 个**交易日**，返回那天的 ISO 日期。
///
/// 为什么按交易日而不是自然日倒退（#20② 的类②聚合）：归档的键空间等于交易日
/// （见 `key_date`），若按 30 个自然日去数，长假 + 周末会把窗口里实际可采的天数
/// 虚报到两倍，「N 天里只有 3 天有快照」这句诊断就变成假话。
///
/// 返回 `None` 只发生在 `to_date` 不是 `YYYY-MM-DD` 的情形 —— 调用方据此**省略**
/// 聚合分句，而不是把无法解析的日期当 0 天。
pub fn trading_window_start(to_date: &str, n: usize) -> Option<String> {
    if n == 0 {
        return None;
    }
    let parsed = chrono::NaiveDate::parse_from_str(to_date, "%Y-%m-%d").ok()?;
    // 锚点先归一到交易日（周末锚点的窗口仍以 09-24 为右端），再倒退 n-1 个交易日
    let mut cursor = crate::calendar::previous_trading_day(parsed);
    for _ in 1..n {
        cursor = crate::calendar::previous_trading_day(cursor - chrono::Duration::days(1));
    }
    Some(cursor.format("%Y-%m-%d").to_string())
}

/// 空结果不是快照：`"[]"` / `""` / `"null"` 一律按「这一天没有快照」处理。
///
/// 为什么必须收在**归档读写的唯一出口**而不是各 method 判（2026-09-27 实测）：
/// 采集 09-24 的那一轮里 `{get_industry_ranking}@-@2026-09-24` 与
/// `{get_hot_stocks}@-@2026-09-24` 都落成了字面 `"[]"` —— 后台 tick 的采集任务
/// 没有 task-local 作用域，`current_as_of()` 走**全局栈**兜底，读到当时正在跑的
/// 回放上下文 ⇒ 走了 as-of 分支（探测 push2\* 失败 → `Ok(vec![])`）→ 采集把空列表
/// 当有效快照写盘。而闸门是 `has_daily_snapshot("get_index_quotes", date)`，
/// `"[]"` 也算「已有」⇒ 之后每一轮整轮 skip，**该日的快照永远修不好**。
///
/// 升格后这道判据留在端口层（`pub fn`），由适配器的 `put`（写侧）与 `get_scoped`
/// （读侧）各过一次 ⇒ 中毒条目**既进不了表、也判不出「已有」**，自愈只需一次重采。
/// 跨日聚合（`count_by_date_range`）因此可以诚实地数行 —— 表里根本不会有空快照。
pub fn usable(json: Option<String>) -> Option<String> {
    json.filter(|v| {
        let t = v.trim();
        !t.is_empty() && t != "[]" && t != "null"
    })
}

/// 写入侧的同一判据（`usable` 的布尔形态）：空快照不落表。
pub fn usable_json(json: &str) -> bool {
    usable(Some(json.to_string())).is_some()
}

/// 归档主键（旧 DiskCache 的 `cache_key` / `stock_cache_key` / `cache_key_with_keyword`
/// 三形在此合一：`scope` 一位承载「全市场 / 个股代码 / 关键词」三种作用域）。
///
/// ⚠ 日期归一**只在这里做**：读写两侧都经由本函数拼主键，故不可能出现
/// 「采集写 09-24、回放查 09-27」那种各走各判据的 miss（见 `key_date`）。
pub fn row_id(method: &str, scope: Option<&str>, date: &str) -> String {
    format!("{method}@{}@{}", scope.unwrap_or(FULL_MARKET_SCOPE), key_date(date))
}

/// 每日快照诊断窗口的宽度（**交易日**数）—— 类②跨日聚合用它算覆盖率。
///
/// 为什么定 20：恰好一个自然月，跨过它还能说「近期」；更重要的是采集侧的幂等 tick
/// 每小时判一次「该归属日采过没有」，20 个交易日里一天都没有 ⇒ 不是漏采而是**整条
/// 采集侧断了**（后台任务没起 / 全源失败 / 装配缺失），这两种缺席在
/// `snapshot_absence_clause` 里必须分句。
pub const SNAPSHOT_DIAGNOSTIC_WINDOW: usize = 20;

/// 每日快照归档的**端口**（实现见 wiring 层 `src-tauri/src/init/daily_snapshot_store.rs`）。
///
/// 为什么是 trait 而不是直接用 `entities`/`dao`：见本文件头「分层」一节。
/// 未注入时客户端按「归档未启用」处理 —— 回放仍可跑，只是 NoHistoricalSemantic
/// 维度直接降级并在面板说明「装配缺失」。
#[async_trait]
pub trait DailySnapshotStore: Send + Sync {
    /// 唯一读出口：按 (method, scope, 归一后的 date) 取回快照原文。
    ///
    /// 实现方**必须**在出口过一次 `usable`（这是「空快照不算快照」的唯一落点）。
    async fn get_scoped(&self, method: &str, scope: Option<&str>, date: &str) -> Option<String>;

    /// 存入快照（upsert 覆盖）。实现方须在写前过一次 `usable_json` —— 空快照不落表，
    /// 否则判重闸门会被中毒条目锁死（见 `usable` 的实证）。
    async fn put(&self, method: &str, scope: Option<&str>, date: &str, json: &str);

    /// **跨日聚合**：`[from_date, to_date]` 窗口内该 method 有（非空）快照的
    /// **交易日天数**（去重、不分 scope）。
    ///
    /// 这条方法是本次升格的验收硬条件 —— DiskCache 没有 prefix-scan API，
    /// 这个数在旧存储里**结构上取不出来**。消费点：`AStockClient::snapshot_absence_clause`
    /// 用它把「这一天没有快照」与「整段窗口都没采到（=采集侧断了）」分句说清。
    ///
    /// 实现方须对 `from_date` / `to_date` 同样走 `key_date` 归一（窗口两端都落在
    /// 非交易日是常态：回放锚点选在周末时 `to_date` 就是周日）。
    ///
    /// ## 返回 `Option`：0 天与「查不出来」必须是两个值（#20② 的假诊断修复）
    ///
    /// `Some(n)` = 聚合**查询成功**（`Some(0)` 语义是真的一个都没采到）；
    /// `None` = 聚合查询本身失败 / 不可得（表没被声明式对账建出来、SQL 形态与实体不匹配、
    /// 连接错误……）。
    ///
    /// ⚠ 塌成 `usize` 会造假：本仓表结构走声明式对账（`MIGRATIONS` 为空，正门是实体列属性
    /// + `dao/src/reconcile/extras.rs`），且启动期是 `additive_only` ⇒ **建表失败只在启动日志
    /// warn**。表没建成时查询报错、若按 `unwrap_or_default()` 返回 0，消费点
    /// `snapshot_absence_clause` 就会把「我这句查询失败了」说成「后台采集整段断供」——
    /// 用户去查 sweep 而病根在存储层。这正是本仓反复登记的「拿不到被伪装成正常结果」族。
    /// 与 `trading_window_start` 同选型：无法得到的数就不产出来，由调用方**省略**该分句。
    async fn count_by_date_range(
        &self,
        method: &str,
        from_date: &str,
        to_date: &str,
    ) -> Option<usize>;

    /// 全市场快照：`scope = None`。
    async fn get(&self, method: &str, date: &str) -> Option<String> {
        self.get_scoped(method, None, date).await
    }

    /// 个股级快照：`scope = Some(stock_code)`。
    ///
    /// **为什么必须有这个方法**：旧 DiskCache 时代写入带股票代码的 key，
    /// 读取侧却只有不带 code 的 `get` ⇒ 个股级快照**只写无读**，
    /// 采集回来的两融/资金流/北向数据在回放模式下一条也取不到（2026-09-25 补）。
    async fn get_stock(&self, method: &str, stock_code: &str, date: &str) -> Option<String> {
        self.get_scoped(method, Some(stock_code), date).await
    }

    /// 带 keyword 的快照（`search_stock` 这类结果与 keyword 强相关的方法）。
    ///
    /// C5.3 修复：旧实现单独发明了 `cache_key_with_keyword`，与个股级同形但不同名 ⇒
    /// 两套 key 互相覆盖。这里让它**复用 scope 位**（keyword 与 stock_code 在键空间里
    /// 本来就是一个东西：「这一行的作用域」），但保留独立方法名以免调用方混淆。
    async fn get_keyword(&self, method: &str, keyword: &str, date: &str) -> Option<String> {
        self.get_scoped(method, Some(keyword), date).await
    }

    /// 特定作用域是否已有快照（供后台 sweep 判重，避免同一天重复打 vendor）。
    ///
    /// 走 `get_scoped` ⇒ 天然继承 `usable` 过滤，判重不会被空快照锁死。
    async fn contains(&self, method: &str, scope: Option<&str>, date: &str) -> bool {
        self.get_scoped(method, scope, date).await.is_some()
    }
}

#[cfg(test)]
pub(crate) mod test_store {
    use super::*;
    use parking_lot::Mutex;
    use std::collections::HashMap;

    /// 归档端口的内存替身（生产实现是 wiring 层的 SQL 适配器）。
    ///
    /// SAFETY: `parking_lot::Mutex` 的 guard 非 Send，编译器会阻止跨 await 持有，
    /// 而本替身的每个临界区内**只有** HashMap 同步操作、不调任何 await
    /// —— 与 `disk_cache.rs` / `calendar.rs` 同选型（不用 `std::sync::Mutex` 是为了
    /// 避开 `clippy::disallowed_types`，不用 `tokio::sync::Mutex` 是因为这里并不需要
    /// 异步锁语义）。
    #[derive(Default)]
    pub(crate) struct MemorySnapshotStore {
        rows: Mutex<HashMap<String, String>>,
        /// 记录 `put` 被 `usable_json` 挡掉的次数（证明写侧闸门真的生效）
        rejected: Mutex<usize>,
        /// 让 `count_by_date_range` 报「查不出来」（`None`）的开关，见 `fail_aggregate`。
        fail_aggregate: Mutex<bool>,
    }

    impl MemorySnapshotStore {
        pub(crate) fn new() -> Self {
            Self::default()
        }

        pub(crate) fn rejected(&self) -> usize {
            *self.rejected.lock()
        }

        pub(crate) fn rows(&self) -> usize {
            self.rows.lock().len()
        }

        /// **模拟聚合查询失败**（生产里对应 SQL 报错 —— 表没建成 / SQL 形态不匹配 / 连接故障）。
        ///
        /// 内存 HashMap 本身不可能「查询失败」，故用这个开关把端口新增的 `None` 语义演出来，
        /// 让消费点 `snapshot_absence_clause` 的那条「不得把查不出来说成断供」负控真的可测。
        pub(crate) fn fail_aggregate(&self) {
            *self.fail_aggregate.lock() = true;
        }

        /// **绕过写侧闸门**直接塞一行（模拟存量数据里已经存在的中毒条目 ——
        /// 旧 DiskCache 时代就是「写进去、读的时候才滤」，回填后表里可能有这种行）。
        pub(crate) fn inject_raw(&self, method: &str, scope: Option<&str>, date: &str, j: &str) {
            self.rows.lock().insert(row_id(method, scope, date), j.to_string());
        }
    }

    #[async_trait]
    impl DailySnapshotStore for MemorySnapshotStore {
        async fn get_scoped(
            &self,
            method: &str,
            scope: Option<&str>,
            date: &str,
        ) -> Option<String> {
            let id = row_id(method, scope, date);
            let raw = self.rows.lock().get(&id).cloned();
            usable(raw)
        }

        async fn put(&self, method: &str, scope: Option<&str>, date: &str, json: &str) {
            if !usable_json(json) {
                *self.rejected.lock() += 1;
                return;
            }
            self.rows.lock().insert(row_id(method, scope, date), json.to_string());
        }

        async fn count_by_date_range(
            &self,
            method: &str,
            from_date: &str,
            to_date: &str,
        ) -> Option<usize> {
            // 开关优先：模拟适配器侧 SQL 报错那条路径 ⇒ 聚合「不可得」而不是 0 天
            if *self.fail_aggregate.lock() {
                return None;
            }
            let from = key_date(from_date);
            let to = key_date(to_date);
            let prefix = format!("{method}@");
            let mut days: std::collections::HashSet<String> = std::collections::HashSet::new();
            for (id, payload) in self.rows.lock().iter() {
                if !id.starts_with(&prefix) {
                    continue;
                }
                if usable(Some(payload.clone())).is_none() {
                    continue;
                }
                // 主键从右往左切：`{method}@{scope}@{date}`，scope 可能含 '@' 之外的
                // 任意字符（关键词），故不能用 split
                let Some((_, date)) = id.rsplit_once('@') else { continue };
                if date >= from.as_str() && date <= to.as_str() {
                    days.insert(date.to_string());
                }
            }
            Some(days.len())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_store::MemorySnapshotStore;
    use super::*;

    async fn put(m: &MemorySnapshotStore, method: &str, scope: Option<&str>, date: &str, j: &str) {
        m.put(method, scope, date, j).await;
    }

    #[test]
    fn row_id_covers_three_scope_shapes() {
        // 三种旧 key 形态在同一主键域里的对应关系（升格前它们是三个不同的私有函数）
        assert_eq!(row_id("get_hot_stocks", None, "2026-06-01"), "get_hot_stocks@-@2026-06-01");
        assert_eq!(
            row_id("get_margin_data", Some("600519"), "2026-06-01"),
            "get_margin_data@600519@2026-06-01"
        );
        assert_eq!(
            row_id("search_stock", Some("贵州茅台"), "2026-06-01"),
            "search_stock@贵州茅台@2026-06-01"
        );
    }

    #[tokio::test]
    async fn put_get_roundtrip() {
        let m = MemorySnapshotStore::new();
        let data =
            r#"[{"stock_code":"000001","stockName":"平安银行","price":10.0,"changePct":1.0}]"#;
        put(&m, "get_hot_stocks", None, "2026-06-01", data).await;
        let back = m.get("get_hot_stocks", "2026-06-01").await;
        assert!(back.as_deref().unwrap_or_default().contains("000001"));
    }

    #[tokio::test]
    async fn contains_and_miss() {
        let m = MemorySnapshotStore::new();
        assert!(!m.contains("get_hot_stocks", None, "2026-06-01").await);
        put(&m, "get_hot_stocks", None, "2026-06-01", "test").await;
        assert!(m.contains("get_hot_stocks", None, "2026-06-01").await);
        assert!(m.get("get_hot_stocks", "2099-01-01").await.is_none());
    }

    #[tokio::test]
    async fn different_dates_and_methods_do_not_collide() {
        let m = MemorySnapshotStore::new();
        put(&m, "get_hot_stocks", None, "2026-06-01", "data_jun1").await;
        put(&m, "get_hot_stocks", None, "2026-06-02", "data_jun2").await;
        put(&m, "get_industry_ranking", None, "2026-06-01", "industry").await;
        assert_eq!(m.get("get_hot_stocks", "2026-06-01").await.as_deref(), Some("data_jun1"));
        assert_eq!(m.get("get_hot_stocks", "2026-06-02").await.as_deref(), Some("data_jun2"));
        assert_eq!(m.get("get_industry_ranking", "2026-06-01").await.as_deref(), Some("industry"));
    }

    #[tokio::test]
    async fn upsert_overwrites_same_key() {
        // 主键即天然去重：同一 (method, scope, 交易日) 重复采集必须覆盖，不留第二行
        let m = MemorySnapshotStore::new();
        put(&m, "get_hot_stocks", None, "2026-06-01", "first").await;
        put(&m, "get_hot_stocks", None, "2026-06-01", "second").await;
        assert_eq!(m.rows(), 1, "重复采集不得产生第二行");
        assert_eq!(m.get("get_hot_stocks", "2026-06-01").await.as_deref(), Some("second"));
    }

    #[test]
    fn snapshot_methods_list_contains_expected() {
        assert!(SNAPSHOT_METHODS.contains(&"get_hot_stocks"));
        assert!(SNAPSHOT_METHODS.contains(&"get_industry_ranking"));
        assert!(SNAPSHOT_METHODS.contains(&"get_cls_flash"));
        assert!(SNAPSHOT_METHODS.contains(&"get_concept_blocks"));
        assert!(SNAPSHOT_METHODS.contains(&"search_stock"));
        assert!(SNAPSHOT_METHODS.contains(&"get_sector_info"));
    }

    /// 类②聚合的窗口宽度判据：**按交易日**倒退，锚点先归一。
    ///
    /// 反面形态是按自然日倒退 —— 长假 + 周末会把窗口里可采的天数虚报到两倍，
    /// 「20 天里只有 3 天有快照 ⇒ 采集侧断了」这句诊断就变成假话。
    /// 日历事实逐字取自上面的 G2' 用例：09-24 周四是最后交易日、09-25 周五中秋休市。
    #[test]
    fn trading_window_start_walks_trading_days_not_calendar_days() {
        // n=1 ⇒ 窗口右端自己（周末锚点先归一到 09-24）
        assert_eq!(trading_window_start("2026-09-27", 1).as_deref(), Some("2026-09-24"));
        assert_eq!(trading_window_start("2026-09-24", 1).as_deref(), Some("2026-09-24"));
        // n=2 ⇒ 跨过 09-24 再退一个交易日 = 09-23 周三
        assert_eq!(trading_window_start("2026-09-27", 2).as_deref(), Some("2026-09-23"));
        // 跨周末：从 09-23 退 1 个交易日 ⇒ 09-21 周一（09-22 周二其实也开市，故 n=2 才到 09-21）
        assert_eq!(trading_window_start("2026-09-23", 2).as_deref(), Some("2026-09-22"));
        assert_eq!(trading_window_start("2026-09-23", 3).as_deref(), Some("2026-09-21"));
        // 无法解析 / n=0 ⇒ None（调用方据此省略聚合分句，而不是当 0 天）
        assert_eq!(trading_window_start("不是日期", 5), None);
        assert_eq!(trading_window_start("2026-09-24", 0), None);
        // 默认窗口宽度变了要说出来（诊断文案里的 N 与它同源）
        assert_eq!(SNAPSHOT_DIAGNOSTIC_WINDOW, 20);
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

    #[tokio::test]
    async fn keyword_snapshot_isolation() {
        // C5.3: 不同 keyword 的快照应互相隔离（scope 位承载 keyword）
        let m = MemorySnapshotStore::new();
        put(&m, "search_stock", Some("贵州茅台"), "2026-06-01", "maotai_result").await;
        put(&m, "search_stock", Some("比亚迪"), "2026-06-01", "byd_result").await;

        assert_eq!(
            m.get_keyword("search_stock", "贵州茅台", "2026-06-01").await.as_deref(),
            Some("maotai_result")
        );
        assert_eq!(
            m.get_keyword("search_stock", "比亚迪", "2026-06-01").await.as_deref(),
            Some("byd_result")
        );
        assert!(
            m.get_keyword("search_stock", "未知股票", "2026-06-01").await.is_none(),
            "未缓存的 keyword 应返回 None"
        );
    }

    /// 回归（2026-09-25）：个股级快照必须可读回。
    ///
    /// 缺陷形态：写入带股票代码的 key，读取侧却只有不带 code 的 `get` ⇒ 回放模式
    /// 一条也取不到（`sweep_daily_snapshots` 逐只采集看起来"有数据"，实际是只写无读的死键）。
    #[tokio::test]
    async fn stock_snapshot_roundtrip() {
        let m = MemorySnapshotStore::new();
        put(&m, "get_margin_data", Some("600519"), "2026-06-01", r#"{"date":"2026-06-01"}"#).await;

        assert!(
            m.get_stock("get_margin_data", "600519", "2026-06-01").await.is_some(),
            "个股级快照必须能按 code + date 读回"
        );
        assert!(
            m.get_stock("get_margin_data", "000001", "2026-06-01").await.is_none(),
            "不同 code 不得串读"
        );
        assert!(
            m.get("get_margin_data", "2026-06-01").await.is_none(),
            "不带 code 的全市场主键读不到个股快照（这正是缺陷形态）"
        );
    }

    /// G2''（2026-09-27 001313 回放实证）：读取侧的锚点日期必须与采集侧同判据。
    ///
    /// 缺陷形态：G2' 只把**归属日**改成最近已完成交易日，读取侧仍用 `as_of_date` 原值拼
    /// 主键 ⇒ 周日 09-27 的回放读 `get_industry_ranking@-@2026-09-27`，永远 miss，
    /// 行业排名继续红在已被本机阻断的 push2\* 合成链上。
    #[tokio::test]
    async fn non_trading_day_anchor_reads_last_trading_day_snapshot() {
        let m = MemorySnapshotStore::new();
        put(&m, "get_industry_ranking", None, "2026-09-24", "ranking_0924").await;
        // 09-25 中秋（周五休市）、09-26 周六、09-27 周日 ⇒ 三个锚点都该回填 09-24 那一条
        for anchor in ["2026-09-25", "2026-09-26", "2026-09-27"] {
            assert_eq!(
                m.get("get_industry_ranking", anchor).await.as_deref(),
                Some("ranking_0924"),
                "{anchor} 锚点必须回填上一交易日的快照"
            );
        }
        // 锚点本身是交易日 ⇒ 原值，不串到别的交易日
        assert_eq!(
            m.get("get_industry_ranking", "2026-09-24").await.as_deref(),
            Some("ranking_0924")
        );
        assert!(
            m.get("get_industry_ranking", "2026-09-23").await.is_none(),
            "归一只允许向后，不得把 09-24 的快照当成 09-23 的数据"
        );
    }

    /// 写入侧同判据：回放里合成结果的回写（锚点=周日）落到 09-24 的主键上，
    /// 不留一条谁也读不到的周末孤儿行。
    #[tokio::test]
    async fn weekend_write_lands_on_last_trading_day_key() {
        let m = MemorySnapshotStore::new();
        put(&m, "get_cls_flash", None, "2026-09-27", "flash_0924").await;
        assert_eq!(m.get("get_cls_flash", "2026-09-24").await.as_deref(), Some("flash_0924"));
        assert!(m.contains("get_cls_flash", None, "2026-09-26").await, "判重也必须按交易日归一");
        put(&m, "get_announcements", Some("001313"), "2026-09-27", "ann_0924").await;
        assert_eq!(
            m.get_stock("get_announcements", "001313", "2026-09-25").await.as_deref(),
            Some("ann_0924"),
            "个股级快照同键空间"
        );
    }

    /// 采集被回放上下文污染时写下的 `"[]"` 不算快照：**写侧直接不落表**，
    /// 于是读取与判重都按 miss 处理，闸门（`has_daily_snapshot`）不会把这一日整轮
    /// skip 掉、**永远修不好**。
    #[tokio::test]
    async fn empty_result_is_not_a_snapshot() {
        let m = MemorySnapshotStore::new();
        for poison in ["[]", "[] ", "", "null"] {
            put(&m, "get_sector_info", None, "2026-10-09", poison).await;
            assert!(
                m.get("get_sector_info", "2026-10-09").await.is_none(),
                "空结果 {poison:?} 必须按 miss 处理"
            );
            assert!(
                !m.contains("get_sector_info", None, "2026-10-09").await,
                "判重不得被空快照锁死"
            );
        }
        assert_eq!(m.rows(), 0, "中毒条目根本不得进表（写侧闸门）");
        assert_eq!(m.rejected(), 4, "四次写全被 usable_json 挡掉");

        put(&m, "get_sector_info", None, "2026-10-09", "[1]").await;
        assert_eq!(m.get("get_sector_info", "2026-10-09").await.as_deref(), Some("[1]"));
        assert!(m.contains("get_sector_info", None, "2026-10-09").await, "有内容才算已采到");
    }

    /// 个股级与 keyword 级同样过滤（采集侧的空臂不止全市场方法）。
    #[tokio::test]
    async fn empty_stock_and_keyword_snapshots_are_misses() {
        let m = MemorySnapshotStore::new();
        put(&m, "get_pledge_data", Some("001313"), "2026-10-09", "[]").await;
        assert!(m.get_stock("get_pledge_data", "001313", "2026-10-09").await.is_none());
        put(&m, "search_stock", Some("粤海"), "2026-10-09", "[]").await;
        assert!(m.get_keyword("search_stock", "粤海", "2026-10-09").await.is_none());
        assert!(!m.contains("search_stock", Some("粤海"), "2026-10-09").await);
        assert_eq!(m.rows(), 0, "两类空快照都不落表");
    }

    /// 类②（本批验收硬条件）：**跨日聚合**真的取得出来 —— DiskCache 没有 prefix-scan，
    /// 这个数在旧存储里结构上取不出来。
    #[tokio::test]
    async fn count_by_date_range_aggregates_across_days() {
        let m = MemorySnapshotStore::new();
        // 09-21 周一 ~ 09-24 周四（09-25 中秋休市）四个交易日里有三天有快照
        for d in ["2026-09-21", "2026-09-22", "2026-09-24"] {
            put(&m, "get_industry_ranking", None, d, "[{\"industry_name\":\"半导体\"}]").await;
        }
        // 别的 method 不得串数
        put(&m, "get_hot_stocks", None, "2026-09-21", "[{\"stock_code\":\"000001\"}]").await;
        // 个股级同 method 的多只票在同一天只算**一天**（聚合口径 = 天数，不是行数）
        put(&m, "get_industry_ranking", Some("600519"), "2026-09-24", "[1]").await;

        assert_eq!(
            m.count_by_date_range("get_industry_ranking", "2026-09-21", "2026-09-24").await,
            Some(3),
            "窗口内三个交易日有快照"
        );
        // 窗口两端落在非交易日 ⇒ 归一后仍按交易日键空间数（09-26 周六 → 09-24）
        assert_eq!(
            m.count_by_date_range("get_industry_ranking", "2026-09-25", "2026-09-27").await,
            Some(1),
            "休市日窗口归一到 09-24，只有一天"
        );
        assert_eq!(
            m.count_by_date_range("get_hot_stocks", "2026-09-21", "2026-09-24").await,
            Some(1)
        );
        assert_eq!(
            m.count_by_date_range("get_cls_flash", "2026-09-21", "2026-09-24").await,
            Some(0),
            "什么都没采到是 Some(0)（查询成功），不是 None（查不出来）"
        );

        // 替身的「查不出来」开关（消费点那条负控依赖它，先在这里自证它生效）
        m.fail_aggregate();
        assert_eq!(
            m.count_by_date_range("get_industry_ranking", "2026-09-21", "2026-09-24").await,
            None,
            "开关打开后聚合必须报不可得，而不是退回 Some(0)"
        );
    }

    /// 负控：跨日聚合**不得**把中毒条目数成一天。
    ///
    /// 生产上 `put` 已经把这些值挡在表外（见 `empty_result_is_not_a_snapshot`），
    /// 但**存量回填**搬的是旧 DiskCache 文件里的历史条目 —— 那个年代是
    /// 「写进去、读的时候才滤」，于是表里可能出现中毒行。聚合与读取都必须仍判 miss。
    #[tokio::test]
    async fn count_by_date_range_ignores_poisoned_rows() {
        let m = MemorySnapshotStore::new();
        // 绕过写侧闸门，模拟存量中毒行
        m.inject_raw("get_cls_flash", None, "2026-09-21", "[]");
        m.inject_raw("get_cls_flash", None, "2026-09-22", "");
        m.inject_raw("get_cls_flash", Some("600519"), "2026-09-23", "null");
        assert_eq!(
            m.count_by_date_range("get_cls_flash", "2026-09-21", "2026-09-23").await,
            Some(0),
            "三行全是中毒值 ⇒ 一天都不能数进去"
        );
        assert!(m.get("get_cls_flash", "2026-09-21").await.is_none(), "读取出口同样判 miss");
        assert!(!m.contains("get_cls_flash", None, "2026-09-21").await, "判重同样不被锁死");

        m.inject_raw("get_cls_flash", None, "2026-09-22", "[1]");
        assert_eq!(
            m.count_by_date_range("get_cls_flash", "2026-09-21", "2026-09-23").await,
            Some(1)
        );
    }
}
