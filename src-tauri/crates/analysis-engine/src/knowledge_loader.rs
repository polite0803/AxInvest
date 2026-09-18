//! 知识图谱加载器 — 从 DB 读取概念/行业/股票关系到 ConceptIndex
//!
//! 数据由 `import_lemonhu_knowledge` 命令（commands/knowledge.rs）从
//! knowledge-sources/lemonhu/ 导入到 DB，本模块只做读取。
//!
//! ## 定位
//!
//! 启动时自动调用，找名为 "开源股票知识库(lemonhu)" 的 knowledge_base，
//! 读取其 `knowledge_entities` + `knowledge_relations` 表，填充 ConceptIndex。

use std::collections::{HashMap, HashSet};

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

use crate::concept_index::{ConceptIndex, ConceptNode};

/// 从 DB 加载 ConceptIndex
///
/// 查找名为 "开源股票知识库(lemonhu)" 的 knowledge_base，
/// 读取其中的 has_concept / in_industry 关系。
/// 若找不到该知识库，返回 Ok(0)（静默降级）。
pub async fn load_concept_index_from_db(
    index: &mut ConceptIndex,
    db: &DatabaseConnection,
) -> Result<usize, String> {
    use axagent_entities::knowledge_bases;
    use axagent_entities::knowledge_entities;
    use axagent_entities::knowledge_relations;

    // 1. 找 knowledge_base
    let kb: Option<knowledge_bases::Model> = knowledge_bases::Entity::find()
        .filter(knowledge_bases::Column::Name.eq("开源股票知识库(lemonhu)"))
        .one(db)
        .await
        .map_err(|e| format!("查 knowledge_bases 失败: {e}"))?;

    let kb_id = match kb {
        Some(k) => k.id,
        None => return Ok(0), // 没导入过，静默降级
    };

    // 2. 读取 has_concept / in_industry 关系
    let relations = knowledge_relations::Entity::find()
        .filter(knowledge_relations::Column::KnowledgeBaseId.eq(&kb_id))
        .filter(
            knowledge_relations::Column::RelationType
                .is_in(vec!["has_concept".to_string(), "in_industry".to_string()]),
        )
        .all(db)
        .await
        .map_err(|e| format!("查 knowledge_relations 失败: {e}"))?;

    if relations.is_empty() {
        return Ok(0);
    }

    // 3. 收集涉及的实体 ID
    let mut entity_ids = HashSet::new();
    for r in &relations {
        entity_ids.insert(r.source_entity_id.clone());
        entity_ids.insert(r.target_entity_id.clone());
    }

    let entity_ids_vec: Vec<String> = entity_ids.into_iter().collect();
    let entities = knowledge_entities::Entity::find()
        .filter(knowledge_entities::Column::Id.is_in(entity_ids_vec))
        .all(db)
        .await
        .map_err(|e| format!("查 knowledge_entities 失败: {e}"))?;

    // 4. 注册概念/行业节点
    //
    // ⚠ 2026-09-14（本体接线 P1）：比较**经本体归一**，不再用裸字面量精确比较。
    //   原实现 `e.entity_type == "concept" || e.entity_type == "industry"` 对 DB 里的
    //   大小写变体（实测 `COMPANY` 2 行 / `CONCEPT` 1 行）**恒 false** ⇒ 这些实体不会被
    //   注册成概念节点，下面的成员关系也建不起来 —— 全程静默，没有任何报错。
    //   而它们在本体登记表里是 `observed: true`（被承认存在）⇒ `validate_entity_type`
    //   也不会报。「登记」只让问题可见，**不等于消费端认得** —— 所以归一必须在这里显式做。
    for e in &entities {
        if is_concept_like(&e.entity_type) {
            let node =
                ConceptNode::new(&e.id, &e.name, &e.entity_type).with_aliases(&[e.name.as_str()]);
            index.register(node);
        }
    }

    // 归一后再入表：下面 5 的两处比较读的是这张表的值，归一放在**构建处**一处即覆盖全部。
    let entity_type_map: HashMap<String, String> = entities
        .into_iter()
        .map(|e| (e.id.clone(), normalized_entity_type(&e.entity_type)))
        .collect();

    // 5. 填充成员关系
    //
    // ⚠ 下面两处 `t == "company"` / `t == "concept"` 之所以**安全**，是因为
    //   `entity_type_map` 里的值在上面的构建处**已经归一**。若哪天有人绕开 map、
    //   直接拿 `entity_type` 的原始值来比，大小写变体又会静默漏配（修复前的形态）。
    let mut total = 0usize;
    for r in &relations {
        let source_is_stock =
            entity_type_map.get(&r.source_entity_id).map(|t| t == "company").unwrap_or(false);
        let target_is_concept = entity_type_map
            .get(&r.target_entity_id)
            .map(|t| t == "concept" || t == "industry")
            .unwrap_or(false);
        if source_is_stock && target_is_concept {
            index.add_membership(&r.target_entity_id, &r.source_entity_id);
            total += 1;
        }
    }

    tracing::info!(
        "[ConceptIndex] 从 DB(kb={kb_id}) 加载 {total} 条概念成员关系, {} 个实体",
        entity_type_map.len()
    );

    Ok(total)
}

/// 实体类型归一：命中本体登记表 ⇒ 规范写法；未登记 ⇒ **原样返回**。
///
/// 未登记值原样返回是刻意的：`entity_type` 是开放词表（LLM 抽取 + 会话记忆各写各的），
/// 把它替换成某个默认类型等于**凭空发明约束**，还会掩盖「LLM 给了个没人认识的值」。
fn normalized_entity_type(raw: &str) -> String {
    axagent_harness::knowledge_graph::normalize_entity_type(raw).unwrap_or(raw).to_string()
}

/// 该实体类型是否属于「概念类」（概念 / 行业）—— 归一后比较。
///
/// 归一**不是**把任意值兜底成 concept：未登记值 `normalize_entity_type` 返回 `None`，
/// 这里落到 `false`，即「不认识就不当概念」——兜底方向与 `ConceptNode::new` 的缺省类型
/// 无关，不要混。
fn is_concept_like(raw: &str) -> bool {
    matches!(
        axagent_harness::knowledge_graph::normalize_entity_type(raw),
        Some("concept") | Some("industry")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 归一必须把大小写变体对齐到规范写法，且**不改写未登记值**。
    ///
    /// 这是 `knowledge_loader` 侧漏配的直接判据：DB 实测 `CONCEPT` 1 行、
    /// `COMPANY` 2 行；修复前它们 `== "concept"` / `== "company"` 恒 false。
    #[test]
    fn test_entity_type_normalization_aligns_db_case_variants() {
        // 修复前会漏配的两个真实存量值
        assert!(is_concept_like("CONCEPT"), "`CONCEPT` 必须被识别为概念（DB 实测 1 行）");
        assert!(is_concept_like("concept"));
        assert!(is_concept_like("INDUSTRY"));
        assert!(is_concept_like("Industry"));

        // 公司类：`COMPANY`（DB 实测 2 行）必须归一到 company
        assert_eq!(normalized_entity_type("COMPANY"), "company");
        assert_eq!(normalized_entity_type("company"), "company");

        // 负对照：非概念类不得被判成概念（防「一律兜底成 concept」的退化）
        assert!(!is_concept_like("person"));
        assert!(!is_concept_like("company"));
        assert!(!is_concept_like("whatever_llm_said"), "未登记值不得兜底成概念");

        // 未登记值原样返回（不发明规范形）
        assert_eq!(normalized_entity_type("whatever_llm_said"), "whatever_llm_said");
        assert_eq!(normalized_entity_type(""), "");
    }
}
