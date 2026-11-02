// SPDX-License-Identifier: AGPL-3.0-only

//! 知识库向量接缝（`knowledge.vector`）— DTO + Trait。
//!
//! 宿主把 `axagent-search` 的 `VectorStore` 与 `src-tauri/src/indexing.rs` 的入库链经本接缝
//! 暴露给插件侧；插件经 `call_seam("knowledge.vector", op, args)` 做语义检索 / 关键词检索 /
//! 文本入库 / 集合信息查询。
//!
//! ## 接缝名：曾用名与交付名
//!
//! 交付名 `knowledge.vector`（裁定见 `PLAN-shell-plus-plugins.md` §56.1-Q1：§47.3 顺序表用的
//! 就是它，且首批 op 面不止 RAG 存储）。曾用名 `rag.vector.store` 是任务 #40 留下的**预留位**
//! 名（见同目录 [`crate::vector_store`] 模块头），此后只出现在历史章节与登记里。**判据只有
//! 一处**：`capability_registry.rs` 里 `ServiceDefinition::knowledge_vector()` 的那个字符串
//! 字面量 ⇒ 新代码里两名并存是最坏解。
//!
//! ## 边界总则：只传文本，embedding 归宿主
//!
//! 三条理由全部有盘上出处：① embedding 要 `master_key` 解密 provider 配置 + `ProviderRegistry`
//! （`src-tauri/src/indexing.rs:618` 的 `build_embed_context`），插件进程两样都没有；② 向量是
//! 大对象（1536 维 ≈ 6 KB/条，[`crate::vector_store`] 模块头的 K4 预算），过帧既贵又无意义；
//! ③ 维度与 provider 不匹配属宿主配置问题，必须由宿主在同一处报错
//! （`KNOWLEDGE_NO_EMBEDDING_PROVIDER`）。
//! ⇒ [`VectorIndexRequest`] **只有三个字段**，`embedding_provider` 与 `dimensions` 都不是入参
//! （PLAN §56.2-1：让外部传 provider 名等于把「任意触发模型计费」的口交给插件）。
//!
//! ## 与任务 #40 三 trait 的关系
//!
//! [`crate::vector_store::VectorCollectionAdmin`] / `VectorWriter` / `VectorReader` 是**存储层**
//! 的方言无关抽象：形参全是 `&str` / `&[f32]` / `Option<&[String]>`、返回含元组，且 `search`
//! 要求调用方自备 `query_embedding` ⇒ 不能原样过线。PLAN §56.1-Q3 同时裁定**不为**
//! `list_collections` 扩它们的 trait 面（三 trait 共 25 方法、宿主 `VectorStore` 的 28 个 pub
//! 方法**零列举能力**，扩面属改冻结契约）⇒ 首批以 `get_collection_info` op 替代。
//! 本模块是**接缝侧的 provider 契约**，宿主 provider 内部再经那三 trait（或直接 `VectorStore`）
//! 落地；两者各自权威，勿混用（同目录 `rag_provider::VectorStoreProvider` 是第三套抽象）。
//!
//! ## 主权边界：本接缝不开的面
//!
//! - **无任何删除 op**（PLAN §56.1-Q2）：`delete_document_embeddings` 只清向量、**不删文档行**，
//!   插件拿到它只能造成半截删除（库里留孤儿行）；宿主侧删文档走
//!   `src-tauri/src/commands/knowledge.rs:982` 的「先清旧向量再删行」同事务路径 ⇒ 删除权留在
//!   宿主命令面。这条与下面的「不许隐式建集合」是一对配套前提，必须同时成立。
//! - `clear_embeddings` / `delete_collection`：一次调用清空整个命名空间，且造完没人能清。
//! - `reinsert_embeddings` / `upsert_document_embeddings` / `update_chunk_embedding`：入参要求
//!   调用方持宿主 `rowid` 与向量本体，违反 K4 的「宿主侧 chunk 引用不出线」。
//! - `ensure_vector_index` / `ensure_keyword_index` / `rebuild_fts_index`：索引生命周期，且
//!   `rebuild_fts_index` 在宿主是 fire-and-forget（不返回 `Result`）⇒ 放进分派层的 `block_on`
//!   会挂住 worker 读线程。
//! - `insert_metadata_only_chunk` / `update_chunk_content`：与宿主文档命令的同事务耦合。
//!
//! ## 写面权限
//!
//! [`KnowledgeVectorProvider::index_text`] 是本接缝唯一的写 op，要求插件已声明 `write` 权限 ——
//! 该检查在 worker 分派层完成（拒绝码 `PLUGIN_PERMISSION_DENIED`），本 trait 不重复校验
//! （先例 `jobs_store.rs` 的 `JobStoreProvider::add_job`）。
//!
//! ## 错误语义
//!
//! 形状 / 准入 / 无 provider 类错误走协议码（`SEAM_CALL_INVALID_ARGS`、`SEAM_CALL_NOT_DECLARED`、
//! `SEAM_PROVIDER_UNAVAILABLE`、`PLUGIN_PERMISSION_DENIED`）；provider 内部任何 `Err` 由分派层
//! 兜底成 `PLUGIN_CALL_FAILED` + message。业务码（`KNOWLEDGE_*` / `VECTOR_*`）若要透出，放
//! payload 的 `code` 字段，**不改协议、不新增码**（PLAN §56.1-Q5：零新增码 ⇒ 零 11 语言译键
//! 负担）。**「查无结果」不是错误**：[`VectorSearchOutcome::results`] 返空数组。
//!
//! ## `as_of`
//!
//! 本接缝的 op 读写的是知识库自身的持久内容，不取行情 ⇒ **一律不收 `as_of`**。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::core_error::{AxAgentError, Result};

// ── 硬上限（契约级数字，单源）────────────────────────────────────────────
//
// ⚠ 宿主 provider 与 plugins 分派层**只准引用这里**，不得另建一套（先例
// `stock_monitor.rs:342-348` 的 `AGGREGATOR_*` 一族）。这些数字是插件侧乱调的成本防线，
// 不随宿主配置漂移；改数只改这一处。

/// 单次 `index_text` 可提交的正文上限（字节，UTF-8 计）。
///
/// 理由：`index_text` 会在宿主侧触发 chunk 切分 + embedding 调用，而 embedding 按 token 计费。
/// 没有速率闸（协议层只有 `write` 一档权限，无粒度 —— PLAN §56.2-3 明示「速率闸首批不做」，
/// 已登记的未修风险就是「一个乱调的插件能把宿主 embedding 花费推高」），上限是**当前唯一的
/// 防线**。超限 ⇒ 宿主 provider 返回 `Err`（分派层落成 `SEAM_CALL_INVALID_ARGS`），不静默截断
/// —— 静默截断会让插件以为整篇都进了库。
pub const KNOWLEDGE_CONTENT_MAX_BYTES: usize = 64 * 1024;

/// `search` / `keyword_search` 的 `top_k` 上界。
///
/// 检索是纯读，但每条结果都带 `snippet` 正文 ⇒ `top_k` 直接决定帧体积与宿主排序开销，
/// 需要一个契约级上限（不借 `MAX_FRAME_LEN`，那 64 MiB 的天花板不是本 op 的预算口径）。
pub const KNOWLEDGE_SEARCH_TOP_K_MAX: usize = 50;

/// [`KnowledgeChunkView::snippet`] 的字符上限。
///
/// 与 `trajectory_store::TRAJECTORY_SUMMARY_SNIPPET_MAX_CHARS` 同值同理由：帧体积 =
/// `top_k × snippet 长度`，两个因子必须各有一处权威，否则宿主 provider
/// 会随手取 500 或 2000，而插件侧的预算口径无从核对（PLAN §55.5-N4 的「两处各夹一份必然漂移」
/// 同一条纪律）。200 字足以判定「命中的是不是这一段」，又短到不能被当作原文的替代品 ——
/// 视图本身不带 `content`，要全文只能回宿主命令面。
pub const KNOWLEDGE_SNIPPET_MAX_CHARS: usize = 200;

/// `top_k` 的合法闭区间（下界 1：`0` 没有「取最相似的 0 条」这种合法语义，
/// 上界与 [`KNOWLEDGE_SEARCH_TOP_K_MAX`] 同值）。
pub const KNOWLEDGE_TOP_K_RANGE: (usize, usize) = (1, 50);

// 区间与单值上限必须同源，否则 provider 与分派层两侧的 clamp 会各说各话。
const _: () = assert!(KNOWLEDGE_TOP_K_RANGE.0 <= KNOWLEDGE_TOP_K_RANGE.1);
const _: () = assert!(KNOWLEDGE_TOP_K_RANGE.1 == KNOWLEDGE_SEARCH_TOP_K_MAX);
// snippet 上限为 0 等于悄悄关掉读面的展示价值（命中列表只剩 id），故编译期钉住非零。
const _: () = assert!(KNOWLEDGE_SNIPPET_MAX_CHARS > 0);

// ── DTO（跨边界视图；宿主内部类型不过线）────────────────────────────────

/// `search` op 的入参（语义检索，文本进）。
///
/// `query` 是自然语言文本，**不是**向量：provider 负责在宿主侧算出 `query` 的 embedding。
/// `top_k` 由宿主 provider 钳位到 [`KNOWLEDGE_TOP_K_RANGE`]。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorSearchRequest {
    /// 知识库 / 集合 id（宿主侧 `knowledge_base_id`）。
    pub knowledge_base_id: String,
    /// 查询文本。
    pub query: String,
    /// 返回条数上界，合法区间 [`KNOWLEDGE_TOP_K_RANGE`]。
    pub top_k: usize,
    /// 相似度下限；`None` 表示不过滤。口径同宿主 `indexing.rs:1244` 的
    /// 「过滤 → 排序 → 截断」，**不是**「先截断再过滤」。
    #[serde(default)]
    pub min_similarity: Option<f32>,
    /// 限定在这些文档 id 内检索；`None` / 空数组表示不限定。
    #[serde(default)]
    pub doc_ids: Option<Vec<String>>,
}

/// `keyword_search` op 的入参（关键词检索，零模型成本）。
///
/// 走宿主的全文索引（SQLite FTS5 那一侧，见 `vector_store.rs` 的 `fts_search`），**不**算
/// embedding ⇒ 插件想省钱就显式选这个 op，而不是让宿主猜。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeywordSearchRequest {
    pub collection_id: String,
    pub query: String,
    /// 合法区间 [`KNOWLEDGE_TOP_K_RANGE`]。
    pub top_k: usize,
}

/// 检索命中块 —— 刻意**不携带 `content` 整块原文**。
///
/// 与宿主侧 `axagent_harness::VectorSearchResult` 的差别只有一处：`content`（整块正文，
/// 一条 1536 维集合的 top_k 结果可达数十 KB 且含用户文档正文）换成 [`Self::snippet`]（截断
/// 预览）。裁剪先例：`jobs_store.rs:15-16`「`last_result.output`：单次执行输出可能很大，
/// 不过帧」。要整块原文只能回宿主命令面按既有权限路径读 ⇒ 视图**不提供**取全文的字段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeChunkView {
    /// chunk id（宿主侧稳定标识）。
    pub id: String,
    pub document_id: String,
    /// 该块在文档内的序号（与宿主 `VectorSearchResult.chunk_index` 同为 `i32`）。
    pub chunk_index: i32,
    /// 正文预览（截断文本），长度上限 = [`KNOWLEDGE_SNIPPET_MAX_CHARS`]（宿主 provider 必须用它截，
    /// 不许随手取 500 / 2000）。**只作展示用**，不得当作原文参与再计算。
    pub snippet: String,
    /// 距离分。语义检索为 L2 距离（**越小越相似**，与宿主一致，别在接缝侧翻方向）；
    /// 关键词检索的口径同 `fts_search`。
    pub score: f32,
    /// 该块是否在向量表里有 embedding（`false` = 只有元数据行，检索时靠全文索引命中）。
    pub has_embedding: bool,
}

/// `search` / `keyword_search` 的返回。
///
/// 命中 0 条 ⇒ `results` 为空数组，**不是错误**（协议层无通用 NOT_FOUND，先例 `jobs.store`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorSearchOutcome {
    pub results: Vec<KnowledgeChunkView>,
}

/// `index_text` op 的入参（**写**，要求 `write` 权限）。
///
/// 只有三个字段，这是 PLAN §56.2-1 的裁定形状：`embedding_provider` 与 `dimensions` **不是**
/// 入参 —— embedding 模型与维度由宿主按配置决定（宿主主权与计费主权）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorIndexRequest {
    /// 目标集合 id。集合**必须已存在**，见 [`KnowledgeVectorProvider::index_text`] 的文档。
    pub collection_id: String,
    /// 宿主侧文档 id（同一 id 重复入库的语义由宿主 provider 定义，接缝不承诺幂等）。
    pub document_id: String,
    /// 待入库正文，≤ [`KNOWLEDGE_CONTENT_MAX_BYTES`]。
    pub content: String,
}

/// `index_text` 的结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorIndexOutcome {
    /// 宿主实际切出并写入的块数（`0` 是合法值：正文切不出块，**不是错误**）。
    pub chunk_count: u32,
    pub document_id: String,
    pub collection_id: String,
}

/// `get_collection_info` op 的返回。
///
/// 落点是宿主已有的 `get_collection_dimensions`（`vector_store.rs:84`），故本接缝**不需要**
/// 「列举集合」的能力（PLAN §56.1-Q3）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionInfoView {
    pub collection_id: String,
    /// 向量维度；`None` = 集合存在但尚未绑定 embedding 模型。
    pub dimensions: Option<u32>,
    /// 集合是否存在。不存在 ⇒ `exists = false` + `dimensions = None`，**不是错误**
    /// （照 `jobs.store get` 的「查无即 null」口径；插件据此先探再写）。
    pub exists: bool,
}

// ── Trait ───────────────────────────────────────────────────────────────

/// 知识库向量提供者（宿主独占 `VectorStore` + embedding 链，插件只经本接缝调用）。
///
/// 默认实现统一返回「该提供者不支持此数据面」，便于外部实现只覆盖关心的 op。
///
/// 写方法 [`Self::index_text`] 要求插件已声明 `write` 权限 —— 该检查在 worker 分派层完成
/// （`PLUGIN_PERMISSION_DENIED`），本 trait 不重复校验。
#[async_trait]
pub trait KnowledgeVectorProvider: Send + Sync {
    /// 语义检索（`search` op，读面）。
    ///
    /// 边界是「文本进」：provider 在宿主侧算 `query` 的 embedding，再走宿主检索链
    /// （`indexing.rs:1244` 的「过滤 → 排序 → 截断」口径）。宿主没配 embedding provider ⇒
    /// `Err`，**不得**静默降级成关键词检索（那是另一个 op，由插件显式选）。
    async fn search(&self, _request: VectorSearchRequest) -> Result<VectorSearchOutcome> {
        Err(AxAgentError::Provider(
            "KnowledgeVectorProvider::search 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 关键词检索（`keyword_search` op，读面）。
    ///
    /// 宿主唯一的原生收文本的检索口（FTS5），零模型成本；`score` 语义与 [`Self::search`]
    /// 一致（越小越匹配），消费方不得按 op 分别假设方向。
    async fn keyword_search(&self, _request: KeywordSearchRequest) -> Result<VectorSearchOutcome> {
        Err(AxAgentError::Provider(
            "KnowledgeVectorProvider::keyword_search 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 文本入库（`index_text` op，**写**，要求 `write` 权限）。
    ///
    /// 边界只能是「插件给文本，宿主做 chunk + embedding + upsert」：插件算不出向量，也不该
    /// 决定用哪个 embedding 模型（入参三字段即此裁定，见 [`VectorIndexRequest`]）。
    ///
    /// ⚠ **不得隐式建集合**（PLAN §56.2-2）：实现方写前必须先读 `get_collection_dimensions`，
    /// 集合不存在 ⇒ `Err`（分派层映射 `SEAM_CALL_INVALID_ARGS`），**禁止**顺手
    /// `ensure_collection`。否则插件可无限造命名空间，而删除权已收归宿主（本模块「主权边界」）
    /// ⇒ 造完没人能清。
    ///
    /// 正文超 [`KNOWLEDGE_CONTENT_MAX_BYTES`] ⇒ `Err`（不截断，见该常量的理由）。
    async fn index_text(&self, _request: VectorIndexRequest) -> Result<VectorIndexOutcome> {
        Err(AxAgentError::Provider(
            "KnowledgeVectorProvider::index_text 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 集合信息（op 名 `get_collection_info` —— ⚠ **op 名 ≠ 方法名**，先例
    /// `trading.engine` 的 `t0_config` 方法对应 `get_t0_config` op；分派批按 op 名建分支）。
    ///
    /// 集合不存在 ⇒ `Ok` + `exists = false`，**不是错误**。
    async fn collection_info(&self, _collection_id: &str) -> Result<CollectionInfoView> {
        Err(AxAgentError::Provider(
            "KnowledgeVectorProvider::collection_info 未实现（该提供者不支持此数据面）".into(),
        ))
    }
}

// ── 测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    /// 把序列化出的 JSON 键集合钉死：本两条件接缝**没有**门禁覆盖，测试就是唯一的形态锁
    /// （改字段名必须同批改这里，否则插件侧静默读到 `undefined`）。
    fn keys_of(value: &Value) -> Vec<String> {
        let mut keys: Vec<String> =
            value.as_object().expect("DTO 必须序列化成对象").keys().cloned().collect();
        keys.sort();
        keys
    }

    fn expect_keys(value: &Value, expected: &[&str]) {
        let mut want: Vec<String> = expected.iter().map(|s| (*s).to_string()).collect();
        want.sort();
        assert_eq!(keys_of(value), want, "键集合漂移：{value:?}");
    }

    fn sample_chunk() -> KnowledgeChunkView {
        KnowledgeChunkView {
            id: "chunk-1".into(),
            document_id: "doc-1".into(),
            chunk_index: 3,
            snippet: "贵州茅台三季度营收……（截断预览）".into(),
            score: 0.21,
            has_embedding: true,
        }
    }

    #[test]
    fn search_request_camel_case_and_roundtrip() {
        let req = VectorSearchRequest {
            knowledge_base_id: "kb-1".into(),
            query: "筹码集中度".into(),
            top_k: 8,
            min_similarity: Some(0.35),
            doc_ids: Some(vec!["doc-1".into(), "doc-2".into()]),
        };
        let val = serde_json::to_value(&req).expect("序列化应成功");
        expect_keys(&val, &["knowledgeBaseId", "query", "topK", "minSimilarity", "docIds"]);
        assert_eq!(val["knowledgeBaseId"], json!("kb-1"));
        assert_eq!(val["topK"], json!(8));
        // ⚠ `min_similarity` 是 **f32**：序列化后按 f64 读出是 `0.3499999940395355`，
        // 与 `json!(0.35)`（f64 字面量）**逐位不等** ⇒ 不许写 `assert_eq!(val[...], json!(0.35))`
        // （本仓已记过「浮点判据不是纯等价改写」那一族坑，这里是它的 serde 侧同型）。
        // 键形态与非零性在这里钉，**数值保真**由下面的类型化往返断言负责。
        let min_sim = val["minSimilarity"]
            .as_f64()
            .expect("minSimilarity 应为数值");
        assert!(
            (min_sim - 0.35).abs() < 1e-6,
            "minSimilarity 序列化值应≈ 0.35，实测 {min_sim}"
        );
        assert_eq!(val["docIds"], json!(["doc-1", "doc-2"]));
        // 不得残留 snake_case 键
        assert!(val.get("knowledge_base_id").is_none());
        assert!(val.get("top_k").is_none());
        assert!(val.get("min_similarity").is_none());
        assert!(val.get("doc_ids").is_none());

        let back: VectorSearchRequest = serde_json::from_value(val).expect("往返应成立");
        assert_eq!(back.knowledge_base_id, "kb-1");
        assert_eq!(back.top_k, 8);
        assert_eq!(back.min_similarity, Some(0.35));
        assert_eq!(back.doc_ids.expect("应带 doc_ids").len(), 2);
    }

    #[test]
    fn search_request_optional_fields_may_be_omitted() {
        let req: VectorSearchRequest = serde_json::from_value(json!({
            "knowledgeBaseId": "kb-1",
            "query": "测试",
            "topK": 5,
        }))
        .expect("两个可选字段应能整体省略");
        assert!(req.min_similarity.is_none(), "缺省必须是 None（不过滤，而非阈值 0）");
        assert!(req.doc_ids.is_none(), "缺省必须是 None（不限定文档，而非空集合）");
    }

    #[test]
    fn keyword_search_request_camel_case_and_roundtrip() {
        let req = KeywordSearchRequest {
            collection_id: "kb-1".into(),
            query: "主力净流入".into(),
            top_k: 20,
        };
        let val = serde_json::to_value(&req).expect("序列化应成功");
        expect_keys(&val, &["collectionId", "query", "topK"]);
        assert_eq!(val["collectionId"], json!("kb-1"));
        assert_eq!(val["topK"], json!(20));
        assert!(val.get("collection_id").is_none());
        assert!(val.get("top_k").is_none());

        let back: KeywordSearchRequest = serde_json::from_value(val).expect("往返应成立");
        assert_eq!(back.collection_id, "kb-1");
        assert_eq!(back.top_k, 20);
    }

    #[test]
    fn chunk_view_has_no_full_content_field() {
        let view = sample_chunk();
        let val = serde_json::to_value(&view).expect("序列化应成功");
        // 视图刻意裁掉整块原文：只允许 snippet
        expect_keys(&val, &["id", "documentId", "chunkIndex", "snippet", "score", "hasEmbedding"]);
        assert_eq!(val["documentId"], json!("doc-1"));
        assert_eq!(val["chunkIndex"], json!(3));
        assert_eq!(val["hasEmbedding"], json!(true));
        assert!(val.get("content").is_none(), "content（整块原文）不得过线");
        assert!(val.get("embedding").is_none(), "向量本体不得过线");
        assert!(val.get("document_id").is_none());
        assert!(val.get("chunk_index").is_none());
        assert!(val.get("has_embedding").is_none());

        let back: KnowledgeChunkView = serde_json::from_value(val).expect("往返应成立");
        assert_eq!(back.id, "chunk-1");
        assert_eq!(back.chunk_index, 3);
        assert!(back.has_embedding);
    }

    #[test]
    fn search_outcome_empty_result_is_not_an_error() {
        let empty = VectorSearchOutcome { results: Vec::new() };
        let val = serde_json::to_value(&empty).expect("序列化应成功");
        expect_keys(&val, &["results"]);
        assert_eq!(val["results"], json!([]), "查无结果 = 空数组，不是错误");

        let full = VectorSearchOutcome { results: vec![sample_chunk()] };
        let val = serde_json::to_value(&full).expect("序列化应成功");
        let hits = val["results"].as_array().expect("results 应为数组");
        assert_eq!(hits.len(), 1);
        assert!(
            hits[0]["snippet"].as_str().expect("snippet 应为字符串").contains("贵州茅台"),
            "snippet 应是正文预览：{hits:?}"
        );

        let back: VectorSearchOutcome = serde_json::from_value(val).expect("往返应成立");
        assert_eq!(back.results.len(), 1);
        assert_eq!(back.results[0].document_id, "doc-1");
    }

    #[test]
    fn index_request_is_exactly_three_fields() {
        // PLAN §56.2-1：embedding_provider 与 dimensions 被明确删掉，不是入参。
        // 这条断言是「宿主主权」的形态锁 —— 谁把它们加回来，此测试即红。
        let req = VectorIndexRequest {
            collection_id: "kb-1".into(),
            document_id: "doc-1".into(),
            content: "正文……".into(),
        };
        let val = serde_json::to_value(&req).expect("序列化应成功");
        expect_keys(&val, &["collectionId", "documentId", "content"]);
        assert!(val.get("embeddingProvider").is_none(), "provider 由宿主决定，不得作入参");
        assert!(val.get("dimensions").is_none(), "维度由宿主决定，不得作入参");
        assert!(val.get("embedding_provider").is_none());

        let parsed: VectorIndexRequest = serde_json::from_value(json!({
            "collectionId": "kb-1",
            "documentId": "doc-1",
            "content": "正文",
        }))
        .expect("三字段应能反序列化");
        assert_eq!(parsed.collection_id, "kb-1");
        assert_eq!(parsed.content, "正文");
    }

    #[test]
    fn index_outcome_camel_case_and_roundtrip() {
        let outcome = VectorIndexOutcome {
            chunk_count: 7,
            document_id: "doc-1".into(),
            collection_id: "kb-1".into(),
        };
        let val = serde_json::to_value(&outcome).expect("序列化应成功");
        expect_keys(&val, &["chunkCount", "documentId", "collectionId"]);
        assert_eq!(val["chunkCount"], json!(7));
        assert!(val.get("chunk_count").is_none());

        let back: VectorIndexOutcome = serde_json::from_value(val).expect("往返应成立");
        assert_eq!(back.chunk_count, 7);
        assert_eq!(back.collection_id, "kb-1");
    }

    #[test]
    fn collection_info_view_camel_case_and_missing_collection_shape() {
        let known = CollectionInfoView {
            collection_id: "kb-1".into(),
            dimensions: Some(1536),
            exists: true,
        };
        let val = serde_json::to_value(&known).expect("序列化应成功");
        expect_keys(&val, &["collectionId", "dimensions", "exists"]);
        assert_eq!(val["dimensions"], json!(1536));
        assert_eq!(val["exists"], json!(true));
        assert!(val.get("collection_id").is_none());

        // 集合不存在：exists=false + dimensions=null，不是错误
        let absent =
            CollectionInfoView { collection_id: "kb-x".into(), dimensions: None, exists: false };
        let val = serde_json::to_value(&absent).expect("序列化应成功");
        assert_eq!(val["exists"], json!(false));
        assert_eq!(val["dimensions"], json!(null));
        let back: CollectionInfoView = serde_json::from_value(val).expect("往返应成立");
        assert!(!back.exists);
        assert!(back.dimensions.is_none());
    }

    #[test]
    fn hard_limits_live_in_the_contract_layer() {
        // 数字住在契约层：宿主 provider 与分派层只准引用这四个常量
        assert_eq!(KNOWLEDGE_CONTENT_MAX_BYTES, 64 * 1024);
        assert_eq!(KNOWLEDGE_SEARCH_TOP_K_MAX, 50);
        assert_eq!(KNOWLEDGE_TOP_K_RANGE, (1, 50));
        assert_eq!(KNOWLEDGE_SNIPPET_MAX_CHARS, 200);
        // 区间的自洽性由模块里的 `const _: () = assert!(…)` 在编译期钉住，
        // 故此处不再写常量对常量的 assert!（会撞 clippy::assertions_on_constants）。
    }

    /// 未覆盖任何方法的实现：四个 op 全部吃默认实现 ⇒ 一律 `Err(Provider)`。
    struct BareKnowledge;
    impl KnowledgeVectorProvider for BareKnowledge {}

    #[test]
    fn default_methods_unimplemented() {
        let provider = BareKnowledge;
        let checks: Vec<(&str, Result<Value>)> = vec![
            (
                "search",
                futures::executor::block_on(provider.search(VectorSearchRequest {
                    knowledge_base_id: "kb-1".into(),
                    query: "测试".into(),
                    top_k: 8,
                    min_similarity: None,
                    doc_ids: None,
                }))
                .map(|v| json!(v)),
            ),
            (
                "keyword_search",
                futures::executor::block_on(provider.keyword_search(KeywordSearchRequest {
                    collection_id: "kb-1".into(),
                    query: "测试".into(),
                    top_k: 8,
                }))
                .map(|v| json!(v)),
            ),
            (
                "index_text",
                futures::executor::block_on(provider.index_text(VectorIndexRequest {
                    collection_id: "kb-1".into(),
                    document_id: "doc-1".into(),
                    content: "正文".into(),
                }))
                .map(|v| json!(v)),
            ),
            (
                "collection_info",
                futures::executor::block_on(provider.collection_info("kb-1")).map(|v| json!(v)),
            ),
        ];
        assert_eq!(checks.len(), 4);
        for (method, outcome) in checks {
            let err = outcome.expect_err("默认实现必须返回 Err");
            match err {
                AxAgentError::Provider(msg) => {
                    assert!(msg.contains("KnowledgeVectorProvider::"), "文案须点明 trait：{msg}");
                    assert!(msg.contains(method), "文案须点明方法 {method}：{msg}");
                },
                other => panic!("{method} 的默认实现应返回 AxAgentError::Provider，实际 {other:?}"),
            }
        }
    }

    #[test]
    fn trait_is_object_safe() {
        // 注册进 capability_registry 靠 `Arc<dyn KnowledgeVectorProvider>`，对象安全是前提
        let provider: std::sync::Arc<dyn KnowledgeVectorProvider> =
            std::sync::Arc::new(BareKnowledge);
        assert!(futures::executor::block_on(provider.collection_info("kb-1")).is_err());
    }
}
