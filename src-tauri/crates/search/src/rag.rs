// SPDX-License-Identifier: AGPL-3.0-only

//! Unified RAG (Retrieval-Augmented Generation) abstraction layer.
//!
//! Provides a trait-based interface for different RAG sources (knowledge bases,
//! memory namespaces, etc.) to share indexing, searching, and context-collection
//! logic without code duplication.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use serde::{Deserialize, Serialize};

use crate::hybrid_search::{HybridSearchOptions, HybridSearchResult, HybridSearcher};
use crate::self_rag::RetrievalQuality;
use crate::sources;
use crate::text_chunker;
use crate::vector_store::{EmbeddingRecord, VectorSearchResult, VectorStore};
use axagent_harness::InferenceEngine;
use axagent_harness::constants::embed::is_deterministic_config_error;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::types::{RagContextResult, RagRetrievedItem, RagSourceResult};

/// 阈值过滤的**单一真源**：量纲定义在产出侧（`hybrid_search`），此处转出给
/// RAG 语义的调用方（本模块 + `commands/{knowledge,memory,wiki,paper}.rs`）。
///
/// ⚠ 2026-09-15：在此之前的实际状态是**五份各写一遍的字面量** —— 四处
/// `20.0`、一处（`commands/paper.rs`）`2.0`，且那处的注释还写着
/// 「与 collect_rag_context 一致」（它并不一致）。见 `DEFAULT_MAX_L2_DISTANCE`。
pub use crate::hybrid_search::{
    DEFAULT_MAX_L2_DISTANCE, distance_ceiling_from_similarity_floor,
    similarity_floor_from_threshold,
};

// ── Trait ────────────────────────────────────────────────────────────────────

/// A source of RAG content that can be searched and indexed.
///
/// Each implementor describes how to look up its embedding provider and
/// what prefix / label to use for vector-store collections and conversation
/// context injection.
#[async_trait]
pub trait RAGSource: Send + Sync {
    /// Collection prefix for vector-store table names (e.g. `"kb"`, `"mem"`).
    fn collection_prefix(&self) -> &'static str;

    /// Human-readable label inserted into conversation context
    /// (e.g. `"Knowledge Base Reference"`, `"Memory Reference"`).
    fn context_label(&self) -> &'static str;

    /// Resolve the `"providerId::model_id"` embedding provider string
    /// configured on the container identified by `container_id`.
    async fn resolve_embedding_provider(
        &self,
        db: &DatabaseConnection,
        container_id: &str,
    ) -> Result<String>;
}

// ── Built-in implementations ─────────────────────────────────────────────────

/// RAG source backed by a knowledge base (documents → parsed → chunked → embedded).
pub struct KnowledgeRAG;

#[async_trait]
impl RAGSource for KnowledgeRAG {
    fn collection_prefix(&self) -> &'static str {
        "kb"
    }

    fn context_label(&self) -> &'static str {
        "Knowledge Base Reference"
    }

    async fn resolve_embedding_provider(
        &self,
        db: &DatabaseConnection,
        container_id: &str,
    ) -> Result<String> {
        let kb = sources::knowledge().get_knowledge_base(container_id).await?;
        if let Some(provider) = kb.embedding_provider {
            return Ok(provider);
        }
        resolve_default_embedding_provider(db).await
    }
}

/// RAG source backed by a memory namespace (text items → directly embedded).
pub struct MemoryRAG;

#[async_trait]
impl RAGSource for MemoryRAG {
    fn collection_prefix(&self) -> &'static str {
        "mem"
    }

    fn context_label(&self) -> &'static str {
        "Memory Reference"
    }

    async fn resolve_embedding_provider(
        &self,
        db: &DatabaseConnection,
        container_id: &str,
    ) -> Result<String> {
        let ns = sources::memory().get_namespace(container_id).await?;
        if let Some(provider) = ns.embedding_provider {
            return Ok(provider);
        }
        resolve_default_embedding_provider(db).await
    }
}

/// RAG source backed by a Wiki vault (notes → chunked → embedded).
pub struct WikiVaultRAG;

#[async_trait]
impl RAGSource for WikiVaultRAG {
    fn collection_prefix(&self) -> &'static str {
        "wiki"
    }

    fn context_label(&self) -> &'static str {
        "Wiki Reference"
    }

    async fn resolve_embedding_provider(
        &self,
        db: &DatabaseConnection,
        container_id: &str,
    ) -> Result<String> {
        let wiki = sources::wiki().get_wiki(container_id).await?;
        if let Some(provider) = wiki.embedding_provider {
            return Ok(provider);
        }
        resolve_default_embedding_provider(db).await
    }
}

/// embedding 配置的**确定性错误**标记①：**没有**配置 provider。
///
/// 权威定义在 `axagent_harness::constants::embed`，此处仅 re-export，使
/// `axagent_search::rag::ERR_NO_EMBEDDING_PROVIDER` 这一既有调用路径保持不变
/// （`axagent_lib::index_queue` 的 R9 通道按该路径引用）。
///
/// 之所以不再在本文件直接定义值：它与 `ERR_EMBEDDING_PROVIDER_GONE`
/// （配置了但指向的 provider 已不存在）被**同一个 R9 判断**消费，两者必须同处
/// 定义才不会被漏掉其中一个 —— 2026-09-12 修掉的正是「R9 只认标记①」这一缺陷。
pub use axagent_harness::constants::embed::ERR_NO_EMBEDDING_PROVIDER;

/// 当容器未显式配置 embedding_provider 时，回退到系统默认 provider。
///
/// 回退不到时产出带 [`ERR_NO_EMBEDDING_PROVIDER`] 标记的错误。
async fn resolve_default_embedding_provider(_db: &DatabaseConnection) -> Result<String> {
    let settings = sources::settings()
        .get_settings()
        .await
        .map_err(|e| AxAgentError::Provider(format!("Failed to load settings: {}", e)))?;
    settings.default_provider_id.ok_or_else(|| {
        AxAgentError::Provider(format!(
            "{}: No embedding provider configured and no default provider found",
            ERR_NO_EMBEDDING_PROVIDER
        ))
    })
}

// ── 统一知识容器（P2: 抽象三个系统的共性字段） ──────────────────────────────

/// Knowledge/Memory/Wiki 三个系统的容器共性。
/// 长期计划（P3）：将 `memory_namespaces` 合并到 `knowledge_bases`，Memory 作为特殊的轻量级知识库。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeContainer {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub container_type: ContainerType,
    pub embedding_provider: Option<String>,
    pub embedding_dimensions: Option<i32>,
    pub retrieval_threshold: Option<f32>,
    pub retrieval_top_k: Option<i32>,
    pub icon_type: Option<String>,
    pub icon_value: Option<String>,
    pub sort_order: i32,
    pub chunk_size: Option<i32>,
    pub chunk_overlap: Option<i32>,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerType {
    KnowledgeBase,
    Memory,
    WikiVault,
}

impl KnowledgeContainer {
    /// 从 memory_namespace 转换
    pub fn from_memory_ns(ns: &axagent_harness::types::MemoryNamespace) -> Self {
        Self {
            id: ns.id.clone(),
            name: ns.name.clone(),
            description: None,
            container_type: ContainerType::Memory,
            embedding_provider: ns.embedding_provider.clone(),
            embedding_dimensions: ns.embedding_dimensions,
            retrieval_threshold: ns.retrieval_threshold,
            retrieval_top_k: ns.retrieval_top_k,
            icon_type: ns.icon_type.clone(),
            icon_value: ns.icon_value.clone(),
            sort_order: ns.sort_order,
            chunk_size: None,
            chunk_overlap: None,
            enabled: true,
        }
    }

    /// 从 knowledge_base 转换
    pub fn from_knowledge_base(kb: &axagent_harness::types::KnowledgeBase) -> Self {
        Self {
            id: kb.id.clone(),
            name: kb.name.clone(),
            description: kb.description.clone(),
            container_type: ContainerType::KnowledgeBase,
            embedding_provider: kb.embedding_provider.clone(),
            embedding_dimensions: kb.embedding_dimensions,
            retrieval_threshold: kb.retrieval_threshold,
            retrieval_top_k: kb.retrieval_top_k,
            icon_type: kb.icon_type.clone(),
            icon_value: kb.icon_value.clone(),
            sort_order: kb.sort_order,
            chunk_size: kb.chunk_size,
            chunk_overlap: kb.chunk_overlap,
            enabled: kb.enabled,
        }
    }

    /// 从 wiki 转换
    pub fn from_wiki(w: &axagent_harness::types::Wiki) -> Self {
        Self {
            id: w.id.clone(),
            name: w.name.clone(),
            description: w.description.clone(),
            container_type: ContainerType::WikiVault,
            embedding_provider: w.embedding_provider.clone(),
            embedding_dimensions: w.embedding_dimensions,
            retrieval_threshold: w.retrieval_threshold,
            retrieval_top_k: w.retrieval_top_k,
            icon_type: None,
            icon_value: None,
            sort_order: 0,
            chunk_size: None,
            chunk_overlap: None,
            enabled: true,
        }
    }

    /// 获取 RAG collection 名称
    pub fn collection_name(&self) -> String {
        match self.container_type {
            ContainerType::KnowledgeBase => format!("kb_{}", self.id),
            ContainerType::Memory => format!("mem_{}", self.id),
            ContainerType::WikiVault => format!("wiki_{}", self.id),
        }
    }

    pub fn source_config(&self) -> axagent_harness::types::SourceConfig {
        axagent_harness::types::SourceConfig {
            embedding_provider: self.embedding_provider.clone(),
            embedding_dimensions: self.embedding_dimensions,
            retrieval_threshold: self.retrieval_threshold,
            retrieval_top_k: self.retrieval_top_k,
        }
    }

    pub fn container_type_str(&self) -> &'static str {
        match self.container_type {
            ContainerType::KnowledgeBase => "KnowledgeBase",
            ContainerType::Memory => "Memory",
            ContainerType::WikiVault => "WikiVault",
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Build the sanitised collection ID for a RAG source.
pub fn collection_id(prefix: &str, container_id: &str) -> String {
    format!("{}_{}", prefix, container_id)
}

// ── Unified search ───────────────────────────────────────────────────────────

/// Search a single RAG source for content relevant to `query`.
///
/// Uses hybrid search (vector similarity + BM25 full-text with trigram
/// tokenizer for Chinese support) with Reciprocal Rank Fusion by default.
/// Falls back to pure vector search if FTS index is unavailable.
///
/// `hybrid`：用户可配的混合检索配置（权重 / 融合算法 / 是否启用），
/// 位于全局设置的 `ragPipelineConfig.hybrid`；传 `None` 时用内置默认值。
///
/// This is the generic replacement for the separate `search_knowledge` /
/// `search_memory` functions.  The concrete `EmbedFn` is injected by the
/// caller (typically `crate::indexing::generate_embeddings`).
#[allow(clippy::too_many_arguments)]
pub async fn search<S: RAGSource + ?Sized>(
    source: &S,
    db: &DatabaseConnection,
    master_key: &[u8; 32],
    vector_store: &VectorStore,
    container_id: &str,
    query: &str,
    top_k: usize,
    dimensions: Option<usize>,
    embed_fn: impl AsyncEmbedFn,
    hybrid: Option<&axagent_harness::rag_config::HybridConfig>,
    // `min_similarity`：相关度下限 ∈ [0,1]，语义与过滤位置见 `search_with_filter`。
    min_similarity: Option<f32>,
) -> Result<Vec<VectorSearchResult>> {
    search_with_filter(
        source,
        db,
        master_key,
        vector_store,
        container_id,
        query,
        top_k,
        dimensions,
        embed_fn,
        None,
        None,
        hybrid,
        min_similarity,
    )
    .await
}

/// `search` with optional `doc_ids` filter (multi-document collaboration).
/// When `doc_ids` is `Some` and non-empty, results are restricted to chunks
/// belonging to one of the listed documents.
///
/// `hybrid`：混合检索配置（见 `search`）。`None` ⇒ 内置默认值。
///
/// `min_similarity`：**相关度下限 ∈ [0,1]**（在 `combined_score` 标尺上 `>=` 判定），
/// `None` ⇒ 不过滤。它被写进 `HybridSearchOptions.min_score`，于是过滤发生在
/// **检索层内部**：`过滤 → 排序 → 截断 top_k`。
///
/// ⚠ 2026-09-15 修（过滤位置）。**同日自查更正了原先写的因果**（见 `MEMORY-RULES` #205）：
/// ① 此前本函数不收阈值，过滤由**调用方**在拿到结果**之后**做，而那时结果已截断成
/// `top_k` —— 但这**不是**「丢合格项」的原因：候选池是 `top_k * 3`，且 `hybrid_search`
/// 的筛选键与排序键**都是** `combined_score` ⇒ 合格集必为**前缀** ⇒ 两种次序**等价**。
/// ② 真正的原因是**两把尺子**：调用方拿**重排/距离尺度**的 `score` 去比一个按**融合尺度**
/// 算出的阈值 ⇒ 合格集非前缀 ⇒ 才真会丢中间合格项，方向还可能整体反（同 #186）。
/// ⇒ 本项的主要收益是**结构**（阈值语义收敛到检索层一处，原先 6 个调用点各写一遍），
/// 不是「修了结果丢失」。现在四个调用点共用这一条过滤真源。
#[allow(clippy::too_many_arguments)]
pub async fn search_with_filter<S: RAGSource + ?Sized>(
    source: &S,
    db: &DatabaseConnection,
    master_key: &[u8; 32],
    vector_store: &VectorStore,
    container_id: &str,
    query: &str,
    top_k: usize,
    dimensions: Option<usize>,
    embed_fn: impl AsyncEmbedFn,
    doc_ids: Option<&[String]>,
    precomputed_embedding: Option<Vec<f32>>,
    hybrid: Option<&axagent_harness::rag_config::HybridConfig>,
    min_similarity: Option<f32>,
) -> Result<Vec<VectorSearchResult>> {
    // 混合检索选项来自用户配置（`HybridConfig`），不再是硬编码 0.7/0.3（2026-09-15 修）。
    // 未传配置时退回 `HybridSearchOptions::default()`（enabled=true / 0.7 / 0.3 / RRF / 60），
    // 与历史写死值同值，保证调用方不传配置时行为不变。
    let mut hybrid_opts = match hybrid {
        Some(cfg) => crate::hybrid_search::hybrid_options_from_config(cfg, top_k),
        None => HybridSearchOptions { top_k, ..Default::default() },
    };
    // 阈值必须在**截断之前**生效 ⇒ 交给检索层的四条收尾路径统一处理
    // （它们都在 `truncate(top_k)` 之前按 `combined_score >= min_score` 过滤）。
    hybrid_opts.min_score = min_similarity;
    // 多引擎 RAG：保留原始 bm25_score 明细，避免下游 rerank 丢失分数信息
    let hybrid_results = search_hybrid_with_filter(
        source,
        db,
        master_key,
        vector_store,
        container_id,
        query,
        dimensions,
        embed_fn,
        hybrid_opts,
        doc_ids,
        precomputed_embedding,
    )
    .await?;

    Ok(hybrid_results
        .into_iter()
        .map(|r| VectorSearchResult {
            id: r.id,
            document_id: r.document_id,
            chunk_index: r.chunk_index,
            content: r.content,
            // `VectorSearchResult.score` 的**唯一量纲**：距离 ∈ [0,1]，**越小越相关**
            // （= `1 - combined_score`，而 `combined_score` 已由 `hybrid_search`
            // 的四条产出路径统一归一到 [0,1] 相关度，见 `DEFAULT_MAX_L2_DISTANCE`）。
            //
            // 方向刻意保持「越小越相关」不变：下游的记忆 tier 加权
            // （`apply_memory_tier_weight` 的 `1/(1+|score|)`）、反馈权重
            // （`item.score -= feedback_score`）、两处升序排序、以及前端的
            // `sorter: (a, b) => a.score - b.score` **全部**按该方向编写，
            // 翻转方向会连带推翻这一整片子系统。本次只统一**标尺**，不动方向。
            score: 1.0 - r.combined_score,
            has_embedding: r.vector_score.is_some(),
        })
        .collect())
}

/// Hybrid search (vector + BM25 FTS5 with trigram tokenizer) returning
/// detailed score breakdown.  Uses Reciprocal Rank Fusion by default for
/// robust score combination without manual weight tuning.
#[allow(clippy::too_many_arguments)]
pub async fn search_hybrid<S: RAGSource + ?Sized>(
    source: &S,
    db: &DatabaseConnection,
    master_key: &[u8; 32],
    _vector_store: &VectorStore,
    container_id: &str,
    query: &str,
    dimensions: Option<usize>,
    embed_fn: impl AsyncEmbedFn,
    options: HybridSearchOptions,
) -> Result<Vec<HybridSearchResult>> {
    search_hybrid_with_filter(
        source,
        db,
        master_key,
        _vector_store,
        container_id,
        query,
        dimensions,
        embed_fn,
        options,
        None,
        None,
    )
    .await
}

/// `search_hybrid` with optional `doc_ids` filter (multi-document collaboration).
///
/// `precomputed_embedding`：调用方已为 `query` 计算好的 query embedding（须与
/// 该源 resolve 出的 embedding provider / dims 一致）。传 `Some` 时跳过重复
/// embed 调用（消除同一查询在同一次检索流程中的双算）。
/// embedding 配置为**确定性错误**时（未配置 / 已悬空）的统一降级：改走纯 FTS（BM25）检索。
///
/// #### 为什么两处 Err 共用本函数
///
/// `resolve_embedding_provider` 失败 与 query embed 失败，触发条件是**同一个根因**
/// （embedding 配置坏了），差别只在错误何时暴露：悬空 provider 属于后者 ——
/// `resolve_embedding_provider` 只读容器上的 provider 字符串、**不校验该 provider
/// 是否还存在**，所以悬空引用时它返回 `Ok`，错误直到 `generate` 才出现。
/// 只处理前者就等于漏了后者，这正是 2026-09-12 实测到的缺陷形态
/// （生产库 `memory_namespaces` 绑定的 provider `af052547-…` 已被删除）。
///
/// #### 为什么必须降级而不是直接 Err
///
/// 本函数的上游 `collect_rag_context_from_refs` 对 Err 只 `tracing::warn!` 然后
/// **跳过该源** ⇒ 用户「静默无结果」（该函数内 2026-07-31 那条注释记录的正是这个体验
/// 问题）。降级后至少能命中关键词，且 WARN 明确说明降级原因，可观测。
///
/// 判定必须用清单化的 `is_deterministic_config_error`，不要写
/// `contains(某个标记)` —— 那样新增标记时会漏判，而漏判不报错、不告警。
async fn fts_only_fallback<S: RAGSource + ?Sized>(
    source: &S,
    db: &DatabaseConnection,
    container_id: &str,
    query: &str,
    options: HybridSearchOptions,
    doc_ids: Option<&[String]>,
    err: &str,
) -> Result<Vec<HybridSearchResult>> {
    tracing::warn!(
        "[RAG] embedding 配置为确定性错误（未配置 / 已悬空），{} {} 降级为纯 FTS 检索: {}",
        source.collection_prefix(),
        container_id,
        err
    );
    let cid = collection_id(source.collection_prefix(), container_id);
    let searcher = HybridSearcher::new(db.clone());
    let _ = searcher.ensure_fts5_index(&cid).await;
    searcher.fts_only_search_with_filter(&cid, query, options, doc_ids).await
}

#[allow(clippy::too_many_arguments)]
pub async fn search_hybrid_with_filter<S: RAGSource + ?Sized>(
    source: &S,
    db: &DatabaseConnection,
    master_key: &[u8; 32],
    _vector_store: &VectorStore,
    container_id: &str,
    query: &str,
    dimensions: Option<usize>,
    embed_fn: impl AsyncEmbedFn,
    options: HybridSearchOptions,
    doc_ids: Option<&[String]>,
    precomputed_embedding: Option<Vec<f32>>,
) -> Result<Vec<HybridSearchResult>> {
    let embedding_provider = match source.resolve_embedding_provider(db, container_id).await {
        Ok(p) => p,
        // R9 遗留：embedding 配置的**确定性错误** ⇒ 降级为纯 FTS（BM25）检索。
        // 会话 RAG 兜底场景下若不降级就是直接 Err → 调用方 `tracing::warn!` 后跳过该源
        // → 用户「静默无结果」。主链路（provider 正常）零影响，降级后至少命中关键词。
        //
        // 判定用清单化谓词而非 `contains(ERR_NO_EMBEDDING_PROVIDER)`：后者只认「未配置」，
        // 接不住「配置了但 provider 已被删除」—— 2026-09-12 实测缺陷，同一根因的第三个出口。
        Err(e) if is_deterministic_config_error(&e.to_string()) => {
            let err_msg = e.to_string();
            return fts_only_fallback(
                source,
                db,
                container_id,
                query,
                options.clone(),
                doc_ids,
                &err_msg,
            )
            .await;
        },
        Err(e) => return Err(e),
    };

    let cid = collection_id(source.collection_prefix(), container_id);
    let query_embedding = match precomputed_embedding {
        Some(e) => e,
        None => {
            // 悬空 provider 在**这一步**才暴露：`resolve_embedding_provider` 只读容器上的
            // provider 字符串、不校验它是否还存在 ⇒ 上面那个 match 拿到的是 `Ok`，
            // 直到这里 `generate` 才发现 provider 已不存在。与上面同属确定性配置错误，
            // 必须走同一降级，否则这条路径下用户依旧是「静默无结果」（2026-09-12 实测）。
            let embed_response = match embed_fn
                .generate(db, master_key, &embedding_provider, vec![query.to_string()], dimensions)
                .await
            {
                Ok(r) => r,
                Err(e) if is_deterministic_config_error(&e.to_string()) => {
                    let err_msg = e.to_string();
                    return fts_only_fallback(
                        source,
                        db,
                        container_id,
                        query,
                        options.clone(),
                        doc_ids,
                        &err_msg,
                    )
                    .await;
                },
                Err(e) => return Err(e),
            };
            embed_response
                .embeddings
                .into_iter()
                .next()
                .ok_or_else(|| AxAgentError::Provider("No query embedding returned".into()))?
        },
    };

    let searcher = HybridSearcher::new(db.clone());
    let _ = searcher.ensure_fts5_index(&cid).await;

    let results =
        searcher.hybrid_search_with_filter(&cid, query, query_embedding, options, doc_ids).await?;

    Ok(results)
}

// ── Unified indexing ─────────────────────────────────────────────────────────

/// Chunking strategy for indexing content.
pub enum ChunkStrategy {
    /// Parse a file and chunk the resulting text.
    ParseAndChunk {
        source_path: String,
        mime_type: String,
        chunk_size: usize,
        overlap: usize,
        separator: Option<String>,
    },
    /// Embed the content directly as a single vector.
    Direct,
    /// Chunk a raw text string (e.g. extracted from a conversation archive).
    FromText { text: String, chunk_size: usize, overlap: usize, separator: Option<String> },
}

/// Index content into a RAG source's vector collection.
///
/// Depending on the `ChunkStrategy`, the content is either:
/// - Parsed from a file, chunked, and batch-embedded (`ParseAndChunk`), or
/// - Embedded directly as a single item (`Direct`).
#[allow(clippy::too_many_arguments)]
pub async fn index(
    vector_store: &VectorStore,
    collection_prefix: &str,
    container_id: &str,
    item_id: &str,
    _content: &str,
    embeddings: Vec<Vec<f32>>,
    chunks: Vec<(String, String, i32)>, // (id, content, chunk_index)
) -> Result<()> {
    if chunks.is_empty() || embeddings.is_empty() {
        return Ok(());
    }

    if embeddings.len() != chunks.len() {
        return Err(AxAgentError::Provider(format!(
            "Embedding count mismatch: got {} embeddings for {} chunks",
            embeddings.len(),
            chunks.len()
        )));
    }

    let cid = collection_id(collection_prefix, container_id);

    let records: Vec<EmbeddingRecord> = chunks
        .into_iter()
        .zip(embeddings)
        .map(|((id, text, chunk_index), embedding)| EmbeddingRecord {
            id,
            document_id: item_id.to_string(),
            chunk_index,
            content: text,
            embedding,
        })
        .collect();

    vector_store.upsert_embeddings(&cid, records).await
}

/// Prepare chunks from content using the given strategy.
///
/// Returns a list of `(chunk_id, chunk_content, chunk_index)` tuples.
pub fn prepare_chunks(
    item_id: &str,
    strategy: &ChunkStrategy,
) -> Result<Vec<(String, String, i32)>> {
    match strategy {
        ChunkStrategy::ParseAndChunk { source_path, mime_type, chunk_size, overlap, separator } => {
            let path = std::path::Path::new(source_path);
            let text = sources::parser().extract_text(path, mime_type)?;

            // 解析成功但结果为空 ⇒ **失败**，而不是「零 chunk 的成功」。
            //
            // 返回 `Ok(vec![])` 会让 `run_indexing` 直接 `Ok(())`，调用方随后把文档状态
            // 置为 `ready` —— 用户看到「索引成功」而知识库零内容（假成功，2026-09-15 修）。
            // 报错后状态机走 failed 分支，用户可见并可重试。
            // 与 `FromText` 分支的区别：那边空文本属「确实没内容可索引」（如空会话归档），
            // 不是解析失败，保持 `Ok(vec![])`。
            if text.trim().is_empty() {
                return Err(AxAgentError::Provider(format!(
                    "文档未提取到任何文本（解析结果为空）: {source_path} (mime={mime_type})"
                )));
            }

            let is_markdown = mime_type == "text/markdown";
            let chunks = text_chunker::chunk_text_with_separator_and_markdown(
                &text,
                *chunk_size,
                *overlap,
                separator.as_deref(),
                is_markdown,
            );

            Ok(chunks
                .into_iter()
                .map(|c| (format!("{}_{}", item_id, c.index), c.content, c.index))
                .collect())
        },
        ChunkStrategy::Direct => {
            // Caller provides content directly; we don't read from strategy.
            // The actual content is passed to `index()` separately.
            // Return a placeholder that the caller fills in.
            Ok(vec![])
        },
        ChunkStrategy::FromText { text, chunk_size, overlap, separator } => {
            if text.trim().is_empty() {
                return Ok(vec![]);
            }

            let chunks = text_chunker::chunk_text_with_separator_and_markdown(
                text,
                *chunk_size,
                *overlap,
                separator.as_deref(),
                true, // conversation archives are markdown-formatted
            );

            Ok(chunks
                .into_iter()
                .map(|c| (format!("{}_{}", item_id, c.index), c.content, c.index))
                .collect())
        },
    }
}

/// Prepare a single direct chunk (for memory items).
pub fn prepare_direct_chunk(item_id: &str, content: &str) -> Vec<(String, String, i32)> {
    if content.trim().is_empty() {
        return vec![];
    }
    vec![(item_id.to_string(), content.to_string(), 0)]
}

pub async fn collect_knowledge_graph_context(
    _db: &DatabaseConnection,
    kb_ids: &[String],
    query: &str,
    top_k: usize,
) -> Vec<String> {
    let mut context_parts = Vec::new();

    for kb_id in kb_ids {
        let entities = match sources::knowledge().search_entities(kb_id, query, top_k).await {
            Ok(e) => e,
            Err(_) => continue,
        };

        if entities.is_empty() {
            continue;
        }

        let mut section = format!("[Knowledge Graph - {}]\n", kb_id);
        for entity in &entities {
            section.push_str(&format!("- {} ({})", entity.name, entity.entity_type));
            if let Some(ref desc) = entity.description
                && !desc.is_empty()
            {
                section.push_str(&format!(" — {}", desc));
            }
            section.push('\n');
        }
        context_parts.push(section);
    }

    context_parts
}

pub async fn collect_cross_source_graph_context(
    db: &DatabaseConnection,
    kb_ids: &[String],
    wiki_ids: &[String],
    query: &str,
    top_k: usize,
) -> Vec<String> {
    let mut context_parts = Vec::new();

    let kg_context = collect_knowledge_graph_context(db, kb_ids, query, top_k).await;
    context_parts.extend(kg_context);

    for wiki_id in wiki_ids {
        // P2-1: 收集 backlinks 作为 Wiki 图谱上下文
        let backlinks = match sources::wiki().get_note_backlinks_by_vault(wiki_id).await {
            Ok(bl) => bl,
            Err(_) => continue,
        };

        if backlinks.is_empty() {
            continue;
        }

        let mut section = format!("[Wiki Graph - {}]\n", wiki_id);

        // 添加 backlinks
        for bl in backlinks.iter().take(top_k) {
            section.push_str(&format!("- {} → {}", bl.source_note_id, bl.target_note_id));
            if !bl.link_text.is_empty() {
                section.push_str(&format!(" ({})", bl.link_text));
            }
            section.push('\n');
        }

        context_parts.push(section);
    }

    context_parts
}

/// 图增强检索每源取回上限（与 `rag_pipeline.rs` 阶段 4 保持一致）。
const ENTITY_GRAPH_TOP_K: usize = 5;

/// Graph RAG 阶段 4（图增强检索）在 **legacy 检索路径**上的取数入口。
///
/// # 为什么需要它
///
/// 阶段 4 原本只存在于 `RAGPipeline` 内部，而 `RAGPipeline` 只在
/// `collect_rag_context_with_pipeline_from_refs` 里被构造。也就是说：当用户没有开启
/// `query_enhancement` / `rerank` / `self_rag` 中任何一项时（`use_pipeline == false`），
/// 检索走 legacy 路径，图增强检索**根本没有机会执行**。
///
/// 本函数补上另一侧：legacy 路径直接取已注入的 provider 做一次图检索。
/// 是否注入由 wiring 层按设置开关决定（见 `crates/search/src/entity_graph.rs`），
/// 未注入时返回 `None` ⇒ 行为与接线前完全一致。
///
/// # 调用方须只对 Knowledge 源调用
///
/// `graph_enhanced_search` 按 `knowledge_entities.kb_id` 过滤，而 Memory / Wiki 源的
/// `container_id` 与之**不同域**（见 `rag_config.rs::EntityGraphConfig` 的注释）——
/// 传进去只会稳定返回空，白跑一次查询并打一条 warn。
async fn search_entity_graph(
    container_id: &str,
    query: &str,
) -> Option<axagent_harness::GraphEnhancedSearchResult> {
    let provider = crate::entity_graph::entity_graph_provider()?;
    let input = axagent_harness::GraphEnhancedSearchInput {
        knowledge_base_id: container_id.to_string(),
        query: query.to_string(),
        entity_type_filters: vec![],
        relation_type_filters: vec![],
        top_k: Some(ENTITY_GRAPH_TOP_K),
        include_neighbors: Some(true),
    };
    match provider.graph_enhanced_search(input).await {
        Ok(r) => Some(r),
        Err(e) => {
            tracing::warn!("Graph enhanced search failed for kb {container_id}: {e}");
            None
        },
    }
}

/// 对「融合后」的源结果逐个做 Path B 图增强检索（只对 Knowledge 源）。
///
/// # 为什么融合路径要自己再查一次，而不是从中转字段拿
///
/// 融合路径（`fuse_rag_context_results`）会**按融合后的 `source_results` 重算**
/// `context_parts`，于是各 run 自己折进去的图文本会丢。此前的做法是把「某个 run 的
/// 图检索结果」塞进 `RagContextResult.graph_context` 当中转字段，再在调用点补回一次 ——
/// 而那个字段全仓零外部读取端，属纯噪声（2026-09-15 删），故改为在此**显式重查**。
///
/// 附带修正一处语义偏差：中转取的是 `find_map` 第一个非空 run 的结果，而 run 对应的是
/// **增强后的查询**（`fq`），与同一分支里 Path A 用的 `effective_query` 并非同一查询；
/// 现在两者统一用 `effective_query`，口径一致。
async fn search_entity_graph_for_sources(
    source_results: &[RagSourceResult],
    query: &str,
) -> Vec<axagent_harness::GraphEnhancedSearchResult> {
    let mut out = Vec::new();
    for sr in source_results {
        if sr.source_type == RAGSourceType::Knowledge.as_str()
            && let Some(g) = search_entity_graph(&sr.container_id, query).await
        {
            out.push(g);
        }
    }
    out
}

/// 把图增强检索结果拼成「要追加到 `context_parts` 末尾的额外上下文」，并打可观测性日志。
///
/// 两条检索路径（legacy / pipeline）共用本函数，保证同一个开关在两条路径下表现一致。
/// 返回的文本与 `kg_context` 同属「回链上下文」，在
/// `rebuild_context_with_citations` 里原样追加到末尾、**不参与 `[cite:N]` 编号**。
fn fold_entity_graph_context(
    graph_results: &[axagent_harness::GraphEnhancedSearchResult],
    source_results: &[RagSourceResult],
) -> Vec<String> {
    let extra: Vec<String> = graph_results
        .iter()
        .map(|g| g.context_text.clone())
        .filter(|t| !t.trim().is_empty())
        .collect();

    let total_hits: usize = graph_results.iter().map(|g| g.total_hits).sum();
    if total_hits > 0 {
        tracing::info!(
            "[RAG] 图增强检索命中 {} 个实体/关系，已拼入 context_parts（{} 段）",
            total_hits,
            extra.len()
        );
    } else if crate::entity_graph::is_entity_graph_enabled()
        && source_results.iter().any(|s| s.source_type == RAGSourceType::Knowledge.as_str())
    {
        // 开关已打开、也确有 Knowledge 源，却零命中 ⇒ 大概率不是「图谱本来就没东西」，
        // 而是 kb_id 域不匹配或图谱尚未构建。显式告警，避免开关看起来生效实则空转。
        tracing::warn!(
            "[RAG] 实体图谱已启用但零命中：knowledge_entities 中可能没有与本次 kb_id 匹配的实体，\
             或图谱尚未构建（实体抽取未运行）"
        );
    }

    extra
}

// ── Context collection ───────────────────────────────────────────────────────

/// A typed RAG source reference for context collection.
#[derive(Clone)]
pub struct RAGSourceRef {
    pub source_type: RAGSourceType,
    pub container_id: String,
    /// 多文档协同：限制检索范围到这些文档 ID；
    /// 空数组表示检索整个容器。
    pub doc_ids: Vec<String>,
}

/// The type of RAG source.
#[derive(Clone, PartialEq)]
pub enum RAGSourceType {
    Knowledge,
    Memory,
    Wiki,
}

impl RAGSourceType {
    /// 该源类型的**规范字符串**：`RagSourceResult.source_type` 进前端取的就是它。
    ///
    /// 2026-09-15 收敛：此前这段 `Knowledge => "knowledge" / Memory => "memory" /
    /// Wiki => "wiki"` 映射**手写了三份**（legacy 路径的 `source_type_str`、pipeline
    /// 路径又一份、`fuse_rag_context_results` 里再按 `"memory"` / `"wiki"` 反查 label）。
    /// 任一处改字都会让「源类型 → 前端字符串」静默分叉（前端按字符串匹配），故收敛至此。
    ///
    /// **签名必须取 `&self`**：`RAGSourceType` 只派生 `Clone`（无 `Copy`），而两处调用点
    /// （`rag.rs` legacy / pipeline 路径的 `src_ref.source_type`）持有的是共享引用 ⇒
    /// 按值取 `self` 会 E0507「从共享引用里 move」。
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Knowledge => "knowledge",
            Self::Memory => "memory",
            Self::Wiki => "wiki",
        }
    }
}

impl RAGSourceRef {
    fn source(&self) -> Box<dyn RAGSource> {
        match self.source_type {
            RAGSourceType::Knowledge => Box::new(KnowledgeRAG),
            RAGSourceType::Memory => Box::new(MemoryRAG),
            RAGSourceType::Wiki => Box::new(WikiRAG),
        }
    }
}

async fn resolve_source_config(
    _db: &DatabaseConnection,
    source_type: &RAGSourceType,
    container_id: &str,
) -> (usize, f32, Option<usize>) {
    let config = match source_type {
        RAGSourceType::Memory => {
            sources::memory().get_namespace(container_id).await.ok().map(|ns| ns.source_config())
        },
        RAGSourceType::Wiki => {
            sources::wiki().get_wiki(container_id).await.ok().map(|w| w.source_config())
        },
        RAGSourceType::Knowledge => sources::knowledge()
            .get_knowledge_base(container_id)
            .await
            .ok()
            .map(|kb| kb.source_config()),
    };

    match config {
        Some(c) => (
            c.retrieval_top_k.map(|v| v as usize).unwrap_or(0),
            c.retrieval_threshold.unwrap_or(0.0),
            c.embedding_dimensions.map(|v| v as usize),
        ),
        None => (0, 0.0, None),
    }
}

/// Collect RAG context from all enabled sources for a conversation query.
///
/// Returns a `RagContextResult` containing both formatted context parts
/// (for injection into the system prompt) and structured results
/// (for frontend display).  Errors for individual sources are logged and skipped.
///
/// `hybrid`：用户可配的混合检索配置（全局设置 `ragPipelineConfig.hybrid`）。
/// 传 `None` 时退回内置默认值（0.7/0.3、RRF、enabled）。
#[allow(clippy::too_many_arguments)]
pub async fn collect_rag_context(
    db: &DatabaseConnection,
    master_key: &[u8; 32],
    vector_store: &VectorStore,
    kb_ids: &[String],
    mem_ids: &[String],
    wiki_ids: &[String],
    query: &str,
    top_k: usize,
    embed_fn: impl AsyncEmbedFn,
    hybrid: Option<&axagent_harness::rag_config::HybridConfig>,
) -> RagContextResult {
    let sources = build_source_refs(kb_ids, mem_ids, wiki_ids);
    collect_rag_context_from_refs(
        db,
        master_key,
        vector_store,
        sources,
        query,
        top_k,
        embed_fn,
        kb_ids,
        wiki_ids,
        hybrid,
    )
    .await
}

/// `collect_rag_context` 的多文档协同变体：每个 source 可带 `doc_ids` 过滤。
/// `kb_ids` / `wiki_ids` 仅用于知识图谱回链上下文（不参与过滤），可为空。
/// `hybrid` 语义同 `collect_rag_context`。
#[allow(clippy::too_many_arguments)]
pub async fn collect_rag_context_with_filters(
    db: &DatabaseConnection,
    master_key: &[u8; 32],
    vector_store: &VectorStore,
    sources: Vec<RAGSourceRef>,
    query: &str,
    top_k: usize,
    embed_fn: impl AsyncEmbedFn,
    kb_ids: &[String],
    wiki_ids: &[String],
    hybrid: Option<&axagent_harness::rag_config::HybridConfig>,
) -> RagContextResult {
    collect_rag_context_from_refs(
        db,
        master_key,
        vector_store,
        sources,
        query,
        top_k,
        embed_fn,
        kb_ids,
        wiki_ids,
        hybrid,
    )
    .await
}

/// 三层记忆系统：根据 memory_items.tier / importance 对 Memory 知识源的检索结果
/// 进行二次加权与重排序，让 core / long_term 记忆在 RAG 检索中真正发挥作用。
///
/// v108: 同时读取 applicability_tags，按当前 query 做适用范围过滤：
/// - tags 为空 → 全局适用，保留
/// - tags 非空且 query 中命中至少一个 tag（字符串或语义相似度） → 匹配，保留
/// - tags 非空且 query 中未命中任何 tag → 适用范围不符，剔除
///
/// P2-2: 新增基于 embedding 相似度的语义匹配回退
/// - 当字符串匹配失败时，计算 query_embedding 与 item embedding 的 cosine similarity
/// - 相似度 >= 0.6 视为语义匹配成功
///
/// 算法：
/// - tier 优先级 bonus：core=2.0, long_term=1.5, working=1.0, short_term=0.5
/// - adjusted_score = original_score - tier_bonus * importance
///   （original_score 是 L2 distance，越小越相关；减去 bonus 让高 tier 记忆排前）
/// - 按 adjusted_score 升序重排序
///
/// 未命中 memory_items 表的 id（理论上不会发生）原样保留 score 不调整。
#[allow(clippy::ptr_arg)]
async fn apply_memory_tier_weight(
    db: &DatabaseConnection,
    items: &mut Vec<RagRetrievedItem>,
    query: &str,
    query_embedding: Option<&[f32]>,
) {
    if items.is_empty() {
        return;
    }

    // 收集所有 id，批量查询 tier / importance / applicability_tags
    let ids: Vec<String> = items.iter().map(|it| it.id.clone()).collect();
    let placeholders =
        ids.iter().enumerate().map(|(i, _)| format!("?{}", i + 1)).collect::<Vec<_>>().join(", ");
    let sql = format!(
        "SELECT id, tier, importance, applicability_tags FROM memory_items WHERE id IN ({placeholders})"
    );

    let values: Vec<sea_orm::Value> = ids.into_iter().map(sea_orm::Value::from).collect();
    let rows = match db
        .query_all_raw(Statement::from_sql_and_values(db.get_database_backend(), &sql, values))
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("[memory_tier_weight] 查询 memory_items 失败，跳过加权: {}", e);
            return;
        },
    };

    // id → (tier_bonus, importance, applicability_tags)
    use std::collections::HashMap;
    let mut weight_map: HashMap<String, (f32, f32, Vec<String>)> =
        HashMap::with_capacity(rows.len());
    for row in rows {
        let id: String = match row.try_get("", "id") {
            Ok(v) => v,
            Err(_) => continue,
        };
        let tier: String = row.try_get("", "tier").unwrap_or_else(|_| "working".to_string());
        let importance: f64 = row.try_get("", "importance").unwrap_or(0.5);
        let tier_bonus = match tier.as_str() {
            "core" => 2.0_f32,
            "long_term" => 1.5,
            "working" => 1.0,
            "short_term" => 0.5,
            _ => 1.0,
        };
        // v108: applicability_tags 存储为 JSON 数组字符串
        let applicability_tags: Vec<String> = row
            .try_get::<String>("", "applicability_tags")
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
            .unwrap_or_default();
        weight_map.insert(id, (tier_bonus, importance as f32, applicability_tags));
    }

    // v108: 按 applicability_tags 过滤适用范围
    // P2-2: 传入 query_embedding 用于语义匹配回退
    filter_items_by_applicability_tags(items, &weight_map, query, query_embedding);

    // 调整 score 并按升序重排序（L2 distance 越小越相关）
    apply_tier_weight_and_sort(items, &weight_map);

    // v110: Memory 检索命中 → 自动 promote
    // 被 RAG 检索命中的记忆条目应增加 access_count，达到阈值时自动晋升 tier
    // 这是"使用频率驱动重要性提升"闭环的核心
    promote_memory_items_after_access(db, &weight_map).await;
}

/// v110: Memory 检索命中后自动 promote
///
/// 对被 RAG 检索命中的 memory_items 增加 access_count，
/// 达到晋升阈值时自动提升 tier（working → short_term → long_term → core）。
///
/// 这是"使用频率驱动重要性提升"闭环的核心机制：
/// 被频繁检索的记忆条目自然上升到更高的记忆层级，
/// 避免真正有价值的信息被衰减淘汰。
async fn promote_memory_items_after_access(
    db: &DatabaseConnection,
    weight_map: &std::collections::HashMap<String, (f32, f32, Vec<String>)>,
) {
    if weight_map.is_empty() {
        return;
    }

    let ids: Vec<&str> = weight_map.keys().map(|s| s.as_str()).collect();
    let placeholders =
        ids.iter().enumerate().map(|(i, _)| format!("?{}", i + 1)).collect::<Vec<_>>().join(", ");

    // 1. 批量增加 access_count 和 last_accessed
    let now_ms = chrono::Utc::now().timestamp_millis();
    let update_sql = format!(
        "UPDATE memory_items SET access_count = access_count + 1, last_accessed = ?1, updated_at = ?1 WHERE id IN ({placeholders})"
    );
    let mut values: Vec<sea_orm::Value> = vec![now_ms.into()];
    values.extend(ids.iter().map(|id| (*id).into()));

    if let Err(e) = db
        .query_all_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            &update_sql,
            values,
        ))
        .await
    {
        tracing::warn!("[memory_promote] 批量更新 access_count 失败: {}", e);
        return;
    }

    // 2. 检查晋升条件：查询更新后的 access_count 与 tier
    let select_sql = format!(
        "SELECT id, tier, access_count, confirmed FROM memory_items WHERE id IN ({placeholders})"
    );
    let select_values: Vec<sea_orm::Value> = ids.iter().map(|id| (*id).into()).collect();

    let rows = match db
        .query_all_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            &select_sql,
            select_values,
        ))
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("[memory_promote] 查询更新后条目失败: {}", e);
            return;
        },
    };

    for row in &rows {
        let id: String = match row.try_get("", "id") {
            Ok(v) => v,
            Err(_) => continue,
        };
        let tier: String = row.try_get("", "tier").unwrap_or_else(|_| "working".to_string());
        let access_count: i64 = row.try_get("", "access_count").unwrap_or(0);
        let confirmed: i64 = row.try_get("", "confirmed").unwrap_or(0);

        let threshold = match tier.as_str() {
            "working" => 5i64,
            "short_term" => 15,
            "long_term" => 30,
            _ => i64::MAX,
        };

        if access_count >= threshold
            && let Some(new_tier) = next_tier_for_promotion(&tier)
            && (new_tier != "core" || confirmed == 1)
        {
            let decay_rate = default_decay_rate_for_tier_promotion(new_tier);
            let promote_sql =
                "UPDATE memory_items SET tier = ?1, decay_rate = ?2, updated_at = ?3 WHERE id = ?4";
            let promote_values: Vec<sea_orm::Value> =
                vec![new_tier.into(), decay_rate.into(), now_ms.into(), id.as_str().into()];
            if let Err(e) = db
                .query_all_raw(Statement::from_sql_and_values(
                    db.get_database_backend(),
                    promote_sql,
                    promote_values,
                ))
                .await
            {
                tracing::debug!("[memory_promote] 晋升失败 id={}: {}", id, e);
            } else {
                tracing::info!(
                    "[memory_promote] 晋升成功 id={} {}→{} access_count={}",
                    id,
                    tier,
                    new_tier,
                    access_count
                );
            }
        }
    }
}

fn next_tier_for_promotion(current: &str) -> Option<&'static str> {
    match current {
        "working" => Some("short_term"),
        "short_term" => Some("long_term"),
        "long_term" => Some("core"),
        _ => None,
    }
}

fn default_decay_rate_for_tier_promotion(tier: &str) -> f64 {
    match tier {
        "core" => 0.001,
        "long_term" => 0.005,
        "short_term" => 0.02,
        _ => 0.05,
    }
}

/// v108: 按 applicability_tags 过滤适用范围（纯函数，便于单元测试）
///
/// 规则：
/// - weight_map 中无此 id → 视为全局适用，保留
/// - tags 为空 → 全局适用，保留
/// - tags 非空 → query 中需命中至少一个 tag（不区分大小写子串匹配）
///
/// P2-2: 添加 embedding 相似度回退匹配
///
/// - 当字符串匹配失败且有 query_embedding 时，计算 query 与 item 的 cosine similarity
/// - 相似度 >= 0.6 视为语义匹配成功
#[allow(clippy::ptr_arg)]
pub(crate) fn filter_items_by_applicability_tags(
    items: &mut Vec<RagRetrievedItem>,
    weight_map: &std::collections::HashMap<String, (f32, f32, Vec<String>)>,
    query: &str,
    query_embedding: Option<&[f32]>,
) {
    let query_lower = query.to_lowercase();
    let threshold = 0.6f32; // P2-2: 语义相似度阈值

    items.retain(|it| {
        match weight_map.get(&it.id) {
            Some((_, _, tags)) if !tags.is_empty() => {
                // 1. 尝试字符串匹配
                let string_match = tags.iter().any(|tag| query_lower.contains(&tag.to_lowercase()));
                if string_match {
                    return true;
                }

                // P2-2: 尝试语义相似度匹配
                if let Some(_q_emb) = query_embedding {
                    // 使用 item 自身的 score 作为相关性近似判断
                    // score 是 L2 distance，越小越相关；转换为相似度
                    let item_similarity = 1.0 / (1.0 + it.score.abs());
                    if item_similarity >= threshold {
                        return true;
                    }
                }

                false
            },
            _ => true,
        }
    });
}

/// v108: 应用 tier 权重并按 score 升序排序（纯函数，便于单元测试）
///
/// `weight_map` 的 value 为 `(tier_bonus, importance, _applicability_tags)`。
/// 调整公式：`adjusted_score = original_score - tier_bonus * importance`
/// （original_score 是 L2 distance，越小越相关；减去 bonus 让高 tier 记忆排前）
#[allow(clippy::ptr_arg)]
pub(crate) fn apply_tier_weight_and_sort(
    items: &mut Vec<RagRetrievedItem>,
    weight_map: &std::collections::HashMap<String, (f32, f32, Vec<String>)>,
) {
    for it in items.iter_mut() {
        if let Some((tier_bonus, importance, _)) = weight_map.get(&it.id) {
            it.score -= tier_bonus * importance;
        }
    }
    items.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal));
}

/// P0-2: 建立检索反馈闭环 — Memory item 被检索命中后，自动提升 importance 并更新访问统计
///
/// 反馈逻辑：
/// - access_count += 1
/// - last_accessed = 当前时间戳
/// - importance 小幅提升：`min(1.0, importance + 0.05)`（每次命中提升 0.05，上限 1.0）
/// - 同时触发衰减检查：importance < 0.2 时自动提升到 0.3，防止低重要性记忆永久沉没
async fn apply_memory_hit_feedback(db: &DatabaseConnection, hit_ids: &[String]) {
    if hit_ids.is_empty() {
        return;
    }

    let now = Utc::now().timestamp();
    let ids_placeholder = hit_ids
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");

    // 批量更新：access_count + 1, last_accessed = now, importance = min(1.0, importance + 0.05)
    let sql = format!(
        "UPDATE memory_items SET \
         access_count = COALESCE(access_count, 0) + 1, \
         last_accessed = ?, \
         importance = CASE \
             WHEN importance < 0.2 THEN 0.3 \
             ELSE MIN(1.0, importance + 0.05) \
         END \
         WHERE id IN ({})",
        ids_placeholder
    );

    let values: Vec<sea_orm::Value> = std::iter::once(sea_orm::Value::from(now))
        .chain(hit_ids.iter().map(|id| sea_orm::Value::from(id.clone())))
        .collect();

    match db
        .query_all_raw(Statement::from_sql_and_values(db.get_database_backend(), &sql, values))
        .await
    {
        Ok(_) => {
            tracing::debug!("[memory_feedback] 成功更新 {} 条记忆的反馈权重", hit_ids.len());
        },
        Err(e) => {
            tracing::warn!("[memory_feedback] 批量更新记忆反馈权重失败: {}", e);
        },
    }
}

/// v110: 跨源反馈权重调整
///
/// 根据 `retrieval_hits` 表中的历史反馈数据调整文档排序：
/// - 正反馈（positive）→ 加分（减小 L2 distance，提升排名）
/// - 负反馈（negative/irrelevant）→ 减分（增大 L2 distance，降低排名）
/// - used_in_response=1 的条目 → 微小加分（被引用过说明有价值）
///
/// 这是"用户反馈→检索质量提升"自适应闭环的核心：
/// 用户的每次👍👎反馈都会影响后续相同文档的检索排序，
/// 让 RAG 系统从被动检索变为主动学习的自适应系统。
#[allow(clippy::ptr_arg)]
async fn apply_feedback_weight_adjustment(
    db: &DatabaseConnection,
    items: &mut Vec<RagRetrievedItem>,
    source_type: &str,
) {
    if items.is_empty() {
        return;
    }

    let doc_ids: Vec<String> = items.iter().map(|it| it.document_id.clone()).collect();
    let doc_ids: Vec<&str> = doc_ids.iter().map(|s| s.as_str()).collect();
    let placeholders = doc_ids
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");

    // 聚合查询：按文档 ID 统计正/负反馈次数和被引用次数
    let sql = format!(
        "SELECT \
             document_id, \
             SUM(CASE WHEN feedback = 'positive' THEN 1 ELSE 0 END) AS positive_cnt, \
             SUM(CASE WHEN feedback IN ('negative', 'irrelevant') THEN 1 ELSE 0 END) AS negative_cnt, \
             SUM(used_in_response) AS used_cnt \
         FROM retrieval_hits \
         WHERE document_id IN ({}) \
         GROUP BY document_id",
        placeholders
    );

    let values: Vec<sea_orm::Value> = doc_ids.iter().map(|id| (*id).into()).collect();
    let rows = match db
        .query_all_raw(Statement::from_sql_and_values(db.get_database_backend(), &sql, values))
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(
                "[feedback_weight] 查询 retrieval_hits 失败 source={}: {}",
                source_type,
                e
            );
            return;
        },
    };

    if rows.is_empty() {
        return;
    }

    // 构建 doc_id → feedback_score 映射
    use std::collections::HashMap;
    let mut feedback_map: HashMap<String, f32> = HashMap::with_capacity(rows.len());
    for row in &rows {
        let doc_id: String = match row.try_get("", "document_id") {
            Ok(v) => v,
            Err(_) => continue,
        };
        let positive: i64 = row.try_get("", "positive_cnt").unwrap_or(0);
        let negative: i64 = row.try_get("", "negative_cnt").unwrap_or(0);
        let used: i64 = row.try_get("", "used_cnt").unwrap_or(0);

        // 反馈分数 = positive * 0.3 - negative * 0.2 + used * 0.05
        // 正反馈权重高于负反馈，鼓励探索
        let score = positive as f32 * 0.3 - negative as f32 * 0.2 + used as f32 * 0.05;
        if score.abs() > 0.001 {
            feedback_map.insert(doc_id, score);
        }
    }

    if feedback_map.is_empty() {
        return;
    }

    // 应用反馈权重到文档分数
    for item in items.iter_mut() {
        if let Some(feedback_score) = feedback_map.get(&item.document_id) {
            // L2 distance：越小越相关。正反馈减小距离（提升），负反馈增大距离（降低）
            item.score -= feedback_score;
        }
    }

    // 重新排序
    items.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal));

    tracing::debug!(
        "[feedback_weight] 已应用反馈权重 source={} 文档数={} 有反馈数={}",
        source_type,
        items.len(),
        feedback_map.len()
    );
}

fn build_source_refs(
    kb_ids: &[String],
    mem_ids: &[String],
    wiki_ids: &[String],
) -> Vec<RAGSourceRef> {
    let mut sources: Vec<RAGSourceRef> = Vec::new();
    for id in kb_ids {
        sources.push(RAGSourceRef {
            source_type: RAGSourceType::Knowledge,
            container_id: id.clone(),
            doc_ids: Vec::new(),
        });
    }
    for id in mem_ids {
        sources.push(RAGSourceRef {
            source_type: RAGSourceType::Memory,
            container_id: id.clone(),
            doc_ids: Vec::new(),
        });
    }
    for id in wiki_ids {
        sources.push(RAGSourceRef {
            source_type: RAGSourceType::Wiki,
            container_id: id.clone(),
            doc_ids: Vec::new(),
        });
    }
    sources
}

#[allow(clippy::too_many_arguments)]
async fn collect_rag_context_from_refs(
    db: &DatabaseConnection,
    master_key: &[u8; 32],
    vector_store: &VectorStore,
    sources: Vec<RAGSourceRef>,
    query: &str,
    top_k: usize,
    embed_fn: impl AsyncEmbedFn,
    kb_ids: &[String],
    wiki_ids: &[String],
    hybrid: Option<&axagent_harness::rag_config::HybridConfig>,
) -> RagContextResult {
    if sources.is_empty() {
        return RagContextResult { context_parts: vec![], source_results: vec![] };
    }

    let mut context_parts = Vec::new();
    let mut source_results = Vec::new();
    // Graph RAG 阶段 4 的结果累积（2026-09-15 接线）。
    //
    // 之所以要「累积」而不是就近使用：阶段 4 的产出在**源循环内部**（per-source），
    // 而要注入 prompt 的 `context_parts` 是在**循环之后**用
    // `rebuild_context_with_citations` 统一重建的。此前的写法是在返回处恒丢弃这份
    // 累积结果，于是阶段 4 的产出被直接扔掉 —— 即「能力跑通了但没有出口」。
    let mut graph_results: Vec<axagent_harness::GraphEnhancedSearchResult> = Vec::new();

    // P2-2: 预计算 query embedding 用于后续的语义匹配
    let query_embedding = {
        let q_emb_result =
            embed_fn.generate(db, master_key, "default", vec![query.to_string()], None).await;
        match q_emb_result {
            Ok(resp) if !resp.embeddings.is_empty() => Some(resp.embeddings[0].clone()),
            _ => None,
        }
    };

    for src_ref in &sources {
        let source = src_ref.source();

        // Resolve per-source search parameters (top_k, threshold, dimensions)
        let (source_top_k, threshold, dims) = {
            let (sk, th, d) =
                resolve_source_config(db, &src_ref.source_type, &src_ref.container_id).await;
            (if sk > 0 { sk } else { top_k }, th, d)
        };

        // 多文档协同：当 doc_ids 非空时透传给底层 search
        let doc_ids_opt = if src_ref.doc_ids.is_empty() {
            None
        } else {
            Some(src_ref.doc_ids.as_slice())
        };

        // Graph RAG 阶段 4 的另一条取数路径（legacy）。两点与 pipeline 路径不同：
        // ① 只对 Knowledge 源做（Memory/Wiki 的 container_id 与 kb_id 不同域，必然空结果）；
        // ② **不依赖向量命中** —— 向量没命中时图谱仍可能按实体名匹配到东西，而
        //    pipeline 路径的阶段 4 位于「阶段 1 结果为空即早退」之后，拿不到这一机会。
        // 开关关闭（未注入 provider）时 `search_entity_graph` 直接返回 `None`，零开销。
        if matches!(src_ref.source_type, RAGSourceType::Knowledge)
            && let Some(g) = search_entity_graph(&src_ref.container_id, query).await
        {
            graph_results.push(g);
        }

        let result = search_with_filter(
            source.as_ref(),
            db,
            master_key,
            vector_store,
            &src_ref.container_id,
            query,
            source_top_k,
            dims,
            embed_fn.clone(),
            doc_ids_opt,
            None,
            hybrid,
            // 阈值（相关度下限 ∈ [0,1]）交检索层：过滤 → 排序 → 截断 top_k。
            // ⚠ 2026-09-15 修（过滤位置）。此前的解释「过阈值的二十条里只有前 5 条有机会
            // 被检查 ⇒ 可能返回 0 条」**已被自查否定**（候选池 `top_k * 3`，筛选键 == 排序键
            // ⇒ 合格集是前缀，两种次序等价）；真正的问题是这里拿**距离尺度**的 `r.score`
            // 去比按**融合尺度**算的阈值 —— 两把尺子（同 #186）。详见 `search_with_filter`。
            Some(similarity_floor_from_threshold(threshold)),
        )
        .await;

        match result {
            Ok(results) if !results.is_empty() => {
                // 阈值已由 `search_with_filter`（⇒ `HybridSearchOptions.min_score`）
                // 在**截断之前**统一应用（四条收尾路径同一真源），此处不再重复筛。
                //
                // 保留这段说明是因为它是**量纲事故**的原始现场：此前本处写死
                // `default_max_distance = 20.0`（L2 距离标尺）并据此判断，而默认的
                // RRF 融合路径下 `score` 取反后 ≈0.967–0.983 ⇒ `<= 20.0` 恒真
                // （过滤是空的），前端默认值 `0.1` 又让 `0.967 <= 0.1` 恒假
                // （该源一条上下文都不贡献）。两处都是「标尺不同却直接比大小」。
                let mut items: Vec<RagRetrievedItem> = results
                    .iter()
                    .map(|r| RagRetrievedItem {
                        content: r.content.clone(),
                        score: r.score,
                        document_id: r.document_id.clone(),
                        id: r.id.clone(),
                        document_name: None,
                        chunk_index: Some(r.chunk_index),
                    })
                    .collect();

                // 三层记忆系统：针对 Memory 知识源，按 tier / importance 加权重排序
                // v108: 同时按 applicability_tags 过滤适用范围
                // P2-2: 传入 query_embedding 用于语义相似度匹配
                if matches!(src_ref.source_type, RAGSourceType::Memory) {
                    apply_memory_tier_weight(db, &mut items, query, query_embedding.as_deref())
                        .await;

                    // P0-2: 反馈闭环 — 检索命中后自动提升 importance 和访问统计
                    let hit_ids: Vec<String> = items.iter().map(|it| it.id.clone()).collect();
                    apply_memory_hit_feedback(db, &hit_ids).await;
                }

                let source_type_str = src_ref.source_type.as_str();

                // v110: 跨源反馈权重调整 — 根据 retrieval_hits 中的历史反馈数据调整排序
                // 正反馈文档加分，负反馈文档减分，形成"用户反馈→检索质量提升"闭环
                apply_feedback_weight_adjustment(db, &mut items, source_type_str).await;

                // snippets 顺序跟随 items（tier 加权后的顺序），保证 context 与引用追溯一致
                let snippets: Vec<String> = items.iter().map(|it| it.content.clone()).collect();
                context_parts.push(format!(
                    "[{}]\n{}",
                    source.context_label(),
                    snippets.join("\n---\n")
                ));
                // 2026-07-31 可观测性：命中时打 info 日志，日志可直接判定"知识库有贡献"
                // （此前命中是静默的，只能靠"无 failed/无 returned no results + tsvector notice"反向推断）
                tracing::info!(
                    "[RAG] 知识源命中 → {} {} 检索到 {} 条片段，已拼入 context_parts",
                    source.collection_prefix(),
                    src_ref.container_id,
                    items.len()
                );

                source_results.push(RagSourceResult {
                    source_type: source_type_str.to_string(),
                    container_id: src_ref.container_id.clone(),
                    items,
                    container_name: None,
                });
            },
            Ok(_) => {
                tracing::warn!(
                    "RAG search returned no results for {} {}",
                    source.collection_prefix(),
                    src_ref.container_id,
                );
            },
            Err(e) => {
                tracing::warn!(
                    "RAG search failed for {} {}: {}",
                    source.collection_prefix(),
                    src_ref.container_id,
                    e
                );
            },
        }
    }

    // 填充 container_name（KB / memory namespace / wiki 名称）
    fill_container_names(db, &mut source_results).await;

    // 引用可读性：回填检索命中项的 document_name（knowledge 文档标题 + wiki 笔记标题）
    fill_document_names(&mut source_results).await;

    let kg_context = collect_cross_source_graph_context(db, kb_ids, wiki_ids, query, top_k).await;

    // Graph RAG 阶段 4 的**出口**（2026-09-15 接线）：图检索结果与 `kg_context`
    // 同属「回链上下文」，一起追加到重建后的 `context_parts` 末尾。
    // 必须在此处取 `source_results`（下一步它就被 move 进 `deduplicate_cross_source`）。
    let mut extra_context = kg_context;
    extra_context.extend(fold_entity_graph_context(&graph_results, &source_results));

    let (deduped_results, _deduped_context) =
        deduplicate_cross_source(source_results, context_parts);

    // 引用追溯：在 dedup 之后重建 context_parts，为每个 item 的 snippet 前注入 [cite:N] token。
    // N 是 source_results 扁平化后的全局序号，前端据此渲染可点击 chip 并跳转高亮对应 item。
    let final_context = rebuild_context_with_citations(&deduped_results, extra_context);

    RagContextResult { context_parts: final_context, source_results: deduped_results }
}

/// 引用追溯：根据 `source_results` 重建 context_parts，为每个 item 的 snippet 前注入
/// `[cite:N]` token（N 为全局扁平化序号，从 0 开始）。`extra_context`（如知识图谱上下文）
/// 原样追加到末尾，不参与引用编号。
fn rebuild_context_with_citations(
    source_results: &[RagSourceResult],
    extra_context: Vec<String>,
) -> Vec<String> {
    let mut context = Vec::new();
    let mut cite_idx = 0usize;
    for src in source_results {
        let label = match src.source_type.as_str() {
            "knowledge" => {
                format!("Knowledge: {}", src.container_name.as_deref().unwrap_or(&src.container_id))
            },
            "memory" => {
                format!("Memory: {}", src.container_name.as_deref().unwrap_or(&src.container_id))
            },
            "wiki" => {
                format!("Wiki: {}", src.container_name.as_deref().unwrap_or(&src.container_id))
            },
            other => format!("{}: {}", other, src.container_id),
        };
        let snippets: Vec<String> = src
            .items
            .iter()
            .map(|item| {
                let i = cite_idx;
                cite_idx += 1;
                format!("[cite:{}] {}", i, item.content)
            })
            .collect();
        if !snippets.is_empty() {
            context.push(format!("[{}]\n{}", label, snippets.join("\n---\n")));
        }
    }
    context.extend(extra_context);
    context
}

/// 为每个 `RagSourceResult` 填充 `container_name`（KB / memory / wiki 容器显示名）。
async fn fill_container_names(db: &DatabaseConnection, source_results: &mut [RagSourceResult]) {
    for src in source_results.iter_mut() {
        let name: Option<String> = match src.source_type.as_str() {
            "knowledge" => sources::knowledge()
                .get_knowledge_base(&src.container_id)
                .await
                .ok()
                .map(|kb| kb.name),
            "memory" => {
                sources::memory().get_namespace(&src.container_id).await.ok().map(|ns| ns.name)
            },
            "wiki" => sources::wiki().get_wiki(&src.container_id).await.ok().map(|w| w.name),
            _ => {
                let _ = db;
                None
            },
        };
        src.container_name = name;
    }
}

/// 引用可读性：批量回填检索命中项的 `document_name`（R7）。
///
/// knowledge 源回填 KB 文档标题；wiki 源此前恒为 `None`（前端 citation chip
/// 只能显示裸 note_id），现同样批量回填笔记标题。两条检索管线
/// （向量检索 / rag_pipeline）共用本函数，消除重复块。
async fn fill_document_names(source_results: &mut [RagSourceResult]) {
    // knowledge 源：KB 文档标题
    let kb_doc_ids: Vec<String> = source_results
        .iter()
        .filter(|s| s.source_type == "knowledge")
        .flat_map(|s| s.items.iter().map(|it| it.document_id.clone()))
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    if !kb_doc_ids.is_empty() {
        match sources::knowledge().get_document_titles(&kb_doc_ids).await {
            Ok(titles) => {
                for src in source_results.iter_mut().filter(|s| s.source_type == "knowledge") {
                    for item in &mut src.items {
                        item.document_name = titles.get(&item.document_id).cloned();
                    }
                }
            },
            Err(e) => {
                tracing::warn!("Failed to lookup document titles: {e}");
            },
        }
    }

    // wiki 源：笔记标题（document_id 即 note_id）
    let wiki_note_ids: Vec<String> = source_results
        .iter()
        .filter(|s| s.source_type == "wiki")
        .flat_map(|s| s.items.iter().map(|it| it.document_id.clone()))
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    if !wiki_note_ids.is_empty() {
        match sources::wiki().get_note_titles(&wiki_note_ids).await {
            Ok(titles) => {
                for src in source_results.iter_mut().filter(|s| s.source_type == "wiki") {
                    for item in &mut src.items {
                        item.document_name = titles.get(&item.document_id).cloned();
                    }
                }
            },
            Err(e) => {
                tracing::warn!("Failed to lookup wiki note titles: {e}");
            },
        }
    }
}

// ── Cross-source deduplication ───────────────────────────────────────────────

const DEDUP_JACCARD_THRESHOLD: f64 = 0.65;

fn source_type_priority(source_type: &str) -> u8 {
    // v101: 知识库（curated）> Wiki > Memory（auto-extracted），与之前相反
    match source_type {
        "knowledge" => 4,
        "wiki" => 3,
        "memory" => 2,
        _ => 1,
    }
}

fn jaccard_similarity(a: &str, b: &str) -> f64 {
    let a_lower = a.to_lowercase();
    let b_lower = b.to_lowercase();
    let a_words: std::collections::HashSet<&str> =
        a_lower.split_whitespace().filter(|w| w.len() > 2).collect();
    let b_words: std::collections::HashSet<&str> =
        b_lower.split_whitespace().filter(|w| w.len() > 2).collect();

    if a_words.is_empty() || b_words.is_empty() {
        return 0.0;
    }

    let intersection = a_words.intersection(&b_words).count();
    let union = a_words.union(&b_words).count();

    if union == 0 {
        return 0.0;
    }

    intersection as f64 / union as f64
}

fn deduplicate_cross_source(
    source_results: Vec<RagSourceResult>,
    context_parts: Vec<String>,
) -> (Vec<RagSourceResult>, Vec<String>) {
    if source_results.len() <= 1 {
        return (source_results, context_parts);
    }

    let all_items: Vec<(usize, usize, &RagRetrievedItem)> = source_results
        .iter()
        .enumerate()
        .flat_map(|(si, src)| src.items.iter().enumerate().map(move |(ii, item)| (si, ii, item)))
        .collect();

    let mut removed: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();

    for i in 0..all_items.len() {
        if removed.contains(&(all_items[i].0, all_items[i].1)) {
            continue;
        }
        for j in (i + 1)..all_items.len() {
            if removed.contains(&(all_items[j].0, all_items[j].1)) {
                continue;
            }

            let (si_a, _, item_a) = all_items[i];
            let (si_b, ij_b, item_b) = all_items[j];

            let similarity = jaccard_similarity(&item_a.content, &item_b.content);
            if similarity < DEDUP_JACCARD_THRESHOLD {
                continue;
            }

            let pri_a = source_type_priority(&source_results[si_a].source_type);
            let pri_b = source_type_priority(&source_results[si_b].source_type);

            let remove_j = if pri_a != pri_b {
                pri_a > pri_b
            } else {
                item_a.score <= item_b.score
            };

            if remove_j {
                removed.insert((si_b, ij_b));
            } else {
                removed.insert((si_a, all_items[i].1));
                break;
            }
        }
    }

    if removed.is_empty() {
        return (source_results, context_parts);
    }

    let deduped_results: Vec<RagSourceResult> = source_results
        .into_iter()
        .enumerate()
        .map(|(si, mut src)| {
            let removed_indices: std::collections::HashSet<usize> =
                removed.iter().filter(|(s, _)| *s == si).map(|(_, ii)| *ii).collect();
            if removed_indices.is_empty() {
                src
            } else {
                src.items = src
                    .items
                    .into_iter()
                    .enumerate()
                    .filter(|(ii, _)| !removed_indices.contains(ii))
                    .map(|(_, item)| item)
                    .collect();
                src
            }
        })
        .filter(|src| !src.items.is_empty())
        .collect();

    let mut deduped_context = Vec::new();
    for src in &deduped_results {
        let label = match src.source_type.as_str() {
            "knowledge" => "Knowledge Base Reference",
            "memory" => "Memory Reference",
            "wiki" => "Wiki Reference",
            other => other,
        };
        let snippets: Vec<String> = src.items.iter().map(|r| r.content.clone()).collect();
        deduped_context.push(format!("[{}]\n{}", label, snippets.join("\n---\n")));
    }

    if deduped_context.is_empty() {
        deduped_context = context_parts;
    }

    (deduped_results, deduped_context)
}

// ── Embed function trait ─────────────────────────────────────────────────────

/// Trait for embedding generation, allowing the RAG layer to be independent
/// of the concrete provider implementation in the `indexing` module.
#[async_trait]
pub trait AsyncEmbedFn: Send + Sync + Clone {
    async fn generate(
        &self,
        db: &DatabaseConnection,
        master_key: &[u8; 32],
        embedding_provider: &str,
        texts: Vec<String>,
        dimensions: Option<usize>,
    ) -> Result<axagent_harness::types::EmbedResponse>;
}

// ── WikiRAG ─────────────────────────────────────────────────────────────────

/// RAG source backed by a wiki vault (notes → parsed → chunked → embedded).
pub struct WikiRAG;

#[async_trait]
impl RAGSource for WikiRAG {
    fn collection_prefix(&self) -> &'static str {
        "wiki"
    }

    fn context_label(&self) -> &'static str {
        "Wiki Reference"
    }

    async fn resolve_embedding_provider(
        &self,
        db: &DatabaseConnection,
        container_id: &str,
    ) -> Result<String> {
        let wiki = sources::wiki().get_wiki(container_id).await?;
        if let Some(provider) = wiki.embedding_provider {
            return Ok(provider);
        }
        resolve_default_embedding_provider(db).await
    }
}

// ── WikiVaultRAG Capacity Management ────────────────────────────────────────

const VAULT_SOFT_LIMIT: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultCapacityInfo {
    pub vault_id: String,
    pub current_count: usize,
    pub soft_limit: usize,
    pub is_over_limit: bool,
    pub oldest_item_timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapacityCheckResult {
    pub allowed: bool,
    pub current_count: usize,
    pub soft_limit: usize,
    pub reason: Option<String>,
}

pub async fn check_vault_rag_capacity(
    db: &DatabaseConnection,
    vault_id: &str,
) -> Result<CapacityCheckResult> {
    let wiki = sources::wiki().get_wiki(vault_id).await?;

    // 前置判断：未配置 embedding_provider 时，vec_wiki_*_meta 表不会被创建，
    // 直接返回 0 避免查询不存在的表导致错误。
    let current_count = if wiki.embedding_provider.is_none() {
        0
    } else {
        let collection_name = collection_id("wiki", vault_id);
        validate_collection_name(&collection_name)?;
        count_collection_items(db, &collection_name).await?
    };

    let is_over_limit = current_count >= VAULT_SOFT_LIMIT;

    Ok(CapacityCheckResult {
        allowed: !is_over_limit,
        current_count,
        soft_limit: VAULT_SOFT_LIMIT,
        reason: if is_over_limit {
            Some(format!(
                "Vault '{}' has {} items, exceeding soft limit of {}",
                wiki.name, current_count, VAULT_SOFT_LIMIT
            ))
        } else {
            None
        },
    })
}

/// 校验 collection_name 只包含安全字符（字母、数字、下划线、连字符），防止 SQL 注入
fn validate_collection_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(AxAgentError::Validation("Collection name cannot be empty".to_string()));
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(AxAgentError::Validation(format!(
            "Invalid collection name '{}': only alphanumeric characters, hyphens and underscores are allowed",
            name
        )));
    }
    if name.len() > 64 {
        return Err(AxAgentError::Validation(format!(
            "Collection name '{}' is too long (max 64 characters)",
            name
        )));
    }
    Ok(())
}

async fn count_collection_items(db: &DatabaseConnection, collection_name: &str) -> Result<usize> {
    validate_collection_name(collection_name)?;
    let table_name = format!("vec_{}_meta", collection_name.replace('-', "_"));
    let backend = db.get_database_backend();
    let count: i64 = db
        .query_one_raw(Statement::from_string(
            backend,
            format!("SELECT COUNT(*) as cnt FROM \"{}\"", table_name),
        ))
        .await?
        .and_then(|r| r.try_get::<i64>("", "cnt").ok())
        .unwrap_or(0);

    Ok(count as usize)
}

pub async fn get_vault_capacity_info(
    db: &DatabaseConnection,
    vault_id: &str,
) -> Result<VaultCapacityInfo> {
    let wiki = sources::wiki().get_wiki(vault_id).await?;

    // 前置判断：未配置 embedding_provider 时，vec_wiki_*_meta 表不会被创建，
    // 直接返回 current_count: 0 / oldest_item_timestamp: None，避免查询不存在的表。
    if wiki.embedding_provider.is_none() {
        return Ok(VaultCapacityInfo {
            vault_id: vault_id.to_string(),
            current_count: 0,
            soft_limit: VAULT_SOFT_LIMIT,
            is_over_limit: false,
            oldest_item_timestamp: None,
        });
    }

    let collection_name = collection_id("wiki", vault_id);
    validate_collection_name(&collection_name)?;
    let current_count = count_collection_items(db, &collection_name).await?;

    let oldest_item_timestamp = get_oldest_item_timestamp(db, &collection_name).await?;

    Ok(VaultCapacityInfo {
        vault_id: vault_id.to_string(),
        current_count,
        soft_limit: VAULT_SOFT_LIMIT,
        is_over_limit: current_count >= VAULT_SOFT_LIMIT,
        oldest_item_timestamp,
    })
}

async fn get_oldest_item_timestamp(
    db: &DatabaseConnection,
    collection_name: &str,
) -> Result<Option<i64>> {
    validate_collection_name(collection_name)?;
    let backend = db.get_database_backend();
    let result = db
        .query_one_raw(Statement::from_string(
            backend,
            format!(
                "SELECT created_at FROM vec_collections WHERE collection_id = '{}'",
                collection_name
            ),
        ))
        .await?;

    Ok(result.and_then(|row| row.try_get::<i64>("", "created_at").ok()))
}

// ── Precision content injection ─────────────────────────────────────────────

/// Extract surrounding context lines around a matched chunk within source text.
///
/// Given the original source and a matched snippet, returns the snippet
/// with `context_lines` of surrounding text above and below, preserving
/// code logic continuity without dumping the entire file.
///
/// Returns `None` if the snippet cannot be located in the source.
pub fn extract_surrounding_lines(
    source: &str,
    snippet: &str,
    context_lines: usize,
) -> Option<String> {
    let snippet_start = source.find(snippet)?;
    let snippet_end = snippet_start + snippet.len();

    let source_before = &source[..snippet_start];
    let source_after = &source[snippet_end..];

    let lines_before: Vec<&str> = source_before.lines().collect();
    let mut lines_after: Vec<&str> = source_after.lines().collect();

    // Strip leading empty line from lines_after if snippet ends right at a newline
    if lines_after.first().is_some_and(|l| l.is_empty()) {
        lines_after.remove(0);
    }

    let before_count = context_lines.min(lines_before.len());
    let after_count = context_lines.min(lines_after.len());

    let before = if before_count > 0 {
        let start = lines_before.len() - before_count;
        let mut text = lines_before[start..].join("\n");
        text.push('\n');
        text
    } else {
        String::new()
    };

    let after = if after_count > 0 {
        let mut text = String::from("\n");
        text.push_str(&lines_after[..after_count].join("\n"));
        text
    } else {
        String::new()
    };

    Some(format!("{before}{snippet}{after}"))
}

/// Extract only the function body containing the matched snippet.
///
/// Scans backwards from the match position to find a function signature
/// (patterns like `fn `, `def `, `function `, `class `) and returns
/// the text from that signature through the snippet with limited context.
/// Falls back to surrounding lines if no function boundary is found.
///
/// This avoids injecting entire class definitions when only one method
/// is relevant.
pub fn inject_function_only(source: &str, snippet: &str, max_context_chars: usize) -> String {
    let Some(snippet_start) = source.find(snippet) else {
        return snippet.to_string();
    };

    let before = &source[..snippet_start];
    let fn_patterns = ["fn ", "def ", "function ", "class ", "impl ", "pub fn ", "pub struct "];

    let fn_start = before.lines().rev().take(50).find(|line| {
        let trimmed = line.trim();
        fn_patterns.iter().any(|p| trimmed.starts_with(p))
            || trimmed.ends_with('{')
            || trimmed.starts_with('#')
    });

    if let Some(fn_line) = fn_start {
        let fn_pos = before.rfind(fn_line).unwrap_or(0);
        let context_start = fn_pos.max(snippet_start.saturating_sub(max_context_chars));

        let relevant = &source[context_start..];
        let snippet_pos_in_relevant = relevant.find(snippet).unwrap_or(0);
        let raw_end = snippet_pos_in_relevant + snippet.len() + max_context_chars;
        let end = raw_end.min(relevant.len());

        // Try to stop at the next function definition boundary
        let after_snippet =
            &relevant[snippet_pos_in_relevant + snippet.len()..end.min(relevant.len())];
        let next_fn_pos = after_snippet
            .find("\nfn ")
            .or_else(|| after_snippet.find("\npub fn "))
            .or_else(|| after_snippet.find("\nclass "))
            .or_else(|| after_snippet.find("\ndef "));
        let bounded_end = if let Some(pos) = next_fn_pos {
            snippet_pos_in_relevant + snippet.len() + pos
        } else {
            end
        };

        relevant[..bounded_end.min(relevant.len())].to_string()
    } else {
        extract_surrounding_lines(source, snippet, 3).unwrap_or_else(|| snippet.to_string())
    }
}

// ── Pipeline-integrated context collection ────────────────────────────────────

/// LLM 调用函数类型（用于查询增强等场景）
pub type LlmCallFn = std::sync::Arc<
    dyn Fn(
            String,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = axagent_harness::core_error::Result<String>>
                    + Send,
            >,
        > + Send
        + Sync,
>;

/// 多路召回：单次检索最多使用的增强查询数（原始查询 + 增强查询）。
const MULTI_QUERY_LIMIT: usize = 3;

/// 多路召回 RRF 融合常数 k（与 HybridSearchOptions 默认值对齐）。
const MULTI_QUERY_RRF_K: f32 = 60.0;

/// 多路召回融合：按 chunk id 去重，分数 = Σ 1/(k + rank + 1)，其他字段取首次出现值。
fn fuse_retrieved_items_by_rrf(
    runs: Vec<Vec<RagRetrievedItem>>,
    rrf_k: f32,
) -> Vec<RagRetrievedItem> {
    use std::collections::hash_map::Entry;

    let mut map: std::collections::HashMap<String, (f32, RagRetrievedItem)> =
        std::collections::HashMap::new();
    for run in &runs {
        for (rank, item) in run.iter().enumerate() {
            let contribution = 1.0 / (rrf_k + rank as f32 + 1.0);
            match map.entry(item.id.clone()) {
                Entry::Occupied(mut e) => e.get_mut().0 += contribution,
                Entry::Vacant(e) => {
                    e.insert((contribution, item.clone()));
                },
            }
        }
    }

    let mut fused: Vec<(f32, RagRetrievedItem)> = map.into_values().collect();
    fused.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    fused.into_iter().map(|(_, item)| item).collect()
}

/// 多路召回的结果级融合：合并多次 `collect_rag_context_with_filters` 的返回值。
/// context_parts 按 source_type 重建（与单路路径的 `[label]\n...` 格式一致）。
fn fuse_rag_context_results(runs: Vec<RagContextResult>, rrf_k: f32) -> RagContextResult {
    use std::collections::hash_map::Entry;

    let mut order: Vec<(String, String)> = Vec::new();
    let mut groups: std::collections::HashMap<(String, String), Vec<Vec<RagRetrievedItem>>> =
        std::collections::HashMap::new();
    let mut container_names: std::collections::HashMap<(String, String), Option<String>> =
        std::collections::HashMap::new();

    for run in &runs {
        for sr in &run.source_results {
            let key = (sr.source_type.clone(), sr.container_id.clone());
            if !groups.contains_key(&key) {
                order.push(key.clone());
                container_names.insert(key.clone(), sr.container_name.clone());
            }
            match groups.entry(key) {
                Entry::Occupied(mut e) => e.get_mut().push(sr.items.clone()),
                Entry::Vacant(e) => {
                    e.insert(vec![sr.items.clone()]);
                },
            }
        }
    }

    let mut source_results = Vec::new();
    let mut context_parts = Vec::new();
    for key in order {
        let items_runs = groups.remove(&key).unwrap_or_default();
        let items = fuse_retrieved_items_by_rrf(items_runs, rrf_k);
        if items.is_empty() {
            continue;
        }
        let container_name = container_names.remove(&key).unwrap_or(None);
        let label = if key.0 == RAGSourceType::Memory.as_str() {
            "Memory Reference"
        } else if key.0 == RAGSourceType::Wiki.as_str() {
            "Wiki Reference"
        } else {
            "Knowledge Base Reference"
        };
        let snippets: Vec<String> = items.iter().map(|it| it.content.clone()).collect();
        context_parts.push(format!("[{}]\n{}", label, snippets.join("\n---\n")));
        source_results.push(RagSourceResult {
            source_type: key.0,
            container_id: key.1,
            items,
            container_name,
        });
    }

    RagContextResult { context_parts, source_results }
}

/// 带管线增强的上下文收集（新入口）
///
/// 相比 collect_rag_context 增加了查询增强、重排序和质检阶段。
///
/// `api_key`：云端 reranker（cohere/jina/voyage）的实际 API Key，
/// 由 wiring 层（`indexing.rs::collect_rag_context`）从 `CredentialManager` 解析后注入。
/// 为 `None` 时云端 backend 自动降级到 `RuleReranker`（本地规则排序）。
#[allow(clippy::too_many_arguments)]
pub async fn collect_rag_context_with_pipeline(
    db: &DatabaseConnection,
    master_key: &[u8; 32],
    vector_store: &VectorStore,
    kb_ids: &[String],
    mem_ids: &[String],
    wiki_ids: &[String],
    query: &str,
    top_k: usize,
    embed_fn: impl AsyncEmbedFn,
    pipeline_config: &axagent_harness::types::RAGPipelineConfig,
    llm_fn: Option<LlmCallFn>,
    api_key: Option<String>,
) -> RagContextResult {
    let sources = build_source_refs(kb_ids, mem_ids, wiki_ids);
    collect_rag_context_with_pipeline_from_refs(
        db,
        master_key,
        vector_store,
        sources,
        query,
        top_k,
        embed_fn,
        pipeline_config,
        llm_fn,
        api_key,
        kb_ids,
        wiki_ids,
    )
    .await
}

/// `collect_rag_context_with_pipeline` 的多文档协同变体。
#[allow(clippy::too_many_arguments)]
pub async fn collect_rag_context_with_pipeline_from_refs(
    db: &DatabaseConnection,
    master_key: &[u8; 32],
    vector_store: &VectorStore,
    sources: Vec<RAGSourceRef>,
    query: &str,
    top_k: usize,
    embed_fn: impl AsyncEmbedFn,
    pipeline_config: &axagent_harness::types::RAGPipelineConfig,
    llm_fn: Option<LlmCallFn>,
    api_key: Option<String>,
    kb_ids: &[String],
    wiki_ids: &[String],
) -> RagContextResult {
    // 阶段 0：查询增强
    let queries: Vec<String> = if pipeline_config.query_enhancement.enabled {
        if let Some(ref llm) = llm_fn {
            let llm_clone = std::sync::Arc::clone(llm);
            let enhancer = crate::query_enhancement::QueryEnhancer::new(
                pipeline_config.query_enhancement.clone(),
                move |s| llm_clone(s),
            );
            match enhancer.enhance(query).await {
                Ok(enhanced) => enhanced.into_iter().map(|eq| eq.text).collect(),
                Err(e) => {
                    tracing::warn!("Query enhancement failed: {}", e);
                    vec![query.to_string()]
                },
            }
        } else {
            vec![query.to_string()]
        }
    } else {
        vec![query.to_string()]
    };

    // 使用第一个增强查询
    let effective_query = queries.first().map(|s| s.as_str()).unwrap_or(query);

    // 如果没有启用 pipeline，直接走原有逻辑
    if !pipeline_config.rerank.enabled && !pipeline_config.self_rag.enabled {
        // 多路召回：查询增强产出多个查询时，逐查询检索后按 RRF 融合，
        // 而非只取第一个增强查询（此前多路召回实际未生效）。
        // 混合检索权重同样透传（2026-09-15 接线）。
        if queries.len() > 1 {
            let mut runs: Vec<RagContextResult> = Vec::new();
            for fq in queries.iter().take(MULTI_QUERY_LIMIT) {
                runs.push(
                    collect_rag_context_with_filters(
                        db,
                        master_key,
                        vector_store,
                        sources.clone(),
                        fq,
                        top_k,
                        embed_fn.clone(),
                        kb_ids,
                        wiki_ids,
                        Some(&pipeline_config.hybrid),
                    )
                    .await,
                );
            }
            let fused = fuse_rag_context_results(runs, MULTI_QUERY_RRF_K);

            // `fuse_rag_context_results` 是按融合后的 `source_results` **重算** `context_parts`
            // 的，因此逐 run 里由 `rebuild_context_with_citations` 追加到末尾的**回链上下文**
            // 不会跟着过来 —— 包括 Path A 的 `kg_context` 与 Path B 的图检索文本。
            // 在此按融合后的结果补回一次（补一次比各 run 各带一份更干净，也天然去重）。
            let mut extra =
                collect_cross_source_graph_context(db, kb_ids, wiki_ids, effective_query, top_k)
                    .await;
            // Path B 按 `effective_query` 显式重查（与上面 Path A 同口径），
            // 不再经由已删除的 `RagContextResult.graph_context` 中转字段。
            let graph_results =
                search_entity_graph_for_sources(&fused.source_results, effective_query).await;
            extra.extend(fold_entity_graph_context(&graph_results, &fused.source_results));

            return RagContextResult {
                context_parts: rebuild_context_with_citations(&fused.source_results, extra),
                source_results: fused.source_results,
            };
        }

        return collect_rag_context_with_filters(
            db,
            master_key,
            vector_store,
            sources,
            effective_query,
            top_k,
            embed_fn,
            kb_ids,
            wiki_ids,
            Some(&pipeline_config.hybrid),
        )
        .await;
    }

    let engine: Arc<dyn InferenceEngine> = crate::inference::global_engine();

    if sources.is_empty() {
        return RagContextResult { context_parts: vec![], source_results: vec![] };
    }

    // P2-2 修正：Memory 源语义匹配所需的 query embedding 改为按源真实 provider
    // 懒计算（此前写死 "default" provider 预计算，与容器实际 provider 可能不一致），
    // 且当首个查询就是原始 query 时复用给检索阶段，消除同一次流程内的双算。
    let mut memory_query_embedding: Option<Vec<f32>> = None;

    let mut context_parts = Vec::new();
    let mut source_results = Vec::new();
    // Graph RAG 阶段 4 的结果累积（2026-09-15 接线，注释同 legacy 路径处的说明）。
    let mut graph_results: Vec<axagent_harness::GraphEnhancedSearchResult> = Vec::new();

    for src_ref in &sources {
        let source = src_ref.source();
        // ⚠ 2026-09-15 修（类型混淆）：原写法是
        //     `let (source_top_k, _threshold, dims) = { let (sk, _, d) = ...; (.., sk, d) };`
        // 即**第一个元素丢弃真实阈值 `th`**，又把 `sk`（一个 `top_k` 计数，`usize`）
        // 填进阈值槽位。因为槽位被 `_threshold` 绑定且从未使用，其类型无人约束
        // ⇒ 用 `usize` 也能编译通过，于是「阈值被丢弃」这件事没有任何编译期信号。
        // 现在取真实的 `th` 并交给检索层（`min_similarity`）在**截断之前**过滤，
        // 与 legacy 路径口径一致。
        let (source_top_k, threshold, dims) = {
            let (sk, th, d) =
                resolve_source_config(db, &src_ref.source_type, &src_ref.container_id).await;
            (if sk > 0 { sk } else { top_k }, th, d)
        };

        // 多文档协同：当 doc_ids 非空时透传给底层 search
        let doc_ids_opt = if src_ref.doc_ids.is_empty() {
            None
        } else {
            Some(src_ref.doc_ids.as_slice())
        };

        // Memory 源：按容器真实 provider 解析并计算原始 query 的 embedding，
        // 供 tier 加权的语义匹配回退使用；enhancement 关闭时（首查询即原始
        // query）同时复用给检索阶段，避免同文本同 provider 重复 embed。
        let mut source_precomputed: Option<Vec<f32>> = None;
        if matches!(src_ref.source_type, RAGSourceType::Memory) {
            if memory_query_embedding.is_none() {
                memory_query_embedding = match source
                    .resolve_embedding_provider(db, &src_ref.container_id)
                    .await
                {
                    Ok(provider) => {
                        match embed_fn
                            .generate(db, master_key, &provider, vec![query.to_string()], dims)
                            .await
                        {
                            Ok(resp) => resp.embeddings.into_iter().next(),
                            Err(e) => {
                                tracing::warn!(
                                    "[RAG] Memory query embedding 计算失败（语义匹配回退停用）: {}",
                                    e
                                );
                                None
                            },
                        }
                    },
                    Err(e) => {
                        tracing::warn!(
                            "[RAG] Memory embedding provider 解析失败（语义匹配回退停用）: {}",
                            e
                        );
                        None
                    },
                };
            }
            // 仅当首个查询就是原始 query 时，检索阶段才能安全复用该 embedding
            if queries.first().is_some_and(|fq| fq == query) {
                source_precomputed = memory_query_embedding.clone();
            }
        }

        // 多路召回：对每个增强查询分别走管线，按 RRF 融合；
        // self-rag 质检仅在首个查询执行，避免成本随查询数线性放大。
        let mut item_runs: Vec<Vec<RagRetrievedItem>> = Vec::new();
        let mut first_quality: Option<RetrievalQuality> = None;
        for (qi, fq) in queries.iter().take(MULTI_QUERY_LIMIT).enumerate() {
            let cfg = if qi == 0 {
                pipeline_config.clone()
            } else {
                let mut c = pipeline_config.clone();
                c.self_rag.enabled = false;
                c
            };
            // 第 4 参：实体图谱提供者。此前恒传 `None` ⇒ `rag_pipeline.rs` 的
            // 阶段 4（图增强检索）永不进入（2026-09-15 接线）。改取 wiring 层
            // 注入的实现；未注入时为 `None`，行为与接线前完全一致。
            //
            // 两处收窄（2026-09-15 复核补）：
            // ① 只对 Knowledge 源传 —— 阶段 4 以 `container_id` 当 `knowledge_base_id`
            //    去查 `knowledge_entities.kb_id`，而 Memory/Wiki 源的 container_id 与之
            //    **不同域**，传进去必然是空结果 + 一条 warn（属噪声，不是信号）。
            // ② 只在首个查询传 —— 多查询增强最多跑 `MULTI_QUERY_LIMIT` 轮，图检索结果
            //    不随增强查询数量线性增值，跑一遍即可（与 self-rag 质检同一策略，见上方注释）。
            let graph_provider =
                if qi == 0 && matches!(src_ref.source_type, RAGSourceType::Knowledge) {
                    crate::entity_graph::entity_graph_provider()
                } else {
                    None
                };
            let pipeline = crate::rag_pipeline::RAGPipeline::new(
                &cfg,
                Some(engine.clone()),
                api_key.clone(),
                graph_provider,
            );
            let result = pipeline
                .execute_with_filter(
                    source.as_ref(),
                    db,
                    master_key,
                    vector_store,
                    &src_ref.container_id,
                    fq,
                    source_top_k,
                    dims,
                    embed_fn.clone(),
                    &cfg.rerank,
                    doc_ids_opt,
                    source_precomputed.clone(),
                    // 阈值在检索层（截断之前）生效，见 `search_with_filter` 的说明。
                    Some(similarity_floor_from_threshold(threshold)),
                )
                .await;

            match result {
                Ok(output) if !output.results.is_empty() => {
                    if first_quality.is_none() {
                        first_quality = Some(output.quality);
                    }
                    // Graph RAG 阶段 4 的产出在此捕获。此前这里完全不读该字段，
                    // 返回处又把累积结果恒丢弃 ⇒ 阶段 4 的实体/关系/邻居
                    // 全部被丢弃（「跑通了但没有出口」）。
                    if let Some(g) = output.graph_context {
                        graph_results.push(g);
                    }
                    // ⚠ 2026-09-15 修（同一字段两个反向标尺）：
                    // `RerankedChunk.score` 是**相关度**（`rerank_score`，越大越相关
                    // —— 它源自 `hybrid_search::combined_score`），而 legacy 路径往
                    // `RagRetrievedItem.score` 写的是**距离**（越小越相关）。
                    // 两条路径共用同一字段却方向相反，后果是 pipeline 路径（**默认**
                    // 路径：`RerankConfig::default().enabled == true`）下
                    // `apply_memory_tier_weight` 的 `-= tier_bonus` 与升序排序
                    // 全部**反向生效** —— 层级/重要度越高的记忆反而被排到越后面。
                    // 统一为「距离、越小越相关」后，阈值过滤也只保留一套写法。
                    //
                    // 2026-09-15 二次修（过滤位置）：原先这里还有一道
                    // `1.0 - r.score <= max_distance_ceiling` 过滤，现已**上移到检索层**
                    // （`min_similarity` ⇒ `min_score`，在截断前生效）。
                    // 上移的理由不只是「先过滤后截断」：此处的 `r.score` 是**重排阶段**
                    // 的分数，而阈值是按**融合分数**标定的 —— 两者虽同为 [0,1] 却是
                    // 两个语义，在重排后按它筛等于用一把尺子读另一把尺子的刻度。
                    let kept: Vec<RagRetrievedItem> = output
                        .results
                        .iter()
                        .map(|r| RagRetrievedItem {
                            content: r.content.clone(),
                            score: 1.0 - r.score,
                            document_id: r.document_id.clone(),
                            id: r.id.clone(),
                            document_name: None,
                            chunk_index: Some(r.chunk_index),
                        })
                        .collect();
                    if kept.is_empty() {
                        // 走到这里只可能是**阶段 3（self-RAG 质检）**把所有片段判为
                        // 不相关 —— 阈值过滤已在检索层完成，不再与本日志混为一谈
                        // （此前两者都表现为「静默无上下文」，无法区分）。
                        tracing::debug!(
                            "[RAG] 源 {} 的查询 {:?} 在阶段 3 质检后无保留片段",
                            src_ref.container_id,
                            fq
                        );
                    } else {
                        item_runs.push(kept);
                    }
                },
                Ok(_) => {},
                Err(e) => {
                    tracing::warn!(
                        "Pipeline failed for {} {} (query #{:?}): {}",
                        source.collection_prefix(),
                        src_ref.container_id,
                        fq,
                        e
                    );
                },
            }
        }

        if item_runs.is_empty() {
            tracing::warn!(
                "Pipeline returned no results for {} {}",
                source.collection_prefix(),
                src_ref.container_id
            );
            continue;
        }

        // 2026-09-15：这处 `match` 是「源类型 → 前端字符串」映射的**第三份**手写副本
        // （legacy 路径与 `as_str()` 各一份）—— 收敛到 `as_str()`，理由见其文档。
        let source_type_str = src_ref.source_type.as_str();

        let mut items: Vec<RagRetrievedItem> =
            fuse_retrieved_items_by_rrf(item_runs, MULTI_QUERY_RRF_K);

        // 三层记忆系统：针对 Memory 知识源，按 tier / importance 加权重排序
        // v108: 同时按 applicability_tags 过滤适用范围
        // P2-2: 传入按源真实 provider 计算的 query_embedding 用于语义相似度匹配
        if matches!(src_ref.source_type, RAGSourceType::Memory) {
            apply_memory_tier_weight(db, &mut items, query, memory_query_embedding.as_deref())
                .await;

            // P0-2: 反馈闭环 — 检索命中后自动提升 importance 和访问统计
            let hit_ids: Vec<String> = items.iter().map(|it| it.id.clone()).collect();
            apply_memory_hit_feedback(db, &hit_ids).await;
        }

        // v110: 跨源反馈权重调整 — 根据 retrieval_hits 中的历史反馈数据调整排序
        // 正反馈文档加分，负反馈文档减分，形成"用户反馈→检索质量提升"闭环
        apply_feedback_weight_adjustment(db, &mut items, source_type_str).await;

        let label = source.context_label();
        // snippets 顺序跟随 items（tier 加权后的顺序），保证 context 与引用追溯一致
        let snippets: Vec<String> = items.iter().map(|it| it.content.clone()).collect();
        context_parts.push(format!("[{}]\n{}", label, snippets.join("\n---\n")));

        if let Some(RetrievalQuality::Poor(ref diag)) = first_quality {
            tracing::warn!(
                "Poor RAG quality for {} {}: {}",
                source_type_str,
                src_ref.container_id,
                diag
            );
        }

        source_results.push(RagSourceResult {
            source_type: source_type_str.to_string(),
            container_id: src_ref.container_id.clone(),
            items,
            container_name: None,
        });
    }

    // 引用追溯：填充 container_name（KB / memory namespace / wiki 名称）
    fill_container_names(db, &mut source_results).await;

    // 引用可读性：回填检索命中项的 document_name（knowledge 文档标题 + wiki 笔记标题）
    fill_document_names(&mut source_results).await;

    let kg_context =
        collect_cross_source_graph_context(db, kb_ids, wiki_ids, effective_query, top_k).await;

    // Graph RAG 阶段 4 的**出口**（2026-09-15 接线）：图检索结果与 `kg_context`
    // 同属「回链上下文」，一起追加到重建后的 `context_parts` 末尾，不参与 [cite:N] 编号。
    // 必须在此处取 `source_results`（下一步它就被 move 进 `deduplicate_cross_source`）。
    let mut extra_context = kg_context;
    extra_context.extend(fold_entity_graph_context(&graph_results, &source_results));

    let (deduped_results, _deduped_context) =
        deduplicate_cross_source(source_results, context_parts);

    // 引用追溯：在 dedup 之后重建 context_parts，为每个 item 的 snippet 前注入 [cite:N] token。
    let final_context = rebuild_context_with_citations(&deduped_results, extra_context);

    RagContextResult { context_parts: final_context, source_results: deduped_results }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_surrounding_lines() {
        let source = "line1\nline2\nline3\nMATCH\nline5\nline6\nline7";
        let result = extract_surrounding_lines(source, "MATCH", 2);
        assert!(result.is_some());
        let result = result.expect("测试应成功");
        assert!(result.contains("line2"));
        assert!(result.contains("line6"));
        assert!(!result.contains("line1"));
        assert!(!result.contains("line7"));
    }

    #[test]
    fn test_extract_surrounding_lines_not_found() {
        let result = extract_surrounding_lines("abc\ndef", "xyz", 3);
        assert!(result.is_none());
    }

    #[test]
    fn test_inject_function_only_finds_fn() {
        let source =
            "// comment\nfn main() {\n    let x = 1;\n    println!(\"{x}\");\n}\nfn other() {}";
        let snippet = "println!(\"{x}\");";
        let result = inject_function_only(source, snippet, 500);
        assert!(result.contains("fn main()"));
        assert!(!result.contains("fn other()"));
    }

    #[test]
    fn test_inject_function_only_fallback() {
        let source = "let x = 1;\nlet y = 2;\nMATCH_HERE\nlet z = 3;";
        let result = inject_function_only(source, "MATCH_HERE", 500);
        assert!(result.contains("MATCH_HERE"));
        // Should include surrounding context even without a function boundary
        assert!(result.contains("let x = 1"));
        assert!(result.contains("let z = 3"));
    }

    #[test]
    fn test_default_l2_threshold_is_reasonable() {
        // ⚠ 2026-09-15 重写：原测试**自己声明** `let default_max_distance = 20.0;`
        // 再断言它 ≥10 —— 即断言自己的字面量，与生产常量毫无关系（改生产代码它不红）。
        // 现改为直接钉住生产常量本体。
        //
        // ⚠ 必须写成 `const { assert!(..) }`（**编译期**断言），不能用裸 `assert!`：
        // 断言对象是常量字面量的表达式 ⇒ 触发 `clippy::assertions_on_constants`，
        // 而本仓 clippy 是 `-D warnings` ⇒ 直接红灯（实测踩到）。
        // 编译期形态反而**更强**：常量一旦越界，构建就失败，不必等到跑测试。
        const {
            assert!(
                DEFAULT_MAX_L2_DISTANCE >= 10.0,
                "L2 饱和点过小 ⇒ 归一后绝大多数结果落到 1.0 一档，等价于把结果全滤掉"
            )
        };
        const {
            assert!(
                DEFAULT_MAX_L2_DISTANCE <= 100.0,
                "L2 饱和点过大 ⇒ 归一后所有结果挤在 1.0 附近，阈值失去区分力"
            )
        };
    }

    /// **阈值换算的方向**（本轮 P0 的回归锁）。
    ///
    /// 用户配置的 `retrieval_threshold` 是「相关度下限」，而 `r.score` 是
    /// 「距离、越小越相关」，故换算必须**取反**。若谁把它改回「当成距离上限」，
    /// 前端默认值 `0.1` 会变成「只留 L2 ≤ 0.1」，该知识库对 RAG 一条都不贡献。
    #[test]
    fn test_similarity_floor_converts_to_distance_ceiling() {
        // 前端默认值 0.1 ⇒ 距离上限 0.9（宽松：丢掉最不相关的那一档）
        assert!((distance_ceiling_from_similarity_floor(0.1) - 0.9).abs() < 1e-6);
        // 未设 / 非有限值 / 负值 ⇒ 不过滤
        for unset in [0.0_f32, -1.0, f32::NAN, f32::INFINITY] {
            assert!(
                (distance_ceiling_from_similarity_floor(unset) - 1.0).abs() < 1e-6,
                "未设阈值（{unset}）必须等价于不过滤"
            );
        }
        // 上限 1.0 ⇒ 只保留完全匹配
        assert!(distance_ceiling_from_similarity_floor(1.0).abs() < 1e-6);
        // 超出 [0,1] 的配置被夹住，不会产生越界上限
        assert!((distance_ceiling_from_similarity_floor(5.0)).abs() < 1e-6);

        // 同一解释的两个投影必须严格互补（改一个忘另一个 = 反向筛选）
        for v in [0.0_f32, 0.1, 0.5, 1.0, 5.0, -1.0, f32::NAN] {
            let floor = similarity_floor_from_threshold(v);
            let ceiling = distance_ceiling_from_similarity_floor(v);
            assert!(
                (floor + ceiling - 1.0).abs() < 1e-6,
                "下限 {floor} 与上限 {ceiling} 不互补（输入 {v}）"
            );
            assert!((0.0..=1.0).contains(&floor), "相关度下限必须落在 [0,1]，实得 {floor}");
        }
    }

    /// **过滤不再是空过滤**：这是 §10.7#4 的直接回归。
    ///
    /// 归一后的距离集（由 RRF 结果集换算而来，见 `hybrid_search` 的
    /// `DEFAULT_MAX_L2_DISTANCE` 说明）在阈值下限 0.1 下必须**仍有结果**，
    /// 且必须**真的丢掉**最不相关的一档。归一前的旧值域 ≈0.967–0.983 会让
    /// 该断言两侧同时失败（前者恒假、后者恒真）。
    #[test]
    fn test_threshold_filter_is_neither_hollow_nor_total() {
        // 归一后的距离：0.0（最佳）… 1.0（完全无关）
        let distances = [0.0_f32, 0.25, 0.5, 0.75, 0.9, 1.0];
        let ceiling = distance_ceiling_from_similarity_floor(0.1);
        let kept: Vec<f32> = distances.iter().copied().filter(|d| *d <= ceiling).collect();

        assert!(!kept.is_empty(), "阈值 0.1 不得把结果集清空（空过滤/全滤都是故障）");
        assert!(
            kept.len() < distances.len(),
            "阈值 0.1 必须真的丢掉最不相关的一档，否则过滤形同虚设"
        );
        assert_eq!(kept, vec![0.0, 0.25, 0.5, 0.75, 0.9]);
    }

    /// 旧口径的反向证据：把归一前的 RRF 取反值域喂给旧表达式 `score <= 20.0`
    /// 恒真、喂给 `score <= 0.1` 恒假 —— 两个方向都坏。
    #[test]
    fn test_pre_fix_rrf_scale_was_unusable_by_any_threshold() {
        let pre_fix_scores = [0.967_f32, 0.975, 0.983];
        assert!(pre_fix_scores.iter().all(|s| *s <= 20.0), "旧值域下 `<= 20.0` 恒真 ⇒ 过滤是空的");
        assert!(
            pre_fix_scores.iter().all(|s| *s > 0.1),
            "旧值域下前端默认值 0.1 恒假 ⇒ 结果集恒空"
        );
    }

    #[test]
    fn test_prepare_chunks_from_text() {
        let strategy = ChunkStrategy::FromText {
            text: "第一章\n这是第一段内容。\n\n第二章\n这是第二段内容。".to_string(),
            chunk_size: 50,
            overlap: 10,
            separator: None,
        };
        let chunks = prepare_chunks("doc-1", &strategy).expect("测试应成功");
        assert!(!chunks.is_empty());
        for (id, _content, index) in &chunks {
            assert!(id.starts_with("doc-1_"));
            assert!(*index >= 0);
        }
    }

    #[test]
    fn test_prepare_chunks_empty_text() {
        let strategy = ChunkStrategy::FromText {
            text: "   ".to_string(),
            chunk_size: 100,
            overlap: 20,
            separator: None,
        };
        let chunks = prepare_chunks("doc-1", &strategy).expect("测试应成功");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_direct_chunk_strategy_returns_empty() {
        let strategy = ChunkStrategy::Direct;
        let chunks = prepare_chunks("item-1", &strategy).expect("测试应成功");
        assert!(chunks.is_empty());
    }

    // ── v108: filter_items_by_applicability_tags 单元测试 ──────────

    /// 构造测试用 RagRetrievedItem
    fn make_item(id: &str, score: f32) -> RagRetrievedItem {
        RagRetrievedItem {
            content: String::new(),
            score,
            document_id: String::new(),
            id: id.to_string(),
            document_name: None,
            chunk_index: None,
        }
    }

    #[test]
    fn test_filter_empty_items_no_op() {
        let mut items: Vec<RagRetrievedItem> = Vec::new();
        let map = std::collections::HashMap::new();
        filter_items_by_applicability_tags(&mut items, &map, "rust", None);
        assert!(items.is_empty());
    }

    #[test]
    fn test_filter_empty_weight_map_keeps_all() {
        let mut items = vec![make_item("a", 1.0), make_item("b", 2.0)];
        let map = std::collections::HashMap::new();
        filter_items_by_applicability_tags(&mut items, &map, "rust", None);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn test_filter_id_not_in_map_keeps() {
        let mut items = vec![make_item("a", 1.0), make_item("b", 2.0)];
        let mut map = std::collections::HashMap::new();
        // 仅注册 a，b 不在 map → b 保留
        map.insert("a".to_string(), (1.0, 0.5, vec!["rust".to_string()]));
        filter_items_by_applicability_tags(&mut items, &map, "rust", None);
        // a 命中 rust tag，b 视为全局适用，均保留
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn test_filter_empty_tags_keeps() {
        let mut items = vec![make_item("a", 1.0)];
        let mut map = std::collections::HashMap::new();
        map.insert("a".to_string(), (1.0, 0.5, Vec::new()));
        filter_items_by_applicability_tags(&mut items, &map, "anything", None);
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_filter_tag_matched_keeps() {
        let mut items = vec![make_item("a", 1.0)];
        let mut map = std::collections::HashMap::new();
        map.insert("a".to_string(), (1.0, 0.5, vec!["rust".to_string()]));
        filter_items_by_applicability_tags(&mut items, &map, "rust programming", None);
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_filter_tag_not_matched_removed() {
        let mut items = vec![make_item("a", 1.0)];
        let mut map = std::collections::HashMap::new();
        map.insert("a".to_string(), (1.0, 0.5, vec!["rust".to_string()]));
        filter_items_by_applicability_tags(&mut items, &map, "python programming", None);
        assert!(items.is_empty());
    }

    #[test]
    fn test_filter_case_insensitive() {
        let mut items = vec![make_item("a", 1.0)];
        let mut map = std::collections::HashMap::new();
        // tag 大写
        map.insert("a".to_string(), (1.0, 0.5, vec!["RUST".to_string()]));
        // query 小写 → 子串匹配应不区分大小写
        filter_items_by_applicability_tags(&mut items, &map, "rust is great", None);
        assert_eq!(items.len(), 1);

        // 反向：tag 小写，query 大写
        let mut items2 = vec![make_item("a", 1.0)];
        let mut map2 = std::collections::HashMap::new();
        map2.insert("a".to_string(), (1.0, 0.5, vec!["rust".to_string()]));
        filter_items_by_applicability_tags(&mut items2, &map2, "RUST IS GREAT", None);
        assert_eq!(items2.len(), 1);
    }

    #[test]
    fn test_filter_multiple_tags_any_match() {
        let mut items = vec![make_item("a", 1.0)];
        let mut map = std::collections::HashMap::new();
        // 多 tag，query 命中第二个
        map.insert("a".to_string(), (1.0, 0.5, vec!["python".to_string(), "rust".to_string()]));
        filter_items_by_applicability_tags(&mut items, &map, "rust coding", None);
        assert_eq!(items.len(), 1);

        // 多 tag，query 未命中任何一个
        let mut items2 = vec![make_item("a", 1.0)];
        filter_items_by_applicability_tags(&mut items2, &map, "golang coding", None);
        assert!(items2.is_empty());
    }

    #[test]
    fn test_filter_mixed_items_partial_removal() {
        let mut items = vec![
            make_item("a", 1.0), // tags=["rust"], query 命中 → 保留
            make_item("b", 2.0), // tags=["python"], query 未命中 → 移除
            make_item("c", 3.0), // tags=[] → 全局适用 → 保留
            make_item("d", 4.0), // 不在 map → 保留
        ];
        let mut map = std::collections::HashMap::new();
        map.insert("a".to_string(), (1.0, 0.5, vec!["rust".to_string()]));
        map.insert("b".to_string(), (1.0, 0.5, vec!["python".to_string()]));
        map.insert("c".to_string(), (1.0, 0.5, Vec::new()));
        filter_items_by_applicability_tags(&mut items, &map, "rust programming", None);
        assert_eq!(items.len(), 3);
        // b 应被移除
        assert!(items.iter().all(|it| it.id != "b"));
    }

    #[test]
    fn test_filter_empty_query_with_nonempty_tags_removes() {
        let mut items = vec![make_item("a", 1.0)];
        let mut map = std::collections::HashMap::new();
        map.insert("a".to_string(), (1.0, 0.5, vec!["rust".to_string()]));
        // query 为空 → 任何 tag 都无法命中 → 移除
        filter_items_by_applicability_tags(&mut items, &map, "", None);
        assert!(items.is_empty());
    }

    // ── v108: apply_tier_weight_and_sort 单元测试 ──────────

    #[test]
    fn test_weight_sort_empty_items_no_op() {
        let mut items: Vec<RagRetrievedItem> = Vec::new();
        let map = std::collections::HashMap::new();
        apply_tier_weight_and_sort(&mut items, &map);
        assert!(items.is_empty());
    }

    #[test]
    fn test_weight_sort_id_not_in_map_score_unchanged() {
        let mut items = vec![make_item("a", 5.0)];
        let map = std::collections::HashMap::new();
        apply_tier_weight_and_sort(&mut items, &map);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].score, 5.0);
    }

    #[test]
    fn test_weight_sort_single_item_adjusted() {
        let mut items = vec![make_item("a", 10.0)];
        let mut map = std::collections::HashMap::new();
        // tier_bonus=2.0, importance=0.5 → adjusted = 10 - 2.0*0.5 = 9.0
        map.insert("a".to_string(), (2.0, 0.5, Vec::new()));
        apply_tier_weight_and_sort(&mut items, &map);
        assert_eq!(items.len(), 1);
        assert!((items[0].score - 9.0).abs() < 1e-6);
    }

    #[test]
    fn test_weight_sort_ascending_order() {
        // 原始 score：a=10, b=5, c=8
        // 加权后：a=10-2.0*0.9=8.2, b=5-0.5*0.5=4.75, c=8-1.5*0.7=6.95
        // 升序：b(4.75) < c(6.95) < a(8.2)
        let mut items = vec![make_item("a", 10.0), make_item("b", 5.0), make_item("c", 8.0)];
        let mut map = std::collections::HashMap::new();
        map.insert("a".to_string(), (2.0, 0.9, Vec::new())); // core
        map.insert("b".to_string(), (0.5, 0.5, Vec::new())); // short_term
        map.insert("c".to_string(), (1.5, 0.7, Vec::new())); // long_term
        apply_tier_weight_and_sort(&mut items, &map);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].id, "b");
        assert_eq!(items[1].id, "c");
        assert_eq!(items[2].id, "a");
        // 验证 adjusted score
        assert!((items[0].score - 4.75).abs() < 1e-6);
        assert!((items[1].score - 6.95).abs() < 1e-6);
        assert!((items[2].score - 8.2).abs() < 1e-6);
    }

    #[test]
    fn test_weight_sort_core_ranks_before_short_term() {
        // 即使原始 L2 distance 相同，core 的 bonus 更大（减得更多），应排前
        let mut items = vec![make_item("short", 5.0), make_item("core", 5.0)];
        let mut map = std::collections::HashMap::new();
        map.insert("core".to_string(), (2.0, 0.9, Vec::new()));
        map.insert("short".to_string(), (0.5, 0.9, Vec::new()));
        apply_tier_weight_and_sort(&mut items, &map);
        // core adjusted = 5 - 2.0*0.9 = 3.2
        // short adjusted = 5 - 0.5*0.9 = 4.55
        // 升序：core(3.2) < short(4.55)
        assert_eq!(items[0].id, "core");
        assert_eq!(items[1].id, "short");
    }

    #[test]
    fn test_weight_sort_nan_score_fallback() {
        // 包含 NaN score 的 item 应不 panic（fallback 到 Equal）
        let mut items = vec![make_item("a", f32::NAN), make_item("b", 1.0)];
        let mut map = std::collections::HashMap::new();
        map.insert("a".to_string(), (1.0, 0.5, Vec::new()));
        map.insert("b".to_string(), (1.0, 0.5, Vec::new()));
        // 不应 panic
        apply_tier_weight_and_sort(&mut items, &map);
        assert_eq!(items.len(), 2);
    }

    /// Graph RAG 阶段 4 的出口转换：空白 `context_text` 必须被过滤，有内容时原样透出。
    ///
    /// 这是「阶段 4 产出 → 注入 prompt 的文本」的唯一转换点（2026-09-15 接线）。
    /// 之所以要单测：该函数的**调用方**只有在真机上开着图谱开关才走得到，
    /// 而一旦它把空白文本也放行，就会往 prompt 里注入空段落 —— 靠人工跑很难发现。
    #[test]
    fn fold_entity_graph_context_filters_blank_text() {
        let mk = |text: &str, hits: usize| axagent_harness::GraphEnhancedSearchResult {
            entities: vec![],
            context_text: text.to_string(),
            total_hits: hits,
        };

        // 无图检索结果 ⇒ 不得注入任何内容（开关关闭时的路径）
        assert!(fold_entity_graph_context(&[], &[]).is_empty(), "无图检索结果时不得注入任何内容");

        // 纯空白 ⇒ 过滤；有内容 ⇒ 透出
        let graph =
            vec![mk("  \n\t ", 0), mk("[Knowledge Graph - kb1]\n- 宁德时代 (COMPANY)\n", 2)];
        let out = fold_entity_graph_context(&graph, &[]);
        assert_eq!(out.len(), 1, "空白 context_text 必须被过滤");
        assert!(out[0].contains("宁德时代"), "有内容的 context_text 必须原样透出");
    }
}
