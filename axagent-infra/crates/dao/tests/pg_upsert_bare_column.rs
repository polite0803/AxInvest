// SPDX-License-Identifier: AGPL-3.0-only
//!
//! 回归锁：`ON CONFLICT ... DO UPDATE SET` 里**不得出现未限定的列引用**。
//!
//! ## 锁的是什么
//!
//! PG 的 `ON CONFLICT DO UPDATE` 命名空间里，目标表与 `excluded` 伪关系**同时在场**，
//! 因此任何未限定的列名都会命中两处 ⇒ 解析期
//! `42702 column reference "..." is ambiguous`。
//! 要点：**与表达式里是否出现 `excluded.` 无关** —— `usage_count + 1` 这种单独一项
//! 也照样报错（判据不是「有没有和 excluded 同框」，而是「有没有出现在 DO UPDATE 的 SET 里」）。
//!
//! ⚠ **为什么不能用 SQLite 内存库来锁**：SQLite 对裸列名宽容（实测 3.53.1 各形态全收），
//! 所以这类缺陷**只在 PG 上暴露** —— SQLite 单测**永远抓不到它**。
//! 本文件必须是 PG 端到端测试，这不是「顺手加个集成测试」，而是唯一能观测到该缺陷的位置。
//!
//! ## 事故（2026-09-17）
//!
//! `save_cached_graph` 的「传 None 时保留旧值」原写成
//! `COALESCE(excluded.communities_json, communities_json)`（第二个参数是裸列名）
//! ⇒ 前端弹 `Query Error: error returned from database: 字段关联 "communities_json"
//! 是不明确的 at line 932`（`at line N` 是 sqlx 0.9 把 PG 后端 C 源码行号一并打出来）。
//! 同一批 2026-09-16「去方言分支」重构引入的 `upsert_execution_feedback` 三处表达式
//! 同为裸名 ⇒ 同病（那个函数的错误包装成 `AxAgentError::Database`，前缀不同）。
//!
//! 修法：`save_cached_graph` 改为「按 Some/None 决定该列是否进 SET」（等价于 COALESCE）；
//! `upsert_execution_feedback` 改用 `Expr::col((表名, 列))` 生成 `"表"."列"`。
//!
//! ## 运行
//!
//! ```bash
//! AXAGENT_TEST_PG_URL="$(node scripts/pg-connect.mjs url)" \
//!   cargo test -p axagent-dao --test pg_upsert_bare_column -- --nocapture
//! ```
//!
//! 变量名必须与 `pg_cjk_fts.rs` / `search/tests/pg_integration.rs` **完全一致** ——
//! 本仓曾同时存在三种写法，后果是这 4 个 PG 测试在 CI 里一条都没跑过（见 `pg_cjk_fts.rs`
//! 头部记述）。未设置时**诚实跳过**：跳过不是通过。
//!
//! ⚠ 前置条件：目标库已有 `wiki_graph_cache` 表（应用启动时由声明式引擎建出）。
//! 表缺失时报 SKIP 并说明原因，**不是**静默通过。
//! ⚠ 副作用：只写一行 `vault_id = __probe_upsert_bare_column__` 的记录，测试末尾删除。

use axagent_dao::repo::wiki_graph_cache as cache;
use axagent_entities::wiki_graph_cache::{Column, Entity as WikiGraphCache};
use axagent_harness::graph_dtos::{GraphData, GraphEdge, GraphNode};
use axagent_harness::louvain_dtos::LouvainResult;
use sea_orm::{
    ColumnTrait, ConnectionTrait, Database, DatabaseConnection, DbBackend, EntityTrait,
    QueryFilter, Statement,
};

/// 与 `pg_cjk_fts.rs` / `search/tests/pg_integration.rs` 必须一致。
const ENV_PG_URL: &str = "AXAGENT_TEST_PG_URL";

/// 探针行的主键。前缀足够独特，便于人工辨认残留。
const PROBE_VAULT_ID: &str = "__probe_upsert_bare_column__";

async fn connect_or_skip() -> Option<DatabaseConnection> {
    let url = match std::env::var(ENV_PG_URL) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            eprintln!(
                "SKIP: 未设置 {ENV_PG_URL}，无法验证 ON CONFLICT 的列限定在真实 PG 上是否成立。\
                 \n      本测试**未通过，也未被验证** —— 跳过仅表示环境不具备条件。"
            );
            return None;
        },
    };
    Some(Database::connect(&url).await.expect("应能连接 PostgreSQL"))
}

/// 表是否已建（声明式引擎负责建表，测试不建表）。
async fn table_exists(db: &DatabaseConnection) -> bool {
    let sql = "SELECT COUNT(*) FROM information_schema.tables \
               WHERE table_name = 'wiki_graph_cache'";
    match db.query_one_raw(Statement::from_string(DbBackend::Postgres, sql.to_string())).await {
        Ok(Some(row)) => row.try_get::<i64>("", "count").map(|n| n > 0).unwrap_or(false),
        _ => false,
    }
}

fn sample_graph() -> GraphData {
    let nodes = vec![
        GraphNode {
            id: "n1".to_string(),
            title: "节点一".to_string(),
            node_type: "note".to_string(),
            tags: vec![],
            link_count: 1,
            backlink_count: 0,
            path: "n1.md".to_string(),
        },
        GraphNode {
            id: "n2".to_string(),
            title: "节点二".to_string(),
            node_type: "note".to_string(),
            tags: vec![],
            link_count: 0,
            backlink_count: 1,
            path: "n2.md".to_string(),
        },
    ];
    let edges = vec![GraphEdge::structural("n1", "n2", "link")];
    GraphData::new(nodes, edges)
}

fn sample_louvain() -> LouvainResult {
    let mut communities = std::collections::HashMap::new();
    communities.insert("n1".to_string(), 0);
    communities.insert("n2".to_string(), 0);
    let mut cohesion_scores = std::collections::HashMap::new();
    cohesion_scores.insert(0, 1.0_f64);
    let mut community_sizes = std::collections::HashMap::new();
    community_sizes.insert(0, 2_usize);
    let mut top_nodes = std::collections::HashMap::new();
    top_nodes.insert(0, "n1".to_string());
    LouvainResult {
        communities,
        cohesion_scores,
        community_sizes,
        top_nodes,
        modularity: 0.0,
        num_communities: 1,
        color_palette: LouvainResult::default_palette(),
        // 刻意给一个**非 None** 值：让「实体侧社区」这个新字段真的进一次
        // 序列化 → 落库 → 读回。本用例的断言只判存在性/相等性，
        // 加值不改变任何既有判据，但能让「字段漏进缓存 JSON」这类缺陷可被观测。
        entity_communities: Some(std::collections::HashMap::from([("entity:n1".to_string(), 3)])),
    }
}

/// 读回 `communities_json`（未命中返回 `None`）。
async fn read_communities(db: &DatabaseConnection, vault_id: &str) -> Option<String> {
    WikiGraphCache::find()
        .filter(Column::VaultId.eq(vault_id))
        .one(db)
        .await
        .expect("读回缓存行应成功")
        .and_then(|m| m.communities_json)
}

#[tokio::test]
async fn upsert_set_must_not_use_bare_column_reference() {
    let Some(db) = connect_or_skip().await else {
        return;
    };

    if !table_exists(&db).await {
        eprintln!(
            "SKIP: 目标库没有 wiki_graph_cache 表（本测试不建表，由声明式引擎建）。\
             \n      本测试**未通过，也未被验证**。"
        );
        return;
    }

    // 清残留（上次异常中断可能留下）
    cache::invalidate_cache(&db, PROBE_VAULT_ID).await.expect("清理残留应成功");

    let graph = sample_graph();

    // ① 首次写入（无冲突）。修复前**这一步就报 42702**：
    //    生成的是 `... DO UPDATE SET ... "communities_json" = COALESCE(excluded.communities_json, communities_json)`
    cache::save_cached_graph(&db, PROBE_VAULT_ID, &graph, None)
        .await
        .expect("① 首次写入必须成功：裸列名会在此报 42702 column reference is ambiguous");
    assert_eq!(read_communities(&db, PROBE_VAULT_ID).await, None, "① None 应写入 NULL");

    // ② 补写社区（有冲突 ⇒ 真走 DO UPDATE SET）
    let louvain = sample_louvain();
    cache::save_cached_graph(&db, PROBE_VAULT_ID, &graph, Some(&louvain))
        .await
        .expect("② 带社区写入必须成功");
    let after_some = read_communities(&db, PROBE_VAULT_ID).await;
    assert!(
        after_some.as_deref().is_some_and(|s| !s.is_empty()),
        "② 社区应已落库，实际 = {after_some:?}"
    );

    // ③ 再以 None 写入：**旧社区必须保留**。
    //    这一条锁住的是「省略该列」与「COALESCE 保留旧值」的语义等价性 ——
    //    只验证 ①② 会漏掉「改成了无条件覆盖」这种退步。
    cache::save_cached_graph(&db, PROBE_VAULT_ID, &graph, None).await.expect("③ None 写入必须成功");
    assert_eq!(
        read_communities(&db, PROBE_VAULT_ID).await,
        after_some,
        "③ 传 None 时不得覆盖既有社区结果（原 COALESCE 语义）"
    );

    cache::invalidate_cache(&db, PROBE_VAULT_ID).await.expect("清理探针行应成功");
    assert_eq!(read_communities(&db, PROBE_VAULT_ID).await, None, "清理后不应有残留");
}
