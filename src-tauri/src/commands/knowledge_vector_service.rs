// SPDX-License-Identifier: AGPL-3.0-only

//! `knowledge.vector` 接缝的**宿主侧实现**（wiring 层，宿主独持 `VectorStore` + embedding 链）。
//!
//! 契约在 `axagent-core/crates/harness/src/knowledge_vector.rs`（已冻结，四个 op：`search` /
//! `keyword_search` / `index_text` / `get_collection_info`；⚠ **op 名 ≠ 方法名**，第四个的方法名
//! 是 [`KnowledgeVectorProvider::collection_info`]，PLAN §58.3-6 定案）。本文件只做「契约形状 →
//! 宿主既有链」的接线，不新造第二套检索 / 入库实现。
//!
//! ## 与命令层的关系（禁区 12：数字与规则只有一处权威）
//!
//! - `search` **直接调** `crate::indexing::search_knowledge_with_doc_filter` —— 与面板检索、
//!   Paper QA 同一条链（`rag::search_with_filter`）。`top_k` / `min_similarity` / `doc_ids` 的
//!   「过滤 → 排序 → 截断」全在检索层完成，本层**不重排、不再筛**（在那里再筛一遍就是第二把尺子，
//!   见下节）。
//! - `keyword_search` **直接调** `VectorStore::fts_search`（宿主唯一的原生收文本的检索口）。
//! - `index_text` 复用 `run_indexing` 的 `FromText` 形态：`rag::prepare_chunks` →
//!   `indexing::generate_embeddings` → `rag::index`，三段都是宿主在用的公开函数，本层不抄正文。
//! - 分层纪律：本文件不出现 `sea_orm::` / `axagent_entities::` 字面量（DB 类型一律走
//!   `axagent_dao::DatabaseConnection` 这个再导出），读写表结构的活全部在 dao / search 侧。
//!
//! ## `score` 的方向（PLAN §58.3-5：契约层钉不住，由本批单测锁）
//!
//! 宿主 [`axagent_harness::vector_store::VectorSearchResult::score`] 的**唯一量纲**是距离
//! （**越小越相似**）：`rag.rs` 在返回前写 `score: 1.0 - r.combined_score`，而 `min_similarity`
//! 是**相关度**下限（`combined_score >= min_score`，在截断**之前**生效）。两把尺子互为补数 ⇒
//! 本层的纪律是**逐字透传**：把 `min_similarity` 原样交给检索层，把 `score` 原样放进视图。
//! 任何「在 provider 侧按 `score <= min_similarity` 再筛一道」的写法都是拿相关度的阈值去比距离，
//! 属宿主 2026-09-15 已经修掉的那个缺陷（`rag.rs` 里那条「两把尺子」的注释就是它的案发现场），
//! 故本批用单测钉住「越近的排越前 + 阈值原样透传」而不是再证一遍「筛得动」。
//!
//! ## 硬闸与数字的单源（禁区 12 + 裁定 N4）
//!
//! 本文件的每一个闸都只引用 harness 的常量：[`KNOWLEDGE_CONTENT_MAX_BYTES`]、
//! [`KNOWLEDGE_TOP_K_RANGE`]（其上界与 [`KNOWLEDGE_SEARCH_TOP_K_MAX`] 同值，编译期已被契约钉住）、
//! [`KNOWLEDGE_SNIPPET_MAX_CHARS`]。越界一律 `Err` 拒收，**不静默夹取、不静默截断** —— 夹取会让
//! 插件以为自己设成了被夹掉的那个数，截断会让插件以为整篇都进了库。
//! 唯一例外是预览：[`KnowledgeChunkView::snippet`] 是「按常量裁预览」，那是视图的定义而非闸。
//! `min_similarity` 的 `[0, 1]` 不是夹取区间而是**量纲定义**（宿主侧它就是相关度），越出量纲的
//! 值没有可解释的语义 ⇒ 同样拒收。NaN 必须单判：`0.0..=1.0` 的 `contains` 对 NaN 恒 `false`
//! （fail-closed，保留），但任何 `v <= 1.0` / `v >= 0.0` 形式的比较对 NaN 恒 `false`
//! （fail-open）—— 先显式 `is_nan()` 拒掉，别让它顺着比较溜进检索层。
//!
//! ## 视图不过帧原文
//!
//! [`KnowledgeChunkView`] 只有 [`KnowledgeChunkView::snippet`]、没有整块 `content`（契约侧已裁掉，
//! 先例 `jobs_store.rs` 的「单次执行输出可能很大，不过帧」）。本文件是**唯一的裁剪点**：
//! 按 [`KNOWLEDGE_SNIPPET_MAX_CHARS`] 以 **char** 为单位截（`chars().take(n)`），不许按字节切 —
//! 中文正文按 `&s[..n]` 切会在非边界字节上 panic。也不许新增「取全文」字段：那会越回契约面。
//!
//! ## 写面：`index_text` 的三条主权边界
//!
//! 1. **不隐式建集合**（PLAN §56.2-2）：写前先读 `get_collection_dimensions`，集合不存在 ⇒
//!    `Err(AxAgentError::Validation)`。**禁止** `ensure_collection` —— 插件可无限造命名空间，
//!    而删除权已收归宿主（契约「主权边界」），造完没人能清。⚠ 变体必须是 `Validation`：
//!    契约承诺该格由分派层映射成 `SEAM_CALL_INVALID_ARGS`（PLAN §58.2-①，#112 落地的共用函数），
//!    抛 `Provider` / `NotFound` 就不命中。
//! 2. **不收 provider 名与维度**（PLAN §56.2-1）：embedding 模型走
//!    `KnowledgeRAG::resolve_embedding_provider`（知识库自有配置，缺省回落宿主全局默认），维度取
//!    知识库行的 `embedding_dimensions`。插件指定 provider 等于把「任意触发模型计费」的口交给它。
//! 3. **文档行必须已存在且属于该集合**：接缝没有建文档行的面（文档行带 title / source_path /
//!    mime 等宿主元数据），放进一个陌生 `document_id` 只会在库里留下检索得到、面板删不掉的孤儿块
//!    —— 正是 PLAN §56.1-Q2 拿来做「不开 delete」理由的那种半截状态。故陌生 id ⇒ `Validation`。
//!
//! ### 幂等语义（接缝不承诺，本层就此定案）
//!
//! 宿主两条 chunk 切分路径的块 id 都是 `{document_id}_{index}`（`rag::prepare_chunks`），而
//! `rag::index` 落的是 `upsert_embeddings` —— 后者在**同一事务**内先按 `document_id` 删除该
//! 文档的全部旧行（meta + vec0），再插入新块 ⇒ 同一 `document_id` 重复入库 = **整文档替换**，
//! 正文变短也不会留尾块（`vector_store.rs` 的 `reindexing_shorter_document_drops_tail_chunks`
//! 钉住该行为，#123）。删除只发生在「写入该文档」这条路径内部；接缝仍**不提供**独立 delete op
//! （插件 supplied 的 `document_id` 可指向宿主的真实文档 ⇒ 独立删除面等于一把覆写用户文档的枪）。
//! 另：该去重/删除路径原先在 SQLite 上会因 `md5()` 不存在而整条写入失败（#133，已按方言分流）。
//!
//! ## 未装配 / 注册时机（裁定 J8）
//!
//! provider 只在 `init/services.rs` 装配成功后就地注册，四个句柄（DB 连接、`VectorStore`、
//! master key、provider registry）在那一刻都是 `AppState` 的**非 Option** 字段，本批不存在
//! 「拿不到依赖」的运行时分支 ⇒ 不写伪装的 `Option` 检查，也不把任何字段改成 `Option` 来凑
//! 「未装配」那一格（PLAN §58.2-② 明令禁止）。未注册的真实表现在接缝上就是分派层取不到
//! provider ⇒ `SEAM_PROVIDER_UNAVAILABLE`。
//!
//! ## 已知未修风险（PLAN §56.2-3 原文登记，不是本批引入）
//!
//! 首批没有速率闸（协议层只有 `write` 一档权限）⇒ 一个乱调 `index_text` 的插件能把宿主
//! embedding 花费推高；[`KNOWLEDGE_CONTENT_MAX_BYTES`] 是**当前唯一**的防线，本层逐字执行它。

use std::sync::Arc;

use async_trait::async_trait;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::knowledge_vector::{
    CollectionInfoView, KNOWLEDGE_CONTENT_MAX_BYTES, KNOWLEDGE_SIMILARITY_RANGE,
    KNOWLEDGE_SNIPPET_MAX_CHARS, KNOWLEDGE_TOP_K_RANGE, KeywordSearchRequest, KnowledgeChunkView,
    KnowledgeVectorProvider, VectorIndexOutcome, VectorIndexRequest, VectorSearchOutcome,
    VectorSearchRequest,
};
use axagent_harness::vector_store::VectorSearchResult;
use axagent_search::rag::{self, ChunkStrategy, KnowledgeRAG, RAGSource};
use axagent_search::vector_store::VectorStore;

/// `knowledge.vector` 的宿主实现。
///
/// 四个字段都是 `AppState` 装配产物的**同一份句柄**（克隆持有，不反向引用 AppState）：
/// 向量集合与 embedding 链的真相只有一份，provider 只是把它投影到契约形状上。
pub struct HostKnowledgeVector {
    /// 与 `state.harness.db()` 同一个连接池（`get_document` / `get_knowledge_base` 用）。
    db: Arc<axagent_dao::DatabaseConnection>,
    /// 与 `AppState.vector_store` 同一个 `Arc`（两份实例就有两份向量表句柄）。
    vector_store: Arc<VectorStore>,
    /// master key 的 owned 副本（`state.harness.master_key_owned()`）：embedding 链要解密
    /// provider 凭证，而契约不收 provider 名 ⇒ 解密主权留在宿主。
    master_key: [u8; 32],
    /// 与 `state.harness.provider_registry()` **同一个** `Arc`（`generate_embeddings` 要求
    /// 调用方传入，注释点名「不再内部 `RuntimeHarness::new` 重复创建，那会丢弃 adapter cache」）。
    provider_registry: Arc<dyn axagent_harness::registry::ProviderRegistry>,
}

impl HostKnowledgeVector {
    pub fn new(
        db: Arc<axagent_dao::DatabaseConnection>,
        vector_store: Arc<VectorStore>,
        master_key: [u8; 32],
        provider_registry: Arc<dyn axagent_harness::registry::ProviderRegistry>,
    ) -> Self {
        Self { db, vector_store, master_key, provider_registry }
    }
}

// ── 纯函数：闸、映射、裁剪（本文件全部数字与方向的落点）────────────────────

/// 物理集合名：宿主用 `{prefix}_{容器 id}`（权威是 `rag::collection_id` +
/// `KnowledgeRAG::collection_prefix()`）；`VectorStore` 的入口要的是**带前缀**的那一个，
/// 而 `indexing::search_knowledge*` 要的是裸知识库 id。两个口径混用就等于查错表。
fn physical_collection_id(collection_id: &str) -> String {
    rag::collection_id(KnowledgeRAG.collection_prefix(), collection_id)
}

/// `top_k` 闸：越出 [`KNOWLEDGE_TOP_K_RANGE`] ⇒ `Err(Validation)`，**不夹取**。
///
/// 下界 1 的语义（契约 [`axagent_harness::knowledge_vector`] 的 `KNOWLEDGE_TOP_K_RANGE` 文档）：
/// 「取最相似的 0 条」没有合法含义，想要空结果就不调用。
fn check_top_k(top_k: usize, op: &str) -> Result<()> {
    if (KNOWLEDGE_TOP_K_RANGE.0..=KNOWLEDGE_TOP_K_RANGE.1).contains(&top_k) {
        return Ok(());
    }
    Err(AxAgentError::Validation(format!(
        "knowledge.vector::{op} 的 top_k={top_k} 越出契约区间 {}..={}（越界拒收，不静默夹取）",
        KNOWLEDGE_TOP_K_RANGE.0, KNOWLEDGE_TOP_K_RANGE.1
    )))
}

/// `min_similarity` 闸：NaN 单判 + 量纲 [`KNOWLEDGE_SIMILARITY_RANGE`]（相关度，不是距离）。
///
/// `None` = 不过滤（契约 `VectorSearchRequest::min_similarity` 的缺省语义），照传。
fn check_min_similarity(min_similarity: Option<f32>, op: &str) -> Result<()> {
    let Some(value) = min_similarity else { return Ok(()) };
    if value.is_nan() {
        return Err(AxAgentError::Validation(format!(
            "knowledge.vector::{op} 的 min_similarity 是 NaN：任何 `<=` / `>=` 比较对 NaN 恒 \
             false（fail-open），必须先显式拒掉再进检索层"
        )));
    }
    if !value.is_finite() {
        return Err(AxAgentError::Validation(format!(
            "knowledge.vector::{op} 的 min_similarity={value} 不是有限数（±∞ 会把合格集变成空集或全集）"
        )));
    }
    if !(KNOWLEDGE_SIMILARITY_RANGE.0..=KNOWLEDGE_SIMILARITY_RANGE.1).contains(&value) {
        return Err(AxAgentError::Validation(format!(
            "knowledge.vector::{op} 的 min_similarity={value} 越出宿主量纲 [`KNOWLEDGE_SIMILARITY_RANGE`]（它是**相关度**\
             下限，与视图里那个距离分互为补数，越界的阈值没有可解释语义）"
        )));
    }
    Ok(())
}

/// 查询文本闸：空白串 ⇒ `Err(Validation)`。
///
/// `search` 会给查询文本算 embedding（按 token 计费），空串是一次**没有任何检索价值**的模型调用；
/// `keyword_search` 走 FTS5，宿主侧空查询返回空集，但接缝对两个 op 只该有一条规则 ⇒ 一并拒收。
fn check_query_text(query: &str, op: &str) -> Result<()> {
    if query.trim().is_empty() {
        return Err(AxAgentError::Validation(format!(
            "knowledge.vector::{op} 的 query 为空：语义检索会白烧一次 embedding 调用，关键词检索\
             恒零命中，两者都不如让插件自己改调用"
        )));
    }
    Ok(())
}

/// 正文预算闸：超 [`KNOWLEDGE_CONTENT_MAX_BYTES`] ⇒ `Err`，**不截断**。
///
/// 按**字节**计（UTF-8），与常量的口径逐字一致：中文正文的字符数远小于字节数，按字符判会放行
/// 三倍于预算的正文，而 embedding 按 token 计费。
fn check_content_budget(content: &str) -> Result<()> {
    let bytes = content.len();
    let budget = KNOWLEDGE_CONTENT_MAX_BYTES;
    if bytes > budget {
        return Err(AxAgentError::Validation(format!(
            "knowledge.vector::index_text 的 content 是 {bytes} 字节，越出契约上限 {budget} 字节\
             （超限拒收不截断：静默截断会让插件以为整篇都进了库）"
        )));
    }
    Ok(())
}

/// `doc_ids` 的「空数组 = 不限定」收敛（契约 `VectorSearchRequest::doc_ids` 的说明）。
///
/// 不加这一层，`Some(vec![])` 会原样传到检索层的 `IN (...)` 空列表上 —— 那是「限定在零个文档里
/// 检索」= 恒空，与插件写的「不限定」正好相反。
fn doc_ids_filter(doc_ids: Option<&[String]>) -> Option<&[String]> {
    doc_ids.filter(|ids| !ids.is_empty())
}

/// 正文预览裁剪：**按 char**，上限 [`KNOWLEDGE_SNIPPET_MAX_CHARS`]。
///
/// 按字节切（`&s[..n]`）会在非字符边界上 panic；中文正文是这里的常态不是边角。
fn snippet(text: &str) -> String {
    text.chars().take(KNOWLEDGE_SNIPPET_MAX_CHARS).collect()
}

/// 宿主命中 → 契约视图（**唯一**映射点：整块原文在这里被换成预览）。
fn chunk_view(hit: VectorSearchResult) -> KnowledgeChunkView {
    KnowledgeChunkView {
        id: hit.id,
        document_id: hit.document_id,
        chunk_index: hit.chunk_index,
        // `hit.content` 是整块正文：只截预览，本体不外传（视图面没有 `content` 字段可塑）。
        snippet: snippet(&hit.content),
        // 距离分逐字透传（越小越相似）—— 见模块头「`score` 的方向」。
        score: hit.score,
        has_embedding: hit.has_embedding,
    }
}

/// 命中列表 → 契约返回：顺序逐字保留（宿主已按距离升序），零命中 ⇒ 空数组，**不是错误**。
fn search_outcome(hits: Vec<VectorSearchResult>) -> VectorSearchOutcome {
    VectorSearchOutcome { results: hits.into_iter().map(chunk_view).collect() }
}

/// 「宿主说查不到这一行」与「接缝承诺的入参非法」之间的换算。
///
/// dao 的 `get_knowledge_base` / `get_document` 用 `NotFound` 表达「库里没这一行」。本接缝里这些
/// 缺席**都是调用方的入参问题**（集合 / 文档得先由宿主命令面建出来），所以落成 `Validation` ——
/// 契约承诺 `SEAM_CALL_INVALID_ARGS` 映射的正是这一族（PLAN §58.2-①）。其余变体原样透出。
fn absent_row_as_invalid_args(err: AxAgentError) -> AxAgentError {
    match err {
        AxAgentError::NotFound(message) => AxAgentError::Validation(format!(
            "knowledge.vector::index_text 的前置对象不存在：{message}（接缝不建知识库、不建文档行，\
             请先在宿主命令面创建）"
        )),
        other => other,
    }
}

/// 集合缺失那一格的错误构造（单独成函数只为让「必须是 `Validation`」这条可被单测钉住）。
fn absent_collection_error(collection_id: &str) -> AxAgentError {
    AxAgentError::Validation(format!(
        "knowledge.vector::index_text 的目标集合不存在：{collection_id}（§56.2-2：不隐式建集合，\
         禁止 ensure_collection —— 插件造完没人能清，删除权在宿主命令面）"
    ))
}

/// embedding 维度：取宿主知识库行的既有配置（契约不收维度入参，PLAN §56.2-1）。
fn collection_dimensions(knowledge_base: &axagent_harness::types::KnowledgeBase) -> Option<usize> {
    knowledge_base.embedding_dimensions.map(|d| d as usize)
}

/// 切分参数：知识库自有配置优先，缺省落回宿主 chunker 的默认常量（不抄数字）。
fn chunk_params(
    knowledge_base: &axagent_harness::types::KnowledgeBase,
) -> (usize, usize, Option<String>) {
    (
        knowledge_base
            .chunk_size
            .map(|v| v as usize)
            .unwrap_or(axagent_search::text_chunker::DEFAULT_CHUNK_SIZE),
        knowledge_base
            .chunk_overlap
            .map(|v| v as usize)
            .unwrap_or(axagent_search::text_chunker::DEFAULT_OVERLAP),
        knowledge_base.separator.clone(),
    )
}

// ── Trait 实现 ────────────────────────────────────────────────────────────

#[async_trait]
impl KnowledgeVectorProvider for HostKnowledgeVector {
    /// 语义检索：整条链交给宿主的 `search_knowledge_with_doc_filter`（过滤 / 排序 / 截断都在
    /// 检索层），本层只做入参闸与视图裁剪。
    async fn search(&self, request: VectorSearchRequest) -> Result<VectorSearchOutcome> {
        check_top_k(request.top_k, "search")?;
        check_min_similarity(request.min_similarity, "search")?;
        check_query_text(&request.query, "search")?;

        let hits = crate::indexing::search_knowledge_with_doc_filter(
            &self.db,
            &self.master_key,
            &self.vector_store,
            &request.knowledge_base_id,
            &request.query,
            request.top_k,
            doc_ids_filter(request.doc_ids.as_deref()),
            request.min_similarity,
        )
        .await?;
        Ok(search_outcome(hits))
    }

    /// 关键词检索：宿主 FTS5 口，零模型成本。`score` 与 [`Self::search`] 同向（越小越匹配）。
    ///
    /// 集合不存在时宿主 `fts_search` 直接返空表 ⇒ 本 op 的「查无」是空数组，不是错误；
    /// 想让插件区分「集合不存在」要先调 `get_collection_info` 探一次（契约那句「插件据此先探再写」
    /// 说的就是这个顺序）。
    async fn keyword_search(&self, request: KeywordSearchRequest) -> Result<VectorSearchOutcome> {
        check_top_k(request.top_k, "keyword_search")?;
        check_query_text(&request.query, "keyword_search")?;

        let hits = self
            .vector_store
            .fts_search(
                &physical_collection_id(&request.collection_id),
                &request.query,
                request.top_k,
            )
            .await?;
        Ok(search_outcome(hits))
    }

    /// 文本入库（**写**；权限闸在 worker 分派层，本层不重复校验）。
    ///
    /// 闸的顺序是有意义的：**先**做完全部纯校验再碰任何 IO —— 越界正文 / 空白集合名不该换来
    /// 一次 embedding 计费。
    async fn index_text(&self, request: VectorIndexRequest) -> Result<VectorIndexOutcome> {
        let VectorIndexRequest { collection_id, document_id, content } = request;
        check_content_budget(&content)?;

        // §56.2-2：先读维度判集合在不在，**禁止** ensure_collection。
        let physical = physical_collection_id(&collection_id);
        if self.vector_store.get_collection_dimensions(&physical).await?.is_none() {
            return Err(absent_collection_error(&collection_id));
        }

        // 文档行必须已存在且属于这个集合（见模块头第 3 条主权边界）。
        let document = axagent_dao::repo::knowledge::get_document(&self.db, &document_id)
            .await
            .map_err(absent_row_as_invalid_args)?;
        if document.knowledge_base_id != collection_id {
            return Err(AxAgentError::Validation(format!(
                "knowledge.vector::index_text 的 document_id={document_id} 属于集合 {}，\
                 不属于请求里的 {collection_id}（跨集合写会造出库里对不上号的块）",
                document.knowledge_base_id
            )));
        }

        // embedding 模型与维度都取宿主既有配置（不收 provider 名 / 维度入参）。
        let knowledge_base =
            axagent_dao::repo::knowledge::get_knowledge_base(&self.db, &collection_id)
                .await
                .map_err(absent_row_as_invalid_args)?;
        let embedding_provider =
            KnowledgeRAG.resolve_embedding_provider(&self.db, &collection_id).await?;
        let dimensions = collection_dimensions(&knowledge_base);
        let (chunk_size, overlap, separator) = chunk_params(&knowledge_base);

        let chunks = rag::prepare_chunks(
            &document_id,
            &ChunkStrategy::FromText { text: content, chunk_size, overlap, separator },
        )?;
        // 「正文切不出块」是合法读数（契约 `VectorIndexOutcome::chunk_count` 明写 `0` 不是错误），
        // 但它**不该**走一次 embedding 与建表写库 ⇒ 在这里收口。
        let chunk_count = chunks.len();
        if chunks.is_empty() {
            return Ok(VectorIndexOutcome {
                chunk_count: chunk_count as u32,
                document_id,
                collection_id,
            });
        }

        let chunk_texts: Vec<String> = chunks.iter().map(|(_, text, _)| text.clone()).collect();
        let embed_response = crate::indexing::generate_embeddings(
            &self.db,
            &self.master_key,
            &self.provider_registry,
            &embedding_provider,
            chunk_texts,
            dimensions,
        )
        .await?;

        rag::index(
            &self.vector_store,
            KnowledgeRAG.collection_prefix(),
            &collection_id,
            &document_id,
            "",
            embed_response.embeddings,
            chunks,
        )
        .await?;

        Ok(VectorIndexOutcome { chunk_count: chunk_count as u32, document_id, collection_id })
    }

    /// 集合信息（op 名 `get_collection_info`，方法名 [`KnowledgeVectorProvider::collection_info`]）。
    ///
    /// 集合不存在 ⇒ `Ok` + `exists = false` + `dimensions = null`，**不是错误**（契约「查无即
    /// null」口径，插件据此先探再写）。
    ///
    /// ⚠ 宿主有**两把尺子**，本层不许把它们合成一把：
    /// - `exists` 取 [`VectorStore::collection_exists`]（注册表行 / vec0 表 / `_meta` 表任一存在）；
    /// - `dimensions` 取 `get_collection_dimensions`（注册表或 pragma 能读出维度时才有值）。
    ///
    /// 此前按「有维度即存在」判，会把「集合存在但维度未知」（注册表行缺失且 pragma 解析失败，
    /// 或 `clear_embeddings` 在维度未知时只改名旧 vec0 表）误报成 `exists = false` —— 这正是
    /// 仓库 #122 的缺口。契约形状不变：`exists` / `dimensions` 本来就是两个字段。
    async fn collection_info(&self, collection_id: &str) -> Result<CollectionInfoView> {
        let physical = physical_collection_id(collection_id);
        let dimensions = self.vector_store.get_collection_dimensions(&physical).await?;
        let exists = self.vector_store.collection_exists(&physical).await?;
        Ok(CollectionInfoView {
            collection_id: collection_id.to_string(),
            // 契约保持 `u32`（PLAN §58.3-2：不收宽），这一次 `as u32` 是预期转换不是缺陷。
            dimensions: dimensions.map(|dims| dims as u32),
            exists,
        })
    }
}

// ── 测试 ──────────────────────────────────────────────────────────────────
//
// 内联测试 mod 追加在文件末尾（`items_after_test_module` 只在 clippy 下暴露，是踩过的那型）。
//
// 覆盖边界如实登记：`search` 的检索链与 `index_text` 的 embedding 链要真 provider 凭证与
// sqlite-vec 扩展，单测里没有 ⇒ 那两段属**未覆盖项**（交回报告 §③）。能被纯函数钉住的
// ——数字闸、`score` 方向、视图裁剪、`Validation` 变体、集合名前缀 —— 这里逐条钉死。

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_harness::knowledge_vector::KNOWLEDGE_SEARCH_TOP_K_MAX;
    use serde_json::{Value, json};

    /// 一条宿主形状的命中：`content` 是整块原文，`score` 是距离（越小越相似）。
    fn host_hit(id: &str, score: f32, content: &str) -> VectorSearchResult {
        VectorSearchResult {
            id: id.to_string(),
            document_id: "doc-1".into(),
            chunk_index: 0,
            content: content.to_string(),
            score,
            has_embedding: true,
        }
    }

    fn long_cjk(chars: usize) -> String {
        "贵".repeat(chars)
    }

    #[test]
    fn top_k_gate_uses_the_contract_range_and_refuses_clamping() {
        // 边界内放行；越界 ⇒ Err 而不是被夹到边界（夹取会让插件以为设成了边界值）。
        assert!(check_top_k(KNOWLEDGE_TOP_K_RANGE.0, "search").is_ok());
        // 单源证明：区间上界与单值上限驱动的是同一个数字 —— 若是本文件另抄的一份，
        // 这里就会出现「一个放行一个拒收」的分叉。
        assert!(
            check_top_k(KNOWLEDGE_SEARCH_TOP_K_MAX, "search").is_ok(),
            "单值上限那条数不该被区间拒掉"
        );
        for bad in [0, KNOWLEDGE_SEARCH_TOP_K_MAX + 1, usize::MAX] {
            let err = check_top_k(bad, "search").expect_err("{bad} 应被拒收，不该静默夹取");
            assert!(
                matches!(err, AxAgentError::Validation(_)),
                "越界应落成 Validation（分派层按它映射 SEAM_CALL_INVALID_ARGS）：{err:?}"
            );
        }
    }

    #[test]
    fn min_similarity_rejects_nan_before_any_comparison() {
        // 量纲数字住在契约层，provider 只引用它；本条把它钉在测试里 —— 契约改数即红
        // （先例 `monitor_seam_service.rs` 的 `aggregator_ranges_are_the_harness_numbers`）。
        assert_eq!(KNOWLEDGE_SIMILARITY_RANGE, (0.0f32, 1.0f32));
        // NaN 与 `0.0..=1.0`：`contains` 对 NaN 恒 false 是 fail-closed，但 `is_nan()` 这一支
        // 必须存在 —— 任何 `v <= 1.0` 形式的改写都会把 NaN 放过去（fail-open）。
        assert!(matches!(
            check_min_similarity(Some(f32::NAN), "search"),
            Err(AxAgentError::Validation(_))
        ));
        for bad in [-0.5f32, 1.5, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(
                matches!(
                    check_min_similarity(Some(bad), "search"),
                    Err(AxAgentError::Validation(_))
                ),
                "{bad} 越出宿主量纲 [0, 1]，应拒收"
            );
        }
        assert!(check_min_similarity(None, "search").is_ok(), "None = 不过滤，照传");
        for ok in [0.0f32, 0.35, 1.0] {
            assert!(check_min_similarity(Some(ok), "search").is_ok(), "{ok} 应放行");
        }
    }

    #[test]
    fn content_budget_is_counted_in_bytes_and_refuses_truncation() {
        assert!(check_content_budget(&"a".repeat(KNOWLEDGE_CONTENT_MAX_BYTES)).is_ok());
        let err = check_content_budget(&"a".repeat(KNOWLEDGE_CONTENT_MAX_BYTES + 1))
            .expect_err("超一个字节也应拒收");
        assert!(matches!(err, AxAgentError::Validation(_)), "应为 Validation：{err:?}");
        // 中文：字符数在预算内**不等于**字节数在预算内（embedding 按 token 计费）。
        let cjk_chars = KNOWLEDGE_CONTENT_MAX_BYTES; // 3 字节/字 ⇒ 远超预算
        assert!(long_cjk(cjk_chars).chars().count() <= KNOWLEDGE_CONTENT_MAX_BYTES);
        assert!(
            matches!(check_content_budget(&long_cjk(cjk_chars)), Err(AxAgentError::Validation(_))),
            "按字符判会放行三倍预算的正文"
        );
    }

    #[test]
    fn empty_doc_ids_means_unrestricted() {
        let ids: Vec<String> = vec![];
        assert!(doc_ids_filter(Some(&ids)).is_none(), "空数组 = 不限定，不是恒空 IN ()");
        let one = vec!["doc-1".to_string()];
        assert_eq!(doc_ids_filter(Some(&one)).map(|v| v.len()), Some(1));
        assert!(doc_ids_filter(None).is_none());
    }

    #[test]
    fn snippet_is_cut_by_chars_never_by_bytes() {
        let cap = KNOWLEDGE_SNIPPET_MAX_CHARS;
        let text = long_cjk(cap + 50);
        let cut = snippet(&text);
        assert_eq!(cut.chars().count(), cap, "预览长度必须等于契约字符上限");
        assert!(
            cut.len() > cap,
            "中文按 char 截 ⇒ 字节数必然超过 cap；按 `&s[..n]` 切会 panic 在非边界字节"
        );
        // 恰在上限内不改动；恰等于上限也不追加省略号（契约没承诺省略号形态）。
        let exact = long_cjk(cap);
        assert_eq!(snippet(&exact), exact);
        assert_eq!(snippet("筹码集中度"), "筹码集中度");
    }

    #[test]
    fn chunk_view_keeps_host_order_and_distance_direction() {
        // 宿主链返回的是**按距离升序**的命中（`rag.rs` 的 sort + LIMIT）。本层不许重排，
        // 否则「越近的排越前」这条插件侧赖以断言的口径就翻。
        let near_score = 0.12f32;
        let far_score = 0.87f32;
        let near = host_hit("chunk-near", near_score, &long_cjk(KNOWLEDGE_SNIPPET_MAX_CHARS + 9));
        let far = host_hit("chunk-far", far_score, "尾部结论");
        let outcome = search_outcome(vec![near, far]);
        assert_eq!(outcome.results.len(), 2);
        assert_eq!(outcome.results[0].id, "chunk-near", "距离小的命中必须仍在前");
        assert!(
            outcome.results[0].score < outcome.results[1].score,
            "score 逐字透传后仍应升序（翻方向即本测试红）"
        );
        // 分值**逐位**不变（`to_bits` 比，避开浮点 `==` 那一族坑）。
        assert_eq!(
            outcome.results[1].score.to_bits(),
            far_score.to_bits(),
            "provider 不许改写距离分"
        );
        assert!(
            outcome.results[0].snippet.chars().count() <= KNOWLEDGE_SNIPPET_MAX_CHARS,
            "整块原文不许原样进视图"
        );
    }

    #[test]
    fn view_wire_shape_has_no_full_content_field() {
        let outcome = search_outcome(vec![host_hit("c1", 0.2, &long_cjk(1_000))]);
        let val = serde_json::to_value(&outcome).expect("序列化应成功");
        let hits = val["results"].as_array().expect("results 应为数组");
        assert_eq!(hits.len(), 1);
        let mut keys: Vec<String> =
            hits[0].as_object().expect("视图应为对象").keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["chunkIndex", "documentId", "hasEmbedding", "id", "score", "snippet"]);
        assert!(hits[0].get("content").is_none(), "整块原文不得过线");
    }

    #[test]
    fn zero_hits_are_an_empty_array_not_an_error() {
        let outcome = search_outcome(vec![]);
        let val = serde_json::to_value(&outcome).expect("序列化应成功");
        assert_eq!(val["results"], json!([]), "查无结果 = 空数组");
    }

    #[test]
    fn absent_collection_error_variant_is_the_one_the_dispatcher_maps() {
        // §58.2-①/#112：只有 `Validation` 会被分派层映射成 `SEAM_CALL_INVALID_ARGS`。
        let err = absent_collection_error("kb-absent");
        assert!(
            matches!(err, AxAgentError::Validation(_)),
            "抛 Provider/NotFound 就不命中承诺：{err:?}"
        );
        let not_found =
            absent_row_as_invalid_args(AxAgentError::NotFound("KnowledgeBase x".into()));
        assert!(matches!(not_found, AxAgentError::Validation(_)));
        let db_err = AxAgentError::Provider("真故障".into());
        assert!(matches!(absent_row_as_invalid_args(db_err), AxAgentError::Provider(_)));
    }

    #[test]
    fn physical_collection_id_carries_the_host_prefix() {
        let prefix = KnowledgeRAG.collection_prefix();
        assert_eq!(physical_collection_id("abc"), format!("{prefix}_abc"));
        assert_ne!(physical_collection_id("abc"), "abc", "漏掉前缀就是查错表");
    }

    #[test]
    fn chunk_params_fall_back_to_the_hosters_defaults() {
        let kb = axagent_harness::types::KnowledgeBase {
            id: "kb-1".into(),
            name: "测试库".into(),
            description: None,
            embedding_provider: Some("openai::text-embedding-3-small".into()),
            enabled: true,
            icon_type: None,
            icon_value: None,
            sort_order: 0,
            embedding_dimensions: Some(1_536),
            retrieval_threshold: None,
            retrieval_top_k: None,
            chunk_size: None,
            chunk_overlap: None,
            separator: None,
            kind: axagent_harness::KbKind::Indexed,
            vault_path: None,
        };
        let (size, overlap, separator) = chunk_params(&kb);
        assert_eq!(size, axagent_search::text_chunker::DEFAULT_CHUNK_SIZE, "缺省该落宿主常量");
        assert_eq!(overlap, axagent_search::text_chunker::DEFAULT_OVERLAP);
        assert_eq!(separator, None);
        assert_eq!(collection_dimensions(&kb), Some(1_536));
    }

    /// 装配点句柄：真库走 `axagent_dao::db::create_test_pool()`（与生产同一个建表链），
    /// 集合表一张都没建 ⇒ 用来钉「查无」那三格而不是 Mock 一个假的 provider 行为。
    async fn provider_on_empty_db() -> HostKnowledgeVector {
        let handle = axagent_dao::db::create_test_pool().await.expect("测试：建连接池应成功");
        let registry: Arc<dyn axagent_harness::registry::ProviderRegistry> =
            Arc::new(axagent_providers::registry::ProviderRegistry::create_default());
        let conn = handle.conn;
        let vector_store = Arc::new(VectorStore::new(conn.clone()));
        HostKnowledgeVector::new(Arc::new(conn), vector_store, [0u8; 32], registry)
    }

    #[tokio::test]
    async fn collection_info_reports_absent_collection_as_ok_not_error() {
        let provider = provider_on_empty_db().await;
        let view = provider
            .collection_info("kb-does-not-exist")
            .await
            .expect("集合不存在是合法读数，不该 Err");
        assert!(!view.exists);
        assert_eq!(view.dimensions, None);
        let val: Value = serde_json::to_value(&view).expect("序列化应成功");
        assert_eq!(val["dimensions"], json!(null), "帧里是 null（契约「查无即 null」）");
    }

    #[tokio::test]
    async fn keyword_search_on_absent_collection_is_empty_array() {
        let provider = provider_on_empty_db().await;
        let outcome = provider
            .keyword_search(KeywordSearchRequest {
                collection_id: "kb-does-not-exist".into(),
                query: "筹码集中度".into(),
                top_k: KNOWLEDGE_SEARCH_TOP_K_MAX,
            })
            .await
            .expect("零命中不该 Err");
        assert!(outcome.results.is_empty(), "查无 = 空数组，不是错误");
    }

    #[tokio::test]
    async fn index_text_rejects_before_touching_any_io() {
        let provider = provider_on_empty_db().await;
        // 正文超限：Err 里点到「字节」，且没有走到 get_collection_dimensions（空库里也一样红）。
        let over = long_cjk(KNOWLEDGE_CONTENT_MAX_BYTES);
        let err = provider
            .index_text(VectorIndexRequest {
                collection_id: "kb-1".into(),
                document_id: "doc-1".into(),
                content: over,
            })
            .await
            .expect_err("超预算正文应被拒收");
        assert!(matches!(err, AxAgentError::Validation(_)), "应为 Validation：{err:?}");

        // 集合不存在：必须是 Validation（分派层映射 SEAM_CALL_INVALID_ARGS 的那一格）。
        let err = provider
            .index_text(VectorIndexRequest {
                collection_id: "kb-absent".into(),
                document_id: "doc-1".into(),
                content: "正常长度正文".into(),
            })
            .await
            .expect_err("集合不存在不该隐式建集合");
        assert!(matches!(err, AxAgentError::Validation(_)), "应为 Validation：{err:?}");
    }

    #[tokio::test]
    async fn search_gates_run_before_the_embedding_call() {
        let provider = provider_on_empty_db().await;
        // top_k = 0 与空查询都该在算 embedding 之前就被拒（这两次 Err 各值一次模型调用）。
        let err = provider
            .search(VectorSearchRequest {
                knowledge_base_id: "kb-1".into(),
                query: "筹码".into(),
                top_k: 0,
                min_similarity: None,
                doc_ids: None,
            })
            .await
            .expect_err("top_k=0 应拒收");
        assert!(matches!(err, AxAgentError::Validation(_)));
        let err = provider
            .search(VectorSearchRequest {
                knowledge_base_id: "kb-1".into(),
                query: "   ".into(),
                top_k: 1,
                min_similarity: Some(f32::NAN),
                doc_ids: Some(vec![]),
            })
            .await
            .expect_err("NaN 阈值应被显式拒掉");
        assert!(matches!(err, AxAgentError::Validation(_)));
    }

    /// sqlite-vec 是进程级自动扩展，且必须在打开连接之前注册（生产由 `init/database.rs` 做）。
    fn register_sqlite_vec_once() {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(axagent_search::vector_store::register_sqlite_vec_extension);
    }

    /// #122 的线级判据：物理集合在 ⇒ `exists=true`，且 `dimensions` 独立读数。
    #[tokio::test]
    async fn collection_info_reports_present_collection_with_dimensions() {
        register_sqlite_vec_once();
        let provider = provider_on_empty_db().await;
        let physical = physical_collection_id("kb-present");
        provider.vector_store.ensure_collection(&physical, 2).await.expect("建集合应成功");

        let view = provider.collection_info("kb-present").await.expect("集合在，不该 Err");
        assert!(view.exists, "物理表在 ⇒ exists 必须为 true");
        assert_eq!(view.dimensions, Some(2));

        // 反向：查无仍是 false + null —— 同一条读口不能把「不存在」也说成存在。
        let absent = provider.collection_info("kb-absent").await.expect("查无不是错误");
        assert!(!absent.exists);
        assert_eq!(absent.dimensions, None);
    }
}
