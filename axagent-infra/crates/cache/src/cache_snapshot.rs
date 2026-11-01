// SPDX-License-Identifier: AGPL-3.0-only

//! Serializable cache snapshot used for persistence between application sessions.
//!
//! When the application shuts down, cached data from L1 memory caches
//! (EmbeddingCache, TextHashCache) is serialized into a `CacheSnapshot` and
//! written atomically to disk. On next startup, the snapshot is deserialized
//! and loaded back into memory, avoiding expensive cold-start cache rebuilds.
//!
//! ⚠ `VectorSearchCache` **不在**上述清单里（2026-09-15 按实测改正）：该类型在
//! `axagent-search::vector_cache` 中确实存在，但从未被实例化到生产路径，因此也
//! 没有内容可进快照 —— 本结构体保留的 `vector_search_cache` 字段目前**恒为空**
//! （见该字段文档）。本文档此前把它与 EmbeddingCache 并列成已接线的 L1 缓存。

use axagent_harness::util_fns::current_rfc3339;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use axagent_harness::cache_interceptor::CacheEntrySnapshot;

/// A snapshot of in-memory cache state, serializable for disk persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheSnapshot {
    /// Schema version for forward compatibility.
    pub version: u32,
    /// ISO 8601 timestamp of when the snapshot was created.
    pub created_at: String,
    /// Entries from the embedding cache (text hash → embedding vector).
    pub embedding_cache: HashMap<String, Vec<f32>>,
    /// Entries from the text hash cache (document ID → hash string).
    pub text_hash_cache: HashMap<String, String>,
    /// Entries from the vector search cache (query key → serialized results).
    ///
    /// ⚠ **当前恒为空数组**（2026-09-15 核实）：唯一可能填它的
    /// `axagent-search::vector_cache::VectorSearchCache` 从未被实例化到生产路径
    /// （全仓 `VectorSearchCache::new` 只出现在它自己的单测里），
    /// `CacheSnapshot::new` 也因此只写 `Vec::new()`。
    ///
    /// 保留该字段而非删除：① 它是**未完成特性**的占位（W9 待裁决，见
    /// PLAN-weknora-borrowings §9.1/§9.3），删掉会丢掉这条线索；
    /// ② 反序列化侧 `serde` 默认忽略未知字段 ⇒ 字段可安全删除，
    /// 但**新增**字段需要 `#[serde(default)]`，故保留占位更省事。
    /// `cache_persister` 会在日志里打出它的长度（当前恒 0），可据此判断是否已接线。
    pub vector_search_cache: Vec<CacheEntrySnapshot>,
}

impl Default for CacheSnapshot {
    fn default() -> Self {
        Self {
            version: 1,
            created_at: current_rfc3339(),
            embedding_cache: HashMap::new(),
            text_hash_cache: HashMap::new(),
            vector_search_cache: Vec::new(),
        }
    }
}
