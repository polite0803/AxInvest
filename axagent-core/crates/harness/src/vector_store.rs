// SPDX-License-Identifier: AGPL-3.0-only

//! 向量存储预留接口（任务 #40，`PLAN-shell-plus-plugins.md` §0.12.1 ① 预留接口）
//!
//! 本模块把宿主侧 `axagent-search::vector_store::VectorStore` 的公开方法面拆成
//! 三个**方言无关** trait（Admin / Writer / Reader），方法归属逐条照 PLAN 那张表。
//!
//! 口径（PLAN 已定，勿在本批翻案）：
//! - **本轮只做「接口化 + 占接缝位」**：唯一实现者是 `VectorStore` 本身（持
//!   `DatabaseConnection` 的 struct，sqlite-vec 是进程内扩展，出进程即失效）；
//!   不实现任何外部后端，不把它变成可远程。
//! - 接缝 id 当初预留为 `rag.vector.store`，**交付名已改为 `knowledge.vector`**（曾用名与本行
//!   的预留名只保留在历史登记里；改名裁定见 `PLAN-shell-plus-plugins.md` §56.1-Q1）。判据只有
//!   一处：`capability_registry.rs` 中 `ServiceDefinition::knowledge_vector()` 的那个字面量。
//!   登记口径不变（PLAN §0.7 的预留位，不进 P5 批次）。
//! - 命名去方言：`ensure_fts5_index` / `ensure_collection_hnsw` 带 SQLite 专属特性，
//!   契约里改名为 `ensure_keyword_index` / `ensure_vector_index`；宿主原方法名保留，
//!   由 search crate 的 trait impl 纯转接桥接。
//! - K4 预算：向量是大对象（1536 维 ≈ 6 KB/条），DTO 走宿主侧 chunk 引用
//!   （`rowid: i64` + 集合名），不引入实现层类型 —— 本模块全部签名仅用
//!   标准库、serde DTO 与 `core_error::Result`。
//!
//! 与同目录 `vector_search::VectorSearch`（wiki 召回面）和 `rag_provider::VectorStoreProvider`
//! （RAG pipeline 最小口）是三个不同抽象，各自权威，勿混用。

use crate::core_error::Result;

/// A single embedding record for storage in the vector database.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EmbeddingRecord {
    /// Unique chunk identifier
    pub id: String,
    /// Parent document identifier
    pub document_id: String,
    /// Position of this chunk within the document
    pub chunk_index: i32,
    /// Text content of the chunk
    pub content: String,
    /// Embedding vector
    pub embedding: Vec<f32>,
}

/// A result returned from vector similarity search.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VectorSearchResult {
    pub id: String,
    pub document_id: String,
    pub chunk_index: i32,
    pub content: String,
    /// Distance score (lower is more similar for L2 distance)
    pub score: f32,
    /// Whether this chunk has an embedding in the vector table
    pub has_embedding: bool,
}

/// 知识库文本检索接缝（P6.0-⑥ 批 D：工作流 `vectorRetrieve` 节点的唯一能力出口）。
///
/// 与 [`VectorReader`] 的区别：`VectorReader` 收**已向量化的**查询向量（调用方自带
/// embedding），本 trait 收**原始查询文本**——embedding 与向量库都在实现层，
/// 调用方（工作流引擎）不接触 embedding 细节。
#[async_trait::async_trait]
pub trait VectorSearchSeam: Send + Sync {
    /// 按查询文本检索知识库分块。
    /// `min_similarity` = 相关度下限 ∈ [0,1]（None = 不过滤）。
    async fn search_text(
        &self,
        knowledge_base_id: &str,
        query: &str,
        top_k: usize,
        min_similarity: Option<f32>,
    ) -> Result<Vec<VectorSearchResult>>;
}

/// Configuration for HNSW (Hierarchical Navigable Small World) index.
/// HNSW provides faster approximate nearest neighbor search for large collections.
///
/// Default values are suitable for most use cases:
/// - Small collections (< 10k vectors): Use default k-NN (exact search)
/// - Medium collections (10k-100k): ef_search=50, m=12, ef_construction=100
/// - Large collections (> 100k): ef_search=100, m=16, ef_construction=200
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HnswConfig {
    /// Construction time search width (higher = slower build, better graph quality)
    /// Default: 100
    pub ef_construction: usize,
    /// Max connections per node (higher = better recall, more memory)
    /// Default: 16
    pub m: usize,
    /// Search width (higher = slower search, better recall)
    /// Default: 50
    pub ef_search: usize,
}

impl Default for HnswConfig {
    fn default() -> Self {
        Self { ef_construction: 100, m: 16, ef_search: 50 }
    }
}

/// 集合与索引生命周期（PLAN §0.12.1 ① 表 · `VectorCollectionAdmin`，7 方法）。
#[async_trait::async_trait]
pub trait VectorCollectionAdmin: Send + Sync {
    async fn ensure_collection(&self, collection_id: &str, dimensions: usize) -> Result<()>;

    async fn get_collection_dimensions(&self, collection_id: &str) -> Result<Option<usize>>;

    async fn prepare_collection_for_indexing(
        &self,
        collection_id: &str,
        dimensions: usize,
    ) -> Result<()>;

    /// 原名 `VectorStore::ensure_collection_hnsw`（去方言名，桥接在 search 侧 impl）。
    async fn ensure_vector_index(
        &self,
        collection_id: &str,
        dimensions: usize,
        hnsw_config: HnswConfig,
    ) -> Result<()>;

    async fn delete_collection(&self, knowledge_base_id: &str) -> Result<()>;

    async fn clear_embeddings(&self, collection_id: &str) -> Result<()>;

    async fn set_collection_embedding_model(
        &self,
        collection_id: &str,
        model: Option<&str>,
    ) -> Result<()>;
}

/// 写入与删除（PLAN §0.12.1 ① 表 · `VectorWriter`，9 方法）。
#[async_trait::async_trait]
pub trait VectorWriter: Send + Sync {
    async fn upsert_embeddings(
        &self,
        collection_id: &str,
        records: Vec<EmbeddingRecord>,
    ) -> Result<()>;

    async fn add_single_chunk(
        &self,
        collection_id: &str,
        document_id: &str,
        content: &str,
        embedding: &[f32],
    ) -> Result<String>;

    async fn delete_document_embeddings(
        &self,
        knowledge_base_id: &str,
        document_id: &str,
    ) -> Result<()>;

    async fn insert_metadata_only_chunk(
        &self,
        collection_id: &str,
        document_id: &str,
        chunk_id: &str,
        content: &str,
    ) -> Result<()>;

    /// `entries` 为宿主侧引用（rowid, embedding），与实现层主键一致（K4 口径）。
    async fn reinsert_embeddings(
        &self,
        collection_id: &str,
        entries: Vec<(i64, Vec<f32>)>,
    ) -> Result<()>;

    async fn upsert_document_embeddings(
        &self,
        collection_id: &str,
        entries: Vec<(i64, Vec<f32>)>,
    ) -> Result<()>;

    async fn delete_chunk(&self, collection_id: &str, chunk_id: &str) -> Result<()>;

    async fn update_chunk_content(
        &self,
        collection_id: &str,
        chunk_id: &str,
        new_content: &str,
    ) -> Result<()>;

    async fn update_chunk_embedding(
        &self,
        collection_id: &str,
        chunk_id: &str,
        embedding: &[f32],
    ) -> Result<()>;
}

/// 查询与列举（PLAN §0.12.1 ① 表 · `VectorReader`，含表内「等」收口后共 9 方法，
/// 见交回报告对 PLAN「11 个」计数的出入说明）。
#[async_trait::async_trait]
pub trait VectorReader: Send + Sync {
    async fn search(
        &self,
        knowledge_base_id: &str,
        query_embedding: Vec<f32>,
        top_k: usize,
    ) -> Result<Vec<VectorSearchResult>>;

    async fn search_with_filter(
        &self,
        knowledge_base_id: &str,
        query_embedding: Vec<f32>,
        top_k: usize,
        doc_ids: Option<&[String]>,
    ) -> Result<Vec<VectorSearchResult>>;

    async fn list_all_chunks(&self, collection_id: &str) -> Result<Vec<(i64, String, String)>>;

    async fn list_document_chunks_raw(
        &self,
        collection_id: &str,
        document_id: &str,
    ) -> Result<Vec<(i64, String, String)>>;

    /// 元数据行列举（rowid, id, document_id, content），供恢复类路径使用。
    async fn list_all_metadata_rows(
        &self,
        collection_id: &str,
    ) -> Result<Vec<(i64, String, String, String)>>;

    /// 原名 `VectorStore::ensure_fts5_index`（去方言名，桥接在 search 侧 impl）。
    async fn ensure_keyword_index(&self, collection_id: &str) -> Result<()>;

    /// 关键词索引重建；宿主实现为 fire-and-forget，不返回 Result（保持原语义）。
    async fn rebuild_fts_index(&self, collection_id: &str);

    /// 关键词检索；`score` 语义与向量检索一致（越小越匹配）。
    async fn fts_search(
        &self,
        collection_id: &str,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<VectorSearchResult>>;

    async fn list_document_chunks(
        &self,
        collection_id: &str,
        document_id: &str,
    ) -> Result<Vec<VectorSearchResult>>;
}
