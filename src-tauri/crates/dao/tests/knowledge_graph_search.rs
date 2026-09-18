// SPDX-License-Identifier: AGPL-3.0-only

//! `graph_enhanced_search` 关系查询的回归测试
//!
//! 防两类泄露：
//! 1. 因果边（`relation_type = "causes"`，行为统计而非文档知识）混入 RAG 检索
//! 2. 跨知识库关系泄露（旧实现 `(kb AND source_in) OR target_in` 的 OR 分支丢掉了 kb 约束）

use axagent_dao::db::create_test_pool;
use axagent_dao::repo::knowledge_graph as kg;
use axagent_harness::types::rag_voice_etc::{
    CreateKnowledgeEntityInput, CreateKnowledgeRelationInput,
};
use serde_json::json;

const CAUSAL_RELATION_TYPE: &str = axagent_harness::knowledge_graph::CAUSAL_RELATION_TYPE;

/// 建一行知识库 —— 这**不是**可选脚手架，是外键生效后的硬前提。
///
/// ⚠ 2026-09-16 实测根因：版本化迁移在 SQLite 上建 `knowledge_entities` 时**没写外键**
/// （**已删的** `v100` 迁移里，SQLite 分支只有 `knowledge_base_id TEXT NOT NULL`），
/// 而实体侧的 `belongs_to knowledge_bases` 是 2026-09-16 P1 才补上的（PLAN §五·二·补）。
/// 删掉迁移、改由声明式引擎建表后，SQLite 上这条外键**真的生效了**
/// （内联在 `CREATE TABLE` 里 + `PRAGMA foreign_keys=ON`）⇒ 父行不存在时插子行直接
/// `code 787 FOREIGN KEY constraint failed`。
///
/// 本文件原先用 `kb_x` / `kb_main` / `kb_other` 三个**不存在的**知识库 id，靠「外键没被
/// 建出来」侥幸通过 —— 而本文件要测的是**关系过滤**（因果边 / 跨 KB 泄露），与外键无关。
/// 所以正确的修法是补齐父行，而不是把外键关掉、把缺陷盖回去。
async fn seed_kb(db: &sea_orm::DatabaseConnection, id: &str) {
    use axagent_entities::knowledge_bases;
    use sea_orm::{ActiveModelTrait, Set};

    knowledge_bases::ActiveModel {
        id: Set(id.to_string()),
        name: Set(id.to_string()),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("create knowledge_base");
}

async fn seed_entity(db: &sea_orm::DatabaseConnection, kb: &str, name: &str) -> String {
    let e = kg::create_knowledge_entity(
        db,
        CreateKnowledgeEntityInput {
            knowledge_base_id: kb.to_string(),
            name: name.to_string(),
            entity_type: "module".to_string(),
            description: Some(name.to_string()),
            source_path: String::new(),
            source_language: None,
            properties: json!({}),
            lifecycle: None,
            behaviors: None,
            metadata: None,
        },
    )
    .await
    .expect("create entity");
    e.id
}

async fn seed_relation(
    db: &sea_orm::DatabaseConnection,
    kb: &str,
    source: &str,
    target: &str,
    rel_type: &str,
) {
    kg::create_knowledge_relation(
        db,
        CreateKnowledgeRelationInput {
            knowledge_base_id: kb.to_string(),
            source_entity_id: source.to_string(),
            target_entity_id: target.to_string(),
            relation_type: rel_type.to_string(),
            description: None,
            properties: None,
            metadata: None,
        },
    )
    .await
    .expect("create relation");
}

#[tokio::test]
async fn graph_search_excludes_causal_and_cross_kb_relations() {
    let handle = create_test_pool().await.expect("test pool");
    let db = &handle.conn;
    let kb_main = "kb_main";
    let kb_other = "kb_other";
    // 父行必须先存在：外键在引擎建出的库上是**有效**的（见 `seed_kb` 的文档）。
    seed_kb(db, kb_main).await;
    seed_kb(db, kb_other).await;

    let a = seed_entity(db, kb_main, "auth module").await;
    let b = seed_entity(db, kb_main, "login flow").await;
    seed_relation(db, kb_main, &a, &b, "follows").await;

    // 对抗样本 1：同 KB 的因果边——行为统计，必须被类型过滤排除
    seed_relation(db, kb_main, &a, &b, CAUSAL_RELATION_TYPE).await;

    // 对抗样本 2：跨 KB 关系，target 撞上 seed——必须被 kb 分组排除
    let c = seed_entity(db, kb_other, "other thing").await;
    seed_relation(db, kb_other, &c, &a, "mentions").await;

    let chunks = kg::graph_enhanced_search(db, kb_main, "auth", 10, true).await.expect("search");

    assert_eq!(chunks.len(), 1, "query 'auth' 只应命中 auth module");
    let chunk = &chunks[0];
    assert_eq!(chunk.entity_name, "auth module");

    let types: Vec<&str> = chunk.relations.iter().map(|r| r.relation_type.as_str()).collect();
    assert_eq!(types, vec!["follows"], "只允许本 KB 的文档关系，实际: {types:?}");
    assert_eq!(chunk.relations[0].target_entity_name, "login flow");

    std::fs::remove_file(&handle.path).ok();
}

#[tokio::test]
async fn graph_search_without_neighbors_has_no_relations() {
    let handle = create_test_pool().await.expect("test pool");
    let db = &handle.conn;
    seed_kb(db, "kb_x").await;

    let a = seed_entity(db, "kb_x", "auth module").await;
    let b = seed_entity(db, "kb_x", "login flow").await;
    seed_relation(db, "kb_x", &a, &b, "follows").await;

    let chunks = kg::graph_enhanced_search(db, "kb_x", "auth", 10, false).await.expect("search");
    assert_eq!(chunks.len(), 1);
    assert!(chunks[0].relations.is_empty(), "include_neighbors=false 不得携带关系");

    std::fs::remove_file(&handle.path).ok();
}
