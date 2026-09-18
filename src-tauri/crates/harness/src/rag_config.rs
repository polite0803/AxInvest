// SPDX-License-Identifier: AGPL-3.0-only

//! RAG 相关配置类型
//!
//! 纯数据 DTO，不依赖重型实现模块。
//! 被 `axagent-core::types` re-export。

pub use crate::note_dtos::Note;
use serde::{Deserialize, Serialize};

/// BGE reranker 的**权威文件名**（GGUF Q4_K_M 量化版）。
///
/// # 为什么单独提出来（2026-09-15 修）
///
/// 这个文件名此前在仓库里**存在 5 份互不一致的字面量**：
/// - `search/src/model_downloader.rs` 的下载清单（**唯一有实际约束的一份** ——
///   下不到这个文件名就没法推理）
/// - `search/src/reranker.rs` 的两处 `unwrap_or_else` 兜底
/// - 本文件的 `RerankConfig::default()`
/// - 前端 `src/stores/feature/settingsStore.ts` 的默认值 —— 写的是
///   `"bge-reranker-v2-m3"`（**缺 `.Q4_K_M.gguf` 后缀**），即指向一个永远不会存在的文件
///
/// 前三份恰好一致所以掩盖了问题，第四份是「同名默认值多个真源」的典型受害者
/// （记忆 W17）。现在下载清单、兜底、类型默认值全部引用本常量，
/// 由 `search::model_downloader::tests::reranker_preset_matches_rerank_config_default`
/// 钉住「下载清单 == 类型默认值」；前端侧由 `rag_config::tests` 依赖契约钉住。
pub const RERANKER_MODEL_FILENAME: &str = "bge-reranker-v2-m3.Q4_K_M.gguf";

/// Rerank 配置
///
/// `backend` 字段支持的取值：
/// - `rule` —— 基于关键词匹配的规则排序（默认，零依赖）
/// - `cross_encoder` —— 本地 candle 推理的 Cross-Encoder 模型
/// - `pipeline` —— 规则 + Cross-Encoder 级联
/// - `cohere` —— 云端 Cohere Rerank API（需配合 `api_key_ref`）
/// - `jina` —— 云端 Jina Rerank API（需配合 `api_key_ref`）
/// - `voyage` —— 云端 Voyage AI Rerank API（需配合 `api_key_ref`）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RerankConfig {
    pub enabled: bool,
    /// 重排后端。**wire 名 = 字段名的 camelCase，即 `backend`**。
    ///
    /// ⚠ 2026-09-15 修：此处原为 `#[serde(rename = "type")]`，于是 wire 名被改成
    /// `type`，而**唯一产出方**（前端 `src/stores/feature/settingsStore.ts` 的默认
    /// `ragPipelineConfig.rerank` 与设置面板 `persistRagConfig`）写的是 `backend`
    /// ⇒ 反序列化时 `type` 恒缺失 ⇒ **整个 `RAGPipelineConfig` 解析失败**，再被
    /// 调用方的 `unwrap_or_default()` 静默吞掉，表现为「RAG 设置面板里的所有开关都
    /// 不生效」（包括 `entityGraph.enabled` 恒为 false，使 Graph RAG 开关永无可能打开）。
    ///
    /// 归正依据：AGENTS.md 禁区 13「DTO 字段命名（全站统一 camelCase）」——
    /// Rust 字段保持 snake_case，靠 `rename_all = "camelCase"` 输出 camelCase，
    /// **禁止**用 `rename` 造出与字段名无关的 wire 名。仓库内已确认无任何产出方写 `type`
    /// （`config/`、前端、后端均无），故不保留 `alias`，只留单一规范名。
    pub backend: String,
    pub cross_encoder_model: Option<String>,
    pub top_n: usize,
    pub candidate_k: usize,
    pub rule_filter_keep: usize,
    pub score_threshold: Option<f32>,
    /// 云端 reranker（cohere/jina/voyage）的 API Key 凭证引用名，
    /// 由 wiring 层（credential store）解析后注入实际 key。
    /// 本地 backend（rule/cross_encoder/pipeline）忽略此字段。
    pub api_key_ref: Option<String>,
    /// 自定义云端 rerank API base URL（可选，覆盖各厂商默认域名）。
    /// 例如自建 Cohere 兼容网关或私有化部署时使用。
    pub api_base: Option<String>,
}

impl Default for RerankConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: "rule".to_string(),
            cross_encoder_model: Some(RERANKER_MODEL_FILENAME.to_string()),
            top_n: 5,
            candidate_k: 30,
            rule_filter_keep: 15,
            score_threshold: None,
            api_key_ref: None,
            api_base: None,
        }
    }
}

/// Self-RAG 配置
///
/// `#[serde(default)]` 为**结构体级**（非逐字段）：任一字段缺失时回落到
/// `SelfRagConfig::default()` 的同名字段值。这是必要的容错 —— 逐字段默认值取的是
/// **字段类型**的 `Default`（`usize` → 0、`String` → ""），会把 `maxRetryRounds` 变成 0、
/// 把阈值变成 0.0，反而更坏；而**没有**默认时，缺一个字段就整份 `RAGPipelineConfig`
/// 解析失败、被调用方 `unwrap_or_default()` 静默吞掉（2026-09-15 实测的故障形态）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SelfRagConfig {
    pub enabled: bool,
    pub judge_model: String,
    pub ollama_endpoint: String,
    pub relevance_threshold: f32,
    pub quality_threshold: f32,
    pub max_retry_rounds: u8,
}

impl Default for SelfRagConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            judge_model: "qwen2.5:0.5b".to_string(),
            ollama_endpoint: "http://localhost:11434".to_string(),
            relevance_threshold: 0.5,
            quality_threshold: 0.6,
            max_retry_rounds: 2,
        }
    }
}

/// 多引擎 RAG 混合检索配置
///
/// 控制三路融合（dense + sparse + BM25）的权重与算法。
/// 权重会被归一化（总和不要求为 1，按比例分配）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridConfig {
    /// 是否启用混合检索（false 时仅走 dense vector）
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// dense vector 检索权重（0.0~1.0，默认 0.7）
    #[serde(default = "default_vector_weight")]
    pub vector_weight: f32,
    /// BM25 关键词检索权重（0.0~1.0，默认 0.3）
    #[serde(default = "default_bm25_weight")]
    pub bm25_weight: f32,
    /// sparse neural 检索权重（0.0~1.0，默认 0.0）。
    /// 当前未接入 sparse encoder，保持 0 即可；后续接入 SPLADE/BGE-M3 时可调高。
    #[serde(default)]
    pub sparse_weight: f32,
    /// 融合算法：`rrf`（默认）或 `weighted`
    #[serde(default = "default_fusion")]
    pub fusion: String,
    /// RRF 算法的 k 参数（默认 60.0）
    #[serde(default = "default_rrf_k")]
    pub rrf_k: f32,
}

fn default_true() -> bool {
    true
}
fn default_vector_weight() -> f32 {
    0.7
}
fn default_bm25_weight() -> f32 {
    0.3
}
fn default_fusion() -> String {
    "rrf".to_string()
}
fn default_rrf_k() -> f32 {
    60.0
}

impl Default for HybridConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            vector_weight: 0.7,
            bm25_weight: 0.3,
            sparse_weight: 0.0,
            fusion: "rrf".to_string(),
            rrf_k: 60.0,
        }
    }
}

/// Graph RAG 增强检索（实体图谱）配置。
///
/// `enabled = true` 时，wiring 层（`src/init/services.rs`）会向
/// `axagent_search::entity_graph` 注入 `dao::KnowledgeGraphProvider`，
/// 于是 `RAGPipeline` 的第 4 阶段（图增强检索）才会真正执行并产出
/// `graph_enhanced_search` 结果（实体 + 关系 + 邻居）。
///
/// # 为什么默认关闭
///
/// 图检索结果会进入注入 prompt 的上下文 ⇒ 属**可见行为变更**，且
/// `graph_enhanced_search` 按 `knowledge_entities.kb_id` 过滤，若图谱内
/// 无数据或 kb_id 与容器 id 不同域会返回空（`total_hits = 0`，会被
/// 启动/检索日志显式记录，便于判断是否真的生效）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityGraphConfig {
    #[serde(default)]
    pub enabled: bool,
}

/// 全局 RAG 管线配置
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RAGPipelineConfig {
    #[serde(default)]
    pub query_enhancement: crate::types::EnhancementConfig,
    #[serde(default)]
    pub rerank: RerankConfig,
    #[serde(default)]
    pub self_rag: SelfRagConfig,
    /// 多引擎 RAG：混合检索权重与融合算法配置
    #[serde(default)]
    pub hybrid: HybridConfig,
    /// Graph RAG 增强检索（实体图谱）
    #[serde(default)]
    pub entity_graph: EntityGraphConfig,
}

/// 笔记检索结果（含完整 Note 对象）
///
/// # `score` 语义
/// `score` 是「相关性分数」：**越大越相关**。
/// - 走 hybrid 检索路径时：值为 `HybridSearcher.combined_score`（归一化后越大越好）
/// - 走 keyword 检索路径时：值为 `bm25_rank + quality_score * 0.3`（越大越好）
///
/// 注意：与 `VectorSearchResult.score`（L2 距离，越小越好）语义**相反**，
/// 前端展示与排序时需按类型区分方向。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteSearchResult {
    pub note: Note,
    pub snippet: String,
    pub score: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **前后端字段名契约**：前端写入的 `ragPipelineConfig` 形状必须能被 Rust 解析。
    ///
    /// 形状来源（两处产出方，形状一致）：
    /// ① 前端默认值真源 `src/stores/feature/settingsStore.ts` 的 `ragPipelineConfig`；
    /// ② 设置面板 `src/components/settings/KnowledgeBaseDocuments.tsx::persistRagConfig`
    ///    写出的同形对象。
    ///
    /// # 为什么必须钉这一层
    ///
    /// 2026-09-15 实测：`RerankConfig` 曾用 `#[serde(rename = "type")]` 把 wire 名
    /// 改成 `type`，而前端一直写 `backend` ⇒ 反序列化恒失败 ⇒ 调用方
    /// `indexing.rs::load_rag_pipeline_config` 的 `unwrap_or_default()` 静默吞掉 ⇒
    /// **RAG 面板所有设置都不生效**（含 `entityGraph.enabled` 恒 false，使 Graph RAG
    /// 开关永远打不开）。这个 bug 能潜伏很久，就是因为**没有任何测试对比过
    /// 「前端写出的 JSON」与「后端 DTO」**。本用例补上这层守卫 —— 字段名再分叉，
    /// 这里会直接红，而不是等到用户发现「开关没用」。
    fn frontend_default_config() -> serde_json::Value {
        serde_json::json!({
            "queryEnhancement": {
                "enabled": false,
                "strategy": "auto",
                "maxVariants": 3,
                "combinedCall": true
            },
            "rerank": {
                "enabled": true,
                "backend": "rule",
                "crossEncoderModel": RERANKER_MODEL_FILENAME,
                "topN": 5,
                "candidateK": 30,
                "ruleFilterKeep": 15,
                "scoreThreshold": null
            },
            "selfRag": {
                "enabled": false,
                "judgeModel": "qwen2.5:0.5b",
                "ollamaEndpoint": "http://localhost:11434",
                "relevanceThreshold": 0.5,
                "qualityThreshold": 0.6,
                "maxRetryRounds": 2
            },
            "hybrid": {
                "enabled": true,
                "vectorWeight": 0.7,
                "bm25Weight": 0.3,
                "sparseWeight": 0.0,
                "fusion": "rrf",
                "rrfK": 60.0
            },
            "entityGraph": { "enabled": true }
        })
    }

    #[test]
    fn frontend_shaped_config_must_parse() {
        let cfg: RAGPipelineConfig = serde_json::from_value(frontend_default_config())
            .expect("前端写入的 ragPipelineConfig 必须能被 Rust 解析（DTO 字段名契约）");

        // 逐组断言「值真的被读到了」—— 仅 `is_ok()` 不够：字段名分叉时也可能靠
        // 结构体级 default 兜住而「解析成功但全是默认值」，那种假阳性正是本用例要拦的。
        assert_eq!(cfg.rerank.backend, "rule", "wire 名必须是 camelCase 的 backend");
        assert!(cfg.rerank.enabled);
        assert_eq!(cfg.rerank.top_n, 5);
        assert_eq!(cfg.rerank.candidate_k, 30);
        assert_eq!(cfg.rerank.rule_filter_keep, 15);
        assert_eq!(cfg.rerank.cross_encoder_model.as_deref(), Some(RERANKER_MODEL_FILENAME));
        assert_eq!(cfg.self_rag.judge_model, "qwen2.5:0.5b");
        assert_eq!(cfg.self_rag.max_retry_rounds, 2);
        assert_eq!(cfg.query_enhancement.strategy, crate::types::EnhancementStrategy::Auto);
        assert_eq!(cfg.query_enhancement.max_variants, 3);
        assert!((cfg.hybrid.vector_weight - 0.7).abs() < 1e-6, "hybrid 权重必须被读到");
        assert!((cfg.hybrid.rrf_k - 60.0).abs() < 1e-6);
        // Graph RAG 开关：W10 的判据就是这一位，必须真的读得到
        assert!(cfg.entity_graph.enabled, "entityGraph.enabled 必须被读到");
    }

    #[test]
    fn partial_config_falls_back_per_field_instead_of_failing() {
        // 缺字段 ⇒ 该字段回落 `RerankConfig::default()` 的同名字段值，
        // **不得**整份配置解析失败（那会让所有设置静默变默认）。
        let cfg: RAGPipelineConfig =
            serde_json::from_value(serde_json::json!({"rerank": {"enabled": false}}))
                .expect("部分字段缺失也必须能解析");

        assert!(!cfg.rerank.enabled, "显式给出的字段必须生效");
        assert_eq!(cfg.rerank.top_n, RerankConfig::default().top_n, "缺失字段须用结构体默认值补齐");
        assert_eq!(cfg.rerank.backend, RerankConfig::default().backend);
        // 未出现的配置组整体回落默认
        assert_eq!(cfg.self_rag.enabled, SelfRagConfig::default().enabled);
    }

    #[test]
    fn null_config_falls_back_to_all_defaults() {
        // `AppSettings::default()` 里 `rag_pipeline_config` 是 `Value::Null`
        // （settings_chat.rs），这是「用户从未保存过 RAG 设置」的形态。
        assert!(serde_json::from_value::<RAGPipelineConfig>(serde_json::Value::Null).is_err());
        // ⇒ 调用方必须用 `unwrap_or_default()` 兜底，不能 unwrap。
    }
}
