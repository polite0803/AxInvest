// SPDX-License-Identifier: AGPL-3.0-only

//! 每日快照归档的 **SQL 适配器**（#20② / B-3，2026-10-08）。
//!
//! `axagent_astock_data::daily_snapshot::DailySnapshotStore` 是声明在下层的**端口**
//! （`astock-data` 只依赖 harness + redb + moka + parking_lot，没有 entities/dao/sea-orm，
//! 给它加数据库依赖会把 implementor 层拧成环 —— AGENTS.md 禁区 1）。本文件是它在
//! wiring 层的实现：这里同时看得见 `axagent_dao` / `axagent_entities` / `sea-orm`。
//! 一模一样的先例：`init/news_archive_sink.rs`（实现 `lib.rs:85` 的 `NewsArchiveSink`），
//! 注入点 `AStockClient::with_daily_snapshot_store`（装配在 `init/state.rs`）。
//!
//! ## 为什么把归档从 DiskCache 换成 SQL 表（不是性能优化，是能力缺口）
//!
//! 1. 旧归档是 DiskCache（LRU 10_000 条 + 64 MB 预算），**K 线条目单条约 70 KB**，
//!    几条就能把最旧快照按 `last_access` 挤掉 ⇒ 回放「哪天有、哪天没」。
//!    归档要「只增不减」，缓存要「淘汰」，同一层必然打架。
//! 2. DiskCache **没有 prefix-scan API** ⇒「某方法跨多日的聚合」结构上取不出来。
//!    本表的 `count_by_date_range` 就是一次走 `(method, snapshot_date)` 多列索引的 SQL
//!    （索引声明在 `crates/dao/src/reconcile/extras.rs` 的 `MULTI_COL_INDEXES`）。
//!    消费点：`AStockClient::snapshot_absence_clause`（呈现/诊断侧，不动决策数值）。
//! 3. DiskCache 每次落盘都是**全量重写**单文件。
//!
//! 同族的既有正门：`crates/entities/src/market_daily_close.rs` +
//! `crates/analysis-engine/src/market_close_store.rs`（文件头写的就是同一句理由）。
//!
//! ## 空快照的唯一出口
//!
//! `daily_snapshot::usable` 在本适配器里过两次：**写侧** `put` 直接拒绝落表，
//! **读侧** `get_scoped` 判 miss。历史缺陷是只在读侧滤（旧 DiskCache 形态），于是
//! 中毒条目把判重闸门 `has_daily_snapshot` 锁死 ⇒ 那一天永远修不好（2026-09-27 实证）。
//! 日期归一（`daily_snapshot::key_date`）同样只在 `row_id` 一处发生，读写同判据。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QuerySelect,
};

use axagent_astock_data::daily_snapshot::{self, DailySnapshotStore};
use axagent_entities::astock_daily_snapshot;

/// 表主键里全市场作用域的占位值 —— 端口侧与实体侧必须逐字一致。
///
/// 两处各留一份字面量是分层禁令的结果（`astock-data` 不依赖 `entities`）；
/// 本常量把二者钉在一起，`full_market_scope_placeholder_matches_entity` 那条测试
/// 就是这道同步契约的门（漏改会让读写各算各的主键 ⇒ 恒 miss 且不报错）。
const SCOPE_PLACEHOLDER: &str = astock_daily_snapshot::NULL_SCOPE;

/// 归档的 sea-orm 实现。
pub struct SqlDailySnapshotStore {
    db: DatabaseConnection,
}

impl SqlDailySnapshotStore {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// (method, scope) → 主键（日期归一在 `row_id` 内部完成，读写同一路径）。
    fn id_of(method: &str, scope: Option<&str>, date: &str) -> String {
        daily_snapshot::row_id(method, scope, date)
    }
}

#[async_trait::async_trait]
impl DailySnapshotStore for SqlDailySnapshotStore {
    async fn get_scoped(&self, method: &str, scope: Option<&str>, date: &str) -> Option<String> {
        let id = Self::id_of(method, scope, date);
        let row = astock_daily_snapshot::Entity::find_by_id(&id)
            .one(&self.db)
            .await
            .map_err(|e| {
                tracing::warn!("[daily-snapshot] 读取失败({id}): {e}");
                e
            })
            .ok()
            .flatten();
        // 读侧唯一出口的空快照判据（存量回填可能带进旧时代的 `"[]"` 行）
        daily_snapshot::usable(row.map(|r| r.payload))
    }

    async fn put(&self, method: &str, scope: Option<&str>, date: &str, json: &str) {
        // 写侧闸门：空快照**不落表**。旧 DiskCache 是「写进去、读时滤」，
        // 于是 has_daily_snapshot 被中毒条目锁死、该日永远修不好。
        if !daily_snapshot::usable_json(json) {
            tracing::debug!(
                "[daily-snapshot] 空快照不落表: {method}/{} /{date}",
                scope.unwrap_or(SCOPE_PLACEHOLDER)
            );
            return;
        }
        let collected_at = chrono::Utc::now().timestamp_millis();
        let am = astock_daily_snapshot::ActiveModel {
            id: Set(Self::id_of(method, scope, date)),
            method: Set(method.to_string()),
            scope: Set(scope.unwrap_or(SCOPE_PLACEHOLDER).to_string()),
            snapshot_date: Set(daily_snapshot::key_date(date)),
            payload: Set(json.to_string()),
            collected_at: Set(collected_at),
        };
        // 主键冲突 = 同一归属日补跑 ⇒ 覆盖 payload 与采集时刻（幂等，upsert 而非报错）
        let res = astock_daily_snapshot::Entity::insert(am)
            .on_conflict(
                OnConflict::column(astock_daily_snapshot::Column::Id)
                    .update_columns([
                        astock_daily_snapshot::Column::Payload,
                        astock_daily_snapshot::Column::CollectedAt,
                    ])
                    .to_owned(),
            )
            .exec(&self.db)
            .await;
        if let Err(e) = res {
            tracing::warn!("[daily-snapshot] {method} 落库失败: {e}");
        }
    }

    /// 跨日聚合：一次 SQL（`method` + 日期区间 + 只取 `snapshot_date` 列），
    /// 走 `(method, snapshot_date)` 多列索引；天数在 Rust 侧去重
    /// （窗口 ≤ 几十个交易日、行数 = 交易日 × 作用域，且**不取 payload 列** ⇒ 内存有界）。
    ///
    /// SQL 里的 `payload NOT IN (...)` 是**第二道保险**：权威判据是 `usable`
    /// （它还做 trim），写侧闸门与回填都已过它，正常情况下这条谓词恒真。
    ///
    /// ## 为什么返回 `Option`（#20② 假诊断修复，别改回 `unwrap_or_default()`）
    ///
    /// 本仓表结构走声明式对账（`MIGRATIONS` 为空，正门是实体列属性 + `extras.rs`），
    /// 启动期还是 `additive_only` ⇒ **建表失败只 warn 进启动日志**、不阻断启动。
    /// 那种情形下这条 SQL 会直接报错，而报错的含义是「**我这句查询失败了**」，
    /// 不是「**后台采集整段断供**」—— 旧的 `unwrap_or_default()` 把前者塌成 0，
    /// 消费点 `snapshot_absence_clause` 就照着 0 输出「0/20 ⇒ 采集侧整段断供，重跑那一天
    /// 也解不了」，用户去查 sweep 永远查不到病根（病根在存储层）。
    /// 故：查询成功 ⇒ `Some(n)`（**含 `Some(0)`**，那是真的没采到）；出错 ⇒ `None`。
    async fn count_by_date_range(
        &self,
        method: &str,
        from_date: &str,
        to_date: &str,
    ) -> Option<usize> {
        let from = daily_snapshot::key_date(from_date);
        let to = daily_snapshot::key_date(to_date);
        let days = astock_daily_snapshot::Entity::find()
            .filter(astock_daily_snapshot::Column::Method.eq(method))
            .filter(astock_daily_snapshot::Column::SnapshotDate.gte(from.as_str()))
            .filter(astock_daily_snapshot::Column::SnapshotDate.lte(to.as_str()))
            .filter(
                astock_daily_snapshot::Column::Payload.is_not_in(["[]", "", "null", "[] ", " []"]),
            )
            .select_only()
            .column(astock_daily_snapshot::Column::SnapshotDate)
            .into_tuple::<String>()
            .all(&self.db)
            .await
            .map_err(|e| {
                // 诊断把三个入参点名（归一前后的日期都给）—— 界面只会说「聚合本次不可得」，
                // 现场要定位「哪个 method、哪个窗口、表到底在不在」全靠这一行日志。
                tracing::warn!(
                    "[daily-snapshot] 跨日聚合查询失败: method={method} \
                     from={from_date}..to={to_date}（归一后 {from}..{to}）: {e} \
                     ⇒ 消费侧按「聚合不可得」处理，不当成 0 天（0 天会被说成采集侧断供）。\
                     若该表尚未建成，请查启动期声明式对账日志。"
                );
                e
            })
            .ok()?;
        let distinct: HashSet<String> = days.into_iter().collect();
        Some(distinct.len())
    }
}

// ═══════════════════════════ 存量回填（一次性、幂等） ═══════════════════════════

/// 旧 DiskCache 归档的文件名（`init/state.rs` 退役前用的就是这个名字）。
pub const LEGACY_SNAPSHOT_FILENAME: &str = "astock_daily_snapshot.json";

/// 一次回填的读数（全部只进日志；界面不消费它）。
#[derive(Debug, Default)]
pub struct LegacyBackfillReport {
    /// 旧文件里解析出的 `daily:` 条目总数
    pub scanned: usize,
    /// 真正搬进表的条数
    pub moved: usize,
    /// 表里该主键已有 ⇒ 跳过（幂等，重复启动不再搬）
    pub skipped_present: usize,
    /// 空快照（`"[]"` / `""` / `"null"`）⇒ 不搬，让采集侧下一轮重采
    pub skipped_poison: usize,
    /// 形状不符（不是 2/3 段）而无法解析的条目
    pub skipped_malformed: usize,
    /// 搬完后旧文件改名到这里（**保留、不删**）；None = 没有旧文件，无需回填
    pub renamed_to: Option<PathBuf>,
}

/// 把旧 DiskCache 单文件 `astock_daily_snapshot.json` 一次性搬进 SQL 表。
///
/// **为什么必须回填**：不回填的话，升格当天回放覆盖率从「有」直接跳「空」——
/// 旧文件里那几个月的快照读不到了，而新表是空的。这是一次**能力倒退**，
/// 比「换存储慢一点」严重得多。
///
/// 形状（裁定）：装配期一次性自动迁移，不做成需要前端调用的命令 ——
/// 本仓对「helper 进树无人调用」判为不闭环，做成命令而无人调更糟。
/// 调用点在 `init/state.rs::run_deferred_init`（首帧之后的后台初始化）。
///
/// 幂等判据：`put` 走主键 upsert，且**先查表内该键是否已有**才搬 ⇒ 重复运行
/// 不覆盖新采集的结果（新表才是权威）。搬完把旧文件改名 `.migrated-<日期>`
/// **保留、不删** —— 删掉就没有回退余地了。任何失败只 `warn`，不阻断启动。
pub async fn backfill_legacy_snapshot_file(
    db: &DatabaseConnection,
    legacy_path: &Path,
) -> Result<LegacyBackfillReport, String> {
    let mut rep = LegacyBackfillReport::default();
    if !legacy_path.exists() {
        return Ok(rep); // 全新安装 / 已回填过（文件已改名）⇒ 零成本返回
    }
    let store = SqlDailySnapshotStore::new(db.clone());
    let text = std::fs::read_to_string(legacy_path)
        .map_err(|e| format!("读取旧归档 {} 失败: {e}", legacy_path.display()))?;
    // 旧 DiskCache 的落盘形状：`{"entries": {"<key>": {"value": "<json>", "expires_at": 0,
    // "last_access": <秒>}}}`。这里**不复用** `DiskCache` 的私有 `CacheEntry` 类型
    // （禁区 12 讲的是「别重复定义同一套模型」，而 `CacheEntry` 是那个 crate 的私有实现细节，
    // 端口层读不到）⇒ 直接按 JSON 值取 `entries`/`value` 两个键，零平行类型。
    let root: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("旧归档不是合法 JSON: {e}"))?;
    let Some(entries) = root.get("entries").and_then(|v| v.as_object()) else {
        return Err("旧归档里没有 entries 字段，形状不是 DiskCache 快照文件".to_string());
    };

    for (key, entry) in entries {
        // 旧三种 key 形态：`daily:{method}:{date}` / `daily:{method}:{code}:{date}`
        //              / `daily:{method}:{keyword}:{date}`
        let Some(rest) = key.strip_prefix("daily:") else { continue };
        rep.scanned += 1;
        let segs: Vec<&str> = rest.split(':').collect();
        let (method, scope, date) = match segs.len() {
            2 => (segs[0], None, segs[1]),
            3 => (segs[0], Some(segs[1]), segs[2]),
            _ => {
                rep.skipped_malformed += 1;
                continue;
            },
        };
        let Some(payload) = entry.get("value").and_then(|v| v.as_str()) else {
            rep.skipped_malformed += 1;
            continue;
        };
        if !daily_snapshot::usable_json(payload) {
            rep.skipped_poison += 1;
            continue;
        }
        if store.contains(method, scope, date).await {
            rep.skipped_present += 1;
            continue;
        }
        store.put(method, scope, date, payload).await;
        rep.moved += 1;
    }

    // 搬完改名保留（不删）：留一条可回退的路，同时让下一次启动零成本跳过
    let stamp = chrono::Local::now().format("%Y-%m-%d");
    let Some(name) = legacy_path.file_name().and_then(|n| n.to_str()) else {
        return Ok(rep);
    };
    let target = legacy_path.with_file_name(format!("{name}.migrated-{stamp}"));
    match std::fs::rename(legacy_path, &target) {
        Ok(()) => rep.renamed_to = Some(target),
        Err(e) => tracing::warn!("[daily-snapshot] 旧归档改名失败（不影响已回填数据）: {e}"),
    }
    Ok(rep)
}

/// 启动装配处的一行式调用：失败只 warn，绝不阻断启动。
///
/// 单独抽出来是为了让 `run_deferred_init` 里那句读起来就是「一次回填 + 一条日志」，
/// 而不是一坨 `match`。
pub async fn backfill_legacy_snapshot_at_startup(app_dir: &Path, db: &DatabaseConnection) {
    let legacy = app_dir.join(LEGACY_SNAPSHOT_FILENAME);
    match backfill_legacy_snapshot_file(db, &legacy).await {
        Ok(rep) if rep.scanned == 0 && rep.renamed_to.is_none() => {
            // 没有旧文件（全新安装或已回填过）⇒ 不打日志，免得每次启动刷一行废话
        },
        Ok(rep) => tracing::info!(
            "[daily-snapshot] 旧 DiskCache 归档回填完成: 解析 {} 条, 搬入 {} 条, 表内已有 {} 条, \
             空快照丢弃 {} 条, 形状异常 {} 条, 旧文件改名为 {:?}",
            rep.scanned,
            rep.moved,
            rep.skipped_present,
            rep.skipped_poison,
            rep.skipped_malformed,
            rep.renamed_to.as_ref().map(|p| p.file_name().and_then(|n| n.to_str()).unwrap_or(""))
        ),
        Err(e) => tracing::warn!("[daily-snapshot] 旧归档回填失败（不阻断启动，新表继续用）: {e}"),
    }
}

/// 便捷构造：把适配器装进 `Arc<dyn ...>` 交给 `with_daily_snapshot_store`。
pub fn arc_store(db: &DatabaseConnection) -> Arc<dyn DailySnapshotStore> {
    Arc::new(SqlDailySnapshotStore::new(db.clone()))
}

#[cfg(test)]
mod tests {
    // 只在测试里用：本适配器的读路径全部走 `find_by_id` / `into_tuple`，不需要计数；
    // 而「表被声明式对账建出来了」那条断言要用 `.count(db)`（属 `PaginatorTrait`）。
    use sea_orm::PaginatorTrait;

    use super::*;

    async fn fresh_store() -> (SqlDailySnapshotStore, axagent_dao::db::DbHandle) {
        // 走与生产**同一个** initialize_schema（声明式对账 ⇒ 本批新表不需要写 migration，
        // 建表与建索引都由实体声明 + extras.rs 产出）。测试因此同时验证了
        // 「`astock_daily_snapshot` 真能被引擎建出来」这件事。
        let handle = axagent_dao::db::create_test_pool().await.expect("建临时测试库失败");
        (SqlDailySnapshotStore::new(handle.conn.clone()), handle)
    }

    /// 主键占位值的同步契约（端口侧字面量 vs 实体侧常量）。
    #[test]
    fn full_market_scope_placeholder_matches_entity() {
        assert_eq!(SCOPE_PLACEHOLDER, daily_snapshot::FULL_MARKET_SCOPE);
        assert_eq!(
            daily_snapshot::row_id("get_hot_stocks", None, "2026-06-01"),
            "get_hot_stocks@-@2026-06-01"
        );
    }

    /// 引擎是否真把新表建出来了（本仓表结构的正门是声明式对账，不是 migration）。
    #[tokio::test]
    async fn table_exists_after_declarative_reconcile() {
        let (_store, handle) = fresh_store().await;
        let n = astock_daily_snapshot::Entity::find()
            .count(&handle.conn)
            .await
            .expect("查询 astock_daily_snapshot 失败 ⇒ 表没被引擎建出来");
        assert_eq!(n, 0, "新库该表应为空");
    }

    #[tokio::test]
    async fn put_upsert_overwrites_and_separates_scopes() {
        let (store, _h) = fresh_store().await;
        let db = store.db.clone();
        store.put("get_hot_stocks", None, "2026-09-24", "[{\"a\":1}]").await;
        store.put("get_hot_stocks", None, "2026-09-24", "[{\"a\":2}]").await;
        assert_eq!(
            store.get("get_hot_stocks", "2026-09-24").await.as_deref(),
            Some("[{\"a\":2}]"),
            "同一归属日重复采集必须覆盖"
        );
        let rows = astock_daily_snapshot::Entity::find().all(&db).await.unwrap();
        assert_eq!(rows.len(), 1, "upsert 不得留下第二行");

        // 三种作用域共用主键域，互不串读
        store.put("get_margin_data", Some("600519"), "2026-09-24", "{\"m\":1}").await;
        store.put("search_stock", Some("贵州茅台"), "2026-09-24", "[]X").await;
        assert_eq!(
            store.get_stock("get_margin_data", "600519", "2026-09-24").await.as_deref(),
            Some("{\"m\":1}")
        );
        assert!(store.get_stock("get_margin_data", "000001", "2026-09-24").await.is_none());
        assert_eq!(
            store.get_keyword("search_stock", "贵州茅台", "2026-09-24").await.as_deref(),
            Some("[]X")
        );
    }

    /// 写侧闸门：空快照根本不落表（旧形态是「写进去、读时滤」⇒ 判重被锁死）。
    #[tokio::test]
    async fn poisoned_payload_never_lands_in_table() {
        let (store, _h) = fresh_store().await;
        for poison in ["[]", "[] ", "", "null"] {
            store.put("get_sector_info", None, "2026-10-09", poison).await;
            assert!(
                store.get("get_sector_info", "2026-10-09").await.is_none(),
                "{poison:?} 必须 miss"
            );
            assert!(
                !store.contains("get_sector_info", None, "2026-10-09").await,
                "判重不得被空快照锁死"
            );
        }
        let rows = astock_daily_snapshot::Entity::find().all(&store.db.clone()).await.unwrap();
        assert_eq!(rows.len(), 0, "四类空快照都不落表");
    }

    /// 日期归一（适配器侧仍是单一函数 `row_id` → `key_date`）。
    #[tokio::test]
    async fn weekend_anchor_reads_back_last_trading_day_row() {
        let (store, _h) = fresh_store().await;
        store
            .put("get_industry_ranking", None, "2026-09-24", "[{\"industry_name\":\"半导体\"}]")
            .await;
        for anchor in ["2026-09-25", "2026-09-26", "2026-09-27"] {
            assert!(
                store.get("get_industry_ranking", anchor).await.is_some(),
                "{anchor} 锚点必须回填 09-24 那条"
            );
        }
        assert!(store.get("get_industry_ranking", "2026-09-23").await.is_none(), "归一只允许向后");
    }

    /// 类②：跨日聚合在**真表**上成立（DiskCache 做不到这件事，本批的立批理由）。
    #[tokio::test]
    async fn count_by_date_range_aggregates_days_via_sql() {
        let (store, _h) = fresh_store().await;
        for d in ["2026-09-21", "2026-09-22", "2026-09-24"] {
            store.put("get_industry_ranking", None, d, "[{\"x\":1}]").await;
        }
        // 同一天的另一作用域只算一天
        store.put("get_industry_ranking", Some("600519"), "2026-09-24", "[{\"x\":2}]").await;
        // 别的 method 不得串数
        store.put("get_hot_stocks", None, "2026-09-21", "[{\"y\":1}]").await;

        assert_eq!(
            store.count_by_date_range("get_industry_ranking", "2026-09-21", "2026-09-24").await,
            Some(3)
        );
        assert_eq!(
            store.count_by_date_range("get_hot_stocks", "2026-09-21", "2026-09-24").await,
            Some(1)
        );
        assert_eq!(
            store.count_by_date_range("get_cls_flash", "2026-09-21", "2026-09-24").await,
            Some(0),
            "该 method 一行都没有 ⇒ 查询成功、结果为零天，必须与「查不出来」区分开"
        );
        // 窗口端点落在休市日同样先归一（09-26 周六 → 09-24）
        assert_eq!(
            store.count_by_date_range("get_industry_ranking", "2026-09-24", "2026-09-27").await,
            Some(1)
        );
    }

    /// 负控（#20② 假诊断修复的存储侧自证）：**表没建成 ⇒ `None`，不是 0**。
    ///
    /// 这是现场的 failure mode：本仓建表走声明式对账、启动期 `additive_only`，
    /// 所以 `astock_daily_snapshot` 没建出来时只会在启动日志 warn、进程照常起来。
    /// 那一刻这条聚合 SQL 是真报错的。若适配器把它塌成 `0`，回放面板就会输出
    /// 「最近 20 个交易日一天都没采到 ⇒ 采集侧**整段断供**」—— 把「我这句查询失败了」
    /// 说成「后台采集断了」，用户去查 sweep 而病根在存储层。
    ///
    /// 用一个**没跑过 initialize_schema** 的裸内存库（与 `dao/src/reconcile/apply.rs`
    /// 等处同选型）演出「表不存在」，断言返回 `None` 而不是 `Some(0)`。
    #[tokio::test]
    async fn count_by_date_range_reports_none_when_table_missing() {
        let db = sea_orm::Database::connect("sqlite::memory:").await.expect("连内存库应成功");
        let store = SqlDailySnapshotStore::new(db);
        assert_eq!(
            store.count_by_date_range("get_industry_ranking", "2026-09-21", "2026-09-24").await,
            None,
            "表不存在 ⇒ SQL 报错 ⇒ 聚合不可得，绝不能当成「那一天都没采到」"
        );
    }

    /// 存量回填：搬得动、幂等、中毒条目不搬、旧文件改名保留。
    #[tokio::test]
    async fn backfill_moves_legacy_file_once_and_renames_it() {
        let (store, handle) = fresh_store().await;
        let dir =
            std::env::temp_dir().join(format!("axagent_snap_backfill_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let legacy = dir.join(LEGACY_SNAPSHOT_FILENAME);
        // 逐字按旧 DiskCache 的落盘形状造样本（三种 key 形态 + 一条中毒 + 一条异形）
        std::fs::write(
            &legacy,
            r#"{"entries":{
              "daily:get_hot_stocks:2026-09-22":{"value":"[{\"stock_code\":\"000001\"}]","expires_at":0,"last_access":1},
              "daily:get_margin_data:600519:2026-09-22":{"value":"{\"m\":1}","expires_at":0,"last_access":1},
              "daily:search_stock:贵州茅台:2026-09-22":{"value":"[{\"n\":1}]","expires_at":0,"last_access":1},
              "daily:get_cls_flash:2026-09-22":{"value":"[]","expires_at":0,"last_access":1},
              "daily:坏形状:2026:a:b":{"value":"x","expires_at":0,"last_access":1},
              "klines:000001:daily":{"value":"[]","expires_at":1,"last_access":1}
            }}"#,
        )
        .unwrap();

        let rep = backfill_legacy_snapshot_file(&handle.conn, &legacy).await.expect("回填不应失败");
        assert_eq!(rep.scanned, 5, "daily: 前缀的条目全计入解析（含中毒与异形各一条）");
        assert_eq!(rep.moved, 3, "三种作用域各搬一条");
        assert_eq!(rep.skipped_poison, 1, "字面 [] 不搬");
        assert_eq!(rep.skipped_malformed, 1, "段数不符的不搬");
        assert!(rep.renamed_to.is_some(), "旧文件必须改名保留");
        assert!(!legacy.exists(), "原路径让位给 .migrated-<日期>");

        // 读回：三种作用域都在新表里
        assert!(store.get("get_hot_stocks", "2026-09-22").await.is_some());
        assert!(store.get_stock("get_margin_data", "600519", "2026-09-22").await.is_some());
        assert!(store.get_keyword("search_stock", "贵州茅台", "2026-09-22").await.is_some());
        assert!(store.get("get_cls_flash", "2026-09-22").await.is_none(), "中毒条目没搬进来");

        // 幂等：旧文件已改名 ⇒ 第二次运行零动作
        let again =
            backfill_legacy_snapshot_file(&handle.conn, &legacy).await.expect("第二次不应失败");
        assert_eq!(again.scanned, 0);
        assert_eq!(again.moved, 0);
        assert!(again.renamed_to.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 幂等的另一半：旧文件里同一主键已有、而表里**已先有更新的数据**时不得倒退覆盖。
    #[tokio::test]
    async fn backfill_does_not_overwrite_fresher_table_rows() {
        let (store, handle) = fresh_store().await;
        let dir =
            std::env::temp_dir().join(format!("axagent_snap_backfill2_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let legacy = dir.join(LEGACY_SNAPSHOT_FILENAME);
        std::fs::write(
            &legacy,
            r#"{"entries":{"daily:get_hot_stocks:2026-09-22":{"value":"OLD","expires_at":0,"last_access":1}}}"#,
        )
        .unwrap();
        store.put("get_hot_stocks", None, "2026-09-22", "NEW").await;

        let rep = backfill_legacy_snapshot_file(&handle.conn, &legacy).await.unwrap();
        assert_eq!(rep.skipped_present, 1, "表内已有该键 ⇒ 跳过，不覆盖新采的值");
        assert_eq!(rep.moved, 0);
        assert_eq!(store.get("get_hot_stocks", "2026-09-22").await.as_deref(), Some("NEW"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
