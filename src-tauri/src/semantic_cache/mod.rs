// SPDX-License-Identifier: AGPL-3.0-only

//! Semantic Cache — reduces duplicate LLM calls for semantically similar prompts.
//!
//! Uses the application's existing sea-orm database connection to store cached
//! LLM responses. Hash-based matching (SHA-256 of normalized prompt) for O(1)
//! lookup. Future: embedding-based cosine similarity search.
//!
//! ## Cache TTL strategy
//!
//! - fact/trivial queries: 7 days
//! - reasoning: 1 hour
//! - code: 24 hours
//! - complex: 1 hour
//!
//! ## 改造记录（2026-09-16）
//!
//! 原实现用 `Statement::from_sql_and_values` 对每个操作手写 SQLite(`?N`) /
//! PostgreSQL(`$N`) 双分支。改走 SeaORM 实体
//! （[`axagent_entities::semantic_cache`]）后双分支消失。
//!
//! 三处方言/语义差异的处置（前两处为等价改写，第三处是**行为变更**）：
//!
//! 1. **`model_id IS NOT DISTINCT FROM $2`** → 按 `model_id` 有无分支为
//!    `= ?` / `IS NULL`。原写法本意即「NULL 也要与 NULL 相等」，两种写法等价，
//!    但新写法不依赖 `IS NOT DISTINCT FROM`（该语法要求 SQLite ≥ 3.39）。
//! 2. **建表 DDL 一表两定义** → 统一为 v100 迁移那一份。原 `create_table` 在
//!    PG 分支写成 `BIGINT` 且**无 NOT NULL**，与 v100 的 `INTEGER NOT NULL
//!    DEFAULT 0` 不一致；由于应用启动总是先跑迁移，生产库的实际形状一直是
//!    v100 那份，此处对齐它。同时删掉了不必要的方言分支（SQLite 接受
//!    `BIGINT` / `TEXT` 关键字）。
//! 3. ⚠ **UPSERT 语义统一（行为变更，仅影响 SQLite）**：原 SQLite 分支用
//!    `INSERT OR REPLACE`，它是「整行替换」⇒ 会把 `hit_count` **重置为 0**；
//!    而原 PG 分支的 `ON CONFLICT ... DO UPDATE` 只更新指定列 ⇒ `hit_count`
//!    保留。现统一为后者：**刷新同一缓存条目不再清零命中计数**。
//!    判定依据：`hit_count` 是缓存的命中统计，仅因条目内容被重写就清零属
//!    明显不合理，PG 分支才是预期语义。（此表无外键引用，`INSERT OR REPLACE`
//!    的级联删除副作用在此不存在。）

use async_trait::async_trait;
use axagent_entities::semantic_cache::{ActiveModel, Column, Entity as SemanticCacheEntity};
use axagent_harness::cache_interceptor::{HarnessCache, LlmCacheKey};
use sea_orm::sea_query::{Expr, ExprTrait, OnConflict};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, QueryTrait, Set,
};
use sha2::{Digest, Sha256};

// ─── Config ───

pub struct CacheConfig {
    pub max_entries: usize,
    pub default_ttl_secs: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self { max_entries: 10_000, default_ttl_secs: 3600 }
    }
}

// ─── Cache entry ───

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub id: String,
    pub response: String,
    pub model_id: Option<String>,
    pub token_count: i64,
    pub hit_count: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CacheStats {
    pub total_entries: usize,
    pub active_entries: usize,
    pub expired_entries: usize,
    pub total_hits: usize,
}

// ─── Time helper ───

/// 当前 Unix 秒时间戳。改造前该表达式在 4 处重复，提取为单一来源。
fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
        as i64
}

// ─── SemanticCache ───

pub struct SemanticCache {
    db: DatabaseConnection,
    config: CacheConfig,
}

impl SemanticCache {
    /// Create a new semantic cache backed by the application database.
    pub async fn new(db: DatabaseConnection, config: CacheConfig) -> Result<Self, String> {
        Self::create_table(&db).await?;
        tracing::info!(
            "Semantic cache initialized (max_entries={}, default_ttl={}s)",
            config.max_entries,
            config.default_ttl_secs,
        );
        Ok(Self { db, config })
    }

    /// 创建缓存表 + 索引。每条 SQL 单独执行（PG 不支持 `execute_raw` 多语句）。
    ///
    /// DDL 与 v100 迁移逐字一致。正常情况下应用启动已跑过迁移、此处是 no-op；
    /// 仅内存 SQLite 占位库（`init/state.rs` 启动阶段未跑迁移）依赖它建表。
    async fn create_table(db: &DatabaseConnection) -> Result<(), String> {
        const CREATE_TABLE: &str = "CREATE TABLE IF NOT EXISTS semantic_cache (\
             id TEXT NOT NULL PRIMARY KEY, prompt_hash TEXT NOT NULL, response TEXT NOT NULL, \
             model_id TEXT, token_count INTEGER NOT NULL DEFAULT 0, \
             task_type TEXT NOT NULL DEFAULT 'moderate', ttl_secs INTEGER NOT NULL, \
             created_at BIGINT NOT NULL, hit_count INTEGER NOT NULL DEFAULT 0)";

        db.execute_unprepared(CREATE_TABLE)
            .await
            .map_err(|e| format!("Failed to create cache table: {}", e))?;

        // 索引分开执行
        db.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS idx_semantic_cache_hash ON semantic_cache(prompt_hash)",
        )
        .await
        .map_err(|e| format!("Failed to create hash index: {}", e))?;

        db.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS idx_semantic_cache_created ON semantic_cache(created_at)",
        )
        .await
        .map_err(|e| format!("Failed to create created_at index: {}", e))?;

        Ok(())
    }

    /// Normalize a prompt for consistent hashing.
    fn normalize_prompt(prompt: &str) -> String {
        let lower = prompt.to_lowercase();
        let mut result = String::with_capacity(lower.len());
        let mut prev_ws = false;
        for ch in lower.chars() {
            if ch.is_whitespace() {
                if !prev_ws {
                    result.push(' ');
                    prev_ws = true;
                }
            } else {
                result.push(ch);
                prev_ws = false;
            }
        }
        result.trim().to_string()
    }

    /// Compute a SHA-256 hash of the normalized prompt.
    fn hash_prompt(normalized: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(normalized.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Check the cache for a matching prompt and model.
    pub async fn check(
        &self,
        prompt: &str,
        model_id: Option<&str>,
    ) -> Result<Option<CacheEntry>, String> {
        let hash = Self::hash_prompt(&Self::normalize_prompt(prompt));
        self.lookup_by_hash(&hash, model_id).await
    }

    /// 按预先算好的哈希键查表（不做 prompt 归一化）。
    /// 供 `HarnessCache` 等以「非原文键」（如消息哈希）复用同一份缓存表。
    pub async fn lookup_by_hash(
        &self,
        hash: &str,
        model_id: Option<&str>,
    ) -> Result<Option<CacheEntry>, String> {
        let now = now_secs();

        let mut query = SemanticCacheEntity::find()
            .filter(Column::PromptHash.eq(hash))
            // 未过期：created_at + ttl_secs > now
            .filter(Expr::col(Column::CreatedAt).add(Expr::col(Column::TtlSecs)).gt(now));
        // 原写法 `model_id IS NOT DISTINCT FROM $2`：NULL 需与 NULL 匹配
        query = match model_id {
            Some(m) => query.filter(Column::ModelId.eq(m)),
            None => query.filter(Column::ModelId.is_null()),
        };

        let Some(row) = query.one(&self.db).await.map_err(|e| format!("Query error: {}", e))?
        else {
            tracing::debug!("Semantic cache MISS for hash={}", &hash[..hash.len().min(12)]);
            return Ok(None);
        };

        let entry = CacheEntry {
            id: row.id.clone(),
            response: row.response.clone(),
            model_id: row.model_id.clone(),
            token_count: row.token_count as i64,
            // 命中统计返回**自增后**的值：本次命中也要计入（测试 `upsert_preserves_hit_count`
            // 断言「1 次命中 + 本次命中」正是此语义；下方 `update_many` 恒定 +1）。
            hit_count: row.hit_count as i64 + 1,
        };

        // Increment hit counter
        let _ = SemanticCacheEntity::update_many()
            .col_expr(Column::HitCount, Expr::col(Column::HitCount).add(1))
            .filter(Column::Id.eq(&row.id))
            .exec(&self.db)
            .await;

        tracing::debug!("Semantic cache HIT for hash={}", &hash[..hash.len().min(12)]);
        Ok(Some(entry))
    }

    /// Store a response in the cache.
    pub async fn store(
        &self,
        prompt: &str,
        response: &str,
        model_id: Option<&str>,
        token_count: i64,
        task_type: &str,
        ttl_secs: Option<u64>,
    ) -> Result<(), String> {
        let hash = Self::hash_prompt(&Self::normalize_prompt(prompt));
        self.store_by_hash(&hash, response, model_id, token_count, task_type, ttl_secs).await
    }

    /// 按预先算好的哈希键写表（不做 prompt 归一化）。
    /// 供 `HarnessCache` 等以「非原文键」（如消息哈希）写入同一份缓存表。
    pub async fn store_by_hash(
        &self,
        hash: &str,
        response: &str,
        model_id: Option<&str>,
        token_count: i64,
        task_type: &str,
        ttl_secs: Option<u64>,
    ) -> Result<(), String> {
        let ttl = ttl_secs.unwrap_or(self.config.default_ttl_secs) as i32;
        let now = now_secs();

        let am = ActiveModel {
            id: Set(hash.to_string()),
            prompt_hash: Set(hash.to_string()),
            response: Set(response.to_string()),
            model_id: Set(model_id.map(|s| s.to_string())),
            token_count: Set(token_count as i32),
            task_type: Set(task_type.to_string()),
            ttl_secs: Set(ttl),
            created_at: Set(now),
            hit_count: Set(0),
        };

        // UPSERT：命中既有 id 时更新内容列，**保留 hit_count**（见文件头 §3）
        SemanticCacheEntity::insert(am)
            .on_conflict(
                OnConflict::column(Column::Id)
                    .update_columns([
                        Column::Response,
                        Column::TokenCount,
                        Column::TaskType,
                        Column::TtlSecs,
                        Column::CreatedAt,
                    ])
                    .to_owned(),
            )
            .exec(&self.db)
            .await
            .map_err(|e| format!("Insert error: {}", e))?;

        self.evict_if_over_limit().await;

        tracing::debug!("Semantic cache STORED hash={}", &hash[..hash.len().min(12)]);
        Ok(())
    }

    /// 超出上限时按 `created_at` 从旧到新淘汰超额条目。
    ///
    /// 单条 `DELETE ... WHERE id IN (SELECT id ... ORDER BY created_at ASC LIMIT n)`
    /// 完成，保持与改造前一致的原子性（不做「先查 ID 再删」的两步）。
    async fn evict_if_over_limit(&self) {
        let Ok(count) = SemanticCacheEntity::find().count(&self.db).await else {
            return;
        };
        let max = self.config.max_entries as u64;
        if count <= max {
            return;
        }
        let excess = count - max;

        let victims = SemanticCacheEntity::find()
            .select_only()
            .column(Column::Id)
            .order_by_asc(Column::CreatedAt)
            .limit(excess)
            .into_query();

        let _ = SemanticCacheEntity::delete_many()
            .filter(Column::Id.in_subquery(victims))
            .exec(&self.db)
            .await;
        tracing::info!("Semantic cache evicted {} entries", excess);
    }

    /// 按预先算好的哈希键删除条目（供 `HarnessCache::invalidate` 使用）。
    pub async fn delete_by_hash(&self, hash: &str) -> Result<(), String> {
        // 注意：原实现按 `prompt_hash` 删，而非按主键 `id`。
        // 两者在 `store_by_hash` 写入时同值，此处保持原列以不改变筛选语义
        // （若历史上有通过 `store()` 写入、id ≠ prompt_hash 的行，会一并删掉）。
        SemanticCacheEntity::delete_many()
            .filter(Column::PromptHash.eq(hash))
            .exec(&self.db)
            .await
            .map_err(|e| format!("Delete error: {}", e))?;
        Ok(())
    }

    /// Get TTL for a given task type (in seconds).
    pub fn ttl_for_task_type(task_type: &str) -> u64 {
        match task_type {
            "fact" | "trivial" => 7 * 24 * 3600,
            "reasoning" => 3600,
            "code" => 24 * 3600,
            "complex" => 3600,
            _ => 3600,
        }
    }

    /// Get cache statistics.
    pub async fn stats(&self) -> Result<CacheStats, String> {
        let now = now_secs();

        let total = SemanticCacheEntity::find()
            .count(&self.db)
            .await
            .map_err(|e| format!("Count error: {}", e))?;

        let active = SemanticCacheEntity::find()
            .filter(Expr::col(Column::CreatedAt).add(Expr::col(Column::TtlSecs)).gt(now))
            .count(&self.db)
            .await
            .map_err(|e| format!("Count error: {}", e))?;

        // COALESCE(SUM(hit_count), 0)：空表时 SUM 为 NULL
        let total_hits = SemanticCacheEntity::find()
            .select_only()
            .column_as(Expr::col(Column::HitCount).sum(), "total_hits")
            .into_tuple::<Option<i64>>()
            .one(&self.db)
            .await
            .map_err(|e| format!("Sum error: {}", e))?
            .flatten()
            .unwrap_or(0);

        Ok(CacheStats {
            total_entries: total as usize,
            active_entries: active as usize,
            expired_entries: total.saturating_sub(active) as usize,
            // 用全限定名消歧：`ExprTrait` 也在作用域内，`i64::max` 会与它冲突（E0034）。
            total_hits: std::cmp::Ord::max(total_hits, 0) as usize,
        })
    }
}

// ─── HarnessCache 适配 ───
//
// 让 SemanticCache 作为 harness 中心化 LLM 入口（execute_llm / execute_llm_stream）
// 的缓存拦截器。LlmCacheKey 只携带 model + messages_hash + temperature（无原文），
// 因此这里把三者组合后再 SHA-256，作为缓存表的 prompt_hash 键，走 *_by_hash 接口。

/// 将 LlmCacheKey 折叠为 64 位十六进制哈希键（含 model / 消息哈希 / 温度）。
fn key_to_hash(key: &LlmCacheKey) -> String {
    let combined = format!(
        "{}\u{1f}{}\u{1f}{}",
        key.model,
        key.messages_hash,
        key.temperature.map(|t| t.to_string()).unwrap_or_default(),
    );
    SemanticCache::hash_prompt(&combined)
}

#[async_trait]
impl HarnessCache for SemanticCache {
    async fn get(&self, key: &LlmCacheKey) -> Option<serde_json::Value> {
        let hash = key_to_hash(key);
        match self.lookup_by_hash(&hash, Some(&key.model)).await {
            Ok(Some(entry)) => serde_json::from_str(&entry.response).ok(),
            _ => None,
        }
    }

    async fn set(&self, key: LlmCacheKey, value: serde_json::Value, ttl_secs: u64) {
        let hash = key_to_hash(&key);
        let response = value.to_string();
        if let Err(e) = self
            .store_by_hash(&hash, &response, Some(&key.model), 0, "moderate", Some(ttl_secs))
            .await
        {
            tracing::warn!("[SemanticCache] HarnessCache set 失败: {e}");
        }
    }

    async fn invalidate(&self, key: &LlmCacheKey) {
        let hash = key_to_hash(key);
        if let Err(e) = self.delete_by_hash(&hash).await {
            tracing::warn!("[SemanticCache] HarnessCache invalidate 失败: {e}");
        }
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_prompt() {
        let result = SemanticCache::normalize_prompt("  Hello   World\n\n  Test   ");
        assert_eq!(result, "hello world test");
    }

    #[test]
    fn test_hash_deterministic() {
        let h1 = SemanticCache::hash_prompt(&SemanticCache::normalize_prompt("Hello World"));
        let h2 = SemanticCache::hash_prompt(&SemanticCache::normalize_prompt("  hello   world  "));
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_ttl_for_task_type() {
        assert_eq!(SemanticCache::ttl_for_task_type("fact"), 7 * 24 * 3600);
        assert_eq!(SemanticCache::ttl_for_task_type("reasoning"), 3600);
        assert_eq!(SemanticCache::ttl_for_task_type("code"), 24 * 3600);
        assert_eq!(SemanticCache::ttl_for_task_type("trivial"), 7 * 24 * 3600);
    }

    /// 防回归（2026-09-16 改造）：UPSERT 刷新条目后 `hit_count` 必须保留。
    ///
    /// 改造前 SQLite 分支用 `INSERT OR REPLACE`（整行替换）会把命中数清零，
    /// 与 PG 分支行为不一致；现已统一为 `ON CONFLICT ... DO UPDATE`。
    #[tokio::test]
    async fn upsert_preserves_hit_count() {
        let db =
            sea_orm::Database::connect("sqlite::memory:").await.expect("测试：连接内存库应成功");
        let cache =
            SemanticCache::new(db, CacheConfig::default()).await.expect("测试：建缓存应成功");

        cache
            .store("prompt-a", "resp-1", Some("m1"), 10, "moderate", None)
            .await
            .expect("测试：首次写入应成功");

        // 制造一次命中，使 hit_count = 1
        let hit = cache
            .check("prompt-a", Some("m1"))
            .await
            .expect("测试：查询应成功")
            .expect("测试：应命中");
        assert_eq!(hit.response, "resp-1");

        // 同键重写：内容更新，hit_count 不应被清零
        cache
            .store("prompt-a", "resp-2", Some("m1"), 20, "moderate", None)
            .await
            .expect("测试：重写应成功");

        let after = cache
            .check("prompt-a", Some("m1"))
            .await
            .expect("测试：查询应成功")
            .expect("测试：应命中");
        assert_eq!(after.response, "resp-2", "重写后内容应更新");
        assert_eq!(after.hit_count, 2, "重写不应重置 hit_count（1 次命中 + 本次命中）");
    }

    /// 防回归：`model_id` 为 NULL 时按 `IS NULL` 匹配（原 `IS NOT DISTINCT FROM` 语义）。
    #[tokio::test]
    async fn null_model_id_matches_null() {
        let db =
            sea_orm::Database::connect("sqlite::memory:").await.expect("测试：连接内存库应成功");
        let cache =
            SemanticCache::new(db, CacheConfig::default()).await.expect("测试：建缓存应成功");

        cache
            .store("prompt-b", "resp-null-model", None, 1, "moderate", None)
            .await
            .expect("测试：写入应成功");

        assert!(
            cache.check("prompt-b", None).await.expect("测试：查询应成功").is_some(),
            "model_id 为 NULL 的条目应能被 NULL 查询命中"
        );
        assert!(
            cache.check("prompt-b", Some("m1")).await.expect("测试：查询应成功").is_none(),
            "model_id 为 NULL 的条目不应被具体 model 查询命中"
        );
    }

    /// 防回归：过期条目不再命中（`created_at + ttl_secs > now`）。
    #[tokio::test]
    async fn expired_entry_misses() {
        let db =
            sea_orm::Database::connect("sqlite::memory:").await.expect("测试：连接内存库应成功");
        let cache =
            SemanticCache::new(db, CacheConfig::default()).await.expect("测试：建缓存应成功");

        // ttl = 0 秒 ⇒ 立即可过期
        cache
            .store("prompt-c", "resp-expired", Some("m1"), 1, "moderate", Some(0))
            .await
            .expect("测试：写入应成功");

        assert!(
            cache.check("prompt-c", Some("m1")).await.expect("测试：查询应成功").is_none(),
            "ttl 为 0 的条目不应命中"
        );
    }
}
