// SPDX-License-Identifier: AGPL-3.0-only
//! 知识图谱契约
use crate::types::rag_voice_etc::{CreateKnowledgeEntityInput, KnowledgeEntity, KnowledgeRelation};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ── 图谱节点类型 ─────────────────────────────────────────────

/// 知识图谱节点类型。
///
/// ⚠ **B2（2026-09-14）裁剪**：本枚举原有 4 个变体，其中 `MemoryItem`（`memory_item`）与
/// `ObsidianNote`（`obsidian_note`）**全仓零引用、DB 零数据**（`knowledge_entities.node_type`
/// 实测只有 `entity` 一种，72816 行）—— 它们和 `GraphSourceType` / `CreateMultiSourceEntityInput`
/// 一起构成了一个「声明齐备但没有接线」的多源假能力面，已整体删除。
///
/// 保留的两个变体都**有真实消费**：
/// - [`Entity`](Self::Entity)：`knowledge_entities.node_type` 的 **8 处写入点**都用它
///   （`GraphNodeType::Entity.as_str()`，DB 实测 72816 行）；
/// - [`Note`](Self::Note)：与 `notes.page_type` 为 NULL 时的默认值 `"note"` 同形
///   （`dao/src/repo/note.rs:543`），也是 `PageType::Note` 的字面量。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GraphNodeType {
    /// 知识库实体（原有）
    Entity,
    /// Wiki 笔记
    Note,
}

impl GraphNodeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Entity => "entity",
            Self::Note => "note",
        }
    }
}

// ── EntityGraphProvider trait ──────────────────────────────

/// 因果边在 `knowledge_relations.relation_type` 上的保留取值。
///
/// 因果边是行为统计（工具序列 / 意图转移），不是文档知识：
/// RAG 图检索必须排除它，避免污染 `graph_enhanced_search` 结果。
/// 权威定义在契约层，trajectory（写入方）与 dao（检索方）都引用此处。
pub const CAUSAL_RELATION_TYPE: &str = "causes";

#[async_trait]
pub trait EntityGraphProvider: Send + Sync {
    async fn get_entities(&self, kb_id: &str) -> Result<Vec<KnowledgeEntity>, String>;
    async fn search_entities(
        &self,
        kb_id: &str,
        query: &str,
    ) -> Result<Vec<KnowledgeEntity>, String>;
    async fn create_entity(
        &self,
        kb_id: &str,
        input: CreateKnowledgeEntityInput,
    ) -> Result<KnowledgeEntity, String>;
    async fn delete_entity(&self, entity_id: &str) -> Result<(), String>;
    async fn get_relations(&self, entity_id: &str) -> Result<Vec<KnowledgeRelation>, String>;
    async fn create_relation(
        &self,
        source_id: &str,
        target_id: &str,
        rel_type: &str,
    ) -> Result<KnowledgeRelation, String>;
    async fn delete_relation(&self, relation_id: &str) -> Result<(), String>;

    /// 核心方法：图增强检索
    /// 根据用户 Query 检索实体，并扩展其邻居关系，最终返回可直接注入 RAG 的上下文
    async fn graph_enhanced_search(
        &self,
        input: GraphEnhancedSearchInput,
    ) -> Result<GraphEnhancedSearchResult, String>;
}

// ── B2（2026-09-14）：删除「多源扩展」假能力面 ─────────────────────────────
//
// 本 trait 原有一组「多源扩展方法（默认实现，便于渐进式采用）」，由三个方法组成：
//
// | 方法 | 默认实现 | 实测 |
// |---|---|---|
// | `get_entities_by_source` | `Ok(vec![])` | 0 调用、0 覆写 |
// | `create_multi_source_entity` | 丢弃 `source_type` / `source_id` / `node_type` / `external_id` / `aliases` | 0 调用、0 覆写 |
// | `search_entities_by_node_type` | `Ok(vec![])` | 0 调用、0 覆写 |
//
// 全仓 `EntityGraphProvider` 只有 2 个实现者（`dao::knowledge_graph_provider`、
// `harness::test_support::NoopEntityGraphProvider`），**都没有覆写这三个方法**
// ⇒ 它们不构成「渐进式采用」的接口，而是一组**会静默返回空 / 静默丢字段的假契约**：
// 任何未来调用方都会拿到「成功但没有数据」的结果，且没有任何信号。
//
// 配套的 `GraphSourceType`（4 值）与 `CreateMultiSourceEntityInput` 因此也一并删除 ——
// 它们在改造前就是**零外部引用**的声明（`GraphNodeType::MemoryItem` / `ObsidianNote` 同理）。
//
// **这不是「保留待实现」，而是裁剪**：真正的多源接入需要先有写入方与消费方，
// 届时 `source_type` 的取值域从「实际写入的列」重新登记即可（登记成本远低于维护一个撒谎的接口）。
// 若将来要恢复，判据是：**同一接口必须同时具备 ≥1 调用方 + ≥1 覆写实现**。

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedEntity {
    pub name: String,
    pub entity_type: String,
    pub aliases: Vec<String>,
    pub description: String,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedRelation {
    pub source: String,
    pub target: String,
    pub relation_type: String,
}
#[async_trait]
pub trait EntityExtractor: Send + Sync {
    async fn extract_entities(&self, text: &str) -> Result<Vec<ExtractedEntity>, String>;
    async fn extract_relations(
        &self,
        text: &str,
        entities: &[ExtractedEntity],
    ) -> Result<Vec<ExtractedRelation>, String>;
}

// ── LightRAG 跨文档实体抽取与图查询增强 DTO ──────────────────────────────

/// 跨文档实体抽取请求（调用方组装后传入）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractEntitiesFromDocumentsInput {
    pub knowledge_base_id: String,
    /// 待抽取的文档 ID 列表（最多 20 个，超出由调用方分批）
    pub document_ids: Vec<String>,
    /// 已抽取的 chunk 内容映射 document_id → Vec<chunk_content>
    /// 由调用方从 vector_store 加载后传入
    pub chunks_by_document: std::collections::HashMap<String, Vec<String>>,
    /// 已存在的实体列表（用于去重/合并判断），由调用方从 DAO 加载
    pub existing_entities: Vec<crate::types::KnowledgeEntity>,
}

/// 跨文档实体抽取结果
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractEntitiesResult {
    /// 新增的实体（已写入 DB，含最终 id）
    pub new_entities: Vec<crate::types::KnowledgeEntity>,
    /// 更新的实体（mention_count 累加 / properties 合并）
    pub updated_entities: Vec<crate::types::KnowledgeEntity>,
    /// 新增的关系
    pub new_relations: Vec<crate::types::KnowledgeRelation>,
    /// 跳过的 chunk 数（LLM 判定无实体）
    pub skipped_chunks: u32,
    /// 总耗时（毫秒）
    pub elapsed_ms: u64,
}

/// 图查询增强上下文片段
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEnhancedContextChunk {
    /// 实体名称
    pub entity_name: String,
    /// 实体类型
    pub entity_type: String,
    /// 实体描述
    pub description: Option<String>,
    /// 命中的关系列表
    pub relations: Vec<GraphRelationEdge>,
    /// 来源（哪个 KB 抽取的）
    pub knowledge_base_id: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphRelationEdge {
    pub target_entity_name: String,
    pub relation_type: String,
    pub description: Option<String>,
    pub weight: f64,
}

/// 图查询增强请求
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEnhancedSearchInput {
    pub knowledge_base_id: String,
    pub query: String,

    /// 限制检索的实体类型 (e.g., ["company", "person"])
    /// 如果为空，则检索所有类型
    #[serde(default)]
    pub entity_type_filters: Vec<String>,

    /// 限制扩展的关系类型 (e.g., ["in_industry", "has_chairman"])
    /// 如果为空，则扩展所有关系
    #[serde(default)]
    pub relation_type_filters: Vec<String>,

    /// 最多返回的实体数（默认 10）
    pub top_k: Option<usize>,
    /// 是否包含 1-hop 邻居关系（默认 true）
    pub include_neighbors: Option<bool>,
}

/// 图查询增强结果
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEnhancedSearchResult {
    /// 命中的实体及其邻居关系
    pub entities: Vec<GraphEnhancedContextChunk>,
    /// 拼接好的上下文文本（可直接注入到 RAG context）
    pub context_text: String,
    /// 命中实体总数
    pub total_hits: usize,
}

/// 图上下文格式化器接口
/// 允许调用方自定义如何将实体关系网络转换为 LLM 可理解的文本格式
#[async_trait]
pub trait GraphContextFormatter: Send + Sync {
    /// 将图检索结果格式化为字符串
    async fn format_context(&self, result: &GraphEnhancedSearchResult) -> Result<String, String>;
}

/// 提供一个默认的简单格式化器
pub struct DefaultGraphFormatter;

#[async_trait]
impl GraphContextFormatter for DefaultGraphFormatter {
    async fn format_context(&self, result: &GraphEnhancedSearchResult) -> Result<String, String> {
        let mut context = String::new();
        for entity in &result.entities {
            context.push_str(&format!("【{} - {}】\n", entity.entity_type, entity.entity_name));
            if let Some(desc) = &entity.description {
                context.push_str(&format!("描述: {}\n", desc));
            }
            for rel in &entity.relations {
                context.push_str(&format!(
                    "- {} (关系: {})\n",
                    rel.target_entity_name, rel.relation_type
                ));
            }
            context.push('\n');
        }
        Ok(context)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  B1（2026-09-14）：图谱**关系 / 实体类型词汇表**（含定义域·值域）
// ═══════════════════════════════════════════════════════════════════════════
//
// ## 为什么要建这张表
//
// `knowledge_relations.relation_type` 与 `knowledge_entities.entity_type` 都是自由文本
// `String` 列，全仓**没有任何一处声明「允许哪些取值、关系的源/目标该是什么类」**。
// 实测可写入 `(Company)-[causes]->(Note)` 而**没有任何机制报错** ——
// 有实体、有边、有类型字符串，但**没有类型系统**。
//
// ## 本表刻意遵守的两条纪律
//
// 1. **不发明约束**：只登记**观察到**的定义域/值域；写入方本身拿不到节点类
//    （`upsert_relation(source_id, target_id, relation_type, weight)` 只有 id）的关系，
//    其 `domain`/`range` 记为 `None` = **宽松**，而不是猜一个。
// 2. **只登记、不硬拦**：LLM 抽取出的关系类型天然是开放集合，
//    硬拦会直接掐断写入链路。故校验产出**违规清单**（可上报、可统计），
//    由调用方决定是否拒绝 —— 是否收紧为硬拦是**产品决策**，不由本模块代劳。
//
// ## 本文件**不替换**任何既有词汇表
//
// `trajectory::RelationshipType`（11 变体）、`conversation.rs` 的裸字面量、
// `knowledge_graph_provider` 的过滤器文档值都仍然存在。本表是它们的**登记处**：
// 先把「实际在写什么」写下来，才谈得上收敛。
//
// ## D5（2026-09-14）：编码收敛 + 由**真实数据**补出的第三类登记
//
// **① 编码收敛**：`relation_type` 原本同列并存两种编码（裸字面量 vs 带引号的 JSON 编码）。
// 写入端已收敛为裸字面量（`trajectory/src/storage.rs::save_relationship` 改用 `Display`），
// 读取端永久兼容两态 ⇒ 本表的 `encoding` 全部为 `Literal`，且**无需数据迁移**
// （实测带引号行 = 0；旧行也能自愈读取）。判据：`test_no_observed_decl_uses_the_legacy_encoding`。
//
// **② 由真实数据补出的第三类**：本表最初只登记「代码常量」形态，于是漏掉了
// **数据驱动列**（`edges.csv` 的 `rtype`、`nodes.csv` 的 `type`）—— 实测该列写入了
// 22 个裸中文职位名、约 6 万行。这类值只能按**形态**放行（见 [`DATA_DRIVEN_COLUMN`]），
// 逐条登记不可行（数据文件换了值域就失效）。教训：**建词表前先数一次真实列分布**，
// 否则第一版词表对存量数据就是刷屏器。

/// 关系值在 DB 列上的**写入编码**。
///
/// 本枚举记录的是「该 id 在 **DB 列里可能出现什么形态**」，不是「写入端有多花哨」：
///
/// - 现状（2026-09-14 D5 收敛后）：**整列只有 [`Literal`](Self::Literal) 这一种编码**，
///   所有 `observed: true` 的声明都必须是 `Literal`（由
///   `test_no_observed_decl_uses_the_legacy_encoding` 锁住）。
/// - [`JsonEncodedVariant`](Self::JsonEncodedVariant) 保留在枚举里，因为**存量数据**里
///   确实可能还躺着带引号的行（本机实测 0 行，但已发布版本写过），而读取端
///   （`trajectory::storage::parse_stored_relation_type`）**永久兼容两态** ——
///   枚举要如实描述这个事实，否则下一个人会以为「历史上没这回事」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationEncoding {
    /// 裸字面量，如 `causes` / `contains`（**当前唯一在用的编码**）
    Literal,
    /// **历史编码**：DB 里是带引号的 `"part_of"`（旧 `serde_json::to_string` 产物）。
    ///
    /// 写入端已于 2026-09-14 停止产生该形态（`trajectory/src/storage.rs::save_relationship`
    /// 改用 `Display`）；读取端仍兼容。**不得再有 `observed: true` 的声明使用它。**
    JsonEncodedVariant,
}

/// 一条关系的声明（id + 定义域 + 值域 + 出处）。
#[derive(Debug, Clone, Copy)]
pub struct RelationDecl {
    /// 写入 DB 的逻辑 id（JSON 编码那条路径在 DB 里会多一对引号，见 `encoding`）
    pub id: &'static str,
    pub encoding: RelationEncoding,
    /// 定义域（源节点允许的节点类）。`None` = **未观察到约束，刻意宽松**。
    pub domain: Option<&'static [GraphNodeType]>,
    /// 值域（目标节点允许的节点类）。`None` = 同上。
    pub range: Option<&'static [GraphNodeType]>,
    pub meaning: &'static str,
    /// `文件:行` 出处 —— 表内**不允许空串**（由 `relation_table_ok` 强制）
    pub evidence: &'static str,
    /// 是否存在**实际写入方**；`false` = 仅文档声明过（**悬空契约**）
    pub observed: bool,
}

const ENTITY_ONLY: &[GraphNodeType] = &[GraphNodeType::Entity];

/// 关系词汇表 —— 每条都带 `文件:行` 出处。
///
/// ⚠ `id` **唯一**（由 [`relation_table_ok`] 机器强制，`test_relation_ids_are_unique` 复核）。
///
/// D5（2026-09-14）前 `contains` 曾**同时**登记两条（裸字面量 = 会话容器关系、
/// JSON 编码 = 记忆实体关系）—— 那时 `(id, encoding)` 才是唯一键。编码收敛后两条链路
/// 在 DB 里是**同一个字面量** ⇒ 合并为一条（两条链路的出处都写在 `evidence` 里保持可见）。
pub const RELATION_DECLS: &[RelationDecl] = &[
    RelationDecl {
        id: "causes",
        encoding: RelationEncoding::Literal,
        domain: None,
        range: None,
        meaning: "行为统计因果边（工具序列 / 意图转移），RAG 图检索必须排除",
        evidence: "trajectory/src/causal.rs:322 ｜ dao/src/repo/knowledge_graph.rs:901 排除",
        observed: true,
    },
    RelationDecl {
        id: "contains",
        encoding: RelationEncoding::Literal,
        domain: Some(ENTITY_ONLY),
        range: Some(ENTITY_ONLY),
        meaning: "包含。**两条链路共用同一个字面量**（D5 收敛前它们是两种编码，见模块文档）：\
                  ① 会话容器实体 ⊃ 该会话第 n 个 Q&A 对实体；\
                  ② 记忆实体图的包含（`trajectory::RelationshipType::Contains`）",
        evidence: "dao/src/repo/conversation.rs:720 ｜ trajectory/src/memory_providers/entity.rs:72",
        observed: true,
    },
    RelationDecl {
        id: "follows",
        encoding: RelationEncoding::Literal,
        domain: Some(ENTITY_ONLY),
        range: Some(ENTITY_ONLY),
        meaning: "时序相邻：第 n 个 Q&A 对实体 → 第 n+1 个",
        evidence: "dao/src/repo/conversation.rs:739",
        observed: true,
    },
    RelationDecl {
        id: "mentions",
        encoding: RelationEncoding::Literal,
        domain: None,
        range: None,
        meaning: "文本提及（LLM 抽取关系缺省值）",
        evidence: "src/commands/knowledge_graph.rs:305",
        observed: true,
    },
    RelationDecl {
        id: "uses",
        encoding: RelationEncoding::Literal,
        domain: None,
        range: None,
        meaning: "使用（LLM 抽取实测输出取值）",
        evidence: "src/commands/knowledge_graph.rs:553（`uses` 断言；`:541` 是 fixture 输入）",
        observed: true,
    },
    // ── 记忆实体图的 11 个变体（`trajectory::RelationshipType`）──
    // 写入方 `trajectory/src/storage.rs::save_relationship` **只传实体 id**，
    // 因此这里无法给出可信的定义域/值域 ⇒ 一律 `None`（宽松），不发明约束。
    //
    // D5（2026-09-14）：这些 id 此前以 **JSON 编码**（`"part_of"`）落库；写入端已改为
    // `Display`（裸字面量）⇒ `encoding` 全部改回 `Literal`。配对读取端
    // `parse_stored_relation_type` 兼容两态，故**无需数据迁移**。
    RelationDecl {
        id: "part_of",
        encoding: RelationEncoding::Literal,
        domain: None,
        range: None,
        meaning: "记忆实体图：部分-整体",
        evidence: "trajectory/src/memory_providers/entity.rs:66",
        observed: true,
    },
    RelationDecl {
        id: "related_to",
        encoding: RelationEncoding::Literal,
        domain: None,
        range: None,
        meaning: "记忆实体图：相关（**同时是 `From<&str>` 的未知兜底变体**，见 :82-97）",
        evidence: "trajectory/src/memory_providers/entity.rs:67",
        observed: true,
    },
    RelationDecl {
        id: "depends_on",
        encoding: RelationEncoding::Literal,
        domain: None,
        range: None,
        meaning: "记忆实体图：依赖",
        evidence: "trajectory/src/memory_providers/entity.rs:68",
        observed: true,
    },
    RelationDecl {
        id: "owns",
        encoding: RelationEncoding::Literal,
        domain: None,
        range: None,
        meaning: "记忆实体图：持有",
        evidence: "trajectory/src/memory_providers/entity.rs:69",
        observed: true,
    },
    RelationDecl {
        id: "defines",
        encoding: RelationEncoding::Literal,
        domain: None,
        range: None,
        meaning: "记忆实体图：定义",
        evidence: "trajectory/src/memory_providers/entity.rs:70",
        observed: true,
    },
    RelationDecl {
        id: "implements",
        encoding: RelationEncoding::Literal,
        domain: None,
        range: None,
        meaning: "记忆实体图：实现",
        evidence: "trajectory/src/memory_providers/entity.rs:71",
        observed: true,
    },
    RelationDecl {
        id: "calls",
        encoding: RelationEncoding::Literal,
        domain: None,
        range: None,
        meaning: "记忆实体图：调用",
        evidence: "trajectory/src/memory_providers/entity.rs:73",
        observed: true,
    },
    RelationDecl {
        id: "method_of",
        encoding: RelationEncoding::Literal,
        domain: None,
        range: None,
        meaning: "记忆实体图：方法归属",
        evidence: "trajectory/src/memory_providers/entity.rs:74",
        observed: true,
    },
    RelationDecl {
        id: "performs",
        encoding: RelationEncoding::Literal,
        domain: None,
        range: None,
        meaning: "记忆实体图：执行",
        evidence: "trajectory/src/memory_providers/entity.rs:75",
        observed: true,
    },
    RelationDecl {
        id: "associated_with",
        encoding: RelationEncoding::Literal,
        domain: None,
        range: None,
        meaning: "记忆实体图：关联",
        evidence: "trajectory/src/memory_providers/entity.rs:76",
        observed: true,
    },
    // ── CSV 导入路径写入的关系（`src/commands/knowledge.rs` 的 `graph_import`）──
    RelationDecl {
        id: "in_industry",
        encoding: RelationEncoding::Literal,
        domain: None,
        range: None,
        meaning: "实体属于某行业（`stock_industry.csv` 导入）",
        evidence: "src/commands/knowledge.rs:1795（另有文档示例 harness/src/knowledge_graph.rs:195）",
        observed: true,
    },
    RelationDecl {
        id: "has_concept",
        encoding: RelationEncoding::Literal,
        domain: None,
        range: None,
        meaning: "实体关联概念（`stock_concept.csv` 导入）",
        evidence: "src/commands/knowledge.rs:1780",
        observed: true,
    },
    // ── 只有文档、没有写入方的取值（悬空契约）──
    // `GraphEnhancedSearchInput.relation_type_filters` 的文档示例里出现过，
    // 但**全仓没有任何写入点** —— 登记为 `observed: false`，让「文档承诺的词汇表」
    // 与「实际存在的词汇表」的差集是可见的。
    RelationDecl {
        id: "has_chairman",
        encoding: RelationEncoding::Literal,
        domain: None,
        range: None,
        meaning: "【悬空】文档称的领域关系：公司有董事长（实际的高管关系走 `employ_*` 前缀族）",
        evidence: "harness/src/knowledge_graph.rs:195（文档示例），零写入点",
        observed: false,
    },
];

/// **前缀族**：id 不固定、按模板生成的关系类型。
///
/// 存在的理由有据可查：`src/commands/knowledge.rs:1811` 用
/// `format!("employ_{position}")` 生成 id —— 职位有多少个，词表就有多少项，
/// 逐条登记不可行，只能登记**族**。
#[derive(Debug, Clone, Copy)]
pub struct RelationFamilyDecl {
    /// 前缀，含下划线
    pub prefix: &'static str,
    pub meaning: &'static str,
    pub evidence: &'static str,
    pub observed: bool,
}

pub const RELATION_FAMILIES: &[RelationFamilyDecl] = &[RelationFamilyDecl {
    prefix: "employ_",
    meaning: "高管任职：公司 → 人物，id 形如 `employ_ceo` / `employ_cfo`",
    evidence: "src/commands/knowledge.rs:1811（`format!(\"employ_{position}\")`）",
    observed: true,
}];

/// **数据驱动列**：取值由**外部数据文件**决定、不由本仓库代码决定的关系类型。
///
/// `src/commands/knowledge.rs:1751` 把 `edges.csv` 的 `rtype` 列**原样**写进
/// `knowledge_relations.relation_type` ⇒ 值域由数据文件决定。
///
/// ## 为什么它必须是一条**显式声明**（而不是一句注释）
///
/// 2026-09-14 实测**两个口径 —— 必须分开记，否则会低估**：
///
/// | 口径 | 测量对象 | 实测值 |
/// |---|---|---|
/// | **数据文件** | `knowledge-sources/lemonhu/edges.csv` 的 `rtype` 列 | **24** 个 distinct（22 个裸中文职位名 + `in_industry` / `has_concept`，后两者另在 [`RELATION_DECLS`] 登记） |
/// | **DB 表**（判据真正面对的对象） | `knowledge_relations.relation_type` 全表 | **112937** 行 / **56** 个 distinct；其中**非 ASCII 53 个 / 74325 行**（占 65.8%），ASCII 仅 3 个（`has_concept` 28326 / `in_industry` 9498 / `mentions` 788） |
///
/// DB 比 CSV **多出 31 个非 ASCII 值**，来自其它写入路径（记忆实体图 / LLM 抽取），
/// 数据文件里根本看不到 —— 所以 `column` 声明的是 DB 列时，**判据依据必须取 DB 全表口径**。
/// 若只按「代码常量」建词表，这 74325 行会被 [`validate_relation_id`] 全部报成
/// `UndeclaredRelationType` —— 校验一上线就对存量数据刷屏，等于没做。
///
/// ⚠ **口径陷阱**：只记 CSV 口径（22 个）会让人误以为「非 ASCII 值可枚举」，从而想把它改成
/// 白名单 —— 那会静默漏掉另外 31 个值。本表因此只按**形态**放行，不按枚举。
///
/// **判据形态**：`rtype` 列的取值是**非 ASCII 标识符**（中文职位名）。ASCII 形态的
/// 关系类型仍然要求逐条登记（它们来自代码常量，可穷举）—— 这样「数据驱动的开放词表」
/// 与「代码里的封闭词表」在判据上被区分开，而不是把整个校验放水。
#[derive(Debug, Clone, Copy)]
pub struct DataDrivenColumnDecl {
    /// DB 列的语义名
    pub column: &'static str,
    /// 数据文件
    pub source: &'static str,
    pub meaning: &'static str,
    pub evidence: &'static str,
}

pub const DATA_DRIVEN_COLUMN: DataDrivenColumnDecl = DataDrivenColumnDecl {
    column: "knowledge_relations.relation_type",
    source: "knowledge-sources/lemonhu/edges.csv",
    meaning: "DB 列原样接收 `rtype` ⇒ 开放词表（DB 全表实测：非 ASCII 53 个 / 74325 行；\
              数据文件 `rtype` 层 24 distinct）",
    evidence: "src/commands/knowledge.rs:2872（读 `edges.csv` 的 rtype 列）",
};

/// 该值是否属于「数据驱动列写入的开放词表形态」：**含非 ASCII 字符**。
///
/// ⚠ 这是**形态判据**，不是白名单：它放行的是「数据文件带来的、本仓库无法穷举的值」。
/// ASCII 值不在放行范围内 —— 它们必然来自代码常量，应当逐条登记。
pub fn is_data_driven_column_value(id: &str) -> bool {
    !id.trim_matches('"').is_ascii()
}

// ── 关系 id 常量（替代散落的裸字面量）──

/// 容器包含关系（裸字面量编码）。
pub const RELATION_CONTAINS: &str = "contains";
/// 时序相邻关系（裸字面量编码）。
pub const RELATION_FOLLOWS: &str = "follows";
/// 文本提及关系（LLM 抽取缺省值）。
pub const RELATION_MENTIONS: &str = "mentions";

/// 实体类型声明（`knowledge_entities.entity_type`）。
///
/// ⚠ 该列是**开放词表**（LLM 抽取 + 会话记忆各写各的），因此这里**不闭合**它，
/// 只登记「已观察到的内核取值」，配合 [`validate_entity_type`] 让未登记值可见。
#[derive(Debug, Clone, Copy)]
pub struct EntityTypeDecl {
    pub id: &'static str,
    pub meaning: &'static str,
    pub evidence: &'static str,
    pub observed: bool,
}

/// 已观察到的实体类型。
///
/// # 命名空间边界（重要，判据 K「一名多义」）
///
/// 本表是**知识图谱实体类型**（写入 `knowledge_entities.entity_type`）。
/// 项目里还存在**另一套字段同名、值域不同**的词表：
/// `analysis-engine/src/opc/capability_pack_config.rs` 的 `CapabilityPackConfig.entity_types`
/// （14 个域包各一组业务记录类型：`invoice` / `portfolio` / `lead` / `bug` …，
/// 实测 44 条），它写的是 OPC **业务记录**类型，不是图谱实体。
///
/// 两者**值域必须不相交**。这条边界由 `capability_pack_config.rs` 的测试
/// `test_opc_record_types_do_not_collide_with_kg_entity_types` 断言（不复制清单，
/// 直接读运行时的 `get_all_configs()`）—— 在此处重复列一份 44 条的副本只会腐烂。
///
/// 为什么不在本表里合并另一套：合并会让 `validate_entity_type` 对业务记录类型放行
/// （或用图谱词表去校验业务记录），那正是「一名多义」要消灭的形态。
pub const ENTITY_TYPE_DECLS: &[EntityTypeDecl] = &[
    EntityTypeDecl {
        id: "conversation",
        meaning: "会话实体（会话归档时创建）",
        evidence: "dao/src/repo/conversation.rs:488",
        observed: true,
    },
    EntityTypeDecl {
        id: "qa_pair",
        meaning: "单轮 Q&A 对实体（会话实体的子节点）",
        evidence: "dao/src/repo/conversation.rs:690",
        observed: true,
    },
    EntityTypeDecl {
        id: "concept",
        meaning: "概念（LLM 抽取缺省类型：抽取结果未给类型时落此值）",
        evidence: "src/commands/knowledge_graph.rs:290 ｜ trajectory/src/nudge.rs:413",
        observed: true,
    },
    EntityTypeDecl {
        id: "language",
        meaning: "编程语言实体（ingest 流水线固定值）",
        evidence: "agent/src/ingest_pipeline.rs:1243",
        observed: true,
    },
    EntityTypeDecl {
        id: "module",
        meaning: "代码模块实体",
        evidence: "dao/tests/knowledge_graph_search.rs:50",
        observed: true,
    },
    EntityTypeDecl {
        id: "person",
        meaning: "人物（**来自数据文件列**：`nodes.csv` 的 `type` 列；DB 实测 62616 行）",
        evidence: "knowledge-sources/lemonhu/nodes.csv（type 列，20872 行）",
        observed: true,
    },
    EntityTypeDecl {
        id: "company",
        meaning: "公司（同上；DB 实测 9564 行）",
        evidence: "knowledge-sources/lemonhu/nodes.csv（type 列，3188 行）",
        observed: true,
    },
    EntityTypeDecl {
        id: "industry",
        meaning: "行业（同上；DB 实测 146 行）",
        evidence: "knowledge-sources/lemonhu/nodes.csv（type 列，49 行）",
        observed: true,
    },
    // ── 已观察到的**大小写不一致**形态（数据质量缺陷，登记为可见）──
    // DB 实测：`COMPANY` 2 行、`CONCEPT` 1 行 —— 与 `company` / `concept` 是同一个概念的两种写法。
    // 不做静默归一（那会改动存量数据），只登记 ⇒ 由 `validate_entity_type` 让它们可见。
    EntityTypeDecl {
        id: "COMPANY",
        meaning: "【大小写不一致】`company` 的重复写法（DB 实测 2 行）",
        evidence: "DB `knowledge_entities.entity_type` 分布（2026-09-14 实测）",
        observed: true,
    },
    EntityTypeDecl {
        id: "CONCEPT",
        meaning: "【大小写不一致】`concept` 的重复写法（DB 实测 1 行）",
        evidence: "DB `knowledge_entities.entity_type` 分布（2026-09-14 实测）",
        observed: true,
    },
    EntityTypeDecl {
        id: "organization",
        meaning: "【悬空】文档示例里的组织类型：**零写入点、DB 实测 0 行**（实测存在的组织类实体走 `company`）",
        evidence: "零写入点；DB `knowledge_entities.entity_type` 分布无此值（2026-09-14 实测）",
        observed: false,
    },
];

// ── 边类型词汇表（`GraphEdge.type`）──

/// **结构性边类型**的声明：由**后端代码**直接写进 `GraphEdge.type` 的取值。
#[derive(Debug, Clone, Copy)]
pub struct EdgeTypeDecl {
    pub id: &'static str,
    pub meaning: &'static str,
    /// `文件:行` 出处 —— 表内**不允许空串**（由 `edge_type_table_ok` 强制）
    pub evidence: &'static str,
    /// 是否存在**实际写入方**；`false` = 只在**前端**类型联合里声明过（**悬空契约**）
    pub observed: bool,
}

/// 结构性边类型表 —— `GraphEdge.type` 的取值域（**不含**关系类型，见下）。
///
/// # ⚠ 与 [`RELATION_DECLS`] 的边界（判据 K「一名多义」）
///
/// `GraphEdge.type` 与 `knowledge_relations.relation_type` **不是同一个东西**，
/// 2026-09-14 起更是分成了两个字段：
///
/// | | 本表 `EDGE_TYPE_DECLS` | [`RELATION_DECLS`] |
/// |---|---|---|
/// | 描述对象 | `GraphEdge.type`（**渲染类别**） | `knowledge_relations.relation_type`（**本体关系 id**） |
/// | 判据入口 | [`is_declared_edge_type`] | [`is_declared_relation`] |
/// | 值域 | `link` / `reference` / `mapping` … | `causes` / `has_concept` / `董事`（形态放行）… |
/// | 载体字段 | `GraphEdge.type` | `GraphEdge.relation_type`（可选） |
///
/// **为什么分成两个字段而不是合并**：合并过一次，代价是**全库 56 个关系类型
/// （112937 行）被抹平成一个常量** —— 见
/// `dao/src/repo/knowledge_graph.rs::get_knowledge_graph_edges_for_wiki` 的注释。
/// 渲染类别与本体身份是两件事：前者决定「画成什么样」，后者决定「这条边是什么关系」。
///
/// # 观测值（2026-09-14 全仓枚举，非猜测）
///
/// 生产写入点**只有 3 处**（测试构造点不计）：
/// `dao/src/repo/note.rs:563`（笔记链接 `link`）、
/// `dao/src/repo/knowledge_graph.rs:1794`（知识库实体关系 `reference`）、
/// `src/commands/wiki.rs:1186`（实体↔笔记标题匹配的合成边 `mapping`）。
///
/// 其余 3 条（`backlink` / `derived_from` / `contradicts`）**只在前端类型联合
/// `GraphEdgeType` 里声明过，后端零产出** —— 它们仍登记在这里（`observed: false`），
/// 因为「前端承诺的类型集」与「后端真的产出的类型集」的差集本身就是要可见的事实。
pub const EDGE_TYPE_DECLS: &[EdgeTypeDecl] = &[
    EdgeTypeDecl {
        id: "link",
        meaning: "笔记 ↔ 笔记的链接边（由 `note_links` 表构造）",
        evidence: "dao/src/repo/note.rs:563",
        observed: true,
    },
    EdgeTypeDecl {
        id: "reference",
        meaning: "知识库实体关系边的**渲染类别** —— 具体的本体关系 id 在 `GraphEdge.relation_type`",
        evidence: "dao/src/repo/knowledge_graph.rs:1794",
        observed: true,
    },
    EdgeTypeDecl {
        id: "mapping",
        meaning: "合成边：实体节点 ↔ 标题同名的笔记节点（消除孤岛，**不对应任何 DB 行**）",
        evidence: "src/commands/wiki.rs:1186",
        observed: true,
    },
    EdgeTypeDecl {
        id: "backlink",
        meaning: "【悬空】仅前端 `GraphView.tsx:164` 的 `GraphEdgeType` 声明过，**后端零写入点**",
        evidence: "前端声明 src/components/wiki/GraphView.tsx:164；全仓 grep 无生产写入点",
        observed: false,
    },
    EdgeTypeDecl {
        id: "derived_from",
        meaning: "【悬空】同上 —— 后端零写入点",
        evidence: "前端声明 src/components/wiki/GraphView.tsx:164；全仓 grep 无生产写入点",
        observed: false,
    },
    EdgeTypeDecl {
        id: "contradicts",
        meaning: "【悬空】同上 —— 后端零写入点（`trajectory/process_reward.rs:60` 的同名串是文本模式，不是边类型）",
        evidence: "前端声明 src/components/wiki/GraphView.tsx:164；全仓 grep 无生产写入点",
        observed: false,
    },
];

/// 结构性边类型表自身的一致性校验（纯函数）。
pub fn edge_type_table_ok(decls: &[EdgeTypeDecl]) -> bool {
    let mut seen: Vec<&str> = Vec::new();
    for d in decls {
        if d.id.is_empty() || d.meaning.is_empty() || d.evidence.is_empty() || seen.contains(&d.id)
        {
            return false;
        }
        seen.push(d.id);
    }
    true
}

// ── 违规类型与校验入口 ──

/// 图谱类型校验的违规项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphTypeViolation {
    /// 关系 id 不在登记表内（自由文本；**合法**但无法校验域/值域）
    UndeclaredRelationType { id: String },
    /// 关系 id 已登记，但源节点类超出定义域
    RelationDomainMismatch { id: String, source: GraphNodeType, allowed: &'static [GraphNodeType] },
    /// 关系 id 已登记，但目标节点类超出值域
    RelationRangeMismatch { id: String, target: GraphNodeType, allowed: &'static [GraphNodeType] },
    /// 实体类型未登记
    UndeclaredEntityType { id: String },
}

/// 词表查询：**同时接受**裸字面量（`part_of`）与历史 JSON 编码形式（`"part_of"`）。
///
/// 编码在 2026-09-14（D5）已收敛为单一裸字面量，但**存量数据**里可能仍有带引号的行，
/// 而读取端永久兼容两态 ⇒ 查询入口也保持宽容，调用方不必关心该行是哪个版本写的。
pub fn relation_decl(id: &str) -> Option<&'static RelationDecl> {
    relation_decls(id).into_iter().next()
}

/// 该 id 的**全部**登记（收敛后恒为 0 或 1 条）。
///
/// 保留「取全部」的形态是**回归保险**：`(id, encoding)` 曾唯一（`contains` 同时存在于
/// 两套编码下），`test_relation_ids_are_unique` 现在锁住「一个 id 只有一条登记」——
/// 哪天有人再引入第二套编码，这个入口能立刻暴露多出来的那条，而不是让调用方静默漏掉。
pub fn relation_decls(id: &str) -> Vec<&'static RelationDecl> {
    let raw = id.trim_matches('"');
    RELATION_DECLS.iter().filter(|d| d.id == raw).collect()
}

/// 关系 id 是否已登记（忽略编码差异）—— **登记项** 或 **前缀族** 命中其一即可。
pub fn is_declared_relation(id: &str) -> bool {
    relation_decl(id).is_some()
        || relation_family_of(id).is_some()
        // 第三支：**数据驱动列**（`edges.csv` 的 `rtype`）带来的开放词表形态。
        // 2026-09-14 实测补入：缺这一支时，CSV 导入的 22 个裸中文职位名 / 约 6 万行
        // 会被全量报成「未登记关系类型」—— 校验收到的第一条反馈就是刷屏。
        || is_data_driven_column_value(id)
}

/// 命中的前缀族（如 `employ_ceo` → `employ_` 族）。
pub fn relation_family_of(id: &str) -> Option<&'static RelationFamilyDecl> {
    let raw = id.trim_matches('"');
    RELATION_FAMILIES.iter().find(|f| raw.starts_with(f.prefix))
}

/// 实体类型是否已登记。
pub fn entity_type_decl(id: &str) -> Option<&'static EntityTypeDecl> {
    ENTITY_TYPE_DECLS.iter().find(|d| d.id == id)
}

/// 只校验 id（写入方拿不到节点类时用这个 —— 例如 `upsert_relation`）。
pub fn validate_relation_id(id: &str) -> Option<GraphTypeViolation> {
    if is_declared_relation(id) {
        None
    } else {
        Some(GraphTypeViolation::UndeclaredRelationType { id: id.to_string() })
    }
}

/// 由**自然键**导出确定性的关系主键（`knowledge_relations.id`）。
///
/// 自然键 = `(knowledge_base_id, source_entity_id, target_entity_id, relation_type)`。
///
/// # 为什么这个规则必须只有一处实现
///
/// `knowledge_relations` 的表级唯一键只有 `id`，因此「同一逻辑关系重复写入不得产生
/// 第二行」**完全**依赖「同一自然键每次派生出同一个 id」。这条规则若在两处各写一份，
/// 两处就会各自腐烂（一处改了分隔符、另一处没改 ⇒ 同一逻辑关系派生出两个键 ⇒
/// 边被静默拆成两行）。故规则收在契约层，写入方统一引用此处。
///
/// # 为什么分隔符是 `\u{1f}`（US，unit separator）
///
/// 四个分量都是**自由文本**（实体 id 与关系类型来自 LLM 抽取、CSV 导入、trajectory
/// 行为统计）。用 `|` / `:` 之类可打印字符分隔，`("a|b", "c")` 与 `("a", "b|c")`
/// 会派生出**同一个键** —— 两条不同的逻辑边被静默合并成一条。
/// `\u{1f}` 是 ASCII 控制字符，正常文本（含 LLM 输出）不会包含它。
///
/// # 为什么取 sha256 前 16 字节（128 bit）
///
/// 行数规模下碰撞概率可忽略；id 长度固定为 `rel_` + 32 hex，不随实体 id 长度膨胀
/// （直接拼接自然键会让 id 长度随输入无界增长，且实体 id 可含任意字符）。
///
/// # 反例：不要用 `Uuid::new_v4()` 之类随机值当主键
///
/// 随机主键 + `ON CONFLICT (id)` ⇒ 冲突永远不发生 ⇒ upsert 静默退化为纯 `INSERT`
/// ⇒ 表按「每次调用 × 每行」无界增长。实测形态见
/// `docs/plans/PLAN-memory-kb-reflow-id-space.md` §5d 类 C（`trajectory_patterns`
/// 2038 行却只有 3 个 `name`）。
pub fn stable_relation_id(
    kb_id: &str,
    source_id: &str,
    target_id: &str,
    relation_type: &str,
) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for part in [kb_id, source_id, target_id, relation_type] {
        h.update(part.as_bytes());
        h.update([0x1f]);
    }
    let digest = h.finalize();
    format!("rel_{}", hex::encode(&digest[..16]))
}

/// 实体类型校验（开放词表 ⇒ 只报「未登记」）。
pub fn validate_entity_type(id: &str) -> Option<GraphTypeViolation> {
    if entity_type_decl(id).is_some() {
        None
    } else {
        Some(GraphTypeViolation::UndeclaredEntityType { id: id.to_string() })
    }
}

/// 实体类型的**规范 id**：大小写不敏感地归一到登记表里的规范写法；未登记 ⇒ `None`。
///
/// # 为什么必须有它（「登记变体」不够）
///
/// DB 里同一个概念存在大小写变体（实测 `COMPANY` 2 行 / `CONCEPT` 1 行），而消费端
/// （`analysis-engine/src/knowledge_loader.rs`，**改造前**——刻意不标行号：该改造已落地，
/// 旧行号现在指向的是**新**代码，标了就是假锚，只会喂给 `check-single-source-facts.mjs`
/// 一条永远灭不掉的软告警）用精确 `== "company"` /
/// `== "concept"` 比字面量 ⇒ 大写变体**恒 false** ⇒ 这些实体不会被注册成概念节点、
/// 概念成员关系也建不起来（静默漏配，没有任何报错）。
///
/// 把变体**登记**进 `ENTITY_TYPE_DECLS`（`observed: true`）只让 `validate_entity_type`
/// 不再报它 —— 「登记」是承认它存在，**不等于**消费端认得它。这个区别就是本条存在的理由：
/// 只登记不归一 = 「问题可见了，但没人修」。
///
/// 归一方向：优先命中**本身就是规范写法**（全 ASCII 小写）的登记项 ——
/// 规范 id 在表里全是小写，变体条目（`COMPANY` / `CONCEPT`）带大写。
/// 本函数**不改写**未登记值（返回 `None`，交调用方决定）—— 开放词表里凭空造一个
/// 「规范形」等于发明约束。
///
/// # 与「不静默归一」的关系（别把两件事混为一谈）
///
/// `test_case_inconsistent_entity_types_are_registered_and_visible` 断言的是
/// **不静默合并登记项、不改存量数据** —— 本函数同样不碰数据、不动登记表，
/// 它只是一个**读取端比较用的纯函数**（`&str → Option<&'static str>`）。
/// 两者不冲突：登记表里 `COMPANY` 与 `company` 仍是两条 key（问题保持可见），
/// 而读取端比较时两者一致（漏配被修掉）。
pub fn normalize_entity_type(id: &str) -> Option<&'static str> {
    let t = id.trim();
    ENTITY_TYPE_DECLS
        .iter()
        .find(|d| d.id.eq_ignore_ascii_case(t) && d.id.to_ascii_lowercase() == d.id)
        .or_else(|| ENTITY_TYPE_DECLS.iter().find(|d| d.id == t))
        .map(|d| d.id)
}

/// 结构性边类型是否已登记（`GraphEdge.type` 的取值）。
pub fn is_declared_edge_type(id: &str) -> bool {
    EDGE_TYPE_DECLS.iter().any(|d| d.id == id)
}

/// 这条边的标签里，**后端解释不了的那个原值**；全部可解释 ⇒ `None`。
///
/// 判据（**只此一份**）：`edge_type` 必须命中 [`EDGE_TYPE_DECLS`]，
/// `relation_type`（若有）必须命中 [`is_declared_relation`]。
/// 两个都是既有词表的查询入口 —— 本函数不新增任何「什么算合法」的判断。
///
/// 检查顺序固定为**先 `edge_type` 后 `relation_type`** ⇒ 返回值确定（同一条边永远报同一个值），
/// 统计结果不会因为调用顺序变化而漂移。
///
/// # ⚠ 本函数**不**统计「前端没有专属配色」
///
/// 那是**前端词表**的问题，不是后端可解释性的问题。特别地：`is_declared_relation`
/// 按**形态**放行非 ASCII 值（数据驱动列，见 [`DATA_DRIVEN_COLUMN`]），
/// 所以 DB 实测的 53 个中文关系类型 / 74325 行在这里**全部算「可解释」**。
/// 把它们算成未识别会让本函数一出生就是对存量数据的刷屏器 ——
/// 那不是发现缺陷，那是制造噪音。需要知道「哪些关系类型没有专属样式」时，
/// 请查前端图例的关系类型分布，不要改这里。
pub fn uninterpreted_edge_label(edge_type: &str, relation_type: Option<&str>) -> Option<String> {
    if !is_declared_edge_type(edge_type) {
        return Some(edge_type.to_string());
    }
    match relation_type {
        Some(rt) if !is_declared_relation(rt) => Some(rt.to_string()),
        _ => None,
    }
}

/// 完整校验：id + 定义域 + 值域。
///
/// `domain`/`range` 为 `None` 的登记项**不校验**对应方向 —— 这是刻意的
/// （写入方不传节点类，凭空断言域/值域只会制造假阳性）。
pub fn validate_relation_write(
    id: &str,
    source: GraphNodeType,
    target: GraphNodeType,
) -> Vec<GraphTypeViolation> {
    let mut out = Vec::new();
    let Some(decl) = relation_decl(id) else {
        out.push(GraphTypeViolation::UndeclaredRelationType { id: id.to_string() });
        return out;
    };
    if let Some(allowed) = decl.domain
        && !allowed.contains(&source)
    {
        out.push(GraphTypeViolation::RelationDomainMismatch {
            id: decl.id.to_string(),
            source,
            allowed,
        });
    }
    if let Some(allowed) = decl.range
        && !allowed.contains(&target)
    {
        out.push(GraphTypeViolation::RelationRangeMismatch {
            id: decl.id.to_string(),
            target,
            allowed,
        });
    }
    out
}

/// 把违规清单落到日志（**不阻断写入** —— 见本模块节首的纪律 2）。
///
/// 放在契约层是为了让「报警格式」只有一份；调用方只需传一个上下文串。
pub fn warn_on_violations(context: &str, violations: &[GraphTypeViolation]) {
    for v in violations {
        tracing::warn!(context = context, violation = ?v, "图谱类型校验违规（未阻断写入）");
    }
}

/// 关系表自身的一致性校验（纯函数，便于负向对照）。
///
/// 判据：`id` **唯一**、`id` 非空且不含引号（引号只在 DB 层出现）、`evidence` 与 `meaning` 非空。
///
/// ⚠ **判据在 D5（2026-09-14）被收紧**：此前唯一键是 `(id, encoding)` —— 那允许同一个 id
/// 以两套编码各登记一条（`contains` 当时正是如此）。编码收敛后这个宽松度变成**漏洞**：
/// 有人再引入第二套编码时，表能过、`relation_decl` 只返回第一条 ⇒ 另一条链路静默消失。
/// 改为按 `id` 判唯一后，「同一 id 两条登记」直接是非法形态 ⇒ 收敛不可被悄悄逆转。
pub fn relation_table_ok(decls: &[RelationDecl]) -> bool {
    let mut seen: Vec<&str> = Vec::new();
    for d in decls {
        if d.id.is_empty() || d.id.contains('"') || d.evidence.is_empty() || d.meaning.is_empty() {
            return false;
        }
        if seen.contains(&d.id) {
            return false;
        }
        seen.push(d.id);
    }
    true
}

/// 前缀族表自身的一致性校验（纯函数）。
///
/// 判据：`prefix` 非空、以 `_` 结尾（防 `employ` 命中 `employer_x`）、互不包含、`evidence` 非空。
pub fn relation_family_table_ok(decls: &[RelationFamilyDecl]) -> bool {
    let mut seen: Vec<&str> = Vec::new();
    for d in decls {
        if d.prefix.is_empty()
            || !d.prefix.ends_with('_')
            || d.evidence.is_empty()
            || seen.contains(&d.prefix)
        {
            return false;
        }
        // 前缀互相包含 ⇒ 命中归属不确定
        if seen.iter().any(|p| d.prefix.starts_with(*p) || p.starts_with(d.prefix)) {
            return false;
        }
        seen.push(d.prefix);
    }
    true
}

/// 实体类型表自身的一致性校验（纯函数）。
pub fn entity_type_table_ok(decls: &[EntityTypeDecl]) -> bool {
    let mut seen: Vec<&str> = Vec::new();
    for d in decls {
        if d.id.is_empty() || d.evidence.is_empty() || seen.contains(&d.id) {
            return false;
        }
        seen.push(d.id);
    }
    true
}

#[cfg(test)]
mod type_vocabulary_tests {
    use super::*;

    #[test]
    fn test_tables_are_self_consistent() {
        assert!(relation_table_ok(RELATION_DECLS), "关系表自检失败（重复 / 空出处）");
        assert!(relation_family_table_ok(RELATION_FAMILIES), "前缀族表自检失败");
        assert!(entity_type_table_ok(ENTITY_TYPE_DECLS), "实体类型表自检失败");
    }

    #[test]
    fn test_table_validators_have_negative_control() {
        let dup = [
            RELATION_DECLS[0],
            RELATION_DECLS[1],
            RELATION_DECLS[1], // 同 id 重复
        ];
        assert!(!relation_table_ok(&dup), "同 id 重复必须红");

        // 反向对照（D5 收紧后新增）：同一 id 以**两套编码**各登记一条也必须红 ——
        // 这正是收敛前 `contains` 的形态。若哪天它又变绿，说明唯一键被改回 `(id, encoding)`。
        let mut second_encoding = RELATION_DECLS[0];
        second_encoding.encoding = RelationEncoding::JsonEncodedVariant;
        assert!(
            !relation_table_ok(&[RELATION_DECLS[0], second_encoding]),
            "同一 id 两条登记（无论编码是否相同）必须红 —— 收敛不得被逆转"
        );

        let mut no_evidence = RELATION_DECLS[0];
        no_evidence.evidence = "";
        assert!(!relation_table_ok(&[no_evidence]), "空出处必须红");

        let mut quoted = RELATION_DECLS[0];
        quoted.id = "\"causes\"";
        assert!(!relation_table_ok(&[quoted]), "id 不得自带引号（引号属 DB 编码层）");

        // 前缀族：缺下划线（`employ` 会错误命中 `employer_x`）
        let mut bad_prefix = RELATION_FAMILIES[0];
        bad_prefix.prefix = "employ";
        assert!(!relation_family_table_ok(&[bad_prefix]), "前缀必须以 `_` 结尾");

        // 前缀族互相包含 ⇒ 归属歧义
        // （用真实的子族形态：`employ_senior_` 以 `employ_` 开头 ⇒ 谁先匹配不确定）
        let mut sub_family = RELATION_FAMILIES[0];
        sub_family.prefix = "employ_senior_";
        assert!(
            !relation_family_table_ok(&[RELATION_FAMILIES[0], sub_family]),
            "前缀互相包含必须红"
        );

        let et_dup = [ENTITY_TYPE_DECLS[0], ENTITY_TYPE_DECLS[0]];
        assert!(!entity_type_table_ok(&et_dup), "实体类型重复必须红");
    }

    /// CSV 导入路径写入的关系必须已登记（2026-09-14 修正：`in_industry` **不是**悬空，
    /// 它在 `src/commands/knowledge.rs:1795` 有写入方）。
    ///
    /// ⚠ **本表对 `knowledge.rs` 的行号引用已漂移五次**：`1725 → 1738 → 1745 → 1749 → 1748`
    /// （第一次见 `AUDIT-ontology-p0123-execution-2026-09-14.md`；第二、三、四次都是
    /// `commands/knowledge.rs` 前段被编辑所致 —— 第三次的根因是把 `let _ =`（吞错）
    /// 改写成显式错误传播，净增 7 行；**第四次（2026-09-15）是给 `graph_import`
    /// 加一个参数，净增 4 行**，于是本文件里 4 处引用一起 +4）。
    ///
    /// ⚠⚠ **第五次（同日）换了机制：`cargo fmt`**。手改之外，**格式化同样会改行数** ——
    /// rustfmt 重排 `knowledge.rs` 前段后该文件净 **−1 行**，于是第四次刚修好的
    /// 4 处引用（1749/1778/1793/1809）**立刻又全错**，变成 1748/1777/1792/1808。
    /// 它被发现的代价很说明问题：`check-single-source-facts.mjs` 只从「软判据 3 条」
    /// 涨到「软判据 5 条」——**硬判据仍是 0**（这次漂移的落点是普通代码行，不是 `continue;`，
    /// 正落在该门禁的自陈盲区里）。⇒ 纪律两条：① **格式化必须排在修行号之前**（否则白修）；
    /// ② **跑完 `cargo fmt` 必须重跑行号引用门禁**，不能假定「只是空白变动」。
    /// ⇒ **改 `knowledge.rs` 前段必须回来对行号**；断言失败信息里已附「当前真值」，
    /// 照抄即可，不必手动数行。
    ///
    /// ⚠⚠ **第四次的教训（比行号本身重要）**：那次只有本用例罩着的
    /// `DATA_DRIVEN_COLUMN.evidence` 被硬拦并修好，另外三条 `evidence:` 引用
    /// （`in_industry` / `has_concept` / `employ_` 族）**静静腐烂了** ——
    /// 它们只走 `check-decl-evidence.mjs` 的软通道，而 `--ci` 把软判据降级成
    /// 「打印不失败」（判据 #147 的正当设计）⇒ **没人看就等于没检查**。
    /// 兜住它们的是 `check-single-source-facts.mjs` 新增的硬判据「引用指向裸控制流语句」
    /// （全仓实测 7/7 真阳性 ⇒ 满足 #147 的硬判据门槛）。**三处判据分工不同，缺一即盲区**：
    /// 结构面（`check-single-source-facts.mjs`）｜语义面（`check-decl-evidence.mjs`）｜
    /// 单点钉死（本用例）。
    #[test]
    fn test_csv_import_relations_are_declared() {
        for id in ["in_industry", "has_concept"] {
            let d = relation_decl(id).unwrap_or_else(|| panic!("{id} 未登记"));
            assert!(d.observed, "{id} 有 CSV 导入写入方 ⇒ observed 应为 true");
        }
        // `edges.csv` 的 rtype 是数据驱动 ⇒ 词表盖不住，只能靠「登记了这条路径」来记录。
        //
        // ⚠ 这里**不锁死行号**：锁死行号会让「引用腐烂」时测试照样绿 —— 2026-09-14 实测，
        // 原断言锁的是 `1725`，而 `1725` 实为 `if src.is_empty() … { continue; }` 里的
        // `continue;`，真正的 rtype 读取在 `1723`（由 `scripts/check-decl-evidence.mjs` 抓出）。
        // 改为按 evidence 给出的行号**去磁盘上读那一行**，断言它真的在讲 rtype。
        let ev = DATA_DRIVEN_COLUMN.evidence;
        let ln: usize = ev
            .split("knowledge.rs:")
            .nth(1)
            .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|s| s.parse().ok())
            .expect("数据驱动路径的 evidence 必须含 src/commands/knowledge.rs:<行号>");
        let cited = include_str!("../../../src/commands/knowledge.rs")
            .lines()
            .nth(ln - 1)
            .unwrap_or_else(|| {
                panic!("evidence 行号 {ln} 越界 ⇒ 引用已腐烂，请更新 DATA_DRIVEN_COLUMN.evidence")
            });
        assert!(
            cited.contains("rtype"),
            "evidence 指向的行不讲 rtype ⇒ 引用腐烂。第 {ln} 行实为：{cited}\n\
             （提示：真正读 `rtype` 的行形如 `let rtype = fields[2]`，它在{} ⇒
             把 `DATA_DRIVEN_COLUMN.evidence` 的行号改成这个数即可）",
            include_str!("../../../src/commands/knowledge.rs")
                .lines()
                .position(|l| l.contains("let rtype = fields[2]"))
                .map(|i| format!("第 {} 行", i + 1))
                .unwrap_or_else(|| "未找到（该行已被重写，判据本身要复核）".to_string())
        );
    }

    /// 数据驱动列的判据是**形态**（含非 ASCII），不是白名单 —— 正反例都要锁。
    ///
    /// 背景：`edges.csv` 的 `rtype` 列实测写入了 22 个裸中文职位名（约 6 万行），
    /// 这些值本仓库**无法穷举**（数据文件换一版值域就变了）⇒ 只能按形态放行；
    /// 而 ASCII 形态的值全部来自代码常量，可穷举 ⇒ 必须逐条登记，不得被这条判据漏掉。
    #[test]
    fn test_data_driven_column_values_are_shape_gated() {
        // 正例：真实存在的裸中文职位名（DB 实测分布里的真实取值）
        for v in ["董事", "独立董事", "董事长", "employ_董事"] {
            assert!(is_data_driven_column_value(v), "{v} 是数据文件写入的开放词表值 ⇒ 必须放行");
        }
        // 纯中文的几个必须靠**第三支**（形态）放行 —— 先把前缀族那两个入口排除掉，
        // 否则「放行」可能来自 `employ_` 族而不是数据驱动判据（测不到想测的东西）。
        for v in ["董事", "独立董事", "董事长"] {
            assert!(relation_decl(v).is_none(), "{v} 不在登记表内（它是开放式值）");
            assert!(relation_family_of(v).is_none(), "{v} 不命中任何前缀族");
            assert!(is_declared_relation(v), "{v} 只能由数据驱动判据放行 ⇒ 第三支必须已接线");
            assert!(validate_relation_id(v).is_none(), "{v} 不得被报成未登记");
        }
        // 正例：带引号的行（存量 JSON 编码）也要能穿透引号看到非 ASCII 部分
        assert!(is_data_driven_column_value("\"董事\""));

        // 反例：ASCII 拼出来的未登记值**不得**被形态判据放行 —— 否则整个校验等于关闭
        for v in ["totally_made_up", "in_industry_2", "", "  "] {
            assert!(!is_data_driven_column_value(v), "{v} 是 ASCII ⇒ 形态判据不得放行");
        }
        assert!(validate_relation_id("totally_made_up").is_some(), "ASCII 未登记值仍必须报出");
    }

    #[test]
    fn test_relation_prefix_family_matches_but_does_not_overreach() {
        for id in ["employ_ceo", "employ_cfo", "employ_board_secretary"] {
            let f = relation_family_of(id).unwrap_or_else(|| panic!("{id} 应命中 employ_ 族"));
            assert_eq!(f.prefix, "employ_");
            assert!(is_declared_relation(id), "{id} 应被判为已登记");
            assert!(validate_relation_id(id).is_none(), "{id} 不得报未登记");
        }

        // 负向对照：不以前缀开头、或前缀后无内容的，都不算命中
        assert!(relation_family_of("employer_acme").is_none(), "`employer_x` 不得命中 `employ_`");
        assert!(relation_family_of("employment").is_none());
        assert!(validate_relation_id("employer_acme").is_some());
    }

    /// 一个 id **只有一条**登记 —— D5 收敛后的新不变量。
    ///
    /// 收敛前 `contains` 有两套编码各一条（键是 `(id, encoding)`）；现在键是 `id`。
    /// 这条测试在两边都有反向对照：① 表里没有重 id；② 有人手工造重 id 时机器校验会红。
    #[test]
    fn test_relation_ids_are_unique() {
        let mut ids: Vec<&str> = RELATION_DECLS.iter().map(|d| d.id).collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), total, "关系表出现重 id ⇒ `relation_decl` 会静默只返回第一条");
        assert!(relation_table_ok(RELATION_DECLS), "关系表自检必须过（含 id 唯一）");
    }

    /// 收敛后 `contains` 只剩一条登记，但**两条链路的出处都必须留在 `evidence` 里**。
    ///
    /// 这是「合并登记」与「删掉一条链路」的分界：合并必须保出处可见。
    #[test]
    fn test_merged_declaration_keeps_every_chain_evidence() {
        let all = relation_decls("contains");
        assert_eq!(all.len(), 1, "收敛后 `contains` 只应有一条登记");
        let d = all[0];
        assert_eq!(d.encoding, RelationEncoding::Literal);
        assert!(d.evidence.contains("dao/src/repo/conversation.rs:720"), "会话容器链路的出处丢了");
        assert!(d.evidence.contains("entity.rs:72"), "记忆实体链路的出处丢了");
    }

    /// `observed: true` 的声明**不得**使用历史 JSON 编码。
    ///
    /// 判据依据：写入端（`trajectory/src/storage.rs::save_relationship`）已改用 `Display`
    /// 写裸字面量，因此任何「实际有写入方」的关系在 DB 里都是裸字面量；
    /// 若某条 `observed: true` 的声明仍标 `JsonEncodedVariant`，说明表的描述与写入端不一致。
    /// （`JsonEncodedVariant` 只允许出现在「只描述存量数据形态」的登记上，本表当前没有这种条目。）
    #[test]
    fn test_no_observed_decl_uses_the_legacy_encoding() {
        // 判据只写一份，真表与合成表共用（否则反向对照测的是另一份实现 ⇒ 测不到想测的东西）
        let offenders = |decls: &[RelationDecl]| -> Vec<String> {
            decls
                .iter()
                .filter(|d| d.observed && d.encoding == RelationEncoding::JsonEncodedVariant)
                .map(|d| d.id.to_string())
                .collect()
        };

        let bad = offenders(RELATION_DECLS);
        assert!(bad.is_empty(), "这些 observed 声明仍标历史编码 ⇒ 与写入端不符：{bad:?}");

        // 正向对照：`observed + JsonEncodedVariant` 必须被抓到
        let mut legacy = RELATION_DECLS[0];
        legacy.encoding = RelationEncoding::JsonEncodedVariant;
        assert_eq!(offenders(&[legacy]).len(), 1, "判据必须能识别该形态，否则它恒绿");

        // 假阳性对照：只用于描述**存量数据形态**的登记（`observed: false`）不得被误报 ——
        // 那正是保留 `JsonEncodedVariant` 变体的唯一目的。
        let mut legacy_unobserved = RELATION_DECLS[0];
        legacy_unobserved.encoding = RelationEncoding::JsonEncodedVariant;
        legacy_unobserved.observed = false;
        assert!(
            offenders(&[legacy_unobserved]).is_empty(),
            "未观察到的登记不得被判为「与写入端不符」"
        );
    }

    /// 查询入口必须容忍历史形态（带引号），编码属性**按 id 登记**、不由查询串推断。
    #[test]
    fn test_lookup_tolerates_legacy_quoted_form() {
        assert!(relation_decl("part_of").is_some());
        assert!(relation_decl("\"part_of\"").is_some(), "存量带引号行必须仍能查到声明");
        assert_eq!(relation_decl("part_of").unwrap().id, relation_decl("\"part_of\"").unwrap().id);
        assert_eq!(relation_decls("part_of").len(), relation_decls("\"part_of\"").len());

        assert_eq!(relation_decl("causes").map(|d| d.encoding), Some(RelationEncoding::Literal));
        assert_eq!(
            relation_decl("\"causes\"").map(|d| d.encoding),
            Some(RelationEncoding::Literal),
            "编码属性按 id 登记，不由查询串的引号推断"
        );
    }

    #[test]
    fn test_validate_relation_id_positive_and_negative() {
        assert!(validate_relation_id("causes").is_none());
        assert!(validate_relation_id("\"part_of\"").is_none());
        assert_eq!(
            validate_relation_id("totally_made_up"),
            Some(GraphTypeViolation::UndeclaredRelationType { id: "totally_made_up".to_string() })
        );
        assert!(validate_relation_id("").is_some(), "空 id 必须报出");
    }

    #[test]
    fn test_stable_relation_id_is_deterministic_and_every_component_matters() {
        let base = stable_relation_id("kb", "s", "t", "part_of");
        assert_eq!(
            base,
            stable_relation_id("kb", "s", "t", "part_of"),
            "同一自然键必须派生同一主键"
        );
        assert!(base.starts_with("rel_"), "{base}");
        assert_eq!(base.len(), 4 + 32, "id 长度必须固定（sha256 前 16 字节 = 32 hex）");

        // 四个分量任一变化都必须换键，否则「不同的边」会被合并成同一行
        assert_ne!(base, stable_relation_id("kb2", "s", "t", "part_of"));
        assert_ne!(base, stable_relation_id("kb", "s2", "t", "part_of"));
        assert_ne!(base, stable_relation_id("kb", "s", "t2", "part_of"));
        assert_ne!(base, stable_relation_id("kb", "s", "t", "related_to"));
        // 分量顺序参与派生：source/target 互换必须换键（有向边 ≠ 反向边）
        assert_ne!(base, stable_relation_id("kb", "t", "s", "part_of"));
    }

    #[test]
    fn test_stable_relation_id_separator_prevents_concatenation_collision() {
        // 若用 `|` 之类可打印字符作分隔符，下面两组会派生出**同一个键** ⇒ 两条不同的
        // 逻辑边被静默合并成一条。这是选 `\u{1f}` 的全部理由。
        let x = stable_relation_id("kb", "a|b", "c", "part_of");
        let y = stable_relation_id("kb", "a", "b|c", "part_of");
        assert_ne!(x, y, "自由文本分量不得因拼接歧义而撞键");
    }

    #[test]
    fn test_domain_range_enforced_only_where_declared() {
        // 已声明域/值域：越界必须报出
        let ok = validate_relation_write("contains", GraphNodeType::Entity, GraphNodeType::Entity);
        assert!(ok.is_empty(), "合法写入不得报违规：{ok:?}");

        let bad_range =
            validate_relation_write("contains", GraphNodeType::Entity, GraphNodeType::Note);
        assert_eq!(bad_range.len(), 1);
        assert!(matches!(bad_range[0], GraphTypeViolation::RelationRangeMismatch { .. }));

        let bad_domain =
            validate_relation_write("follows", GraphNodeType::Note, GraphNodeType::Entity);
        assert_eq!(bad_domain.len(), 1);
        assert!(matches!(bad_domain[0], GraphTypeViolation::RelationDomainMismatch { .. }));

        // 宽松登记项（域/值域未观察到）**不得**被凭空约束
        let permissive =
            validate_relation_write("part_of", GraphNodeType::Note, GraphNodeType::Note);
        assert!(permissive.is_empty(), "未声明域/值域的关系不得报错：{permissive:?}");

        // 未登记 id：只报「未登记」，不再叠加域/值域判断
        let undeclared =
            validate_relation_write("nope", GraphNodeType::Entity, GraphNodeType::Entity);
        assert_eq!(undeclared.len(), 1);
        assert!(matches!(undeclared[0], GraphTypeViolation::UndeclaredRelationType { .. }));
    }

    #[test]
    fn test_entity_type_open_vocabulary_reports_without_rejecting() {
        assert!(validate_entity_type("concept").is_none());
        assert!(validate_entity_type("qa_pair").is_none());
        assert!(matches!(
            validate_entity_type("whatever_llm_said"),
            Some(GraphTypeViolation::UndeclaredEntityType { .. })
        ));
    }

    /// 悬空契约清单必须与文档一致。
    ///
    /// 2026-09-14 修正：`in_industry` 曾被我误登记为悬空 —— 实际在
    /// `src/commands/knowledge.rs:1795` 有写入方（CSV 导入）。教训：
    /// **grep 字面量 `"in_industry"` 找不到写入方，不代表没有** ——
    /// 那条路径的值是 `format!` / `.into()` 构造出来的。悬空判定必须沿
    /// **数据来源**追，不能只 grep 常量。
    #[test]
    fn test_dangling_contracts_are_exactly_the_documented_ones() {
        let dangling: Vec<&str> =
            RELATION_DECLS.iter().filter(|d| !d.observed).map(|d| d.id).collect();
        assert_eq!(dangling, vec!["has_chairman"]);

        // 2026-09-14 修正：`person` 曾是悬空 —— 它由**数据文件**写入
        // （`knowledge-sources/lemonhu/nodes.csv` 的 `type` 列；DB 实测 62616 行）⇒ 改 observed: true。
        // 现在唯一悬空的是 `organization`（文档示例里的组织类型，零写入点、DB 实测 0 行）。
        let et_dangling: Vec<&str> =
            ENTITY_TYPE_DECLS.iter().filter(|d| !d.observed).map(|d| d.id).collect();
        assert_eq!(et_dangling, vec!["organization"]);

        // 边类型侧：`backlink` / `derived_from` / `contradicts` 只在前端类型联合里声明过，
        // 后端零写入点。锁成清单而不是「非空即可」—— 哪天有人接了其中一个，
        // 这条会红，逼他把 `observed` 改成 true 并补出处（否则表就开始撒谎）。
        let edge_dangling: Vec<&str> =
            EDGE_TYPE_DECLS.iter().filter(|d| !d.observed).map(|d| d.id).collect();
        assert_eq!(edge_dangling, vec!["backlink", "derived_from", "contradicts"]);
    }

    /// 边标签的可解释性判据（`uninterpreted_edge_label`）正/负对照。
    ///
    /// 与 A2 的 `test_unresolved_types_reported_sorted` 对称：那条管节点，这条管边。
    #[test]
    fn test_uninterpreted_edge_label_judges_both_fields() {
        // ① 结构类型 + 已登记关系 ⇒ 可解释
        assert_eq!(uninterpreted_edge_label("reference", Some("has_concept")), None);
        assert_eq!(uninterpreted_edge_label("reference", Some("in_industry")), None);
        assert_eq!(uninterpreted_edge_label("reference", Some("employ_ceo")), None, "前缀族");
        assert_eq!(uninterpreted_edge_label("link", None), None, "笔记链接不带关系类型");
        assert_eq!(uninterpreted_edge_label("mapping", None), None, "合成边");
        // 历史 JSON 编码形式仍可解释（读取端永久兼容两态）
        assert_eq!(uninterpreted_edge_label("reference", Some("\"part_of\"")), None);

        // ② 结构类型本身未登记 ⇒ 报结构类型的原值（**先查 type**）
        assert_eq!(
            uninterpreted_edge_label("unknown_kind", Some("has_concept")),
            Some("unknown_kind".to_string()),
            "type 未登记时必须报 type，且优先级高于 relation_type"
        );

        // ③ 结构类型合法但关系未登记 ⇒ 报关系原值
        assert_eq!(
            uninterpreted_edge_label("reference", Some("totally_made_up")),
            Some("totally_made_up".to_string())
        );

        // ④ **反向对照（口径边界）**：非 ASCII 关系值按形态放行 ⇒ 必须算「可解释」。
        //    这条锁的是「本函数不是『前端无配色』计数器」—— 把 DB 实测的 53 个中文
        //    关系类型算成未识别，会让它一出生就是刷屏器。
        assert_eq!(uninterpreted_edge_label("reference", Some("董事")), None);
        assert_eq!(uninterpreted_edge_label("reference", Some("独立董事")), None);

        // ⑤ 空串不算合法（它既不在边类型表、也不在关系表）
        assert_eq!(uninterpreted_edge_label("", None), Some(String::new()));
    }

    /// 结构性边类型表必须**恰好**覆盖前端 `GraphEdgeType` 联合类型的 6 个成员。
    ///
    /// 为什么值得锁：前端那个联合类型是**唯一**的「前端有配色的类型集」，
    /// 后端这张表是「后端真的会产出的类型集」。两者分叉的两种形态都有各自的坑 ——
    /// 前端多（出现永远不会有的筛选项）、后端多（前端静默落 fallback 样式）。
    /// 这里把两边的**键**对齐（`observed` 不同是**合法的**：见悬空三条），
    /// 值域差异由 `test_dangling_contracts_are_exactly_the_documented_ones` 单独锁。
    #[test]
    fn test_edge_type_table_covers_frontend_union_keys() {
        // 与 src/components/wiki/GraphView.tsx:164 的 `GraphEdgeType` 逐字一致。
        // ⚠ 前端改这个联合类型时必须同步本测试与 `EDGE_TYPE_DECLS`。
        let frontend_union =
            ["link", "backlink", "reference", "derived_from", "contradicts", "mapping"];
        let mut declared: Vec<&str> = EDGE_TYPE_DECLS.iter().map(|d| d.id).collect();
        declared.sort_unstable();
        let mut expected = frontend_union.to_vec();
        expected.sort_unstable();
        assert_eq!(declared, expected, "边类型表与前端 GraphEdgeType 联合的键必须一致");

        // 反向对照：表里少一个 / 多一个都必须判不等（防上面比较写反了还全绿）
        assert_ne!(declared, vec!["link", "reference", "mapping"], "缺 3 条必须不等");
    }

    /// 大小写不一致的实体类型必须**登记为可见**，而不是被静默归一。
    ///
    /// DB 实测：`COMPANY` 2 行 / `CONCEPT` 1 行（与 `company` / `concept` 是同概念的两种写法）。
    /// 静默归一会改动存量数据；登记则让 `validate_entity_type` 能报出来由人决定。
    #[test]
    fn test_case_inconsistent_entity_types_are_registered_and_visible() {
        for (upper, lower) in [("COMPANY", "company"), ("CONCEPT", "concept")] {
            let d = entity_type_decl(upper).unwrap_or_else(|| panic!("{upper} 未登记"));
            assert!(d.observed, "{upper} 有存量行 ⇒ observed 应为 true");
            assert!(d.meaning.contains("大小写不一致"), "{upper} 的登记必须点明它是重复写法");
            assert!(entity_type_decl(lower).is_some(), "{lower} 也必须同时登记（两者并存）");
            // 两者是**不同的** key ⇒ 不会被静默合并（这正是不做归一的证据）
            assert_ne!(upper, lower);
        }
    }

    /// 读取端归一：大小写变体必须**在使用侧**与规范写法等价。
    ///
    /// 这条与上一条是一对：上一条锁「登记表不合并、问题保持可见」，
    /// 本条锁「消费端做比较时两者一致、漏配被修掉」。
    /// 只有上一条 ⇒ 问题可见但没人修；只有本条 ⇒ 变体被悄悄吞掉、再也看不见。
    #[test]
    fn test_normalize_entity_type_aligns_case_variants_without_touching_data() {
        // 规范形自映射
        assert_eq!(normalize_entity_type("company"), Some("company"));
        assert_eq!(normalize_entity_type("concept"), Some("concept"));
        assert_eq!(normalize_entity_type("industry"), Some("industry"));
        assert_eq!(normalize_entity_type("qa_pair"), Some("qa_pair"));
        // 大小写变体归一到规范形（DB 实测存在的两个）
        assert_eq!(normalize_entity_type("COMPANY"), Some("company"));
        assert_eq!(normalize_entity_type("CONCEPT"), Some("concept"));
        // 其它大小写写法同样归一（LLM 抽取会给出各种拼法）
        assert_eq!(normalize_entity_type("Person"), Some("person"));
        assert_eq!(normalize_entity_type("  Industry  "), Some("industry"));

        // 未登记值**不得**被凭空规范化（归一不是「兜底成某个默认类型」）
        assert_eq!(normalize_entity_type("whatever_llm_said"), None);

        // 与「不静默归一数据」不冲突：登记表里两条 key 都还在，本函数不碰数据
        assert!(entity_type_decl("COMPANY").is_some(), "变体条目必须仍在表里（问题保持可见）");
        assert!(entity_type_decl("company").is_some(), "规范条目同样在表里");
        // 负对照：若哪天有人把归一写成「改登记表 / 删变体条目」，上面两条会先失败
    }

    /// 关系 id 常量必须与登记表一致（防「常量改了、表没改」）。
    #[test]
    fn test_constants_match_declarations() {
        for id in [RELATION_CONTAINS, RELATION_FOLLOWS, RELATION_MENTIONS] {
            assert!(is_declared_relation(id), "常量 {id} 不在登记表内");
        }
        assert_eq!(CAUSAL_RELATION_TYPE, "causes");
        assert!(is_declared_relation(CAUSAL_RELATION_TYPE));
    }

    /// 词汇表必须覆盖 `trajectory::RelationshipType` 的全部 11 个变体 ——
    /// 但本 crate **不能**依赖 trajectory（会成环），故这里硬编码那 11 个字面量。
    /// 该断言的作用是：trajectory 侧新增变体时，有人会被提醒来同步。
    ///
    /// D5（2026-09-14）后这些变体在 DB 里是**裸字面量**（写入端改用 `Display`），
    /// 故断言 `Literal` 而不是历史形态；「写入端是否真的改了」由
    /// `trajectory/src/storage.rs` 侧的测试负责，这里只管词表。
    #[test]
    fn test_all_trajectory_variants_are_declared() {
        let variants = [
            "part_of",
            "related_to",
            "depends_on",
            "owns",
            "defines",
            "implements",
            "contains",
            "calls",
            "method_of",
            "performs",
            "associated_with",
        ];
        for v in variants {
            let all = relation_decls(v);
            assert_eq!(all.len(), 1, "{v} 应有且只有一条登记");
            assert_eq!(all[0].encoding, RelationEncoding::Literal, "{v} 收敛后必须是裸字面量编码");
            assert!(all[0].observed, "{v} 由 trajectory 写入 ⇒ observed 应为 true");
        }
    }

    /// `relation_decls` 的契约在收敛后仍要成立：能取到全部、空 id 取空、带引号形式等价。
    ///
    /// 保留这个入口（而非只留 `relation_decl`）是**回归保险**：若哪天又出现同 id 两条登记，
    /// 这个入口能让它立刻可见，而不是让调用方静默只看到第一条。
    #[test]
    fn test_relation_decls_returns_exactly_one_per_id() {
        assert_eq!(relation_decls("contains").len(), 1, "收敛后 contains 只应有一条登记");
        assert_eq!(relation_decl("contains").map(|d| d.encoding), Some(RelationEncoding::Literal));
        assert_eq!(relation_decls("causes").len(), 1);
        assert!(relation_decls("nope").is_empty());
        assert!(relation_decls("").is_empty());
        // 带引号的历史形态：取到的条数与裸字面量一致（不因引号产生第二条/漏条）
        assert_eq!(relation_decls("\"contains\"").len(), relation_decls("contains").len());
    }
}
