// SPDX-License-Identifier: AGPL-3.0-only

// `Expr` 不在 `sea_orm::*` 的再导出里（`use sea_orm::*;` 之后仍要显式引入），
// 与本 crate 其它 repo 的写法一致（见 `repo/agent_session.rs`）。
use sea_orm::sea_query::Expr;
use sea_orm::*;

use axagent_entities::{
    knowledge_attributes, knowledge_entities, knowledge_flows, knowledge_interfaces,
    knowledge_relations,
};
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::graph_dtos::{GraphEdge, GraphNode};
use axagent_harness::types::{
    CreateKnowledgeAttributeInput, CreateKnowledgeEntityInput, CreateKnowledgeFlowInput,
    CreateKnowledgeInterfaceInput, CreateKnowledgeRelationInput, KnowledgeAttribute,
    KnowledgeEntity, KnowledgeFlow, KnowledgeInterface, KnowledgeRelation,
};
use axagent_harness::util_fns::gen_id;

/// Sentinel KB ID for trajectory-derived entities (v101 merge).
///
/// 权威定义在 [`axagent_harness::constants::sentinel`]；此处 re-export
/// 仅为保持既有跨 crate 引用路径
/// （`src/commands/proactive.rs` 用 `axagent_dao::repo::knowledge_graph::TRAJECTORY_KB_ID`）。
pub use axagent_harness::constants::sentinel::TRAJECTORY_KB_ID;

fn model_to_entity(m: knowledge_entities::Model) -> KnowledgeEntity {
    KnowledgeEntity {
        id: m.id,
        knowledge_base_id: m.knowledge_base_id,
        name: m.name,
        entity_type: m.entity_type,
        description: m.description,
        source_path: m.source_path,
        source_language: m.source_language,
        properties: m.properties,
        lifecycle: m.lifecycle,
        behaviors: m.behaviors,
        metadata: m.metadata,
        created_at: m.created_at,
        updated_at: m.updated_at,
        // v101: trajectory entity fields
        aliases: m.aliases,
        mention_count: m.mention_count,
        confidence: m.confidence,
        first_seen_at: m.first_seen_at,
        last_seen_at: m.last_seen_at,
    }
}

fn model_to_attribute(m: knowledge_attributes::Model) -> KnowledgeAttribute {
    KnowledgeAttribute {
        id: m.id,
        knowledge_base_id: m.knowledge_base_id,
        entity_id: m.entity_id,
        name: m.name,
        attribute_type: m.attribute_type,
        data_type: m.data_type,
        description: m.description,
        is_required: m.is_required,
        default_value: m.default_value,
        constraints: m.constraints,
        validation_rules: m.validation_rules,
        metadata: m.metadata,
        created_at: m.created_at,
        updated_at: m.updated_at,
    }
}

fn model_to_relation(m: knowledge_relations::Model) -> KnowledgeRelation {
    KnowledgeRelation {
        id: m.id,
        knowledge_base_id: m.knowledge_base_id,
        source_entity_id: m.source_entity_id,
        target_entity_id: m.target_entity_id,
        relation_type: m.relation_type,
        description: m.description,
        properties: m.properties,
        metadata: m.metadata,
        created_at: m.created_at,
        updated_at: m.updated_at,
        // v101: trajectory relationship weight
        weight: m.weight,
    }
}

fn model_to_flow(m: knowledge_flows::Model) -> KnowledgeFlow {
    KnowledgeFlow {
        id: m.id,
        knowledge_base_id: m.knowledge_base_id,
        name: m.name,
        flow_type: m.flow_type,
        description: m.description,
        source_path: m.source_path,
        steps: m.steps,
        decision_points: m.decision_points,
        error_handling: m.error_handling,
        preconditions: m.preconditions,
        postconditions: m.postconditions,
        metadata: m.metadata,
        created_at: m.created_at,
        updated_at: m.updated_at,
    }
}

fn model_to_interface(m: knowledge_interfaces::Model) -> KnowledgeInterface {
    KnowledgeInterface {
        id: m.id,
        knowledge_base_id: m.knowledge_base_id,
        name: m.name,
        interface_type: m.interface_type,
        description: m.description,
        source_path: m.source_path,
        input_schema: m.input_schema,
        output_schema: m.output_schema,
        error_codes: m.error_codes,
        communication_pattern: m.communication_pattern,
        version: m.version,
        metadata: m.metadata,
        created_at: m.created_at,
        updated_at: m.updated_at,
    }
}

// ── P1-write（2026-09-14）：实体类型写入观测 ──
//
// 背景：`ENTITY_TYPE_DECLS` 与 `validate_entity_type` 建于 B1，但**当时零消费** ——
// 读端（`knowledge_loader.rs` 的大小写比较）已在 P1 修好，写端一直没接。
// 「读端归一」只修了**已知**的漏配；「写端观测」让**新出现**的异形值在进入时可见 ——
// 这是两件事，缺一个都会留下盲区（前者管存量、后者管增量）。
//
// ⚠ **不阻断写入**（与 `upsert_relation` 同一纪律）：`entity_type` 是开放词表
// （LLM 抽取 + CSV 导入 + 会话记忆三条来源），硬拦会直接掐断链路。

/// 把单条违规送进「不阻断」通道 —— 唯一的出口是 `warn_on_violations`。
fn observe_violation(
    context: &str,
    violation: Option<axagent_harness::knowledge_graph::GraphTypeViolation>,
) {
    if let Some(v) = violation {
        axagent_harness::knowledge_graph::warn_on_violations(context, &[v]);
    }
}

/// 批量写入路径的实体类型观测：**按类型去重**后逐类报警。
///
/// 为什么必须去重：批量 upsert 一次可能带几百个实体，同一个未登记类型会重复几百次，
/// 日志被同一条刷满等于不可观测（这不是「逐条 warn」纪律的反例 ——
/// 那条针对**单次写入**，本条针对**批量**）。
///
/// 上限 `MAX_DISTINCT_UNREGISTERED` 防「每个实体一种随机类型」打爆日志；
/// 超出时**自报被截断的种数** —— 静默截断会让「只有 10 种未登记类型」变成假结论。
/// 报警的**种数**上限：防「每个实体一种随机类型」打爆日志。
pub const MAX_DISTINCT_UNREGISTERED: usize = 10;

/// **纯函数**（唯一判据）：从一批实体类型里取出「未登记」的**去重**清单。
///
/// 返回 `(按升序去重后的未登记类型，被上限截断的种数)`。
/// 抽成纯函数而不是「只打日志」的理由：**只产生日志的函数无法被测试**，
/// 而它恰恰是那条「写端让异形值可见」的判据 —— 没有测试就等于没人复核过它。
///
/// 判据本身只有一份：[`axagent_harness::knowledge_graph::validate_entity_type`]。
pub fn unregistered_entity_types<'a>(types: impl Iterator<Item = &'a str>) -> (Vec<String>, usize) {
    use axagent_harness::knowledge_graph::validate_entity_type;

    let mut unregistered: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for t in types {
        if validate_entity_type(t).is_some() {
            unregistered.insert(t.to_string());
        }
    }
    let total = unregistered.len();
    let kept: Vec<String> = unregistered.into_iter().take(MAX_DISTINCT_UNREGISTERED).collect();
    let suppressed = total - kept.len();
    (kept, suppressed)
}

/// 批量写入路径的实体类型观测：**按类型去重**后逐类报警，超上限自报截断数。
///
/// `pub` 是刻意的：命令层存在**直写**实体的路径
/// （`src/commands/knowledge.rs` 的 `graph_import` 在循环里直接 `ActiveModel::insert`，
/// 不走本模块的写函数 —— 实测 6 万+ 行）。让它调用**同一个** helper 而不是自己再写一份
/// 校验+去重，才谈得上「判据只有一份」。
///
/// 为什么要去重：批量 upsert 一次可能带几百个实体，同一个未登记类型会重复几百次，
/// 日志被同一条刷满等于不可观测（这不是「逐条 warn」纪律的反例 ——
/// 那条针对**单次写入**，本条针对**批量**）。
pub fn observe_entity_types<'a>(context: &str, types: impl Iterator<Item = &'a str>) {
    let (kept, suppressed) = unregistered_entity_types(types);

    for t in &kept {
        if let Some(v) = axagent_harness::knowledge_graph::validate_entity_type(t) {
            axagent_harness::knowledge_graph::warn_on_violations(context, &[v]);
        }
    }

    // 静默截断会让「只有 10 种未登记类型」变成假结论 ⇒ 必须自报。
    if suppressed > 0 {
        tracing::warn!(
            context,
            distinct_unregistered = kept.len() + suppressed,
            suppressed,
            "未登记实体类型种数超上限，已截断报警（真值见 knowledge_entities.entity_type 分布）"
        );
    }
}

pub async fn create_knowledge_entity(
    db: &DatabaseConnection,
    input: CreateKnowledgeEntityInput,
) -> Result<KnowledgeEntity> {
    let id = gen_id();
    let now = chrono::Utc::now().timestamp();

    observe_violation(
        "dao::repo::knowledge_graph::create_knowledge_entity",
        axagent_harness::knowledge_graph::validate_entity_type(&input.entity_type),
    );

    let am = knowledge_entities::ActiveModel {
        id: Set(id.clone()),
        knowledge_base_id: Set(input.knowledge_base_id),
        name: Set(input.name),
        entity_type: Set(input.entity_type),
        description: Set(input.description),
        source_path: Set(input.source_path),
        source_language: Set(input.source_language),
        properties: Set(input.properties),
        lifecycle: Set(input.lifecycle),
        behaviors: Set(input.behaviors),
        metadata: Set(input.metadata),
        created_at: Set(now),
        updated_at: Set(now),
        aliases: Set("[]".to_string()),
        mention_count: Set(1),
        confidence: Set(0.5),
        first_seen_at: Set(None),
        last_seen_at: Set(None),
        source_type: Set(String::from("knowledge_base")),
        source_id: Set(String::new()),
        node_type: Set(String::from(
            axagent_harness::knowledge_graph::GraphNodeType::Entity.as_str(),
        )),
        external_id: Set(None),
    };

    am.insert(db).await?;

    let model = knowledge_entities::Entity::find_by_id(&id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("KnowledgeEntity {}", id)))?;

    Ok(model_to_entity(model))
}

pub async fn list_knowledge_entities(
    db: &DatabaseConnection,
    base_id: &str,
) -> Result<Vec<KnowledgeEntity>> {
    let mut select =
        knowledge_entities::Entity::find().filter(knowledge_entities::Column::Lifecycle.is_null());
    if !base_id.is_empty() {
        select = select.filter(knowledge_entities::Column::KnowledgeBaseId.eq(base_id));
    }
    let models = select.order_by_asc(knowledge_entities::Column::Name).all(db).await?;

    Ok(models.into_iter().map(model_to_entity).collect())
}

pub async fn create_knowledge_attribute(
    db: &DatabaseConnection,
    input: CreateKnowledgeAttributeInput,
) -> Result<KnowledgeAttribute> {
    let id = gen_id();
    let now = chrono::Utc::now().timestamp();

    let am = knowledge_attributes::ActiveModel {
        id: Set(id.clone()),
        knowledge_base_id: Set(input.knowledge_base_id),
        entity_id: Set(input.entity_id),
        name: Set(input.name),
        attribute_type: Set(input.attribute_type),
        data_type: Set(input.data_type),
        description: Set(input.description),
        is_required: Set(input.is_required),
        default_value: Set(input.default_value),
        constraints: Set(input.constraints),
        validation_rules: Set(input.validation_rules),
        metadata: Set(input.metadata),
        created_at: Set(now),
        updated_at: Set(now),
    };

    am.insert(db).await?;

    let model = knowledge_attributes::Entity::find_by_id(&id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("KnowledgeAttribute {}", id)))?;

    Ok(model_to_attribute(model))
}

pub async fn list_knowledge_attributes(
    db: &DatabaseConnection,
    entity_id: &str,
) -> Result<Vec<KnowledgeAttribute>> {
    let models = knowledge_attributes::Entity::find()
        .filter(knowledge_attributes::Column::EntityId.eq(entity_id))
        .order_by_asc(knowledge_attributes::Column::Name)
        .all(db)
        .await?;

    Ok(models.into_iter().map(model_to_attribute).collect())
}

pub async fn create_knowledge_relation(
    db: &DatabaseConnection,
    input: CreateKnowledgeRelationInput,
) -> Result<KnowledgeRelation> {
    let id = gen_id();
    let now = chrono::Utc::now().timestamp();

    let am = knowledge_relations::ActiveModel {
        id: Set(id.clone()),
        knowledge_base_id: Set(input.knowledge_base_id),
        source_entity_id: Set(input.source_entity_id),
        target_entity_id: Set(input.target_entity_id),
        relation_type: Set(input.relation_type),
        description: Set(input.description),
        properties: Set(input.properties),
        metadata: Set(input.metadata),
        created_at: Set(now),
        updated_at: Set(now),
        weight: Set(1.0),
        source_type: Set(String::from("knowledge_base")),
        source_id: Set(String::new()),
    };

    am.insert(db).await?;

    let model = knowledge_relations::Entity::find_by_id(&id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("KnowledgeRelation {}", id)))?;

    Ok(model_to_relation(model))
}

pub async fn list_knowledge_relations(
    db: &DatabaseConnection,
    base_id: &str,
) -> Result<Vec<KnowledgeRelation>> {
    let models = knowledge_relations::Entity::find()
        .filter(knowledge_relations::Column::KnowledgeBaseId.eq(base_id))
        .all(db)
        .await?;

    Ok(models.into_iter().map(model_to_relation).collect())
}

pub async fn create_knowledge_flow(
    db: &DatabaseConnection,
    input: CreateKnowledgeFlowInput,
) -> Result<KnowledgeFlow> {
    let id = gen_id();
    let now = chrono::Utc::now().timestamp();

    let am = knowledge_flows::ActiveModel {
        id: Set(id.clone()),
        knowledge_base_id: Set(input.knowledge_base_id),
        name: Set(input.name),
        flow_type: Set(input.flow_type),
        description: Set(input.description),
        source_path: Set(input.source_path),
        steps: Set(input.steps),
        decision_points: Set(input.decision_points),
        error_handling: Set(input.error_handling),
        preconditions: Set(input.preconditions),
        postconditions: Set(input.postconditions),
        metadata: Set(input.metadata),
        created_at: Set(now),
        updated_at: Set(now),
    };

    am.insert(db).await?;

    let model = knowledge_flows::Entity::find_by_id(&id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("KnowledgeFlow {}", id)))?;

    Ok(model_to_flow(model))
}

pub async fn list_knowledge_flows(
    db: &DatabaseConnection,
    base_id: &str,
) -> Result<Vec<KnowledgeFlow>> {
    let models = knowledge_flows::Entity::find()
        .filter(knowledge_flows::Column::KnowledgeBaseId.eq(base_id))
        .order_by_asc(knowledge_flows::Column::Name)
        .all(db)
        .await?;

    Ok(models.into_iter().map(model_to_flow).collect())
}

pub async fn create_knowledge_interface(
    db: &DatabaseConnection,
    input: CreateKnowledgeInterfaceInput,
) -> Result<KnowledgeInterface> {
    let id = gen_id();
    let now = chrono::Utc::now().timestamp();

    let am = knowledge_interfaces::ActiveModel {
        id: Set(id.clone()),
        knowledge_base_id: Set(input.knowledge_base_id),
        name: Set(input.name),
        interface_type: Set(input.interface_type),
        description: Set(input.description),
        source_path: Set(input.source_path),
        input_schema: Set(input.input_schema),
        output_schema: Set(input.output_schema),
        error_codes: Set(input.error_codes),
        communication_pattern: Set(input.communication_pattern),
        version: Set(input.version),
        metadata: Set(input.metadata),
        created_at: Set(now),
        updated_at: Set(now),
    };

    am.insert(db).await?;

    let model = knowledge_interfaces::Entity::find_by_id(&id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("KnowledgeInterface {}", id)))?;

    Ok(model_to_interface(model))
}

pub async fn search_entities(
    db: &DatabaseConnection,
    kb_id: &str,
    query: &str,
    top_k: usize,
) -> Result<Vec<KnowledgeEntity>> {
    let all = list_knowledge_entities(db, kb_id).await?;
    let query_lower = query.to_lowercase();
    let keywords: Vec<&str> = query_lower.split_whitespace().collect();
    if keywords.is_empty() {
        let limited: Vec<_> = all.into_iter().take(top_k).collect();
        return Ok(limited);
    }
    let mut scored: Vec<(i64, KnowledgeEntity)> = all
        .into_iter()
        .map(|e| {
            let name_lower = e.name.to_lowercase();
            let desc_lower = e.description.as_deref().unwrap_or("").to_lowercase();
            let type_lower = e.entity_type.to_lowercase();
            let mut score: i64 = 0;
            for kw in &keywords {
                if name_lower.contains(kw) {
                    score += 10;
                }
                if desc_lower.contains(kw) {
                    score += 5;
                }
                if type_lower.contains(kw) {
                    score += 3;
                }
            }
            (score, e)
        })
        .filter(|(s, _)| *s > 0)
        .collect();
    scored.sort_by_key(|b| std::cmp::Reverse(b.0));
    Ok(scored.into_iter().take(top_k).map(|(_, e)| e).collect())
}

/// 支持类型过滤的图谱实体搜索。
///
/// 相比 `search_entities`，本函数支持按 `entity_type` 过滤，
/// 利用数据库层面的 WHERE 提前筛选，避免全表加载。
/// 当 `kb_id` 为空字符串时不按知识库过滤（搜索全部）。
pub async fn search_entities_with_filter(
    db: &DatabaseConnection,
    kb_id: &str,
    query: &str,
    top_k: usize,
    entity_type_filter: Option<&str>,
) -> Result<Vec<KnowledgeEntity>> {
    let mut select =
        knowledge_entities::Entity::find().filter(knowledge_entities::Column::Lifecycle.is_null());
    if !kb_id.is_empty() {
        select = select.filter(knowledge_entities::Column::KnowledgeBaseId.eq(kb_id));
    }

    if let Some(et) = entity_type_filter {
        select = select.filter(knowledge_entities::Column::EntityType.eq(et));
    }

    let limit = top_k as u64;
    select = select.limit(limit * 3);

    let models = select.all(db).await?;
    let query_lower = query.to_lowercase();
    let keywords: Vec<&str> = query_lower.split_whitespace().collect();

    let mut scored: Vec<(i64, KnowledgeEntity)> = models
        .into_iter()
        .map(model_to_entity)
        .map(|e| {
            let name_lower = e.name.to_lowercase();
            let desc_lower = e.description.as_deref().unwrap_or("").to_lowercase();
            let type_lower = e.entity_type.to_lowercase();
            let mut score: i64 = 0;
            for kw in &keywords {
                if name_lower.contains(kw) {
                    score += 10;
                }
                if desc_lower.contains(kw) {
                    score += 5;
                }
                if type_lower.contains(kw) {
                    score += 3;
                }
            }
            (score, e)
        })
        .filter(|(s, _)| *s > 0)
        .collect();

    scored.sort_by_key(|b| std::cmp::Reverse(b.0));
    Ok(scored.into_iter().take(top_k).map(|(_, e)| e).collect())
}

pub async fn list_knowledge_interfaces(
    db: &DatabaseConnection,
    base_id: &str,
) -> Result<Vec<KnowledgeInterface>> {
    let models = knowledge_interfaces::Entity::find()
        .filter(knowledge_interfaces::Column::KnowledgeBaseId.eq(base_id))
        .order_by_asc(knowledge_interfaces::Column::Name)
        .all(db)
        .await?;

    Ok(models.into_iter().map(model_to_interface).collect())
}

// ── v101: Trajectory-style entity operations ───────────────────────────────

/// Get a single entity by ID.
pub async fn get_entity_by_id(
    db: &DatabaseConnection,
    id: &str,
) -> Result<Option<KnowledgeEntity>> {
    let model = knowledge_entities::Entity::find_by_id(id).one(db).await?;
    Ok(model.map(model_to_entity))
}

/// Get all entities by knowledge_base_id, ordered by last_seen_at desc.
pub async fn get_all_entities_by_kb(
    db: &DatabaseConnection,
    kb_id: &str,
) -> Result<Vec<KnowledgeEntity>> {
    let models = knowledge_entities::Entity::find()
        .filter(knowledge_entities::Column::Lifecycle.is_null())
        .filter(knowledge_entities::Column::KnowledgeBaseId.eq(kb_id))
        .order_by_desc(knowledge_entities::Column::UpdatedAt)
        .all(db)
        .await?;
    Ok(models.into_iter().map(model_to_entity).collect())
}

/// Name-based search for entities (like trajectory storage's search_entities).
pub async fn search_entities_by_name(
    db: &DatabaseConnection,
    kb_id: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<KnowledgeEntity>> {
    let pattern = format!("%{}%", query);
    let models = knowledge_entities::Entity::find()
        .filter(knowledge_entities::Column::Lifecycle.is_null())
        .filter(
            knowledge_entities::Column::KnowledgeBaseId
                .eq(kb_id)
                .and(knowledge_entities::Column::Name.like(&pattern)),
        )
        .all(db)
        .await?
        .into_iter()
        .take(limit)
        .map(model_to_entity)
        .collect();
    Ok(models)
}

/// Upsert an entity —— 按**自然键 `(knowledge_base_id, name)`** 去重。
// 8 params justified: it maps 1:1 to the DB insert/upsert columns with distinct semantics.
///
/// ## 为什么必须按自然键（2026-09-17 修）
///
/// 原实现是「先 `gen_id()` 生成新 id，再把 `on_conflict` 打在 `Column::Id` 上」：
/// 冲突键就是**这一次刚生成的值** ⇒ 不存在第二行与它相同 ⇒ `on_conflict` **永不触发**
/// ⇒ 语义上等价于纯 `INSERT`（同一形态的另一例见 [`upsert_relation`]）。
///
/// 它在**周期性**调用方上发作：`reflow_memory_to_knowledge` 每轮跑一次，每轮都会为
/// 同一批 `(kb_id, name)` 再插一遍 ⇒ **数据无界增长**（原先该路径因外键违反恒空转，
/// 所以这个洞从未被暴露出来）。
///
/// ## 为什么不加 DB 唯一约束兜底（本轮取舍，明确登记）
///
/// 更彻底的做法是给 `knowledge_entities` 加 `UNIQUE(knowledge_base_id, name)` 并把
/// `on_conflict` 改打到它上。**本轮没做**，两个理由：
/// ① **存量数据风险**：该约束要求存量无重复行，而生产是 PG、本机无法查证
///    （SQLite 侧 `axagent.db` 该表 0 行且 schema 落后，不能代表 PG）⇒ 盲加可能让
///    引擎在启动时直接失败；
/// ② `crates/entities` 的 SeaORM 实体层目前**没有**复合唯一索引的写法（只有列级
///    `#[sea_orm(indexed)]`），绕道 L2 声明会显著放大改动面。
///
/// ⚠ **本实现因此不抗并发**：两个并发调用可能都 `find` 到 `None` 再各插一行。
/// 当前两个调用点都是**串行 for 循环** ⇒ 实际不会触发；
/// **新增任何并发调用方之前，必须先补上唯一约束。**
#[allow(clippy::too_many_arguments)]
pub async fn upsert_entity(
    db: &DatabaseConnection,
    kb_id: &str,
    name: &str,
    entity_type: &str,
    aliases: &str,
    confidence: f64,
    first_seen_at: Option<String>,
    last_seen_at: Option<String>,
) -> Result<KnowledgeEntity> {
    use sea_orm::sea_query::OnConflict;

    observe_violation(
        "dao::repo::knowledge_graph::upsert_entity",
        axagent_harness::knowledge_graph::validate_entity_type(entity_type),
    );

    let now = chrono::Utc::now().timestamp();

    // ① 自然键命中 ⇒ 更新既有行（不新增）。这是本函数幂等性的**唯一**来源。
    if let Some(existing) = knowledge_entities::Entity::find()
        .filter(knowledge_entities::Column::KnowledgeBaseId.eq(kb_id))
        .filter(knowledge_entities::Column::Name.eq(name))
        .one(db)
        .await?
    {
        let am = knowledge_entities::ActiveModel {
            id: Set(existing.id.clone()),
            // 命中即「又见了一次」——`mention_count` 是这条回流路径唯一能自增的信号。
            mention_count: Set(existing.mention_count + 1),
            confidence: Set(confidence),
            updated_at: Set(now),
            aliases: Set(aliases.to_string()),
            // ⚠ 只在**本次真的给了**时间戳时覆盖，否则用 `NotSet` 保留原值：
            //   两个调用点都传 `None`，无条件 `Set(None)` 会把已有值抹成 NULL。
            last_seen_at: match last_seen_at {
                Some(v) => Set(Some(v)),
                None => NotSet,
            },
            first_seen_at: match first_seen_at {
                Some(v) => Set(Some(v)),
                None => NotSet,
            },
            ..Default::default()
        };
        let updated = knowledge_entities::Entity::update(am).exec(db).await?;
        return Ok(model_to_entity(updated));
    }

    // ② 未命中 ⇒ 插入新行。
    let id = format!("ent_{}", gen_id());

    let am = knowledge_entities::ActiveModel {
        id: Set(id.clone()),
        knowledge_base_id: Set(kb_id.to_string()),
        name: Set(name.to_string()),
        entity_type: Set(entity_type.to_string()),
        description: Set(None),
        source_path: Set(String::new()),
        source_language: Set(None),
        properties: Set(serde_json::Value::Object(Default::default())),
        lifecycle: Set(None),
        behaviors: Set(None),
        metadata: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        aliases: Set(aliases.to_string()),
        mention_count: Set(1),
        confidence: Set(confidence),
        first_seen_at: Set(first_seen_at),
        last_seen_at: Set(last_seen_at),
        source_type: Set(String::from("knowledge_base")),
        source_id: Set(String::new()),
        node_type: Set(String::from(
            axagent_harness::knowledge_graph::GraphNodeType::Entity.as_str(),
        )),
        external_id: Set(None),
    };

    // ⚠ 这里的 `on_conflict(Id)` 打的是**上面刚生成的** id ⇒ **永不触发**；
    //   保留它只为防御「`gen_id()` 撞值」这一极低概率事件。幂等性**不靠它**，
    //   靠的是上面 ① 的自然键查找。
    knowledge_entities::Entity::insert(am)
        .on_conflict(
            OnConflict::column(knowledge_entities::Column::Id)
                .update_columns([
                    knowledge_entities::Column::Name,
                    knowledge_entities::Column::LastSeenAt,
                    knowledge_entities::Column::MentionCount,
                    knowledge_entities::Column::Confidence,
                    knowledge_entities::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(db)
        .await?;

    get_entity_by_id(db, &id)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("KnowledgeEntity {}", id)))
}

/// Delete an entity and cascade-delete its relations.
pub async fn delete_entity_cascade(db: &DatabaseConnection, id: &str) -> Result<()> {
    let txn = db.begin().await?;
    knowledge_relations::Entity::delete_many()
        .filter(
            knowledge_relations::Column::SourceEntityId
                .eq(id)
                .or(knowledge_relations::Column::TargetEntityId.eq(id)),
        )
        .exec(&txn)
        .await?;
    knowledge_entities::Entity::delete_by_id(id).exec(&txn).await?;
    txn.commit().await?;
    Ok(())
}

/// Upsert a relationship (trajectory-style save).
///
/// ## 为什么主键必须由自然键派生（2026-09-17 修）
///
/// 原实现是「先 `gen_id()` 生成新 id，再把 `on_conflict` 打在 `Column::Id` 上」：
/// 冲突键就是**这一次刚生成的值** ⇒ 不存在第二行与它相同 ⇒ `on_conflict` **永不触发**
/// ⇒ 语义上等价于纯 `INSERT`。同型的另一处（已修）见 [`upsert_entity`]。
///
/// 派生规则**只有一处实现**：`axagent_harness::knowledge_graph::stable_relation_id`
/// （契约层）。本文件**不要**再内联一份 —— `commands::memory` 的关系回流也引用它，
/// 两份实现一旦有一份改了分隔符，同一逻辑关系就会被派生成两个键、静默拆成两行。
///
/// ## 为什么这里用确定性主键，而 `upsert_entity` 用 read-then-write
///
/// 不是风格不一，是两个约束不同：
/// - `upsert_entity` 是**活链且在跑**，表里已有**随机 id 的存量行**。若改成确定性主键，
///   `find_by_id(确定性id)` 找不到那些旧行 ⇒ 反而会插出重复行。read-then-write 按
///   自然键 `find`，能命中存量行 ⇒ **无需数据迁移**。
/// - 本函数**零调用方**（全 workspace 仅此定义，无 `use`/调用点）⇒ 表里没有「由本函数
///   写入的存量行」需要被它找回来 ⇒ 确定性主键不需要迁移，且比 read-then-write
///   **更抗并发**（`on_conflict` 是原子的，不存在两个并发调用各自 find 到 `None` 再各插一行）。
///
/// ## 本函数没解决的（明确登记，勿误读为已修）
///
/// `knowledge_relations` 表上**仍无** `(knowledge_base_id, source, target, relation_type)`
/// 唯一约束，而该表有**四个写者**（本函数、`trajectory::storage::save_relationship`、
/// `trajectory::causal::observe_edge`、`commands::memory` 的关系回流）。
/// 确定性主键只保证**本函数自己**的幂等，**不保证**跨写者去重：
/// 别处以同一自然键但不同 id（如 `rel_causal_{uuid}`）写入时，仍会并存两行。
/// ⇒ **`relation_type = "causes"` 的边不要走本函数**，那条路径有自己的
///   read-modify-write（累加统计量）与 id 前缀，见 `axagent_trajectory::causal::observe_edge`。
///
/// ⚠ `knowledge_base_id` 写空串会**直接触发外键失败**（FK → `knowledge_bases(id)`，
///   `code 787`），与 `causal::observe_edge` 曾经的缺陷同型；这里写 sentinel。
pub async fn upsert_relation(
    db: &DatabaseConnection,
    source_id: &str,
    target_id: &str,
    relation_type: &str,
    weight: f64,
) -> Result<KnowledgeRelation> {
    use sea_orm::sea_query::OnConflict;

    // B1（2026-09-14）：`relation_type` 是自由文本列，写入前过一次**关系词表**。
    // 只校验 id（本函数的入参只有实体 id，拿不到节点类 ⇒ 域/值域无从校验）。
    // **不阻断写入**：该列天然开放（trajectory 记忆图谱、LLM 抽取、CSV 导入三条来源），
    // 硬拦会直接掐断链路；这里只把「未登记值」变成日志可见。
    // P1-write：出口收敛到 `observe_violation`（原先这里内联了 6 行同样的写法，
    // 与实体侧的观测是**两份**；收敛后「怎么报警」只有一处）。
    observe_violation(
        "dao::repo::knowledge_graph::upsert_relation",
        axagent_harness::knowledge_graph::validate_relation_id(relation_type),
    );

    let now = chrono::Utc::now().timestamp();
    // 主键由自然键派生 ⇒ 同一逻辑关系重复 upsert 才会命中 PK 冲突并更新。
    // 若用 `gen_id()` 每次新生成，冲突键就是本次刚生成的那个值 ⇒ 永不触发。
    let id = axagent_harness::knowledge_graph::stable_relation_id(
        TRAJECTORY_KB_ID,
        source_id,
        target_id,
        relation_type,
    );

    let am = knowledge_relations::ActiveModel {
        id: Set(id.clone()),
        knowledge_base_id: Set(TRAJECTORY_KB_ID.to_string()),
        source_entity_id: Set(source_id.to_string()),
        target_entity_id: Set(target_id.to_string()),
        relation_type: Set(relation_type.to_string()),
        description: Set(None),
        properties: Set(None),
        metadata: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        weight: Set(weight),
        source_type: Set(String::from("knowledge_base")),
        source_id: Set(String::new()),
    };

    knowledge_relations::Entity::insert(am)
        .on_conflict(
            OnConflict::column(knowledge_relations::Column::Id)
                .update_columns([
                    knowledge_relations::Column::Weight,
                    knowledge_relations::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(db)
        .await?;

    let model = knowledge_relations::Entity::find_by_id(&id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("KnowledgeRelation {}", id)))?;

    Ok(model_to_relation(model))
}

// ── LightRAG 图查询增强 ────────────────────────────────────────────────

/// 根据查询关键词在 knowledge_entities 表中检索实体，并扩展 1-hop 邻居关系。
///
/// 算法：
/// 1. 复用 `search_entities` 的关键词打分逻辑拿到 seed 实体（限制 top_k）
/// 2. 收集 seed 实体 id 集合
/// 3. 查询 knowledge_relations 中所有 source_entity_id 或 target_entity_id 命中的关系
/// 4. 反查邻居实体的 name/entity_type，组装 [`GraphEnhancedContextChunk`]
/// 5. 去重：同一实体可能被多个 seed 命中
pub async fn graph_enhanced_search(
    db: &DatabaseConnection,
    kb_id: &str,
    query: &str,
    top_k: usize,
    include_neighbors: bool,
) -> Result<Vec<axagent_harness::GraphEnhancedContextChunk>> {
    // 1. 关键词打分拿到 seed 实体
    let seeds = search_entities(db, kb_id, query, top_k).await?;
    if seeds.is_empty() {
        return Ok(Vec::new());
    }

    // 2. 收集 seed id → entity 映射（用于最终输出实体信息）
    let mut seed_map: std::collections::HashMap<String, KnowledgeEntity> =
        std::collections::HashMap::with_capacity(seeds.len());
    let mut seed_ids: Vec<String> = Vec::with_capacity(seeds.len());
    for e in seeds {
        seed_ids.push(e.id.clone());
        seed_map.insert(e.id.clone(), e);
    }

    // 3. 不需要邻居关系时，直接组装无 relations 的 chunk
    if !include_neighbors {
        let mut chunks: Vec<axagent_harness::GraphEnhancedContextChunk> =
            Vec::with_capacity(seed_map.len());
        for e in seed_map.values() {
            chunks.push(axagent_harness::GraphEnhancedContextChunk {
                entity_name: e.name.clone(),
                entity_type: e.entity_type.clone(),
                description: e.description.clone(),
                relations: Vec::new(),
                knowledge_base_id: e.knowledge_base_id.clone(),
            });
        }
        return Ok(chunks);
    }

    // 4. 查询所有命中 seed 的关系（双向）
    //
    // 分组语义必须是 `(kb_id = kb AND (source IN seeds OR target IN seeds))`。
    // 不能写成链式 `a.and(b).or(c)` —— 那会变成 `(kb AND source) OR target`，
    // OR 分支丢掉 kb_id 约束，导致跨知识库关系（含因果边，其 kb_id 为空串）泄露进检索。
    // 因果边（relation_type = "causes"）是行为统计而非文档知识，显式排除。
    let relations = knowledge_relations::Entity::find()
        .filter(
            Condition::all()
                .add(knowledge_relations::Column::KnowledgeBaseId.eq(kb_id))
                .add(
                    Condition::any()
                        .add(knowledge_relations::Column::SourceEntityId.is_in(seed_ids.clone()))
                        .add(knowledge_relations::Column::TargetEntityId.is_in(seed_ids.clone())),
                )
                .add(
                    knowledge_relations::Column::RelationType
                        .ne(axagent_harness::knowledge_graph::CAUSAL_RELATION_TYPE),
                ),
        )
        .all(db)
        .await?;

    // 5. 收集所有邻居实体 id（去重）
    let mut neighbor_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for r in &relations {
        if seed_map.contains_key(&r.source_entity_id) {
            // source 是 seed，target 是邻居
            neighbor_ids.insert(r.target_entity_id.clone());
        } else {
            // target 是 seed，source 是邻居
            neighbor_ids.insert(r.source_entity_id.clone());
        }
    }

    // 6. 批量反查邻居实体（用 IN 查询，参数化）
    let neighbor_ids_vec: Vec<String> = neighbor_ids.into_iter().collect();
    let neighbors: Vec<knowledge_entities::Model> = if neighbor_ids_vec.is_empty() {
        Vec::new()
    } else {
        knowledge_entities::Entity::find()
            .filter(knowledge_entities::Column::Lifecycle.is_null())
            .filter(knowledge_entities::Column::Id.is_in(neighbor_ids_vec))
            .all(db)
            .await?
    };
    let mut neighbor_map: std::collections::HashMap<String, knowledge_entities::Model> =
        std::collections::HashMap::with_capacity(neighbors.len());
    for m in neighbors {
        neighbor_map.insert(m.id.clone(), m);
    }

    // 7. 按 seed id 分组关系，组装 GraphRelationEdge
    // 对每个 seed，遍历所有关系：若 source == seed，则是出边（target 为邻居）；
    // 若 target == seed，则是入边（source 为邻居）。
    // 由于 GraphRelationEdge 字段名为 target_entity_name，对入边也按"另一端"语义填充。
    let mut rels_by_seed: std::collections::HashMap<
        String,
        Vec<axagent_harness::GraphRelationEdge>,
    > = std::collections::HashMap::new();
    for r in &relations {
        let (seed_id, other_id) = if seed_map.contains_key(&r.source_entity_id) {
            (r.source_entity_id.clone(), r.target_entity_id.clone())
        } else if seed_map.contains_key(&r.target_entity_id) {
            (r.target_entity_id.clone(), r.source_entity_id.clone())
        } else {
            continue;
        };
        let other_name =
            neighbor_map.get(&other_id).map(|m| m.name.clone()).unwrap_or_else(|| other_id.clone());
        let edge = axagent_harness::GraphRelationEdge {
            target_entity_name: other_name,
            relation_type: r.relation_type.clone(),
            description: r.description.clone(),
            weight: r.weight,
        };
        rels_by_seed.entry(seed_id).or_default().push(edge);
    }

    // 8. 组装最终 chunks（按 seed 原始顺序输出）
    let mut chunks: Vec<axagent_harness::GraphEnhancedContextChunk> =
        Vec::with_capacity(seed_map.len());
    for id in &seed_ids {
        let e = match seed_map.get(id) {
            Some(e) => e,
            None => continue,
        };
        let rels = rels_by_seed.remove(id).unwrap_or_default();
        chunks.push(axagent_harness::GraphEnhancedContextChunk {
            entity_name: e.name.clone(),
            entity_type: e.entity_type.clone(),
            description: e.description.clone(),
            relations: rels,
            knowledge_base_id: e.knowledge_base_id.clone(),
        });
    }

    Ok(chunks)
}

/// 构造图查询增强的可注入上下文文本
pub fn build_graph_context_text(
    kb_id: &str,
    chunks: &[axagent_harness::GraphEnhancedContextChunk],
) -> String {
    if chunks.is_empty() {
        return String::new();
    }
    let mut text = format!("[Knowledge Graph - {}]\n", kb_id);
    for chunk in chunks {
        let desc = chunk.description.as_deref().unwrap_or("");
        text.push_str(&format!("- {} ({}): {}\n", chunk.entity_name, chunk.entity_type, desc));
        for rel in &chunk.relations {
            let rel_desc = rel.description.as_deref().unwrap_or("");
            text.push_str(&format!(
                "  → {} [{}]: {}\n",
                rel.target_entity_name, rel.relation_type, rel_desc
            ));
        }
    }
    text
}

/// 合并 aliases：将已有 JSON 数组字符串与新 aliases 去重后合并，返回新的 JSON 数组字符串。
fn merge_aliases(existing: &str, new_aliases: &[String]) -> String {
    let mut set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    if let Ok(arr) = serde_json::from_str::<Vec<String>>(existing) {
        for a in arr {
            set.insert(a);
        }
    }
    for a in new_aliases {
        if !a.is_empty() {
            set.insert(a.clone());
        }
    }
    serde_json::to_string(&set.into_iter().collect::<Vec<_>>()).unwrap_or_else(|_| "[]".to_string())
}

/// 跨文档批量 upsert 实体与关系
///
/// 用于 LLM 抽取后的写入：对每个 [`ExtractedEntity`]，先按 (kb_id, name) 查询，
/// 存在则 mention_count += 1 并合并 aliases；不存在则新建。
/// 关系同理：按 (kb_id, source_entity_id, target_entity_id, relation_type) 去重。
///
/// `source_type` / `source_id` 落 v113 统一图谱字段，标识实体/关系的来源体系
/// （如 KB 抽取传 `("knowledge_base", "")`，Wiki 抽取传 `("wiki", wiki_id)`），
/// 避免 Wiki 实体被误标为 knowledge_base 而混入 KB 图谱。
pub async fn batch_upsert_entities_and_relations(
    db: &DatabaseConnection,
    kb_id: &str,
    source_type: &str,
    source_id: &str,
    entities: Vec<axagent_harness::ExtractedEntity>,
    relations: Vec<axagent_harness::ExtractedRelation>,
) -> Result<axagent_harness::ExtractEntitiesResult> {
    use axagent_harness::util_fns::gen_id;

    // P1-write：实体类型观测（**按类型去重**，批量路径不逐条报）。
    // 放在循环**之前**：这是唯一能一次看到全部待写类型的位置，
    // 且此时 `entities` 尚未被消费。
    observe_entity_types(
        "dao::repo::knowledge_graph::batch_upsert_entities_and_relations",
        entities.iter().map(|e| e.entity_type.as_str()),
    );

    let started_at = std::time::Instant::now();
    let mut new_entities: Vec<KnowledgeEntity> = Vec::new();
    let mut updated_entities: Vec<KnowledgeEntity> = Vec::new();
    let mut new_relations: Vec<KnowledgeRelation> = Vec::new();
    let skipped_chunks = 0u32;

    // 用事务保证原子性
    let txn = db.begin().await?;
    let now = chrono::Utc::now().timestamp();

    // 1. 实体 upsert：按 (kb_id, name) 去重
    // name → 最终 entity id（用于后续关系写入）
    let mut name_to_id: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    for ent in entities {
        if ent.name.is_empty() {
            continue;
        }
        // 按 (kb_id, name) 查询（参数化）
        // ⚠ 必须排除已合并行：否则新抽取的实体命中同名合并行后会被 update 进去
        //   （写入成功但读端过滤掉 ⇒ 数据静默丢失）。
        let existing = knowledge_entities::Entity::find()
            .filter(knowledge_entities::Column::Lifecycle.is_null())
            .filter(
                knowledge_entities::Column::KnowledgeBaseId
                    .eq(kb_id)
                    .and(knowledge_entities::Column::Name.eq(&ent.name)),
            )
            .one(&txn)
            .await?;

        if let Some(m) = existing {
            // 存在：mention_count += 1，合并 aliases，刷新 last_seen_at，
            // confidence 随 mention_count 单调上升（多次被抽到的实体更可信）
            let existing_desc_is_none = m.description.is_none();
            let merged_aliases = merge_aliases(&m.aliases, &ent.aliases);
            let new_mention_count = m.mention_count + 1;
            let new_confidence = mention_based_confidence(new_mention_count);
            let mut am: knowledge_entities::ActiveModel = m.into();
            am.aliases = Set(merged_aliases);
            am.mention_count = Set(new_mention_count);
            am.confidence = Set(new_confidence);
            am.last_seen_at = Set(Some(rfc3339_now()));
            am.updated_at = Set(now);
            // 若新抽取提供了 description 且原描述为空，则补上
            if existing_desc_is_none && !ent.description.is_empty() {
                am.description = Set(Some(ent.description.clone()));
            }
            let updated_model = am.update(&txn).await?;
            let entity = model_to_entity(updated_model);
            name_to_id.insert(entity.name.clone(), entity.id.clone());
            updated_entities.push(entity);
        } else {
            // 不存在：新建
            let id = gen_id();
            let aliases_str =
                serde_json::to_string(&ent.aliases).unwrap_or_else(|_| "[]".to_string());
            let description = if ent.description.is_empty() {
                None
            } else {
                Some(ent.description.clone())
            };
            let seen_at = Some(rfc3339_now());
            let am = knowledge_entities::ActiveModel {
                id: Set(id.clone()),
                knowledge_base_id: Set(kb_id.to_string()),
                name: Set(ent.name.clone()),
                entity_type: Set(ent.entity_type),
                description: Set(description),
                source_path: Set(String::new()),
                source_language: Set(None),
                properties: Set(serde_json::Value::Object(Default::default())),
                lifecycle: Set(None),
                behaviors: Set(None),
                metadata: Set(None),
                created_at: Set(now),
                updated_at: Set(now),
                aliases: Set(aliases_str),
                mention_count: Set(1),
                confidence: Set(mention_based_confidence(1)),
                first_seen_at: Set(seen_at.clone()),
                last_seen_at: Set(seen_at),
                source_type: Set(source_type.to_string()),
                source_id: Set(source_id.to_string()),
                node_type: Set(String::from(
                    axagent_harness::knowledge_graph::GraphNodeType::Entity.as_str(),
                )),
                external_id: Set(None),
            };
            let model = am.insert(&txn).await?;
            let entity = model_to_entity(model);
            name_to_id.insert(entity.name.clone(), entity.id.clone());
            new_entities.push(entity);
        }
    }

    // 2. 关系 upsert：按 (kb_id, source_entity_id, target_entity_id, relation_type) 去重
    for rel in relations {
        let rel_source_id = match name_to_id.get(&rel.source) {
            Some(id) => id.clone(),
            None => continue,
        };
        let target_id = match name_to_id.get(&rel.target) {
            Some(id) => id.clone(),
            None => continue,
        };
        if rel_source_id == target_id {
            continue;
        }
        // 查询是否已存在相同关系
        let existing_rel = knowledge_relations::Entity::find()
            .filter(
                knowledge_relations::Column::KnowledgeBaseId
                    .eq(kb_id)
                    .and(knowledge_relations::Column::SourceEntityId.eq(&rel_source_id))
                    .and(knowledge_relations::Column::TargetEntityId.eq(&target_id))
                    .and(knowledge_relations::Column::RelationType.eq(&rel.relation_type)),
            )
            .one(&txn)
            .await?;
        if existing_rel.is_some() {
            continue;
        }
        let rel_id = format!("rel_{}", gen_id());
        let am = knowledge_relations::ActiveModel {
            id: Set(rel_id.clone()),
            knowledge_base_id: Set(kb_id.to_string()),
            source_entity_id: Set(rel_source_id),
            target_entity_id: Set(target_id),
            relation_type: Set(rel.relation_type),
            description: Set(None),
            properties: Set(None),
            metadata: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            weight: Set(1.0),
            source_type: Set(source_type.to_string()),
            source_id: Set(source_id.to_string()),
        };
        let model = am.insert(&txn).await?;
        new_relations.push(model_to_relation(model));
    }

    txn.commit().await?;

    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    Ok(axagent_harness::ExtractEntitiesResult {
        new_entities,
        updated_entities,
        new_relations,
        skipped_chunks,
        elapsed_ms,
    })
}

/// 基于被提及次数的实体置信度：1 次提及 0.58，5 次及以上封顶 0.9。
/// 多次被独立抽取命中的实体更可信，confidence 不再恒为 0.5。
fn mention_based_confidence(mention_count: i32) -> f64 {
    0.5 + 0.4 * (mention_count.clamp(1, 5) as f64) / 5.0
}

/// 当前时间的 RFC3339 表示（first_seen_at / last_seen_at 为 TEXT 列）。
fn rfc3339_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

// ═══════════════════════════════════════════════════════════════════════════
// 知识库域一致性审计（B-3，2026-09-17）
//
// ## 为什么需要它
//
// 知识库（`knowledge_base_id`）在这套代码里**是命名空间，不是标签**，三条独立证据：
//   ① `upsert_entity` 的去重自然键是 `(knowledge_base_id, name)`（见该函数文档）；
//   ② 读端 `list_knowledge_entities` / `list_knowledge_relations` 都按它过滤；
//   ③ `knowledge_entities.knowledge_base_id` 上有指向 `knowledge_bases(id)` 的外键。
//
// 而**边**（`knowledge_relations`）也有自己的 `knowledge_base_id`。于是存在一类
// 只有「域」这个视角才能看见的缺陷：**边的域与它端点的域不一致** ——
// 那条边在 DB 里行行俱全、外键也不违反（两个 id 都存在），但在**任何**按域过滤的
// 读端上都画不出来，因为它的端点根本不在这个域的可见实体集里。
//
// 实测形态（2026-09-17）：某个 wiki 绑定的库有 38,171 条边，其端点 **100%**
// 属于另一个库（实体被一次跨库合并搬走了，边留下）⇒ 图谱界面「一条线都没有」，
// 而后端与前端**都没有任何一处统计过这件事**。本函数把它变成一等指标。
//
// ## 判据口径（⚠ 与 harness 的 `GraphData::dangling_edges` **不是同一个判据**）
//
// 本函数问的是「**DB 里**这条边的两端在**它自己的域**里可见吗」（`lifecycle IS NULL`
// 且 `knowledge_base_id` 等于边的域）。harness 那个问的是「**返回给前端的节点数组**里
// 有没有它的两端」。两者不同答案本身就是定位信息，对照表见 `DanglingEdgeSummary` 文档。
//
// ⚠ **最容易写错的一步**是把这里写成「端点在**全表**里存在吗」：
// 那样 f9b2b050（已于 2026-09-18 退役，见 `merge_entities_by_name` 的文档）的 38,171 条边**一条都不悬空**（端点行都在，只是属于别的库），
// 审计会报「全绿」而界面全空 —— 第一版只读探针正是这么写的，差点把根因判成前端问题。
// ═══════════════════════════════════════════════════════════════════════════

/// 单个知识库的域一致性实测（一行一域）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct KbDomainRow {
    pub knowledge_base_id: String,
    /// 本域可见（`lifecycle IS NULL`）的实体数 —— 即读端会返回的节点数
    pub visible_entities: usize,
    /// 本域的边总数
    pub edges: usize,
    /// **两端都在本域可见集**里的边数（能被画出来的那部分）
    pub edges_both_visible: usize,
    /// **两端都不在**本域可见集里的边数（最严重的一类）
    pub edges_both_missing: usize,
    /// 至少一端不在的边数（= `edges - edges_both_visible`）
    pub dangling: usize,
}

impl KbDomainRow {
    /// 悬空占比。**无边时返回 `0.0`**（不是 `1.0`、也不是 `NaN`）：
    /// 「这个域没有边」与「这个域的边全断了」是两件事，返回 1.0 会让空域冒充故障。
    pub fn dangling_ratio(&self) -> f64 {
        if self.edges == 0 {
            0.0
        } else {
            self.dangling as f64 / self.edges as f64
        }
    }

    /// 恒等式自检：`edges_both_visible + dangling == edges`。
    ///
    /// 这不是「顺便检查」，而是**定义**：两类互斥且穷尽。任一审计结果不满足它，
    /// 说明统计逻辑本身错了（记忆 #8：统计量恒等 ⇒ 先怀疑测量工具），
    /// 此时任何「悬空占比」的结论都不可引用。
    pub fn is_self_consistent(&self) -> bool {
        self.edges_both_visible + self.dangling == self.edges
    }
}

/// 审计**全部知识库**的域一致性，按 `knowledge_base_id` 升序（确定性）。
///
/// 两条口径要点（都踩过）：
/// * **按域算，不按全局算** —— 判据是「端点 ∈ 本域可见集」，不是「端点行存在」。
/// * **无边的域也要出现**（`edges = 0`），否则「这个域里一条边都没有」这件事
///   与「这个域不存在」在输出上无法区分 —— 而前者恰恰是「实体合并把边全带走」的
///   终态，正是要看见的东西。
pub async fn audit_kb_domain_consistency<C>(conn: &C) -> Result<Vec<KbDomainRow>>
where
    C: ConnectionTrait,
{
    let entities = knowledge_entities::Entity::find()
        .filter(knowledge_entities::Column::Lifecycle.is_null())
        .all(conn)
        .await?;

    let mut visible: std::collections::HashMap<String, std::collections::HashSet<&str>> =
        std::collections::HashMap::new();
    for e in &entities {
        visible.entry(e.knowledge_base_id.clone()).or_default().insert(e.id.as_str());
    }

    let mut rows: std::collections::BTreeMap<String, KbDomainRow> =
        std::collections::BTreeMap::new();
    // 先把「有可见实体」的域建出来（哪怕一条边都没有）
    for (kb, ids) in &visible {
        rows.insert(
            kb.clone(),
            KbDomainRow {
                knowledge_base_id: kb.clone(),
                visible_entities: ids.len(),
                edges: 0,
                edges_both_visible: 0,
                edges_both_missing: 0,
                dangling: 0,
            },
        );
    }

    let relations = knowledge_relations::Entity::find().all(conn).await?;
    let empty: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for r in &relations {
        let row = rows.entry(r.knowledge_base_id.clone()).or_insert_with(|| KbDomainRow {
            knowledge_base_id: r.knowledge_base_id.clone(),
            visible_entities: 0,
            edges: 0,
            edges_both_visible: 0,
            edges_both_missing: 0,
            dangling: 0,
        });
        let local = visible.get(&r.knowledge_base_id).unwrap_or(&empty);
        row.edges += 1;
        let src_visible = local.contains(r.source_entity_id.as_str());
        let tgt_visible = local.contains(r.target_entity_id.as_str());
        match (src_visible, tgt_visible) {
            (true, true) => row.edges_both_visible += 1,
            (false, false) => {
                row.edges_both_missing += 1;
                row.dangling += 1;
            },
            _ => row.dangling += 1,
        }
    }

    Ok(rows.into_values().collect())
}

/// 全库悬空边求和（审计结果 → 一个标量，供日志与 `MergeEntitiesResult` 用）。
///
/// ⚠ 求和**只用于展示**：验收判据必须走 [`dangling_regressions`]（逐域比）。
/// 总和口径下「A 域修好 100 条、B 域新断 120 条」会被看成 +20 的恶化和 -100 的改善
/// 相互抵消 —— 而实际发生的是「新增了一个断点域」。
fn sum_dangling(rows: &[KbDomainRow]) -> usize {
    rows.iter().map(|r| r.dangling).sum()
}

/// 逐域比对两次审计，返回**悬空数上升**的域 `(kb, before, after)`，按 kb 升序。
///
/// 只报「上升」：下降（修好了）是期望方向，不构成违规。
/// **按域比而不是比总和** —— 总和口径下「A 域修好 100 条、B 域新断 120 条」会显示为改善，
/// 而那次改动实际制造了一个新断点。
pub fn dangling_regressions(
    before: &[KbDomainRow],
    after: &[KbDomainRow],
) -> Vec<(String, usize, usize)> {
    let before_map: std::collections::HashMap<&str, usize> =
        before.iter().map(|r| (r.knowledge_base_id.as_str(), r.dangling)).collect();
    let mut out: Vec<(String, usize, usize)> = after
        .iter()
        .filter_map(|r| {
            let prev = before_map.get(r.knowledge_base_id.as_str()).copied().unwrap_or(0);
            if r.dangling > prev {
                Some((r.knowledge_base_id.clone(), prev, r.dangling))
            } else {
                None
            }
        })
        .collect();
    out.sort();
    out
}

/// P1-3: 跨源实体合并 — **按 `(knowledge_base_id, name, entity_type)`** 在知识图谱中查找
/// 重复实体并合并。
///
/// 解决 Wiki/KB/Memory 三套实体系统各自为政的问题。
/// 合并策略：
/// 1. 遍历所有**可见**实体，按 `(knowledge_base_id, name, entity_type)` 分组
/// 2. 同组内保留最早创建的实体作为"主实体"，其他实体合并到主实体
/// 3. 合并 aliases、description、mention_count 等字段
/// 4. 更新所有引用被合并实体的关系，指向主实体
/// 5. **验收**：合并前后逐域比对悬空边数，任一域上升即回滚（见下方「为什么不变量是必须的」）
///
/// # ⚠ 2026-09-17：分组键为什么必须带 `knowledge_base_id`
///
/// 原键是 `(name, entity_type)`，**不含 `knowledge_base_id`** ⇒ 同名同类型的实体
/// 会被**跨库合并**，被合并行软删（`lifecycle.merged_into = 主实体`），而
/// `update_relation_references` 只改边两端的引用 id、**不改 `knowledge_relations.knowledge_base_id`**
/// ⇒ 边留在了原域，端点却进了另一个域。
///
/// 后果（2026-09-17 实测，生产 PG；下表是**退役前**值 —— `f9b2b050`（「A股知识图谱」）已于 2026-09-18 退役：1 行 `merged_into` 改指 lemonhu 同指纹存活行，其余 24,271 文档 / 24,274 实体 / 6 关系在对拍确认 lemonhu 已持有同内容后删除。保留不改，它是「分组键必须带 kb」的证据；但别拿它再去库里找这个库）：
///
/// | 量 | `f9b2b050`（wiki 绑定的库） | `lemonhu_knowledge_graph`（对照） |
/// |---|---|---|
/// | 可见实体 | **4**（24,274 行里） | 22,608 |
/// | 本域边 | 38,171 | 74,766 |
/// | **两端都可见的边** | **0** | 74,490（99.6%） |
///
/// 读端按 `knowledge_base_id` 取节点、按同一个键取边 ⇒ 节点 4 个、边 38,171 条、
/// 两端全不在 ⇒ 界面「一个节点一条线都没有」，而工具栏照旧写着 `74711E`。
///
/// **这与本模块自身的语义直接矛盾**：`upsert_entity` 的去重自然键是
/// `(knowledge_base_id, name)`、读端也按 kb 过滤（三条独立证据见本文件上方的
/// 「知识库域一致性审计」段）。即**知识库是命名空间，不是标签** ——
/// 在命名空间上做全局合并，等于让「隔离」这件事只在写入端成立。
///
/// ## 跨库同名实体怎么办（**不静默丢弃**）
///
/// 不再合并，但**计数上报**（`cross_kb_groups_skipped`）并打 `warn` 日志。
/// 消除跨库重复的正确手段是**跨库实体链接**（把两份实体用一条 `mapping` 边关联），
/// 而不是把其中一份删掉 —— 后者会让「这个库自己的知识」永久消失，
/// 且反向映射**非 1:1**（实测 fanout 最大 11）⇒ 事后无法自动还原。
/// 跨库链接属独立工作，**本轮不做**，此处只保证缺陷不再扩大。
///
/// # 为什么不变量是必须的（B-1 的验收面）
///
/// 上表那个缺陷在合并**前后**是「同一条边、同一个 id、外键都不违反」——
/// 任何单点断言都抓不到它。唯一能抓住的是**域视角的完整性度量**：
/// 合并不该让任何域的「两端可见边数」下降。故本函数在事务内做前后两次
/// [`audit_kb_domain_consistency`]，任一域 `dangling` 上升即 `rollback` + `Err`。
///
/// 这是 fail-stop 而不是 `warn`：此刻提交下去的数据是**静默损坏**的
/// （界面全空、无日志、无告警），其修复成本远超一次合并失败的代价。
///
/// ⚠ 不变量在**同库合并**下恒成立（端点迁移不改变端点所属的域），
/// 所以它平时不会误报 —— 它只在「域被跨过」时触发。
///
/// 返回合并统计信息
#[derive(Debug, serde::Serialize)]
pub struct MergeEntitiesResult {
    pub groups_found: usize,
    pub entities_merged: usize,
    /// **真实**受影响边数（`UpdateResult::rows_affected` 求和）。
    ///
    /// ⚠ 2026-09-17 前这里是伪造的：`update_relation_references` 用
    /// `query_all_raw` 执行 UPDATE 并对每个分支「假设至少更新了一行」，返回 `Ok(1)`。
    /// 也就是说这个字段历史上恒等于 **2×合并实体数**（或 0），与真实边数无关 ——
    /// 任何基于它做的「合并影响面」判断都是错的。
    pub relations_updated: usize,
    /// 因**跨库**同名同类型而被跳过的分组数（B-1，2026-09-17 新增）
    pub cross_kb_groups_skipped: usize,
    /// 合并前全库悬空边总数（所有域求和）
    pub dangling_before: usize,
    /// 合并后全库悬空边总数；若发生回滚则与 `dangling_before` 相等
    pub dangling_after: usize,
}

pub async fn merge_duplicate_entities_across_all(
    db: &DatabaseConnection,
) -> Result<MergeEntitiesResult> {
    let started = std::time::Instant::now();
    let now = chrono::Utc::now().timestamp();

    let txn = db.begin().await?;

    // 0. 合并前的域一致性基线 —— 步骤 5 的不变量校验要用（B-1，2026-09-17）
    let audit_before = audit_kb_domain_consistency(&txn).await?;

    // 1. 收集所有实体
    // ⚠ 只取「活跃」实体（lifecycle IS NULL）。已合并行必须排除，否则同一组会被
    //   反复判为待合并分组、合并时间被无限刷新（本函数因此不具备幂等性）。
    let all_entities = knowledge_entities::Entity::find()
        .filter(knowledge_entities::Column::Lifecycle.is_null())
        .all(&txn)
        .await?;

    if all_entities.is_empty() {
        let d = sum_dangling(&audit_before);
        return Ok(MergeEntitiesResult {
            groups_found: 0,
            entities_merged: 0,
            relations_updated: 0,
            cross_kb_groups_skipped: 0,
            dangling_before: d,
            dangling_after: d,
        });
    }

    // 2. 按 (knowledge_base_id, name, entity_type) 分组
    //
    // ⚠ `knowledge_base_id` 必须在键里（2026-09-17 修，见函数文档）：
    //   去掉它就等于跨库合并，会让边的域与端点的域分离 ⇒ 边在任何按域过滤的读端上
    //   都画不出来，而库里一切「正常」（外键不违反、行都在）。
    use std::collections::HashMap;
    let mut groups: HashMap<(String, String, String), Vec<&knowledge_entities::Model>> =
        HashMap::new();
    for entity in &all_entities {
        let key =
            (entity.knowledge_base_id.clone(), entity.name.clone(), entity.entity_type.clone());
        groups.entry(key).or_default().push(entity);
    }

    // 2b. 跨库同名同类型：**不合并，只计数上报**
    //
    // 这是 P1-3 原意（消除跨源重复）与本轮修正（kb 是命名空间）的交接点：
    // 原意不被丢弃，只是**换一种手段**实现 —— 跨库重复应该用实体链接表达，
    // 而不是把其中一个库的实体删掉。此处把这批规模显式报出来，避免「静默不合并」
    // 让一个真实的数据质量问题从指标上消失。
    let mut kb_sets: HashMap<(String, String), std::collections::HashSet<&str>> = HashMap::new();
    for entity in &all_entities {
        kb_sets
            .entry((entity.name.clone(), entity.entity_type.clone()))
            .or_default()
            .insert(entity.knowledge_base_id.as_str());
    }
    let cross_kb_groups_skipped = kb_sets.values().filter(|kbs| kbs.len() >= 2).count();
    if cross_kb_groups_skipped > 0 {
        // 只给样例 —— 这类分组通常成百上千（实测同名同类型跨 2 库是常态），
        // 全量打日志会把真正要看的那行淹没。
        let mut sample: Vec<String> = kb_sets
            .iter()
            .filter(|(_, kbs)| kbs.len() >= 2)
            .map(|((name, ty), kbs)| format!("{name}({ty}) → {} 个库", kbs.len()))
            .collect();
        sample.sort();
        sample.truncate(5);
        tracing::warn!(
            count = cross_kb_groups_skipped,
            samples = %sample.join(" | "),
            "存在跨知识库的同名同类型实体：**按命名空间语义不合并**（kb 是隔离域，合并会让边的域与端点的域分离）；如需跨库去重请用实体链接"
        );
    }

    let mut groups_found = 0usize;
    let mut entities_merged = 0usize;
    let mut relations_updated = 0usize;

    // 3. 处理每个分组
    for (_key, mut entities) in groups {
        if entities.len() < 2 {
            continue;
        }
        groups_found += 1;

        // 按创建时间排序，最早的作为主实体
        entities.sort_by_key(|a| a.created_at);

        let main_entity = entities[0].clone();
        let main_id = main_entity.id.clone();

        // ⚠ mention / aliases / description 必须用**累加器**，不能用循环外快照。
        //   原实现每轮都算 `main_entity.mention_count + entity.mention_count` 再写回，
        //   而 `main_entity` 是循环外的副本 ⇒ 后一轮用旧快照覆盖前一轮结果，
        //   最终只有「最后一个被合并实体」的 mention 生效（实测活跃行多为 1、而组内总和 11，
        //   且 confidence 由 mention_count 推导 ⇒ 一并算错）。
        let mut acc_aliases = main_entity.aliases.clone();
        let mut acc_desc = main_entity.description.clone();
        let mut acc_mention = main_entity.mention_count;

        for entity in entities.iter().skip(1) {
            let merge_target_id = &entity.id;

            acc_aliases = merge_aliases_opt(&acc_aliases, &entity.aliases);
            acc_desc = match (&acc_desc, &entity.description) {
                (Some(a), Some(b)) if a.len() >= b.len() => Some(a.clone()),
                (_, Some(b)) => Some(b.clone()),
                (Some(a), None) => Some(a.clone()),
                _ => None,
            };
            acc_mention += entity.mention_count;

            // 更新关系：将所有引用被合并实体的关系改为引用主实体
            let updated_count = update_relation_references(&txn, merge_target_id, &main_id).await?;
            relations_updated += updated_count;

            // 软删除：**不再改写 knowledge_base_id**。
            //   该列是外键，塞哨兵值会让这条 FK 永远建不出来 —— 实测 `__merged__` 曾占
            //   51277/72816 = 70.4%，而父表 knowledge_bases 里根本没有这个 id。
            //   「已被合并」是**行状态**，用 lifecycle 表达；且只有在读端按
            //   `lifecycle IS NULL` 过滤之后，软删除才真正生效
            //   （原实现只在写入端写标记、读端零过滤 ⇒ 合并等于没做，已合并实体照样出现在
            //    列表/搜索/图查询里，而 mention 已累加到主实体 ⇒ 重复计数）。
            let mut del_am: knowledge_entities::ActiveModel = (*entity).clone().into();
            del_am.lifecycle = Set(Some(serde_json::json!({
                "merged_into": main_id.clone(),
                "merged_at_ms": now * 1000,
            })));
            del_am.updated_at = Set(now);
            del_am.update(&txn).await?;

            entities_merged += 1;
        }

        // 主实体一次性写回累加结果
        let mut am: knowledge_entities::ActiveModel = main_entity.clone().into();
        am.aliases = Set(acc_aliases);
        am.description = Set(acc_desc);
        am.mention_count = Set(acc_mention);
        am.updated_at = Set(now);
        am.update(&txn).await?;
    }

    // 5. 验收：逐域比对悬空边数，任一域上升即回滚（B-1）
    //
    // 这一步是**本函数的出口判据**，不是日志。理由见函数文档：被它拦下的那种损坏
    // （边的域 ≠ 端点的域）在库层面完全「合规」—— 外键不违反、行数不少、没有报错，
    // 唯一暴露它的观测面就是「按域算，有多少条边画得出来」。
    let audit_after = audit_kb_domain_consistency(&txn).await?;
    let dangling_before = sum_dangling(&audit_before);
    let dangling_after = sum_dangling(&audit_after);
    let regressions = dangling_regressions(&audit_before, &audit_after);
    if !regressions.is_empty() {
        txn.rollback().await?;
        let detail = regressions
            .iter()
            .map(|(kb, before, after)| format!("{kb}: {before} -> {after}"))
            .collect::<Vec<_>>()
            .join("; ");
        tracing::error!(
            regressions = regressions.len(),
            detail = %detail,
            "跨源实体合并会导致域一致性回归，已回滚（该缺陷在库层面无任何症状，只在此处可观测）"
        );
        return Err(AxAgentError::Validation(format!(
            "merge aborted: dangling edges increased in {} knowledge base(s): {detail}",
            regressions.len()
        )));
    }

    txn.commit().await?;

    tracing::info!(
        "[merge_entities] 合并完成: {} 个分组, {} 个实体, {} 个关系, 跨库分组跳过 {} 个, 悬空边 {} -> {}, 耗时 {}ms",
        groups_found,
        entities_merged,
        relations_updated,
        cross_kb_groups_skipped,
        dangling_before,
        dangling_after,
        started.elapsed().as_millis()
    );

    Ok(MergeEntitiesResult {
        groups_found,
        entities_merged,
        relations_updated,
        cross_kb_groups_skipped,
        dangling_before,
        dangling_after,
    })
}

/// 合并两个 aliases JSON 数组
fn merge_aliases_opt(a: &str, b: &str) -> String {
    let aliases_a: Vec<String> = serde_json::from_str::<Vec<String>>(a).unwrap_or_default();
    let aliases_b: Vec<String> = serde_json::from_str::<Vec<String>>(b).unwrap_or_default();

    let mut merged = aliases_a;
    for alias in aliases_b {
        if !merged.contains(&alias) {
            merged.push(alias);
        }
    }

    if merged.is_empty() {
        String::from("[]")
    } else {
        serde_json::to_string(&merged).unwrap_or_else(|_| String::from("[]"))
    }
}

/// 更新关系表中引用旧实体 ID 的记录，改为引用新的主实体 ID
async fn update_relation_references(
    txn: &DatabaseTransaction,
    old_id: &str,
    new_id: &str,
) -> Result<usize> {
    if old_id == new_id {
        return Ok(0);
    }
    // 空 id 会把边指向一个不存在的端点 —— 而这条边的「悬空」在本函数看来是成功的。
    // 宁可返回 0（调用方只会看到 relations_updated 少），也不要写入一条必然断链的引用。
    if new_id.is_empty() || old_id.is_empty() {
        tracing::warn!(
            old_id = %old_id,
            new_id = %new_id,
            "关系引用迁移收到空实体 id，已跳过（写入会产生必然断链的边）"
        );
        return Ok(0);
    }

    let now = chrono::Utc::now().timestamp();

    // ⚠ 用 SeaORM builder 而不是 `format!` 拼 SQL（2026-09-17）。
    //   原实现拼串 + `query_all_raw`，两个后果：
    //   ① 返回值与真实边数无关 —— 它 `match` 的是「UPDATE 有没有报错」，
    //      然后把每个分支硬编码成 `1`，注释写着「假设至少更新了一行」。
    //      `MergeEntitiesResult::relations_updated` 因此恒为 `2 × 合并实体数`。
    //   ② 值被拼进 SQL 文本 ⇒ 一个引号或一次形态意外就是**静默的全表误改**，
    //      而这里改的是「谁指向谁」——改错等于把知识图谱的边接错。
    //   builder 还顺带解决方言差异（原 `query_all_raw` 走的是原始文本）。
    let as_source = knowledge_relations::Entity::update_many()
        .col_expr(knowledge_relations::Column::SourceEntityId, Expr::value(new_id))
        .col_expr(knowledge_relations::Column::UpdatedAt, Expr::value(now))
        .filter(knowledge_relations::Column::SourceEntityId.eq(old_id))
        .exec(txn)
        .await?;

    let as_target = knowledge_relations::Entity::update_many()
        .col_expr(knowledge_relations::Column::TargetEntityId, Expr::value(new_id))
        .col_expr(knowledge_relations::Column::UpdatedAt, Expr::value(now))
        .filter(knowledge_relations::Column::TargetEntityId.eq(old_id))
        .exec(txn)
        .await?;

    Ok((as_source.rows_affected + as_target.rows_affected) as usize)
}

// ── Wiki 图谱融合：实体节点和关系边 ────────────────────────────────

/// 获取指定知识库下的实体，转换为图谱节点格式用于 Wiki 图谱融合。
pub async fn get_knowledge_graph_nodes_for_wiki(
    db: &DatabaseConnection,
    kb_id: &str,
) -> Result<Vec<GraphNode>> {
    let entities = list_knowledge_entities(db, kb_id).await?;

    let nodes = entities
        .into_iter()
        .map(|entity| GraphNode {
            id: entity.id,
            title: entity.name,
            node_type: "entity".to_string(),
            tags: vec![entity.entity_type],
            link_count: 0,
            backlink_count: 0,
            path: entity.source_path,
        })
        .collect();

    Ok(nodes)
}

/// 把 DB 里的关系标签转成传给前端的形态：去首尾空白、去历史 JSON 引号（D5 前的编码）。
///
/// 空值 ⇒ `None`（该边退回纯结构性边）。**DB 实测 0 行**（112937 行里
/// `btrim` 后为空的是 0、带引号的是 0），所以这是防御性分支 ——
/// 它存在是为了「哪天真出现空值时有日志」而不是静默假装成 `reference`。
fn normalize_relation_label(raw: &str) -> Option<String> {
    let t = raw.trim().trim_matches('"').trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// 获取指定知识库下的实体关系，转换为图谱边格式用于 Wiki 图谱融合。
///
/// # ⚠ 2026-09-14：这里曾把关系类型**读出来又扔掉**
///
/// 旧实现是 `edge_type: "reference".to_string()` —— 常量。`list_knowledge_relations`
/// 明明返回了 `relation_type`，此处却不用它 ⇒ **DB 实测 56 个关系类型 / 112937 行
/// 全部塌成同一个值**（`lemonhu_knowledge_graph` 一个库就有 74766 条边 / 55 个类型），
/// 图谱上「有概念」「属于行业」「高管任职」看起来一模一样，而信息在**出口**就已丢失，
/// 前端拿到的 JSON 里根本没有区分它们的依据、无从补救。
///
/// 现在拆成两件事（见 [`GraphEdge`] 的字段文档）：渲染类别仍是 `reference`（**零视觉变更**），
/// 真实本体关系 id 进 `relationType`。前端据此出图例分布与统计。
pub async fn get_knowledge_graph_edges_for_wiki(
    db: &DatabaseConnection,
    kb_id: &str,
) -> Result<Vec<GraphEdge>> {
    let relations = list_knowledge_relations(db, kb_id).await?;

    let edges = relations
        .into_iter()
        .map(|rel| match normalize_relation_label(&rel.relation_type) {
            Some(rt) => GraphEdge::relation(rel.source_entity_id, rel.target_entity_id, rt),
            None => {
                tracing::warn!(
                    kb_id = %kb_id,
                    relation_id = %rel.id,
                    "knowledge relation has a blank relation_type —— 该边退回纯结构性边（不静默假装成 reference）"
                );
                GraphEdge::structural(rel.source_entity_id, rel.target_entity_id, "reference")
            },
        })
        .collect();

    Ok(edges)
}

// ═══════════════════════════════════════════════════════════════════════════
// Memory → 知识图谱回流（**单一实现**）
//
// 原先这段逻辑在两处**逐字重复**：
//   * `src/init/services.rs` 的周期任务 `knowledge_consolidation`（步骤 3）
//   * `src/commands/knowledge_graph.rs` 的 Tauri 命令 `sync_memory_to_knowledge_graph`
//
// 两处犯的是**同一个错**：把 `memory_namespaces.id` 当成 `knowledge_bases.id` 用。
// 这是两个 id 空间（都是 TEXT ⇒ 类型系统拦不住），而 `knowledge_entities.knowledge_base_id`
// 上有指向 `knowledge_bases.id` 的外键 ⇒ 两方言上都会**外键违反**，
// 失败又被 `debug!` 吞掉 ⇒ 整条路径**静默空转**（判据：fail-silent 是 fail-open 的镜像形态）。
//
// 改一处漏一处等于没改（判据 #256：同型缺陷按量纲穷举），故收敛到本模块单一实现。
// ═══════════════════════════════════════════════════════════════════════════

/// Memory → 知识图谱回流的统计。
///
/// 单独定义而不复用 Tauri 命令层 DTO：两个调用点各自把它转成自己的返回类型，
/// dao 不必知道 UI 的形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryReflowStats {
    pub items_read: usize,
    pub entities_created: usize,
    pub failures: usize,
}

/// 回流口径：`importance` 阈值。
///
/// ⚠ **不得在实现里顺手调整**：多大算「高重要性」是**产品口径**，
/// 未经裁决不动（见 `PLAN-memory-kb-reflow-id-space.md` §4.3 与 §7）。
pub const REFLOW_IMPORTANCE_THRESHOLD: f64 = 0.7;

/// 回流口径：单轮条数上限。理由同 `REFLOW_IMPORTANCE_THRESHOLD`。
pub const REFLOW_MAX_ITEMS: u32 = 100;

/// 把高重要性 Memory 条目回流成知识图谱实体（`entity_type = "memory_item"`）。
///
/// ## `kb_id` 为什么是参数而不是写死常量
/// ① 测试可以用临时 KB 隔离；② 让「这批实体归属哪个 KB」在调用点**可见**——
/// 这正是原先被藏起来的地方（调用点各自拼了一个错的 id）。
/// 生产的两个调用点都必须传
/// `axagent_harness::constants::sentinel::MEMORY_REFLOW_KB_ID`。
///
/// ## 幂等性
/// 靠 `upsert_entity` 内部的自然键 `(kb_id, name)` 查找，不靠 `on_conflict`
/// （后者原先打在同一次新生成的 id 上，永不触发）。本函数所在的两个调用点都是
/// **周期性/可重复触发**的 ⇒ 「连跑两轮行数不涨」是本函数的验收判据。
///
/// ## 为什么失败只发**一条**汇总 warn
/// 逐条 `warn!` 在最坏情况下是 100 行/轮 的噪声 —— 那是把「静默」换成「吵闹」，
/// 不是修复。逐条细节留在 `debug!`，汇总走 `warn!` 并带上**首个**错误。
pub async fn reflow_memory_to_knowledge(
    db: &DatabaseConnection,
    kb_id: &str,
    importance_threshold: f64,
    max_items: u32,
) -> Result<MemoryReflowStats> {
    let items = crate::repo::memory::list_high_importance_items(
        db,
        Some(importance_threshold),
        Some(max_items),
    )
    .await?;

    let items_read = items.len();
    let mut entities_created = 0usize;
    let mut failures = 0usize;
    let mut first_error: Option<String> = None;

    for item in &items {
        // `name` = content 前 100 字 —— 这是**自然键的另一半**，与 `upsert_entity` 的去重
        // 口径耦合：内容微改即变新的 name，届时会被判成「另一个实体」而新增一行。
        // 这是**语义选择**不是 bug，但它是「同一记忆改了错字就多一条」的来源（见 §4.2）。
        let name: String = item.content.chars().take(100).collect();
        let confidence = item.importance.min(1.0);

        match upsert_entity(db, kb_id, &name, "memory_item", "[]", confidence, None, None).await {
            Ok(_) => entities_created += 1,
            Err(e) => {
                failures += 1;
                tracing::debug!(
                    "[memory_reflow] 单条回流失败 kb={} item={}: {}",
                    kb_id,
                    item.id,
                    e
                );
                if first_error.is_none() {
                    first_error = Some(e.to_string());
                }
            },
        }
    }

    if failures > 0 {
        tracing::warn!(
            "[memory_reflow] kb={} 回流部分失败：读取 {} 条，成功 {}，失败 {}；首个错误：{}",
            kb_id,
            items_read,
            entities_created,
            failures,
            first_error.as_deref().unwrap_or("(无)")
        );
    } else {
        tracing::info!(
            "[memory_reflow] kb={} 回流完成：读取 {} 条，成功 {}",
            kb_id,
            items_read,
            entities_created
        );
    }

    Ok(MemoryReflowStats { items_read, entities_created, failures })
}

#[cfg(test)]
mod memory_reflow_tests {
    use super::*;
    use axagent_harness::constants::sentinel::MEMORY_REFLOW_KB_ID;

    /// **幂等性**：同一 `(kb_id, name)` 连续 upsert，只应存在 1 行。
    ///
    /// ⚠ 本测试是 `PLAN-memory-kb-reflow-id-space.md` §4.2 前置检查的**验收式**：
    /// 原实现的 `on_conflict` 打在同一次新生成的随机 id 上 ⇒ **永不触发** ⇒
    /// 两次调用会留下 **2** 行。改按自然键 `(kb_id, name)` 查找后必须只剩 1 行。
    ///
    /// 为什么必须**连调两次**而不是「调一次看有没有行」：调一次时「有行」对
    /// 「插入分支写对了」和「去重分支写对了」**同样成立**（假绿）；只有第二次
    /// 才能把两者区分开。这与 `seed.rs` 的「先破坏再修复」是同一条纪律。
    #[tokio::test]
    async fn upsert_entity_is_idempotent_on_natural_key() {
        let handle = crate::db::create_test_pool().await.expect("测试库应可建立");
        let db = &handle.conn;

        let first = upsert_entity(
            db,
            MEMORY_REFLOW_KB_ID,
            "同一条记忆",
            "memory_item",
            "[]",
            0.8,
            None,
            None,
        )
        .await
        .expect("首次 upsert 应成功");
        let second = upsert_entity(
            db,
            MEMORY_REFLOW_KB_ID,
            "同一条记忆",
            "memory_item",
            "[]",
            0.9,
            None,
            None,
        )
        .await
        .expect("二次 upsert 应成功");

        assert_eq!(first.id, second.id, "同一自然键的两次 upsert 必须落在**同一行**上");
        assert_eq!(second.mention_count, 2, "二次命中应把 mention_count 自增到 2");
        assert!(
            (second.confidence - 0.9).abs() < 1e-9,
            "二次 upsert 应更新 confidence（实测 {}，期望 0.9）",
            second.confidence
        );

        let rows = knowledge_entities::Entity::find()
            .filter(knowledge_entities::Column::KnowledgeBaseId.eq(MEMORY_REFLOW_KB_ID))
            .filter(knowledge_entities::Column::Name.eq("同一条记忆"))
            .count(db)
            .await
            .expect("计数应成功");
        assert_eq!(rows, 1, "自然键下必须只有 1 行 —— 原先这里会累积成 2 行（无界增长）");

        // 反向对照：**不同** name 仍应各占一行。
        // ⚠ 缺了这条，「去重」被误写成「恒返回第一行 / 全表只留一行」也会通过。
        upsert_entity(db, MEMORY_REFLOW_KB_ID, "另一条记忆", "memory_item", "[]", 0.7, None, None)
            .await
            .expect("不同 name 的 upsert 应成功");
        let all = knowledge_entities::Entity::find()
            .filter(knowledge_entities::Column::KnowledgeBaseId.eq(MEMORY_REFLOW_KB_ID))
            .count(db)
            .await
            .expect("计数应成功");
        assert_eq!(all, 2, "不同 name 必须各占一行（去重不得退化成「只留一行」）");
    }

    /// 哨兵 KB 行必须真实存在 —— 否则上面那条测试会因**外键**（或未来的外键强制）
    /// 直接失败，而失败信息会指向 upsert 而非「seed 没播种」。
    ///
    /// 单列一条是为了让失败**指向正确的模块**：本条的失败形态是「seed 漏播」，
    /// 不是「upsert 写错」。判据 #112 的形态：把两种失败分开，别让一个掩盖另一个。
    #[tokio::test]
    async fn reflow_sentinel_kb_exists() {
        use axagent_entities::knowledge_bases;
        use sea_orm::EntityTrait;

        let handle = crate::db::create_test_pool().await.expect("测试库应可建立");
        let row = knowledge_bases::Entity::find_by_id(MEMORY_REFLOW_KB_ID)
            .one(&handle.conn)
            .await
            .expect("查询应成功")
            .expect("回流哨兵 KB 行必须由 ensure_sentinels 播种 —— 缺行 ⇒ 回流路径外键违反");
        assert!(
            !row.name.is_empty(),
            "回流 KB 的 name 不得为空（空名会让它在知识库列表里不可辨认）"
        );
    }
}

#[cfg(test)]
mod relation_upsert_tests {
    use super::*;

    /// 取某条关系的 DB 行（刻意**不**依赖 DTO 字段形态，只断言库里的真值）。
    async fn row_of(
        db: &sea_orm::DatabaseConnection,
        src: &str,
        tgt: &str,
        rt: &str,
    ) -> knowledge_relations::Model {
        knowledge_relations::Entity::find()
            .filter(knowledge_relations::Column::SourceEntityId.eq(src))
            .filter(knowledge_relations::Column::TargetEntityId.eq(tgt))
            .filter(knowledge_relations::Column::RelationType.eq(rt))
            .one(db)
            .await
            .expect("查询应成功")
            .expect("关系行必须存在")
    }

    /// **幂等性**：同一自然键连续 upsert，只应存在 1 行。
    ///
    /// ⚠ 这是 `PLAN-memory-kb-reflow-id-space.md` §5d 类 C-#1 的**验收式**：
    /// 原实现的 `on_conflict` 打在同一次新生成的 `gen_id()` 上 ⇒ **永不触发** ⇒
    /// 两次调用会留下 **2** 行。改按自然键派生主键后必须只剩 1 行。
    ///
    /// 为什么必须**连调两次**而不是「调一次看有没有行」：调一次时「有行」对
    /// 「插入分支写对了」与「冲突分支写对了」**同样成立**（假绿）；只有第二次
    /// 才能把两者区分开。与 `upsert_entity_is_idempotent_on_natural_key` 同一条纪律。
    #[tokio::test]
    async fn upsert_relation_is_idempotent_on_natural_key() {
        let handle = crate::db::create_test_pool().await.expect("测试库应可建立");
        let db = &handle.conn;

        upsert_relation(db, "e_a", "e_b", "part_of", 0.5).await.expect("首次 upsert 应成功");
        upsert_relation(db, "e_a", "e_b", "part_of", 0.9).await.expect("二次 upsert 应成功");

        // ⚠ **行数断言必须排在 weight 断言之前**（2026-09-17 变异实证的产出）：
        //   把主键改回随机（模拟原缺陷）后实测，测试**先在 weight 断言处红**（`实测 0.5`）——
        //   因为 `row_of` 用 `.one()`，而 `.one()` 在多行时**静默取第一行**，
        //   于是「有 2 行」这个真根因被掩盖成「weight 没更新」。
        //   先断行数，失败信息才直指幂等性本身。
        let rows = knowledge_relations::Entity::find().count(db).await.expect("计数应成功");
        assert_eq!(rows, 1, "同一自然键必须只有 1 行 —— 原先每次调用都会新增一行");

        let r = row_of(db, "e_a", "e_b", "part_of").await;
        assert!((r.weight - 0.9).abs() < 1e-9, "二次 upsert 应更新 weight（实测 {}）", r.weight);

        // 反向对照：自然键的每个分量都必须参与派生。
        // ⚠ 缺了这些，「去重」被误写成「全表只留一行 / 恒返回第一行」也会通过。
        upsert_relation(db, "e_a", "e_c", "part_of", 0.5).await.expect("不同 target 应成功");
        upsert_relation(db, "e_a", "e_b", "related_to", 0.5).await.expect("不同 type 应成功");
        let all = knowledge_relations::Entity::find().count(db).await.expect("计数应成功");
        assert_eq!(all, 3, "不同自然键必须各占一行（去重不得退化成「只留一行」）");
    }

    /// `knowledge_base_id` 必须写 sentinel。
    ///
    /// 写空串会**直接触发外键失败**（FK → `knowledge_bases(id)`，`code 787`），使本函数
    /// 一旦被接线就整条不可用 —— 这与 `causal::observe_edge` 曾经的缺陷完全同型
    /// （那处已修，见 `trajectory/src/causal.rs::CAUSAL_KB_ID`）。
    #[tokio::test]
    async fn upsert_relation_writes_sentinel_kb() {
        let handle = crate::db::create_test_pool().await.expect("测试库应可建立");
        let db = &handle.conn;

        upsert_relation(db, "e_x", "e_y", "part_of", 1.0).await.expect("upsert 应成功");
        let r = row_of(db, "e_x", "e_y", "part_of").await;
        assert_eq!(r.knowledge_base_id, TRAJECTORY_KB_ID, "必须写 sentinel KB（空串 ⇒ 787）");
    }
}

#[cfg(test)]
mod entity_type_observation_tests {
    use super::*;

    /// 正/负对照：只有**未登记**的类型才进清单。
    #[test]
    fn test_unregistered_entity_types_positive_and_negative() {
        // 负对照：全是已登记值（含 DB 实测的大小写变体 `COMPANY` / `CONCEPT`）
        let (kept, suppressed) = unregistered_entity_types(
            ["company", "person", "industry", "concept", "COMPANY", "CONCEPT", "conversation"]
                .into_iter(),
        );
        assert!(kept.is_empty(), "全登记的输入不得产生任何项：{kept:?}");
        assert_eq!(suppressed, 0);

        // 正对照：混入一个未登记值 ⇒ 只报它
        let (kept, suppressed) =
            unregistered_entity_types(["company", "brand_new_kind", "person"].into_iter());
        assert_eq!(kept, vec!["brand_new_kind".to_string()]);
        assert_eq!(suppressed, 0);

        // 大小写不一致的**未登记**值不算已登记（登记表是精确匹配，归一在读取端）
        let (kept2, _) = unregistered_entity_types(["BrAnD"].into_iter());
        assert_eq!(kept2, vec!["BrAnD".to_string()]);
    }

    /// 去重：同一未登记类型出现 100 次只报 1 项（批量路径不刷屏）。
    #[test]
    fn test_unregistered_entity_types_dedupes() {
        let types: Vec<&str> = std::iter::repeat_n("spam_type", 100).collect();
        let (kept, suppressed) = unregistered_entity_types(types.into_iter());
        assert_eq!(kept.len(), 1, "同一类型必须只出现一次");
        assert_eq!(suppressed, 0);
    }

    /// 上限与**截断自报**：超上限时必须报出被截断的种数（静默截断 = 假结论）。
    #[test]
    fn test_unregistered_entity_types_caps_and_reports_suppressed() {
        let names: Vec<String> =
            (0..MAX_DISTINCT_UNREGISTERED + 3).map(|i| format!("unknown_kind_{i}")).collect();
        let (kept, suppressed) = unregistered_entity_types(names.iter().map(|s| s.as_str()));
        assert_eq!(kept.len(), MAX_DISTINCT_UNREGISTERED);
        assert_eq!(suppressed, 3, "被截断的种数必须如实自报");
        assert_eq!(kept.len() + suppressed, names.len(), "两者之和 = 未登记种数（算术闭合）");

        // 反向对照：恰好等于上限时不得虚报截断
        let exact: Vec<String> =
            (0..MAX_DISTINCT_UNREGISTERED).map(|i| format!("exact_{i}")).collect();
        let (k2, s2) = unregistered_entity_types(exact.iter().map(|s| s.as_str()));
        assert_eq!(k2.len(), MAX_DISTINCT_UNREGISTERED);
        assert_eq!(s2, 0, "恰好在上限时截断数应为 0");
    }
}

#[cfg(test)]
mod domain_audit_tests {
    use super::*;

    fn row(
        kb: &str,
        entities: usize,
        edges: usize,
        both_visible: usize,
        dangling: usize,
    ) -> KbDomainRow {
        KbDomainRow {
            knowledge_base_id: kb.to_string(),
            visible_entities: entities,
            edges,
            edges_both_visible: both_visible,
            edges_both_missing: dangling,
            dangling,
        }
    }

    /// 恒等式的**定义**：两类互斥穷尽（`edges_both_visible + dangling == edges`）。
    #[test]
    fn test_kb_domain_row_self_consistency() {
        assert!(row("kb1", 10, 100, 60, 40).is_self_consistent());
        assert!(!row("kb1", 10, 100, 60, 39).is_self_consistent(), "不闭合必须被报出来");
    }

    /// 空域（0 条边）的悬空占比是 `0.0`，不是 `1.0` 也不是 `NaN`。
    ///
    /// 返 1.0 会让「这个域里没有边」冒充「这个域的边全断了」；
    /// 返 NaN 会毒化任何下游比较（`NaN > 0.0` 恒 false ⇒ 静默不告警）。
    #[test]
    fn test_kb_domain_row_empty_domain_ratio_is_zero() {
        let empty = row("kb_empty", 0, 0, 0, 0);
        assert_eq!(empty.dangling_ratio(), 0.0);
        assert!(empty.dangling_ratio().is_finite(), "不得是 NaN");
        assert_eq!(row("kb_all_dangling", 4, 38171, 0, 38171).dangling_ratio(), 1.0);
    }

    /// 只报**上升**：下降是期望方向，不构成违规。
    #[test]
    fn test_dangling_regressions_only_reports_increases() {
        let before = vec![row("kb_a", 10, 100, 50, 50), row("kb_b", 10, 100, 100, 0)];
        let after = vec![row("kb_a", 10, 100, 90, 10), row("kb_b", 10, 100, 100, 0)];
        assert!(dangling_regressions(&before, &after).is_empty(), "全下降/持平 ⇒ 无违规");

        // 越界回归：kb_b 从 0 升到 20（另起一域也一并覆盖）
        let worse = vec![
            row("kb_a", 10, 100, 90, 10),
            row("kb_b", 10, 100, 80, 20),
            row("kb_new", 5, 7, 0, 7),
        ];
        let regs = dangling_regressions(&before, &worse);
        assert_eq!(
            regs,
            vec![("kb_b".to_string(), 0, 20), ("kb_new".to_string(), 0, 7)],
            "before 里没有的域按 0 比（新出现的断链域必须报），且按 kb 升序"
        );
    }

    /// **为什么必须逐域比而不是比总和** —— 这条测试就是那个反例。
    ///
    /// 总和口径：before 100 → after 20，看起来是「大幅改善」；
    /// 而实际发生的是「修好了旧断点 A，同时**制造了**新断点 B」。
    #[test]
    fn test_total_dangling_hides_newly_broken_domain() {
        let before = vec![row("kb_a", 10, 200, 100, 100), row("kb_b", 10, 200, 200, 0)];
        let after = vec![row("kb_a", 10, 200, 200, 0), row("kb_b", 10, 200, 180, 20)];

        assert!(sum_dangling(&after) < sum_dangling(&before), "前提：总和口径显示为改善");
        assert_eq!(
            dangling_regressions(&before, &after),
            vec![("kb_b".to_string(), 0, 20)],
            "按域比必须抓住 kb_b 这个新断点 —— 否则验收门禁形同虚设"
        );
    }
}
