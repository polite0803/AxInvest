// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, Value};
use serde::{Deserialize, Serialize};

use crate::vector_store::{VectorSearchResult, VectorStore};
use axagent_harness::core_error::{AxAgentError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridSearchResult {
    pub id: String,
    pub document_id: String,
    pub chunk_index: i32,
    pub content: String,
    pub vector_score: Option<f32>,
    pub bm25_score: Option<f32>,
    /// 多引擎 RAG：sparse neural 检索分数（SPLADE/BGE-M3 sparse 等）。
    /// 当前实现暂未接入 sparse encoder，该字段始终为 None，留作扩展位。
    #[serde(default)]
    pub sparse_score: Option<f32>,
    /// 融合后的**相关度 ∈ [0,1]，越大越相关**（唯一标尺，见下方模块级说明）。
    ///
    /// 此前四条产出路径各自返回**互不可比**的量纲：向量-only 是 `1 - L2`
    /// （可为负、可达 40+）、加权是 [0,1]、RRF 是 `Σ w/(k+rank+1)` ≈ 0.017–0.033、
    /// 纯 FTS 是原始 BM25。于是「用同一个阈值过滤」这件事在数学上不成立
    /// （详见 `DEFAULT_MAX_L2_DISTANCE` 的说明）。
    pub combined_score: f32,
}

/// L2 距标尺的**饱和点**：`L2 == 20.0` 视为「完全无关」，对应相关度 0。
///
/// # 这个常量同时是四条产出路径的统一标尺
///
/// `HybridSearchResult.combined_score` / `VectorSearchResult.score` 在全仓被当作
/// **同一个数**使用（`min_score` 过滤、rerank 候选、`retrieval_threshold` 过滤、
/// 记忆 tier 加权、前端排序）。但四条产出路径原本返回三个不同量纲：
///
/// | 路径 | 原 `combined_score` | 量纲 |
/// |---|---|---|
/// | 向量-only（`finish_vector_only`）| `1 - L2` | L2 派生，可为负、无上界语义 |
/// | 加权（`merge_results_weighted`）| `nv*vw + nb*bw` | 相对 [0,1]，`nv` 以**本次结果集最大距离**归一 |
/// | RRF（默认，`merge_results_rrf`）| `Σ w/(k+rank+1)` | 纯排名派生的**相对**分，≈0.017–0.033 |
/// | 纯 FTS（`fts_only_search_with_filter`）| 原始 BM25 | 无上界 |
///
/// 后果（2026-09-15 实测）：`retrieval_threshold` 的过滤表达式
/// `score <= 20.0` 在 RRF 路径下**恒真**（分数 ≈0.017–0.033，经
/// `search_with_filter` 取反后 ≈0.967–0.983）⇒ **过滤器是空的**；
/// 而前端设置面板把这个字段默认为 `0.1`，此时 RRF 路径
/// `0.967 <= 0.1` **恒假** ⇒ 该知识库对 RAG **一条上下文都不贡献**
/// （用户可见故障：保存过知识库设置后，Agent 再也检索不到该库内容）。
///
/// 修法：把四条路径的产出**全部归一到 `[0,1]` 相关度**，其中向量派生的一律以
/// `L2 / DEFAULT_MAX_L2_DISTANCE` 作为归一基准（绝对标尺，保留「彻底无关就别要」
/// 的原始意图），排名/BM25 派生的按**本次结果集最大值**归一（相对标尺 —— RRF
/// 本身不含绝对质量信号，这一点无法伪造）。归一**只做缩放、不改变排序**。
pub const DEFAULT_MAX_L2_DISTANCE: f32 = 20.0;

/// 把向量 L2 距离归一到 `[0,1]` 相关度：`L2 == 0` ⇒ 1.0，`L2 >= 20` ⇒ 0.0。
///
/// 独立成函数是因为四条路径里**只有**向量派生的那几条能用这个绝对标尺；
/// 排名/BM25 派生必须用结果集内的相对归一（见 `normalize_by_max`）。
/// `pub(crate)`：`rag_pipeline` 的阶段 1 转换也要用同一标尺（此前手写了一份 `/ 20.0`）。
pub(crate) fn l2_distance_to_similarity(l2_distance: f32) -> f32 {
    1.0 - (l2_distance / DEFAULT_MAX_L2_DISTANCE).clamp(0.0, 1.0)
}

/// 按**本次结果集最大值**把无上界分数归一到 `[0,1]`（最大值 ⇒ 1.0）。
/// 排序不变；`max <= 0`（空集 / 全零）时整体回退 0.0，不产生 NaN。
fn normalize_by_max(score: f32, max: f32) -> f32 {
    if max > 0.0 {
        (score / max).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// 用户配置的 `retrieval_threshold` ⇒ **相关度下限 ∈ [0,1]**（不做取反的原形）。
///
/// # 为什么统一按「下限」而不是「上限」解释
///
/// 该字段的前端默认值是 `0.1`（`KnowledgeBaseDocuments.tsx` / `MemorySettings.tsx`
/// 的 `?? 0.1`），UI 只给一个 0–1 数字框、标签写作「检索阈值」。读成**距离上限**
/// 会让 `0.1` 变成「只保留近乎重复的项目」⇒ 结果集恒空；读成**相关度下限**
/// 则 `0.1` 是「丢掉最不相关的那一档」，与 UI 语义和默认值都自洽。
///
/// 非有限值 / `<= 0` ⇒ `0.0`（不过滤）；`> 1` 夹到 `1.0`。
///
/// 两个投影共用本函数，按**消费端 `score` 的方向**选用：
/// - 消费端 `score` 是**相关度**（越大越相关）⇒ 直接用本函数，`score >= 下限`
///   （例：`NoteSearchResult.score`、`HybridSearchOptions.min_score`）。
/// - 消费端 `score` 是**距离**（越小越相关）⇒ 用
///   [`distance_ceiling_from_similarity_floor`]，`score <= 上限`
///   （例：`VectorSearchResult.score` 及其全部 RAG 过滤点）。
///
/// ⚠ 混用的后果是**反向筛选**（留下的恰好是最不相关的那批），不是报错。
pub fn similarity_floor_from_threshold(configured: f32) -> f32 {
    if configured.is_finite() && configured > 0.0 {
        configured.min(1.0)
    } else {
        0.0
    }
}

/// [`similarity_floor_from_threshold`] 的距离侧投影：`距离上限 = 1 - 相关度下限`。
///
/// 供 `score <= ceiling` / `combined_score >= 1.0 - ceiling` 使用。
/// 未设阈值 ⇒ `1.0`（= 不过滤，因为归一化已把「彻底无关」压到 1.0 这一档）。
pub fn distance_ceiling_from_similarity_floor(configured: f32) -> f32 {
    1.0 - similarity_floor_from_threshold(configured)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub enum FusionAlgorithm {
    /// Weighted linear combination of normalized scores.
    Weighted,
    /// Reciprocal Rank Fusion — robust to score scale differences, default k=60.
    #[default]
    Rrf,
}

#[derive(Debug, Clone)]
pub struct HybridSearchOptions {
    /// 是否启用混合检索（由 `HybridConfig.enabled` 驱动，2026-09-15 接线）。
    /// `false` ⇒ 只走向量召回，不查 BM25、不做融合。
    pub enabled: bool,
    pub vector_weight: f32,
    pub bm25_weight: f32,
    /// 多引擎 RAG：sparse neural 路径权重（默认 0，表示不启用 sparse 路径）。
    /// 当 sparse_weight > 0 且 sparse encoder 可用时，会走三路融合。
    pub sparse_weight: f32,
    pub top_k: usize,
    /// **相关度下限 ∈ [0,1]**（在 `combined_score` 的标尺上过滤，`>=` 判定）。
    /// `None` ⇒ 不过滤。四条产出路径（向量-only / 加权 / RRF / 纯 FTS）
    /// 均已归一到该标尺，故此处的 min_score 对任意路径同义
    /// —— 这正是 `distance_ceiling_from_similarity_floor` 存在的前提。
    pub min_score: Option<f32>,
    pub fusion: FusionAlgorithm,
    pub rrf_k: f32,
}

impl Default for HybridSearchOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            vector_weight: 0.7,
            bm25_weight: 0.3,
            sparse_weight: 0.0,
            top_k: 10,
            min_score: None,
            fusion: FusionAlgorithm::Rrf,
            rrf_k: 60.0,
        }
    }
}

/// 从用户可配的 `HybridConfig` 构造混合检索选项。
///
/// 这是 `HybridSearchOptions` 的**唯一装配入口**（2026-09-15 接线）：此前
/// `HybridConfig`（harness 层、设置面板可改）全仓零读取点，检索权重被硬编码成
/// 0.7/0.3 两处 ⇒ 用户改配置不生效且无任何告警。
///
/// `top_k` 由调用方给出（各调用点语义不同）；`min_score` 保持 `None`，
/// 需要收窄时由调用方另行设置。
pub fn hybrid_options_from_config(
    hybrid: &axagent_harness::rag_config::HybridConfig,
    top_k: usize,
) -> HybridSearchOptions {
    HybridSearchOptions {
        enabled: hybrid.enabled,
        vector_weight: hybrid.vector_weight,
        bm25_weight: hybrid.bm25_weight,
        sparse_weight: hybrid.sparse_weight,
        top_k,
        min_score: None,
        // 未知取值回退 RRF（默认算法）：配置里写错字符串不应静默变成 Weighted
        fusion: match hybrid.fusion.as_str() {
            "weighted" => FusionAlgorithm::Weighted,
            _ => FusionAlgorithm::Rrf,
        },
        rrf_k: hybrid.rrf_k,
    }
}

pub struct HybridSearcher {
    db: DatabaseConnection,
    vector_store: VectorStore,
}

impl HybridSearcher {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { vector_store: VectorStore::new(db.clone()), db }
    }

    pub fn vector_store(&self) -> &VectorStore {
        &self.vector_store
    }

    pub async fn ensure_fts5_index(&self, collection_id: &str) -> Result<()> {
        if self.db.get_database_backend() == DbBackend::Postgres {
            // PostgreSQL 关键词检索由 VectorStore 的 content_tsv 生成列 + GIN 索引承担。
            // 直接复用 VectorStore 同名方法创建/校验 GIN 索引（幂等）。
            return self.vector_store.ensure_fts5_index(collection_id).await;
        }

        let safe_name = sanitize_name_for_table(collection_id);
        let meta_table = format!("vec_{safe_name}_meta");
        let fts_table = format!("{meta_table}_fts");

        let table_exists: bool = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "SELECT name FROM sqlite_master WHERE type='table' AND name=?1",
                vec![fts_table.clone().into()],
            ))
            .await
            .map(|r| r.is_some())
            .unwrap_or(false);

        // FTS 表已存在时直接返回。内容变更后的索引维护由写入路径的
        // `rebuild_fts_index`（fire-and-forget）负责，查询路径不再每次全量
        // 'rebuild'——此前每次搜索都全量重建 FTS，在大知识库上是主要性能瓶颈。
        if table_exists {
            return Ok(());
        }

        let create_sql = format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS {fts_table} USING fts5(
                id UNINDEXED,
                document_id UNINDEXED,
                chunk_index UNINDEXED,
                content,
                content={meta_table},
                content_rowid=rowid,
                tokenize='trigram'
            )"
        );

        self.db.execute_unprepared(&create_sql).await.map_err(|e| {
            AxAgentError::Provider(format!("FTS5 trigram index creation failed: {}", e))
        })?;

        let populated: Option<i64> = self
            .db
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("SELECT COUNT(*) as cnt FROM {fts_table}"),
            ))
            .await
            .ok()
            .flatten()
            .and_then(|r| r.try_get::<i64>("", "cnt").ok());

        if populated.unwrap_or(0) == 0 {
            let populate_sql = format!(
                "INSERT INTO {fts_table}(rowid, id, document_id, chunk_index, content) \
                 SELECT rowid, id, document_id, chunk_index, content FROM {meta_table}"
            );
            if let Err(e) = self.db.execute_unprepared(&populate_sql).await {
                tracing::debug!("FTS5 initial population failed (non-critical): {}", e);
            }
        }

        Ok(())
    }

    pub async fn hybrid_search(
        &self,
        collection_id: &str,
        query: &str,
        query_embedding: Vec<f32>,
        options: HybridSearchOptions,
    ) -> Result<Vec<HybridSearchResult>> {
        self.hybrid_search_with_filter(collection_id, query, query_embedding, options, None).await
    }

    /// Hybrid search with optional `document_id` list filter
    /// (multi-document collaboration).
    ///
    /// When `doc_ids` is `Some` and non-empty, both the vector path
    /// (`vector_store::search_with_filter`) and the BM25 path apply the same
    /// `document_id IN (...)` predicate so the fused result set is scoped to
    /// the requested subset of documents.
    pub async fn hybrid_search_with_filter(
        &self,
        collection_id: &str,
        query: &str,
        query_embedding: Vec<f32>,
        options: HybridSearchOptions,
        doc_ids: Option<&[String]>,
    ) -> Result<Vec<HybridSearchResult>> {
        let vector_results = self
            .vector_store
            .search_with_filter(collection_id, query_embedding.clone(), options.top_k * 3, doc_ids)
            .await?;

        // enabled=false（HybridConfig 里关掉混合检索）⇒ 纯向量路径：
        // 不查 BM25、不做融合；收尾语义与混合路径一致，调用方无需区分结果来源。
        if !options.enabled {
            return Ok(Self::finish_vector_only(vector_results, options.top_k, options.min_score));
        }

        let bm25_results =
            self.bm25_search_with_filter(collection_id, query, options.top_k * 3, doc_ids).await?;

        let combined = match options.fusion {
            FusionAlgorithm::Weighted => self.merge_results_weighted(
                vector_results,
                bm25_results,
                options.vector_weight,
                options.bm25_weight,
            ),
            // 加权 RRF：权重此前在默认融合算法下**完全没被读取**（只传 rrf_k），
            // 于是 HybridConfig 的 vector/bm25 权重怎么改都不生效（2026-09-15 修）。
            // 参考 WeKnora `GetEffectiveRRFWeights()`：RRF 分数乘权重后累加。
            FusionAlgorithm::Rrf => Self::merge_results_rrf(
                vector_results,
                bm25_results,
                options.rrf_k,
                options.vector_weight,
                options.bm25_weight,
            ),
        };

        let mut filtered: Vec<HybridSearchResult> = combined
            .into_iter()
            .filter(|r| options.min_score.is_none_or(|min| r.combined_score >= min))
            .collect();

        // ⚠ 2026-09-15 修：此处原为「`.take(top_k)` 之后再 `sort_by`」。`combined`
        // 来自 `score_map.into_values()`（HashMap，**顺序不定**）⇒ 先截断等于
        // **随机留下 top_k 个**，真正的 top_k 会被丢掉。另两条收尾路径
        // （`finish_vector_only` / `fts_only_search_with_filter`）都是
        // 「先排序后截断」，只有这里反了 —— 三份收尾逻辑不一致。
        filtered.sort_by(|a, b| {
            b.combined_score.partial_cmp(&a.combined_score).unwrap_or(std::cmp::Ordering::Equal)
        });
        filtered.truncate(options.top_k);

        Ok(filtered)
    }

    /// `enabled=false`（纯向量）路径的收尾。
    ///
    /// 把向量命中转成统一的 `HybridSearchResult` 形状，排序 / `min_score` 过滤 /
    /// `top_k` 截断语义与 `hybrid_search_with_filter` 保持一致，
    /// 调用方无需区分结果来自哪条路径。
    fn finish_vector_only(
        vector_results: Vec<VectorSearchResult>,
        top_k: usize,
        min_score: Option<f32>,
    ) -> Vec<HybridSearchResult> {
        let mut results: Vec<HybridSearchResult> = vector_results
            .into_iter()
            .map(|vr| HybridSearchResult {
                id: vr.id,
                document_id: vr.document_id,
                chunk_index: vr.chunk_index,
                content: vr.content,
                // 归一到 [0,1] 相关度：此处 `vr.score` 是**原始 L2 距离**，
                // 用绝对标尺（非结果集内相对）—— 这条路没有排名信息可依，
                // 且「L2 ≥ 20 就是无关」的原始意图要保住。
                vector_score: Some(l2_distance_to_similarity(vr.score)),
                bm25_score: None,
                sparse_score: None,
                combined_score: l2_distance_to_similarity(vr.score),
            })
            .filter(|r| min_score.is_none_or(|min| r.combined_score >= min))
            .collect();

        results.sort_by(|a, b| {
            b.combined_score.partial_cmp(&a.combined_score).unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(top_k);
        results
    }

    /// 纯 FTS（BM25）检索：embedding 未配置时的降级路径（R9 遗留）。
    /// 不生成 query embedding、不做向量召回与融合，直接返回 BM25 命中，
    /// 收尾语义（min_score 过滤 / 降序 / top_k 截断）与 `hybrid_search_with_filter` 保持一致，
    /// 调用方无需区分结果来源。
    pub async fn fts_only_search_with_filter(
        &self,
        collection_id: &str,
        query: &str,
        options: HybridSearchOptions,
        doc_ids: Option<&[String]>,
    ) -> Result<Vec<HybridSearchResult>> {
        let bm25_results =
            self.bm25_search_with_filter(collection_id, query, options.top_k, doc_ids).await?;

        // BM25 无上界（且跨后端量纲不同：SQLite FTS5 vs PG tsvector）⇒ 只能按本次
        // 结果集最大值归一到 [0,1]（相对标尺）。归一不改排序。
        let max_bm25_score = bm25_results.iter().map(|r| r.bm25_score).fold(0f32, f32::max);

        let mut filtered: Vec<HybridSearchResult> = bm25_results
            .into_iter()
            .map(|br| HybridSearchResult {
                id: br.id,
                document_id: br.document_id,
                chunk_index: br.chunk_index,
                content: br.content,
                vector_score: None,
                bm25_score: Some(br.bm25_score),
                sparse_score: None,
                combined_score: normalize_by_max(br.bm25_score, max_bm25_score),
            })
            // ⚠ 2026-09-15 修：此处原为 `is_some_and`，即 `min_score == None` 时
            // 谓词恒假 ⇒ **无过滤条件时反而把所有结果全丢掉**，纯 FTS 降级路径
            // 恒返回空。与 `finish_vector_only`（`is_none_or`）及混合路径
            // （显式 `if let Some`）语义相反，属三份收尾逻辑不一致。
            .filter(|r| options.min_score.is_none_or(|min| r.combined_score >= min))
            .collect();

        filtered.sort_by(|a, b| {
            b.combined_score.partial_cmp(&a.combined_score).unwrap_or(std::cmp::Ordering::Equal)
        });
        filtered.truncate(options.top_k);

        Ok(filtered)
    }

    /// BM25 keyword search with optional `document_id` list filter.
    /// Filters apply identically to the FTS5 (SQLite) and tsvector (PG) paths.
    async fn bm25_search_with_filter(
        &self,
        collection_id: &str,
        query: &str,
        top_k: usize,
        doc_ids: Option<&[String]>,
    ) -> Result<Vec<Bm25Result>> {
        // 集合表不存在（如记忆命名空间尚未写入任何条目）时优雅返回空结果，
        // 避免对缺失的 _meta 表执行 SQL 导致查询报错。
        let safe_name_early = sanitize_name_for_table(collection_id);
        let meta_exists = self
            .vector_store
            .table_exists(&format!("vec_{safe_name_early}_meta"))
            .await
            .unwrap_or(false);
        if !meta_exists {
            return Ok(vec![]);
        }

        if self.db.get_database_backend() == DbBackend::Postgres {
            return self.bm25_search_pg_with_filter(collection_id, query, top_k, doc_ids).await;
        }

        let safe_name = sanitize_name_for_table(collection_id);
        let meta_table = format!("vec_{safe_name}_meta");
        let fts_table = format!("{meta_table}_fts");

        // 提取查询 token：fts_tokens（字符数 ≥3）供 FTS5 trigram MATCH；
        // all_tokens 全量保留，FTS 无命中时供 LIKE 降级（中文两字词、AI/PE 等短 token
        // 无法被 trigram MATCH，必须走降级路径才能命中）。
        let (fts_tokens, all_tokens) = extract_query_tokens(query);
        let sanitized = fts_tokens.join(" OR ");
        if sanitized.is_empty() {
            if all_tokens.is_empty() {
                return Ok(vec![]);
            }
            return self
                .bm25_search_fallback_with_filter(&meta_table, &all_tokens, top_k, doc_ids)
                .await;
        }

        let (fts_sql, mut params) = match doc_ids {
            Some(ids) if !ids.is_empty() => {
                // SQLite 占位符全部用显式编号避免歧义：
                //   ?1 = query, ?2..?(N+1) = doc_ids, ?(N+2) = top_k
                let placeholders = ids
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", i + 2))
                    .collect::<Vec<_>>()
                    .join(", ");
                let limit_ph = format!("?{}", ids.len() + 2);
                let in_clause = format!(" AND m.document_id IN ({placeholders})");
                let sql = format!(
                    "SELECT m.id, m.document_id, m.chunk_index, m.content, bm25({fts_table}) as bm25_score \
                     FROM {fts_table} f \
                     JOIN {meta_table} m ON m.rowid = f.rowid \
                     WHERE {fts_table} MATCH ?1{in_clause} \
                     ORDER BY bm25_score \
                     LIMIT {limit_ph}"
                );
                (sql, ids.iter().cloned().map(Value::from).collect::<Vec<_>>())
            },
            _ => {
                let sql = format!(
                    "SELECT m.id, m.document_id, m.chunk_index, m.content, bm25({fts_table}) as bm25_score \
                     FROM {fts_table} f \
                     JOIN {meta_table} m ON m.rowid = f.rowid \
                     WHERE {fts_table} MATCH ?1 \
                     ORDER BY bm25_score \
                     LIMIT ?2"
                );
                (sql, Vec::new())
            },
        };

        // SQLite 参数顺序: [query, doc_ids..., top_k]
        let mut values = Vec::with_capacity(params.len() + 2);
        values.push(sanitized.clone().into());
        values.append(&mut params);
        values.push((top_k as i64).into());

        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(DbBackend::Sqlite, &fts_sql, values))
            .await;

        match rows {
            Ok(rows) if !rows.is_empty() => {
                let results: Vec<Bm25Result> = rows
                    .into_iter()
                    .filter_map(|row| {
                        let id: String = row.try_get("", "id").ok()?;
                        let document_id: String = row.try_get("", "document_id").ok()?;
                        let chunk_index: i32 = row.try_get("", "chunk_index").ok()?;
                        let content: String = row.try_get("", "content").ok()?;
                        let bm25_raw: f64 = row.try_get("", "bm25_score").ok().unwrap_or(0.0);
                        let bm25_score = (-bm25_raw as f32).max(0.0);

                        Some(Bm25Result { id, document_id, chunk_index, content, bm25_score })
                    })
                    .collect();

                if !results.is_empty() {
                    return Ok(results);
                }

                self.bm25_search_fallback_with_filter(&meta_table, &all_tokens, top_k, doc_ids)
                    .await
            },
            _ => {
                self.bm25_search_fallback_with_filter(&meta_table, &all_tokens, top_k, doc_ids)
                    .await
            },
        }
    }

    /// PostgreSQL keyword search with optional `document_id` list filter.
    async fn bm25_search_pg_with_filter(
        &self,
        collection_id: &str,
        query: &str,
        top_k: usize,
        doc_ids: Option<&[String]>,
    ) -> Result<Vec<Bm25Result>> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(vec![]);
        }

        // ── 查询侧 n-gram 归一化（与索引侧共用同一套规范）──
        //
        // 索引由 PG 侧的 ax_cjk_ngram() 计算（生成列），查询由 Rust 侧的
        // text_ngram::cjk_ngram 计算。两者必须产出**逐字节相同**的 token 集合，
        // 否则索引里存在的 token 在查询里取不到 —— 静默零结果，不报错。
        // 一致性由共享 fixture 双向锁定（crates/search/tests/ngram_consistency.rs
        // 与 scripts/check-ngram-consistency.mjs）。
        //
        // 用 `to_tsquery(... | ...)` 而非 `plainto_tsquery(原文)`，两个原因：
        //   1. plainto 对中文无效 —— PG 把连续 CJK 当作单个词元，
        //      '向量索引实现方案' 会变成一个查询词，任何子串都永不命中；
        //   2. plainto 是 AND 语义，长查询（如整句自然语言）会因要求所有词元
        //      同时出现而必然零结果。OR + ts_rank 排序更符合检索预期。
        let tokens = crate::text_ngram::cjk_ngram_query_tokens(trimmed);
        if tokens.is_empty() {
            // 归一化后无 token（纯标点等）：内容层面无可检索项，
            // 与"归一化失败"不同，不应视为错误。
            return Ok(vec![]);
        }
        let unsafe_tokens: Vec<&String> =
            tokens.iter().filter(|t| !crate::text_ngram::is_tsquery_safe_token(t)).collect();
        if !unsafe_tokens.is_empty() {
            // 归一化产物本不该含 tsquery 元字符（它们都归在分隔符类里）。
            // 一旦出现，说明 text_ngram 的字符类被改动却未同步，此时构造 tsquery
            // 会导致语法错误乃至语义篡改 —— fail-closed，不静默过滤后继续。
            return Err(AxAgentError::Provider(format!(
                "查询归一化产出了不可安全嵌入 tsquery 的 token: {unsafe_tokens:?}；\
                 请检查 text_ngram::is_separator 是否覆盖全部 tsquery 元字符"
            )));
        }
        let tsquery = tokens.iter().map(|t| format!("'{t}'")).collect::<Vec<_>>().join(" | ");

        let safe_name = sanitize_name_for_table(collection_id);
        let meta_table = format!("vec_{safe_name}_meta");

        // Build optional IN clause; PG placeholders are positional ($n).
        let (in_clause, mut params) = match doc_ids {
            Some(ids) if !ids.is_empty() => {
                let placeholders = ids
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("${}", i + 3))
                    .collect::<Vec<_>>()
                    .join(", ");
                let clause = format!(" AND m.document_id IN ({placeholders})");
                (clause, ids.iter().cloned().map(Value::from).collect::<Vec<_>>())
            },
            _ => (String::new(), Vec::new()),
        };

        // $1 是**已归一化的 tsquery 表达式**（如 `'向量' | '量索' | '索引'`），
        // 不是用户原文 —— 见上方归一化说明。
        let sql = format!(
            "SELECT m.id, m.document_id, m.chunk_index, m.content, \
             ts_rank(m.content_tsv, query) AS bm25_score \
             FROM {meta_table} m, to_tsquery('simple', $1) query \
             WHERE m.content_tsv @@ query{in_clause} \
             ORDER BY bm25_score DESC \
             LIMIT $2"
        );

        let mut values = Vec::with_capacity(params.len() + 2);
        values.push(tsquery.into());
        values.push((top_k as i64).into());
        values.append(&mut params);

        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(DbBackend::Postgres, &sql, values))
            .await;

        match rows {
            Ok(rows) if !rows.is_empty() => {
                let results: Vec<Bm25Result> = rows
                    .into_iter()
                    .filter_map(|row| {
                        let id: String = row.try_get("", "id").ok()?;
                        let document_id: String = row.try_get("", "document_id").ok()?;
                        let chunk_index: i32 = row.try_get("", "chunk_index").ok()?;
                        let content: String = row.try_get("", "content").ok()?;
                        let rank: f64 = row.try_get("", "bm25_score").ok().unwrap_or(0.0);
                        let bm25_score = rank as f32;
                        Some(Bm25Result { id, document_id, chunk_index, content, bm25_score })
                    })
                    .collect();

                if !results.is_empty() {
                    return Ok(results);
                }
                let (_, pg_tokens) = extract_query_tokens(trimmed);
                self.bm25_search_fallback_pg_with_filter(&meta_table, &pg_tokens, top_k, doc_ids)
                    .await
            },
            _ => {
                let (_, pg_tokens) = extract_query_tokens(trimmed);
                self.bm25_search_fallback_pg_with_filter(&meta_table, &pg_tokens, top_k, doc_ids)
                    .await
            },
        }
    }

    async fn bm25_search_fallback_with_filter(
        &self,
        meta_table: &str,
        tokens: &[String],
        top_k: usize,
        doc_ids: Option<&[String]>,
    ) -> Result<Vec<Bm25Result>> {
        if self.db.get_database_backend() == DbBackend::Postgres {
            return self
                .bm25_search_fallback_pg_with_filter(meta_table, tokens, top_k, doc_ids)
                .await;
        }

        let words: Vec<&str> = tokens.iter().map(|s| s.as_str()).take(8).collect();
        if words.is_empty() {
            return Ok(vec![]);
        }

        let conditions: Vec<String> =
            words.iter().map(|w| format!("content LIKE '%{}%'", w.replace('\'', "''"))).collect();
        let where_clause = conditions.join(" OR ");

        // SQLite 路径：占位符全部用显式编号避免歧义。
        //   无 doc_ids: VALUES=[top_k], LIMIT ?1
        //   有 doc_ids: VALUES=[doc_ids..., top_k], IN(?1..?N), LIMIT ?(N+1)
        let (where_with_filter, mut params, limit_placeholder) = match doc_ids {
            Some(ids) if !ids.is_empty() => {
                let placeholders = ids
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(", ");
                let limit_ph = format!("?{}", ids.len() + 1);
                let where_with_in = format!("({where_clause}) AND document_id IN ({placeholders})");
                (where_with_in, ids.iter().cloned().map(Value::from).collect::<Vec<_>>(), limit_ph)
            },
            _ => (where_clause, Vec::new(), "?1".to_string()),
        };

        let sql = format!(
            "SELECT id, document_id, chunk_index, content, \
             (CASE WHEN content LIKE '%{}%' THEN 1.0 ELSE 0.3 END) as bm25_score \
             FROM {meta_table} \
             WHERE {where_with_filter} \
             LIMIT {limit_placeholder}",
            words.first().unwrap_or(&"").replace('\'', "''")
        );

        let mut values: Vec<Value> = Vec::with_capacity(params.len() + 1);
        values.append(&mut params);
        values.push((top_k as i64).into());

        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(DbBackend::Sqlite, &sql, values))
            .await
            .map_err(|e| AxAgentError::Provider(format!("BM25 fallback search failed: {}", e)))?;

        let results: Vec<Bm25Result> = rows
            .into_iter()
            .filter_map(|row| {
                let id: String = row.try_get("", "id").ok()?;
                let document_id: String = row.try_get("", "document_id").ok()?;
                let chunk_index: i32 = row.try_get("", "chunk_index").ok()?;
                let content: String = row.try_get("", "content").ok()?;
                let bm25_score: f32 = row.try_get("", "bm25_score").ok()?;

                Some(Bm25Result { id, document_id, chunk_index, content, bm25_score })
            })
            .collect();

        Ok(results)
    }

    /// PostgreSQL keyword fallback: substring (`ILIKE`) match when the tsvector
    /// path yields nothing. Mirrors the SQLite `LIKE` fallback semantics.
    async fn bm25_search_fallback_pg_with_filter(
        &self,
        meta_table: &str,
        tokens: &[String],
        top_k: usize,
        doc_ids: Option<&[String]>,
    ) -> Result<Vec<Bm25Result>> {
        let words: Vec<&str> = tokens.iter().map(|s| s.as_str()).take(8).collect();
        if words.is_empty() {
            return Ok(vec![]);
        }

        let conditions: Vec<String> =
            words.iter().map(|w| format!("content ILIKE '%{}%'", w.replace('\'', "''"))).collect();
        let mut where_clause = conditions.join(" OR ");

        // PostgreSQL 占位符必须从 $1 连续编号且不可跳号：
        //   values 顺序 = [top_k, doc_ids...] → $1=top_k(LIMIT), $2..$(N+1)=doc_ids
        // 修复前 bug：无 doc_ids 时 LIMIT $2（缺 $1）、有 doc_ids 时 LIMIT $N+2（跳号），
        // 导致 sqlx 报 "绑定消息提供了1个参数,但是已准备好语句要求2个参数"。
        let (in_clause, mut params, limit_ph) = match doc_ids {
            Some(ids) if !ids.is_empty() => {
                let placeholders = ids
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("${}", i + 2))
                    .collect::<Vec<_>>()
                    .join(", ");
                let clause = format!(" AND document_id IN ({placeholders})");
                (clause, ids.iter().cloned().map(Value::from).collect::<Vec<_>>(), "$1".to_string())
            },
            _ => (String::new(), Vec::new(), "$1".to_string()),
        };

        where_clause = format!("({where_clause}){in_clause}");

        let sql = format!(
            "SELECT id, document_id, chunk_index, content, \
             (CASE WHEN content ILIKE '%{}%' THEN 1.0 ELSE 0.3 END) as bm25_score \
             FROM {meta_table} \
             WHERE {where_clause} \
             LIMIT {limit_ph}",
            words.first().unwrap_or(&"").replace('\'', "''")
        );

        let mut values: Vec<Value> = Vec::with_capacity(params.len() + 1);
        values.push((top_k as i64).into());
        values.append(&mut params);

        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(DbBackend::Postgres, &sql, values))
            .await
            .map_err(|e| AxAgentError::Provider(format!("BM25 fallback search failed: {}", e)))?;

        let results: Vec<Bm25Result> = rows
            .into_iter()
            .filter_map(|row| {
                let id: String = row.try_get("", "id").ok()?;
                let document_id: String = row.try_get("", "document_id").ok()?;
                let chunk_index: i32 = row.try_get("", "chunk_index").ok()?;
                let content: String = row.try_get("", "content").ok()?;
                let bm25_score: f32 = row.try_get("", "bm25_score").ok()?;

                Some(Bm25Result { id, document_id, chunk_index, content, bm25_score })
            })
            .collect();

        Ok(results)
    }

    /// 加权 RRF 融合。
    ///
    /// 每一路的贡献是 `weight / (k + rank + 1)` —— 与**无权 RRF**
    /// （`1 / (k + rank + 1)`）的区别是两路可按权重配比，这正是
    /// `HybridConfig.vector_weight` / `bm25_weight` 的消费点。
    /// 语义与 `merge_results_weighted` 一致（分数越大越相关），但 RRF 只用排名、
    /// 不吃原始分数量纲，因此跨后端（SQLite FTS5 / PG tsvector）更稳。
    fn merge_results_rrf(
        vector_results: Vec<VectorSearchResult>,
        bm25_results: Vec<Bm25Result>,
        k: f32,
        vector_weight: f32,
        bm25_weight: f32,
    ) -> Vec<HybridSearchResult> {
        let mut score_map: std::collections::HashMap<String, HybridSearchResult> =
            std::collections::HashMap::new();

        for (rank, vr) in vector_results.iter().enumerate() {
            let rrf_score = vector_weight / (k + (rank as f32) + 1.0);
            score_map.insert(
                vr.id.clone(),
                HybridSearchResult {
                    id: vr.id.clone(),
                    document_id: vr.document_id.clone(),
                    chunk_index: vr.chunk_index,
                    content: vr.content.clone(),
                    vector_score: Some(l2_distance_to_similarity(vr.score)),
                    bm25_score: None,
                    sparse_score: None,
                    combined_score: rrf_score,
                },
            );
        }

        for (rank, br) in bm25_results.iter().enumerate() {
            let rrf_score = bm25_weight / (k + (rank as f32) + 1.0);
            if let Some(existing) = score_map.get_mut(&br.id) {
                existing.bm25_score = Some(br.bm25_score);
                existing.combined_score += rrf_score;
            } else {
                score_map.insert(
                    br.id.clone(),
                    HybridSearchResult {
                        id: br.id.clone(),
                        document_id: br.document_id.clone(),
                        chunk_index: br.chunk_index,
                        content: br.content.clone(),
                        vector_score: None,
                        bm25_score: Some(br.bm25_score),
                        sparse_score: None,
                        combined_score: rrf_score,
                    },
                );
            }
        }

        // 归一到 [0,1] 相关度（**相对**标尺：RRF 分只由排名得出，本身不含绝对质量
        // 信号）。除以本结果集最大值只做缩放，排序完全不变。
        //
        // 这一步是 `retrieval_threshold` 在 RRF 路径上能生效的**唯一**前提：
        // 归一前 `combined_score ∈ [0.017, 0.033]`，`search_with_filter` 取反后
        // `score ∈ [0.967, 0.983]` ⇒ 任何 `score <= 约定上限` 的过滤都失效。
        let max_rrf_score = score_map.values().map(|r| r.combined_score).fold(0f32, f32::max);
        score_map
            .into_values()
            .map(|mut r| {
                r.combined_score = normalize_by_max(r.combined_score, max_rrf_score);
                r
            })
            .collect()
    }

    fn merge_results_weighted(
        &self,
        vector_results: Vec<VectorSearchResult>,
        bm25_results: Vec<Bm25Result>,
        vector_weight: f32,
        bm25_weight: f32,
    ) -> Vec<HybridSearchResult> {
        let mut score_map: std::collections::HashMap<String, HybridSearchResult> =
            std::collections::HashMap::new();

        // 向量侧用**绝对**标尺（`DEFAULT_MAX_L2_DISTANCE` 为饱和点），与
        // `finish_vector_only` / `rag_pipeline` 一致；此前这里用「本次结果集最大距离」
        // 归一 ⇒ 同一个 L2 值在不同融合算法下算出不同的 `vector_score`。
        // BM25 侧仍为相对归一（BM25 跨后端无绝对标尺）。
        let max_bm25_score =
            bm25_results.iter().map(|r| r.bm25_score).fold(0f32, f32::max).max(f32::EPSILON);

        for vr in vector_results {
            let normalized_vector = l2_distance_to_similarity(vr.score);

            let (bm25_part, bm25_raw) = bm25_results
                .iter()
                .find(|b| b.id == vr.id)
                .map(|b| {
                    let norm = b.bm25_score / max_bm25_score;
                    (Some(norm), Some(b.bm25_score))
                })
                .unwrap_or((None, None));

            let combined =
                normalized_vector * vector_weight + bm25_part.unwrap_or(0.0) * bm25_weight;

            score_map.insert(
                vr.id.clone(),
                HybridSearchResult {
                    id: vr.id,
                    document_id: vr.document_id,
                    chunk_index: vr.chunk_index,
                    content: vr.content,
                    vector_score: Some(normalized_vector),
                    bm25_score: bm25_raw,
                    sparse_score: None,
                    combined_score: combined,
                },
            );
        }

        for br in bm25_results {
            if score_map.contains_key(&br.id) {
                continue;
            }
            let normalized_bm25 = br.bm25_score / max_bm25_score;
            let combined = if vector_weight > 0.0 {
                normalized_bm25 * bm25_weight
            } else {
                normalized_bm25
            };

            score_map.insert(
                br.id.clone(),
                HybridSearchResult {
                    id: br.id,
                    document_id: br.document_id,
                    chunk_index: br.chunk_index,
                    content: br.content,
                    vector_score: None,
                    bm25_score: Some(br.bm25_score),
                    sparse_score: None,
                    combined_score: combined,
                },
            );
        }

        score_map.into_values().collect()
    }
}

#[derive(Debug, Clone)]
struct Bm25Result {
    id: String,
    document_id: String,
    chunk_index: i32,
    content: String,
    bm25_score: f32,
}

fn sanitize_name_for_table(collection_id: &str) -> String {
    collection_id.chars().map(|c| if c == '-' { '_' } else { c }).collect()
}

/// 从查询中提取 token（按 Unicode 字符计数，修复此前按字节计数导致中文 token 被误删的问题）。
///
/// 返回 `(fts_tokens, all_tokens)`：
/// - `fts_tokens`：字符数 ≥3 的 token，可被 FTS5 trigram tokenizer MATCH
///   （trigram 的硬性限制：少于 3 个字符的查询词无法命中任何行）；
/// - `all_tokens`：全部有效 token（字符数 ≥1），供 LIKE 降级路径使用，
///   保证中文两字词（如"茅台"）和英文短词（如 AI、PE）仍可通过子串匹配命中。
fn extract_query_tokens(query: &str) -> (Vec<String>, Vec<String>) {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let mut all_tokens: Vec<String> = Vec::new();
    let mut current = String::new();

    for c in trimmed.chars() {
        if c.is_alphanumeric() || c == '-' || c == '_' || ('\u{4e00}'..='\u{9fff}').contains(&c) {
            current.push(c);
        } else if !current.is_empty() {
            all_tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        all_tokens.push(current);
    }

    let fts_tokens: Vec<String> = all_tokens
        .iter()
        .filter(|t| t.chars().count() >= 3)
        .map(|t| t.replace('\'', "''"))
        .collect();

    (fts_tokens, all_tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hybrid_search_options_default_fusion_rrf() {
        let opts = HybridSearchOptions::default();
        assert_eq!(opts.fusion, FusionAlgorithm::Rrf);
        assert!((opts.vector_weight - 0.7).abs() < f32::EPSILON);
        assert!((opts.bm25_weight - 0.3).abs() < f32::EPSILON);
        assert_eq!(opts.top_k, 10);
        assert_eq!(opts.rrf_k, 60.0);
    }

    #[test]
    fn test_hybrid_search_options_weighted_fusion() {
        let opts = HybridSearchOptions {
            fusion: FusionAlgorithm::Weighted,
            vector_weight: 0.5,
            bm25_weight: 0.5,
            ..Default::default()
        };
        assert_eq!(opts.fusion, FusionAlgorithm::Weighted);
        assert!((opts.vector_weight - 0.5).abs() < f32::EPSILON);
        assert!((opts.bm25_weight - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_hybrid_options_from_config_reads_user_weights() {
        // W2（2026-09-15）：`HybridConfig` 此前全仓零读取点 ⇒ 用户改权重不生效。
        // 本测试锁定「配置 → 选项」的映射，含 enabled / fusion / rrf_k。
        let cfg = axagent_harness::rag_config::HybridConfig {
            enabled: false,
            vector_weight: 0.25,
            bm25_weight: 0.75,
            sparse_weight: 0.5,
            fusion: "weighted".to_string(),
            rrf_k: 42.0,
        };
        let opts = hybrid_options_from_config(&cfg, 7);
        assert!(!opts.enabled, "enabled 必须来自配置");
        assert!((opts.vector_weight - 0.25).abs() < f32::EPSILON);
        assert!((opts.bm25_weight - 0.75).abs() < f32::EPSILON);
        assert!((opts.sparse_weight - 0.5).abs() < f32::EPSILON);
        assert_eq!(opts.fusion, FusionAlgorithm::Weighted);
        assert!((opts.rrf_k - 42.0).abs() < f32::EPSILON);
        assert_eq!(opts.top_k, 7, "top_k 由调用方给出");

        // 未知 fusion 字符串回退 RRF（默认算法），不得静默变成 Weighted
        let cfg_unknown =
            axagent_harness::rag_config::HybridConfig { fusion: "bogus".to_string(), ..cfg };
        assert_eq!(hybrid_options_from_config(&cfg_unknown, 3).fusion, FusionAlgorithm::Rrf);
    }

    #[test]
    fn test_weighted_rrf_weights_change_ranking() {
        // W2（2026-09-15）：加权 RRF。接线前 RRF 只收 `rrf_k`、两路权重被忽略，
        // 因此同一个命中在 0.9/0.1 与 0.1/0.9 两种权重下相对名次**不变**。
        // 本测试要求名次反转 —— 权重未被读取时必然失败。
        let make_vector_hits = || {
            vec![VectorSearchResult {
                id: "v-only".to_string(),
                document_id: "d-vec".to_string(),
                chunk_index: 0,
                content: "只有向量命中".to_string(),
                score: 0.1,
                has_embedding: true,
            }]
        };
        let make_bm25_hits = || {
            vec![Bm25Result {
                id: "b-only".to_string(),
                document_id: "d-bm25".to_string(),
                chunk_index: 0,
                content: "只有关键词命中".to_string(),
                bm25_score: 9.0,
            }]
        };
        let score_of = |hits: &[HybridSearchResult], id: &str| {
            hits.iter().find(|r| r.id == id).map(|r| r.combined_score).expect("测试：应含该 id")
        };

        let vector_first =
            HybridSearcher::merge_results_rrf(make_vector_hits(), make_bm25_hits(), 60.0, 0.9, 0.1);
        assert!(
            score_of(&vector_first, "v-only") > score_of(&vector_first, "b-only"),
            "向量权重 0.9 时 v-only 应领先"
        );

        let bm25_first =
            HybridSearcher::merge_results_rrf(make_vector_hits(), make_bm25_hits(), 60.0, 0.1, 0.9);
        assert!(
            score_of(&bm25_first, "b-only") > score_of(&bm25_first, "v-only"),
            "关键词权重 0.9 时 b-only 应领先 —— 若权重未被读取，此处必然失败"
        );
    }

    #[test]
    fn test_hybrid_search_result_serialization() {
        let result = HybridSearchResult {
            id: "test-id".to_string(),
            document_id: "doc-id".to_string(),
            chunk_index: 3,
            content: "test content".to_string(),
            vector_score: Some(0.85),
            bm25_score: Some(0.42),
            sparse_score: None,
            combined_score: 0.65,
        };
        let json = serde_json::to_value(&result).expect("测试：to_value 应成功");
        assert_eq!(json["id"], "test-id");
        assert_eq!(json["chunk_index"], 3);
        assert!((json["combined_score"].as_f64().expect("测试应成功") - 0.65).abs() < 0.001);
    }

    #[test]
    fn test_fusion_algorithm_serde() {
        assert_eq!(serde_json::to_value(FusionAlgorithm::Rrf).expect("测试应成功"), "Rrf");
        assert_eq!(
            serde_json::to_value(FusionAlgorithm::Weighted).expect("测试应成功"),
            "Weighted"
        );
    }

    #[test]
    fn test_fusion_algorithm_default_is_rrf() {
        assert_eq!(FusionAlgorithm::default(), FusionAlgorithm::Rrf);
    }

    #[test]
    fn test_sanitize_name_for_table() {
        assert_eq!(sanitize_name_for_table("my-collection"), "my_collection");
        assert_eq!(sanitize_name_for_table("simple"), "simple");
        assert_eq!(sanitize_name_for_table("a-b-c"), "a_b_c");
    }

    #[test]
    fn test_hybrid_search_result_default_combined() {
        let result = HybridSearchResult {
            id: "id".into(),
            document_id: "doc".into(),
            chunk_index: 0,
            content: "".into(),
            vector_score: None,
            bm25_score: None,
            sparse_score: None,
            combined_score: 0.0,
        };
        assert_eq!(result.combined_score, 0.0);
        assert!(result.vector_score.is_none());
    }

    fn vector_hit(id: &str, l2_distance: f32) -> VectorSearchResult {
        VectorSearchResult {
            id: id.to_string(),
            document_id: format!("doc-{id}"),
            chunk_index: 0,
            content: format!("内容 {id}"),
            score: l2_distance,
            has_embedding: true,
        }
    }

    fn bm25_hit(id: &str, bm25: f32) -> Bm25Result {
        Bm25Result {
            id: id.to_string(),
            document_id: format!("doc-{id}"),
            chunk_index: 0,
            content: format!("内容 {id}"),
            bm25_score: bm25,
        }
    }

    /// L2 → 相关度的绝对标尺：`0 ⇒ 1.0`、`DEFAULT_MAX_L2_DISTANCE ⇒ 0.0`、超出被夹住。
    #[test]
    fn test_l2_similarity_is_absolute_and_clamped() {
        assert!((l2_distance_to_similarity(0.0) - 1.0).abs() < 1e-6);
        assert!((l2_distance_to_similarity(DEFAULT_MAX_L2_DISTANCE / 2.0) - 0.5).abs() < 1e-6);
        assert!(l2_distance_to_similarity(DEFAULT_MAX_L2_DISTANCE).abs() < 1e-6);
        assert!(
            l2_distance_to_similarity(DEFAULT_MAX_L2_DISTANCE * 10.0).abs() < 1e-6,
            "超出饱和点必须夹到 0.0，不得产出负相关度"
        );
    }

    /// `normalize_by_max` 在空集 / 全零下不得产出 NaN（除零）。
    #[test]
    fn test_normalize_by_max_is_nan_free() {
        assert!((normalize_by_max(3.0, 6.0) - 0.5).abs() < 1e-6);
        assert!((normalize_by_max(6.0, 6.0) - 1.0).abs() < 1e-6);
        assert!(normalize_by_max(0.0, 0.0).is_finite());
        assert!(normalize_by_max(0.0, 0.0).abs() < 1e-6);
        assert!(normalize_by_max(1.0, -1.0).is_finite(), "负基准不得翻转或产生 NaN");
    }

    /// **RRF 融合结果已归一到 [0,1] 且阈值可过滤**（§10.7#4 的核心回归）。
    ///
    /// 归一前 `combined_score = Σ w/(k+rank+1) ∈ [0.017, 0.033]`：
    /// 任何「相关度下限」过滤（哪怕低到 0.1）都会把结果**全部**滤掉，
    /// 而任何「距离上限 ≤ 20」过滤又恒真 —— 两个方向同时坏。
    #[test]
    fn test_rrf_scores_are_normalized_into_unit_range() {
        let vector = vec![vector_hit("a", 0.3), vector_hit("b", 0.8), vector_hit("c", 1.5)];
        let bm25 = vec![bm25_hit("b", 9.0), bm25_hit("d", 4.0)];

        let fused = HybridSearcher::merge_results_rrf(vector, bm25, 60.0, 0.7, 0.3);

        assert!(!fused.is_empty());
        for r in &fused {
            assert!(
                (0.0..=1.0).contains(&r.combined_score),
                "归一后 combined_score 必须落在 [0,1]，实得 {}（id={}）",
                r.combined_score,
                r.id
            );
        }
        let max = fused.iter().map(|r| r.combined_score).fold(0f32, f32::max);
        assert!((max - 1.0).abs() < 1e-6, "最相关的一条必须归一到 1.0，实得 {max}");

        // 归一前的值域（≈0.017–0.033）必须已经消失
        assert!(
            fused.iter().all(|r| r.combined_score > 0.05),
            "仍存在归一前的 RRF 原始量纲 ⇒ 缩放这一步没生效"
        );

        // 阈值从此有意义：下限越高，留下的越少
        let keep = |floor: f32| fused.iter().filter(|r| r.combined_score >= floor).count();
        assert_eq!(keep(0.0), fused.len(), "下限 0 应全留");
        assert!(keep(0.5) < fused.len(), "下限 0.5 必须真的丢掉一部分 —— 否则过滤又是空过滤");
        assert!(keep(0.5) > 0, "下限 0.5 不应把结果全丢掉");
    }

    /// 向量-only 收尾路径：绝对标尺 + `min_score` 真的生效。
    #[test]
    fn test_vector_only_finish_normalizes_and_filters() {
        let hits = vec![
            vector_hit("perfect", 0.0),                          // ⇒ 1.0
            vector_hit("mid", DEFAULT_MAX_L2_DISTANCE / 2.0),    // ⇒ 0.5
            vector_hit("far", DEFAULT_MAX_L2_DISTANCE),          // ⇒ 0.0
            vector_hit("beyond", DEFAULT_MAX_L2_DISTANCE * 3.0), // ⇒ 0.0（夹住）
        ];

        let all = HybridSearcher::finish_vector_only(hits.clone(), 10, None);
        assert_eq!(all.len(), 4, "min_score=None 时不得过滤（此处曾用 is_some_and 反向判断）");
        assert!((all[0].combined_score - 1.0).abs() < 1e-6, "最相关的一条应排首位");

        let filtered = HybridSearcher::finish_vector_only(hits, 10, Some(0.5));
        let ids: Vec<&str> = filtered.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["perfect", "mid"], "下限 0.5 应只留 perfect/mid");
    }

    /// 收尾顺序：**先排序后截断**（`top_k` 必须留给最相关的那几条）。
    ///
    /// 混合路径此前是「先 `.take(top_k)` 后 `sort_by`」，而输入来自
    /// `HashMap::into_values()`（顺序不定）⇒ 留下的是随机子集。
    #[test]
    fn test_top_k_keeps_the_most_relevant_not_an_arbitrary_subset() {
        let hits: Vec<VectorSearchResult> = (0..10)
            .map(|i| vector_hit(&format!("h{i}"), i as f32)) // h0 最相关 … h9 最不相关
            .collect();

        let truncated = HybridSearcher::finish_vector_only(hits, 3, None);
        let ids: Vec<&str> = truncated.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["h0", "h1", "h2"], "截断后留下必须是最相关的 3 条且有序");
    }
}
