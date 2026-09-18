// SPDX-License-Identifier: AGPL-3.0-only

//! L2 disk cache — persists search results and index snapshots to SQLite for
//! cold-data storage.
//!
//! Works alongside the L1 memory caches to form a complete hot/cold separation
//! architecture:
//! - L1: In-memory, TTL-based, persisted via CacheSnapshot on shutdown
//! - L2: SQLite-backed, time-partitioned, auto-eviction of stale entries
//!
//! ⚠ 实际已接线的 L1 缓存只有 `EmbeddingCache` 与 `TextHashCache`
//! （见 `axagent-cache`）。`VectorSearchCache` 这个类型**存在**
//! （`axagent-search::vector_cache`，含 TTL+LRU 与单测），但**从未被实例化到
//! 生产路径**：全仓 `VectorSearchCache::new` 只出现在它自己的测试里，检索路径
//! 没有接它。（本文档此前把它与 `EmbeddingCache` 并列成「已存在的 L1 缓存」，
//! 属把「类型存在」当成「已接线」，2026-09-15 按实测改正。）
//!
//! # 访问层（2026-09-16 改造）
//!
//! 本 crate 原先用原生 `rusqlite` + 手写 DDL；现全部改走 SeaORM 实体：
//! - 建表由实体派生（`Schema::create_table_from_entity`），实体即 schema 真相源；
//! - 读写走实体 API，`INSERT OR REPLACE` / `DELETE ... WHERE id NOT IN (子查询)`
//!   等 SQLite 方言 SQL 整体消失 ⇒ 同一套代码对 PostgreSQL 也成立；
//! - 三张表都是**侧车库**（本 crate 自持一个 SQLite 文件），**不在主库** ——
//!   故不参与主库 schema 自愈（`dao::migrations::schema_diff::heal_all` 对主库里
//!   没有的表直接跳过，见 `crates/entities/src/lib.rs` 的侧车库说明）。
//!
//! # 接线状态（2026-09-16）
//!
//! | 表 | 生产写入端 | 生产读取端 | 状态 |
//! |---|---|---|---|
//! | `l2_search_results` | `axagent-search::search::execute_search` | 同左（命中即返回） | ✅ 已接线 |
//! | `l2_index_snapshots` | `src/indexing_triggers.rs` | 同左（写入前取上一轮做增量） | ✅ 已接线 |
//!
//! ⚠ **写入端 ≠ 接线**：本仓判据 #183 —— 只补写入端、没有读取端，产出的是噪声而非
//! 功能。上表两行之所以算「已接线」，是**两列都非空**。
//!
//! # `l2_summaries` 删除记录（2026-09-16，用户裁决）
//!
//! 原第三张表 `l2_summaries`（会话摘要缓存）已**整链删除** —— 实体、`CachedSummary`、
//! 三个方法（`store_summary` / `get_summaries` / `evict_old_summaries`）、建表调用、
//! 两个 config 字段与自测全部移除。三条理由，任一条单独都足以删：
//!
//! 1. **零调用**：三个方法全仓只有 crate 自测调用（读端写端皆空，不满足上表判据）；
//! 2. **功能重复**：会话摘要已由主库 `conversation_summaries` 承载 ——
//!    `axagent-dao::repo::conversation::upsert_summary` 写入，字段更全
//!    （多 `compressed_until_message_id` / `token_count` / `model_used`），
//!    且是权威真相源（见 `src/commands/conversations/compress.rs:253`）；
//! 3. **数据必然成为孤儿**：侧车库与主库之间**没有外键** ⇒ 会话被删除后，本表的
//!    摘要行不会被级联清理，也没有任何 TTL 之外的主体去清它 ⇒ 只增不减。
//!
//! **零损失论证**：`l2_cache.db` 位于 `~/.axagent/`，实测**该文件从未被创建过**
//! （接线是 09-16 做的，应用最后启动是 09-15）⇒ 表从未建出 ⇒ 本次删除**零 DROP、
//! 零数据迁移、零 DB 变更**。如将来确需跨进程摘要缓存，应先在主库侧与
//! `conversation_summaries` 合并口径，而不是在侧车库重建一份。
//!
//! ⚠ **一处此前的错误声明已更正**：本文件原先写「把同一份摘要再写进侧车库会被
//! `scripts/check-single-source-facts.mjs` 拦下」。实测（2026-09-16）该脚本（712 行）
//! 只扫两类对象 —— rustdoc 里的 `文件:行` 引用、reranker 模型文件名；**表级重复
//! 不在它的扫描面内**（脚本内 grep `l2_summaries` 零命中）。即：那道「护栏」是空的，
//! 项目里并不存在拦这类重复的单源门禁。删表是因为上述三条实质理由，不是因为会被拦。

use axagent_entities::{l2_index_snapshots, l2_search_results};
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ColumnTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbBackend,
    EntityTrait, QueryFilter, QueryOrder, QuerySelect, Schema, Set, Statement,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedSearchResult {
    pub id: i64,
    pub query_hash: String,
    pub query_text: String,
    pub results_json: String,
    pub result_count: usize,
    pub hit_count: u32,
    pub created_at: u64,
    pub last_accessed_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexSnapshotMeta {
    pub snapshot_id: String,
    pub file_count: usize,
    pub definition_count: usize,
    pub snapshot_path: String,
    pub created_at: u64,
}

#[derive(Debug, Clone)]
pub struct DiskCacheConfig {
    pub max_search_results: usize,
    pub max_snapshots: usize,
    pub search_result_ttl_days: u32,
}

impl Default for DiskCacheConfig {
    fn default() -> Self {
        Self { max_search_results: 1000, max_snapshots: 10, search_result_ttl_days: 30 }
    }
}

pub struct DiskCache {
    db: DatabaseConnection,
    config: DiskCacheConfig,
}

impl DiskCache {
    /// 用既有连接构造（测试注入内存库）。
    pub async fn new(db: DatabaseConnection, config: DiskCacheConfig) -> Result<Self, String> {
        let cache = Self { db, config };
        cache.ensure_tables().await?;
        Ok(cache)
    }

    /// 按路径打开（或创建）L2 缓存库 —— 生产入口。
    ///
    /// `max_connections(1)`：L2 是单文件低频写入的旁路缓存，且**必须单连接**，
    /// 否则测试用的 `sqlite::memory:` 会出现「建表落在连接 A、写入落到连接 B」
    /// （内存库每连接一份，表现为「表不存在」）。
    pub async fn open(path: &Path, config: DiskCacheConfig) -> Result<Self, String> {
        let url = format!("sqlite:{}?mode=rwc", path.to_string_lossy().replace('\\', "/"));
        let mut opt = ConnectOptions::new(&url);
        opt.max_connections(1)
            .min_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(15))
            .sqlx_logging(false);
        let db = Database::connect(opt)
            .await
            .map_err(|e| format!("打开 L2 缓存库失败 {}: {e}", path.display()))?;

        // 与主库同款 PRAGMA（见 `crates/dao/src/db.rs:59-75`）。WAL 在只读介质 /
        // `:memory:` 上会失败，故忽略错误 —— 它不是致命条件。
        for pragma in
            ["PRAGMA journal_mode=WAL;", "PRAGMA busy_timeout=5000;", "PRAGMA synchronous=NORMAL;"]
        {
            let _ =
                db.execute_raw(Statement::from_string(DbBackend::Sqlite, pragma.to_string())).await;
        }

        Self::new(db, config).await
    }

    /// 建表语句**由实体生成** —— 不再手写列清单。
    ///
    /// 手写 DDL 与实体一旦漂移不会报错，只会表现为「写入成功但读出的字段是 None」；
    /// 本 crate 改造前正是手写 DDL（三张表 8/5/5 列），与 `entities` 里的实体是
    /// 两份各自腐烂的载体。
    ///
    /// ⚠ `create_table_from_entity` **同时**负责 `#[sea_orm(unique)]` 列的唯一约束
    /// （`sea-orm-2.0.2/src/schema/entity.rs:258-260` 把 `ColumnDef::unique` 落到
    /// 建表语句里）。但它**不**负责 `#[sea_orm(unique_key = "...")]` 与 `#[sea_orm(indexed)]`
    /// —— 那两类要走 `create_index_from_entity`（同文件 :143-183），本 crate 未调用，
    /// 故本 crate 的实体只允许用 `unique`。回归锁见 `test_query_hash_unique_constraint`。
    async fn ensure_tables(&self) -> Result<(), String> {
        let backend = self.db.get_database_backend();
        for stmt in [
            Schema::new(backend).create_table_from_entity(l2_search_results::Entity),
            Schema::new(backend).create_table_from_entity(l2_index_snapshots::Entity),
        ] {
            let mut stmt = stmt;
            stmt.if_not_exists();
            self.db.execute(&stmt).await.map_err(|e| format!("创建 L2 缓存表失败: {e}"))?;
        }
        Ok(())
    }

    fn now_secs() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
    }

    // ── Search result caching ───────────────────────────────────────────────

    /// Look up cached search results by query hash.
    pub async fn get_search_results(
        &self,
        query_hash: &str,
    ) -> Result<Option<CachedSearchResult>, String> {
        let found = l2_search_results::Entity::find()
            .filter(l2_search_results::Column::QueryHash.eq(query_hash))
            .one(&self.db)
            .await
            .map_err(|e| format!("get search results: {e}"))?;

        let Some(row) = found else {
            return Ok(None);
        };

        let now = Self::now_secs();
        let hits = row.hit_count.saturating_add(1);

        let mut am: l2_search_results::ActiveModel = row.clone().into();
        am.hit_count = Set(hits);
        am.last_accessed_at = Set(now as i64);
        l2_search_results::Entity::update(am)
            .exec(&self.db)
            .await
            .map_err(|e| format!("update hit: {e}"))?;

        Ok(Some(CachedSearchResult {
            id: row.id as i64,
            query_hash: row.query_hash,
            query_text: row.query_text,
            results_json: row.results_json,
            result_count: row.result_count.max(0) as usize,
            hit_count: hits.max(0) as u32,
            created_at: row.created_at.max(0) as u64,
            last_accessed_at: now,
        }))
    }

    /// 写入（或覆盖）某 query hash 的搜索结果 —— **真 upsert，单条原子语句**。
    ///
    /// ⚠ **演进史**（每一步的修法自身都留过缺陷，改动前请读完）：
    /// 1. 最初用 `INSERT OR REPLACE`：但 `query_hash` 当时**无唯一约束** ——
    ///    SQLite 的 `OR REPLACE` 只在撞上 UNIQUE / PRIMARY KEY 时才替换，故它
    ///    **从不触发替换**，每次 store 都新增一行。重复行被 `get` 的「取首行」掩盖，
    ///    表现为 `hit_count` 永远从 1 重来 + 表无限膨胀。
    /// 2. 改成「先按 hash `DELETE`、再 `INSERT`」：累积止住了，但那是**两条非原子
    ///    语句** ⇒ 中途失败（或并发写入交错）会留下「已删未插」的空洞：旧缓存没了、
    ///    新缓存没写，而调用方看到的是成功。且每次写入要两次数据库往返。
    /// 3. 现在：实体加 `#[sea_orm(unique)]`（见 `entities/src/l2_search_results.rs`）
    ///   + `ON CONFLICT (query_hash) DO UPDATE` ⇒ 一条语句完成，唯一性由数据库保证。
    ///     语义与 1 的本意一致（整行替换）、与 2 的结果一致（只留一行），且无 2 的原子性缺口。
    ///
    /// 与 2 的一处**有意差异**：upsert 保留原行的 `id`（`DELETE`+`INSERT` 会换新 id）。
    /// `id` 不承载对外语义，保留它反而让 `CachedSearchResult.id` 在覆盖前后稳定。
    ///
    /// `hit_count` 在覆盖时重置为 1：缓存**内容已变**（`results_json` / `result_count`
    /// 都是新值），旧内容的命中次数不再可加。
    pub async fn store_search_results(
        &self,
        query_hash: &str,
        query_text: &str,
        results_json: &str,
        result_count: usize,
    ) -> Result<(), String> {
        let now = Self::now_secs() as i64;

        let am = l2_search_results::ActiveModel {
            query_hash: Set(query_hash.to_string()),
            query_text: Set(query_text.to_string()),
            results_json: Set(results_json.to_string()),
            result_count: Set(result_count as i32),
            hit_count: Set(1),
            created_at: Set(now),
            last_accessed_at: Set(now),
            ..Default::default()
        };
        l2_search_results::Entity::insert(am)
            .on_conflict(
                OnConflict::column(l2_search_results::Column::QueryHash)
                    .update_columns([
                        l2_search_results::Column::QueryText,
                        l2_search_results::Column::ResultsJson,
                        l2_search_results::Column::ResultCount,
                        l2_search_results::Column::HitCount,
                        l2_search_results::Column::CreatedAt,
                        l2_search_results::Column::LastAccessedAt,
                    ])
                    .to_owned(),
            )
            .exec(&self.db)
            .await
            .map_err(|e| format!("store search: {e}"))?;

        self.evict_old_search_results().await?;
        Ok(())
    }

    /// TTL + 容量双重淘汰。
    ///
    /// 原 SQL 是单条 `DELETE ... WHERE ... AND id NOT IN (SELECT ... LIMIT ?)`；
    /// 实体 API 无子查询 ⇒ 拆成「先取要保留的 id，再删其余」，语义相同。
    async fn evict_old_search_results(&self) -> Result<(), String> {
        let cutoff = Self::now_secs()
            .saturating_sub(self.config.search_result_ttl_days as u64 * 86400)
            as i64;

        let keep: Vec<i32> = l2_search_results::Entity::find()
            .select_only()
            .column(l2_search_results::Column::Id)
            .order_by_desc(l2_search_results::Column::LastAccessedAt)
            .limit(self.config.max_search_results as u64)
            .into_tuple::<i32>()
            .all(&self.db)
            .await
            .map_err(|e| format!("evict search (select keep): {e}"))?;

        let mut q = l2_search_results::Entity::delete_many()
            .filter(l2_search_results::Column::LastAccessedAt.lt(cutoff));
        // 空集合不能拼 `NOT IN ()`（SQLite 语法错误）。
        if !keep.is_empty() {
            q = q.filter(l2_search_results::Column::Id.is_not_in(keep));
        }
        q.exec(&self.db).await.map_err(|e| format!("evict search: {e}"))?;
        Ok(())
    }

    // ── Index snapshot tracking ──────────────────────────────────────────────

    /// Record an index snapshot（`snapshot_id` 是主键 ⇒ 真 upsert）。
    pub async fn record_snapshot(
        &self,
        snapshot_id: &str,
        file_count: usize,
        definition_count: usize,
        snapshot_path: &str,
    ) -> Result<(), String> {
        let am = l2_index_snapshots::ActiveModel {
            snapshot_id: Set(snapshot_id.to_string()),
            file_count: Set(file_count as i32),
            definition_count: Set(definition_count as i32),
            snapshot_path: Set(snapshot_path.to_string()),
            created_at: Set(Self::now_secs() as i64),
        };
        l2_index_snapshots::Entity::insert(am)
            .on_conflict(
                OnConflict::column(l2_index_snapshots::Column::SnapshotId)
                    .update_columns([
                        l2_index_snapshots::Column::FileCount,
                        l2_index_snapshots::Column::DefinitionCount,
                        l2_index_snapshots::Column::SnapshotPath,
                        l2_index_snapshots::Column::CreatedAt,
                    ])
                    .to_owned(),
            )
            .exec(&self.db)
            .await
            .map_err(|e| format!("record snapshot: {e}"))?;

        self.evict_old_snapshots().await?;
        Ok(())
    }

    /// 读单条快照（按主键 `snapshot_id`）。
    ///
    /// **存在的意义**：`l2_index_snapshots` 若只有写入端，就是「零读取端的补写」
    /// ——即纯噪声（本仓判据 #183）。`indexing_triggers` 用本方法在**写入前**取
    /// 上一轮计数，从而在日志里给出**真实的增减**：「本次扫到 1200 个文件」回答不了
    /// 「相对上次多了还是少了 / 这是不是首次索引」，而那是维护方唯一关心的事。
    ///
    /// ⚠ 取的是**上一轮**（写入前），而非「读回刚写的那条」—— 后者只能证明
    /// 「我自己刚才写进去了」，是同义反复。
    pub async fn get_snapshot(
        &self,
        snapshot_id: &str,
    ) -> Result<Option<IndexSnapshotMeta>, String> {
        let row = l2_index_snapshots::Entity::find_by_id(snapshot_id.to_string())
            .one(&self.db)
            .await
            .map_err(|e| format!("get snapshot: {e}"))?;

        Ok(row.map(|r| IndexSnapshotMeta {
            snapshot_id: r.snapshot_id,
            file_count: r.file_count.max(0) as usize,
            definition_count: r.definition_count.max(0) as usize,
            snapshot_path: r.snapshot_path,
            created_at: r.created_at.max(0) as u64,
        }))
    }

    /// List recent snapshots.
    pub async fn list_snapshots(&self) -> Result<Vec<IndexSnapshotMeta>, String> {
        let rows = l2_index_snapshots::Entity::find()
            .order_by_desc(l2_index_snapshots::Column::CreatedAt)
            .limit(self.config.max_snapshots as u64)
            .all(&self.db)
            .await
            .map_err(|e| format!("list snapshots: {e}"))?;

        Ok(rows
            .into_iter()
            .map(|r| IndexSnapshotMeta {
                snapshot_id: r.snapshot_id,
                file_count: r.file_count.max(0) as usize,
                definition_count: r.definition_count.max(0) as usize,
                snapshot_path: r.snapshot_path,
                created_at: r.created_at.max(0) as u64,
            })
            .collect())
    }

    async fn evict_old_snapshots(&self) -> Result<(), String> {
        let keep: Vec<String> = l2_index_snapshots::Entity::find()
            .select_only()
            .column(l2_index_snapshots::Column::SnapshotId)
            .order_by_desc(l2_index_snapshots::Column::CreatedAt)
            .limit(self.config.max_snapshots as u64)
            .into_tuple::<String>()
            .all(&self.db)
            .await
            .map_err(|e| format!("evict snapshots (select keep): {e}"))?;

        let mut q = l2_index_snapshots::Entity::delete_many();
        if !keep.is_empty() {
            q = q.filter(l2_index_snapshots::Column::SnapshotId.is_not_in(keep));
        }
        q.exec(&self.db).await.map_err(|e| format!("evict snapshots: {e}"))?;
        Ok(())
    }

    /// Compute a stable hash for a query string.
    pub fn query_hash(query: &str) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        query.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }
}

// ── 进程级注册 ──────────────────────────────────────────────────────────────

/// 进程级 L2 缓存实例。
///
/// **为什么用全局注册而非逐点注入**：消费方横跨两个 crate（`axagent-search`
/// 的 `execute_search` 与主 crate 的 `indexing_triggers`），逐点注入要么改
/// `execute_search` 的 6 参签名（波及全部调用点），要么给 `AppState` 加槽位
/// （本文档改造前该槽位正是「悬空链」的来源）。
///
/// ⚠ 与 `PLAN-weknora-borrowings.md §12.11.2`「不接受 `OnceLock`」的关系：
/// 那条裁定针对**索引槽位**（`file_index`/`ast_index`），理由是它们需要可替换
/// （测试 / 多工作区）；L2 缓存是**按 query hash 寻址的旁路缓存**，不承载状态，
/// 且未注册时所有调用点静默退化为「无 L2」（行为与改造前一致）。
static L2_CACHE: OnceLock<DiskCache> = OnceLock::new();

/// 注册进程级 L2 缓存（启动时调用一次；重复调用返回 `Err`）。
pub async fn init_l2(path: &Path, config: DiskCacheConfig) -> Result<(), String> {
    let cache = DiskCache::open(path, config).await?;
    L2_CACHE.set(cache).map_err(|_| "L2 缓存已注册（应只调用一次）".to_string())
}

/// 取进程级 L2 缓存。**未注册时返回 `None`** ⇒ 调用方必须静默降级为「不缓存」。
pub fn l2() -> Option<&'static DiskCache> {
    L2_CACHE.get()
}

#[cfg(test)]
mod tests {
    use super::*;
    // `Select::count` 来自 `PaginatorTrait`，而本文件只有测试用到它 ——
    // 放在模块级 import 会让**生产构建**报 `unused_imports`（`-D warnings` 直接 fail）。
    use sea_orm::PaginatorTrait;

    /// ⚠ `sqlite::memory:` 必须 `max_connections(1)`：sqlx 的每条池连接各持一份
    /// 独立内存库，多连接下建表与写入会落到不同库（表现为「表不存在」）。
    async fn test_cache() -> DiskCache {
        let mut opt = ConnectOptions::new("sqlite::memory:");
        opt.max_connections(1).min_connections(1).sqlx_logging(false);
        let db = Database::connect(opt).await.expect("测试：打开内存 SQLite 失败");
        DiskCache::new(db, DiskCacheConfig::default()).await.expect("测试：创建 DiskCache 失败")
    }

    #[tokio::test]
    async fn test_search_cache_miss_then_hit() {
        let cache = test_cache().await;
        let hash = DiskCache::query_hash("find user auth");

        assert!(cache.get_search_results(&hash).await.expect("测试：get 应成功").is_none());

        cache
            .store_search_results(&hash, "find user auth", r#"[{"file":"auth.rs"}]"#, 1)
            .await
            .expect("测试：存储搜索结果失败");

        let cached = cache
            .get_search_results(&hash)
            .await
            .expect("测试：get 应成功")
            .expect("测试：应有缓存命中");
        assert_eq!(cached.query_text, "find user auth");
        assert!(cached.results_json.contains("auth.rs"));
        assert_eq!(cached.hit_count, 2); // get + implicit increment
    }

    /// 回归锁：同一 query 反复 store **不得**累积重复行。
    ///
    /// 改造前 `INSERT OR REPLACE` 对无唯一约束的 `query_hash` 从不触发替换，
    /// 每次 store 都新增一行 —— 该缺陷被「取首行」的读法掩盖。
    #[tokio::test]
    async fn test_repeated_store_does_not_accumulate() {
        let cache = test_cache().await;
        let hash = DiskCache::query_hash("dup");
        for _ in 0..5 {
            cache
                .store_search_results(&hash, "dup", r#"[{"a":1}]"#, 1)
                .await
                .expect("测试：存储应成功");
        }
        let total =
            l2_search_results::Entity::find().count(&cache.db).await.expect("测试：count 应成功");
        assert_eq!(total, 1, "同一 query_hash 应只留一行");
    }

    /// 回归锁：`query_hash` 的 `#[sea_orm(unique)]` **必须真的落成库内唯一约束**。
    ///
    /// 这条锁不是形式主义 —— `store_search_results` 用 `ON CONFLICT (query_hash)`
    /// 做真 upsert，而 SQLite 要求冲突目标必须有 UNIQUE / PRIMARY KEY 约束，否则
    /// 直接报「ON CONFLICT clause does not match any PRIMARY KEY or UNIQUE constraint」。
    ///
    /// 而 sea-orm 对唯一性的处理**分两条路**（`sea-orm-2.0.2/src/schema/entity.rs`）：
    /// `#[sea_orm(unique)]` 由 `create_table_from_entity` 落进建表语句（:258-260）；
    /// `#[sea_orm(unique_key = "...")]` 则要 `create_index_from_entity`（:143-183）
    /// 另发一条 `CREATE UNIQUE INDEX` —— **本 crate 没调它**。⇒ 谁把属性从 `unique`
    /// 误改成 `unique_key`，约束会**静默消失**，upsert 退化成「每次 INSERT 都成功」，
    /// 于是重新开始累积重复行（正是最初那个坑）。本测试专拦这一种改法。
    #[tokio::test]
    async fn test_query_hash_unique_constraint_exists() {
        let cache = test_cache().await;
        let mk = |hash: &str| l2_search_results::ActiveModel {
            query_hash: Set(hash.to_string()),
            query_text: Set("t".to_string()),
            results_json: Set("[]".to_string()),
            result_count: Set(0),
            hit_count: Set(1),
            created_at: Set(0),
            last_accessed_at: Set(0),
            ..Default::default()
        };

        l2_search_results::Entity::insert(mk("same-hash"))
            .exec(&cache.db)
            .await
            .expect("测试：首次插入应成功");

        let dup = l2_search_results::Entity::insert(mk("same-hash")).exec(&cache.db).await;
        assert!(
            dup.is_err(),
            "query_hash 上应存在唯一约束 ⇒ 重复插入必须失败。\
             实际成功了，说明约束已丢失（检查实体是否被从 unique 改成 unique_key）"
        );
    }

    /// 回归锁：覆盖写入必须**原地更新**（保留原行 id），而非「删旧的、插新的」。
    #[tokio::test]
    async fn test_store_overwrites_in_place() {
        let cache = test_cache().await;
        let hash = DiskCache::query_hash("overwrite");

        cache
            .store_search_results(&hash, "overwrite", r#"[{"v":1}]"#, 1)
            .await
            .expect("测试：首次写入应成功");
        let first =
            cache.get_search_results(&hash).await.expect("测试：读应成功").expect("测试：应命中");
        assert_eq!(first.hit_count, 2); // store 置 1，本次 get 自增到 2

        cache
            .store_search_results(&hash, "overwrite", r#"[{"v":2}]"#, 1)
            .await
            .expect("测试：覆盖应成功");
        let second =
            cache.get_search_results(&hash).await.expect("测试：读应成功").expect("测试：应命中");

        assert_eq!(second.id, first.id, "upsert 应保留原行 id（DELETE+INSERT 会换新 id）");
        assert!(second.results_json.contains(r#""v":2"#), "内容应被覆盖为最新");
        assert_eq!(second.hit_count, 2, "内容已变 ⇒ hit_count 先重置为 1，再被本次 get 自增到 2");
    }

    #[tokio::test]
    async fn test_snapshot_recording() {
        let cache = test_cache().await;
        assert!(cache.record_snapshot("snap1", 100, 500, "/cache/snap1.json").await.is_ok());
        assert!(cache.record_snapshot("snap2", 200, 800, "/cache/snap2.json").await.is_ok());

        let snapshots = cache.list_snapshots().await.expect("测试：获取快照列表应成功");
        assert_eq!(snapshots.len(), 2);
        let ids: Vec<&str> = snapshots.iter().map(|s| s.snapshot_id.as_str()).collect();
        assert!(ids.contains(&"snap1"));
        assert!(ids.contains(&"snap2"));
    }

    /// 回归锁：`snapshot_id` 是主键 ⇒ 同名重录必须**覆盖**而非报错/新增。
    #[tokio::test]
    async fn test_snapshot_upsert_overwrites() {
        let cache = test_cache().await;
        cache.record_snapshot("snap1", 100, 500, "/old").await.expect("测试：首录应成功");
        cache.record_snapshot("snap1", 999, 888, "/new").await.expect("测试：重录应成功");

        let snapshots = cache.list_snapshots().await.expect("测试：列表应成功");
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].file_count, 999);
        assert_eq!(snapshots[0].snapshot_path, "/new");
    }

    /// 回归锁：未登记的 `snapshot_id` 必须返回 `None` 而非报错 ——
    /// `indexing_triggers` 依赖「首次索引 = 上一轮为空」，若此处报错就会
    /// 把正常首次索引渲染成失败。
    #[tokio::test]
    async fn test_get_snapshot_absent_returns_none() {
        let cache = test_cache().await;
        assert!(
            cache.get_snapshot("never-recorded").await.expect("测试：读取应成功").is_none(),
            "未登记的 snapshot_id 应返回 None"
        );

        cache.record_snapshot("snap1", 7, 9, "/p").await.expect("测试：首录应成功");
        let found = cache
            .get_snapshot("snap1")
            .await
            .expect("测试：读取应成功")
            .expect("测试：已登记的 snapshot_id 应能读回");
        assert_eq!(found.file_count, 7);
        assert_eq!(found.definition_count, 9);
        assert_eq!(found.snapshot_path, "/p");
    }

    /// 回归锁：未注册全局实例时 `l2()` 必须返回 `None`（调用方据此静默降级）。
    #[test]
    fn test_l2_unregistered_returns_none() {
        // 本测试进程内 `init_l2` 未被调用 ⇒ 恒为 None。
        // ⚠ 若将来有别的测试调到 `init_l2`，本断言会失败 —— 那正是它的价值。
        assert!(l2().is_none());
    }
}
