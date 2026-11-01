// SPDX-License-Identifier: AGPL-3.0-only
//! Wiki 图谱缓存：持久化 GraphData + LouvainResult 到 `wiki_graph_cache` 表。
//!
//! ## 设计
//!
//! 10 万节点规模下，每次 `get_wiki_graph` 三次 DB 扫描 + 内存聚合，
//! `wiki_graph_communities` 还要跑 Louvain 算法，单次数秒到数十秒。
//! 缓存到独立表后，前端读取直接命中缓存（< 10ms）。
//!
//! ## 失效策略
//!
//! `notes` 表有写入/更新/删除时，调用方应调用 `invalidate_cache(vault_id)`
//! 清除对应 vault 的缓存。`updated_at` 字段用于手动判断缓存新鲜度。
//!
//! ## 改造记录（2026-09-16）
//!
//! 原实现用 `Statement::from_sql_and_values` 手写双方言分支。改走 SeaORM 实体
//! （[`axagent_entities::wiki_graph_cache`]）后双分支消失，并顺带修掉一处竞态：
//! 原 SQLite 分支是「先 `UPDATE`，`rows_affected()==0` 再 `INSERT`」两步，
//! 两个并发写入者可能同时走到 INSERT 而主键冲突；现统一为单条 UPSERT。
//!
//! ## 改造记录 2（2026-09-17）：裸列名在 PG 上必报 42702
//!
//! 上面那次重构把 `communities_json` 的「传 NULL 时保留旧值」写成
//! `COALESCE(excluded.communities_json, communities_json)`，理由写的是
//! 「两方言均支持裸列名引用现有行」。**这个判断是错的**：
//!
//! PG 的 `ON CONFLICT ... DO UPDATE SET` 命名空间里**同时**存在目标表与 `excluded`
//! 伪关系，因此任何**未限定**的列名都会命中两处 ⇒ 解析期报
//! `42702 column reference "communities_json" is ambiguous`
//! （`sqlx` 的 Display 会把 PG 后端源码行号一起打出来，即那句 `at line 932`）。
//! 注意：**即使表达式里不出现 `excluded.` 也一样**（`x`、`x + 1` 同样报错）——
//! 判据不是「有没有和 excluded 同框」，而是「有没有出现在 DO UPDATE 的 SET 里」。
//! SQLite 侧对裸列名宽容（实测 3.53.1 全部形态都收），故该缺陷**只在 PG 暴露**。
//!
//! 真库实测（PG 18，2026-09-17）：
//! `COALESCE(excluded.b, b)` → 42702 · `COALESCE(excluded.b, t.b)` → 通过 ·
//! 「不把该列放进 SET」→ 通过（且旧值保留）。
//!
//! ⇒ 修法：**不用裸列名，改用「按 Some/None 决定该列是否进 SET」**表达同一语义。
//! 这样既不写裸列名，也不需要表限定（少一处方言相关写法）。

use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, Set};

use axagent_entities::wiki_graph_cache::{ActiveModel, Column, Entity as WikiGraphCache};
use axagent_harness::graph_dtos::GraphData;
use axagent_harness::louvain_dtos::LouvainResult;

/// 缓存命中的图谱数据 + 社区检测结果（社区可能为 None 表示未计算）。
pub struct GraphCacheEntry {
    pub graph_data: GraphData,
    pub communities: Option<LouvainResult>,
    pub computed_at: i64,
}

/// 读取缓存的图谱数据。未命中返回 Ok(None)。
pub async fn get_cached_graph(
    db: &DatabaseConnection,
    vault_id: &str,
) -> Result<Option<GraphCacheEntry>, DbErr> {
    let Some(row) = WikiGraphCache::find_by_id(vault_id.to_string()).one(db).await? else {
        return Ok(None);
    };

    // DTO 演进（新增字段）可能导致旧缓存 JSON 反序列化失败。
    // 失败时清缓存、返回 None，让调用方重建，比硬报错让整个 wiki 页面挂掉强。
    let graph_data = match serde_json::from_str::<GraphData>(&row.graph_data_json) {
        Ok(g) => g,
        Err(_) => {
            let _ = invalidate_cache(db, vault_id).await;
            return Ok(None);
        },
    };

    let communities = match row.communities_json {
        Some(json) if !json.is_empty() => serde_json::from_str::<LouvainResult>(&json).ok(),
        _ => None,
    };

    Ok(Some(GraphCacheEntry { graph_data, communities, computed_at: row.computed_at }))
}

/// 写入/更新缓存。如果 communities 为 None，保留原有 communities（若存在）。
pub async fn save_cached_graph(
    db: &DatabaseConnection,
    vault_id: &str,
    graph_data: &GraphData,
    communities: Option<&LouvainResult>,
) -> Result<(), DbErr> {
    let now = chrono::Utc::now().timestamp();
    let graph_data_json = serde_json::to_string(graph_data)
        .map_err(|e| DbErr::Custom(format!("序列化 graph_data 失败: {e}")))?;
    let communities_json = match communities {
        Some(c) => serde_json::to_string(c).ok(),
        None => None,
    };
    // 「传 None 时保留旧值」= **不把该列放进 SET**（不是 `COALESCE(..., 裸列名)`）。
    // 以「实际要写入的值」为准判定，与原来的 COALESCE 语义逐例等价：
    // 写入 NULL ⇔ 不进 SET ⇔ 旧值保留；写入非 NULL ⇔ 进 SET ⇔ 覆盖。
    let write_communities = communities_json.is_some();
    let node_count = graph_data.nodes.len() as i32;
    let edge_count = graph_data.edges.len() as i32;

    let am = ActiveModel {
        vault_id: Set(vault_id.to_string()),
        graph_data_json: Set(graph_data_json),
        communities_json: Set(communities_json),
        node_count: Set(node_count),
        edge_count: Set(edge_count),
        computed_at: Set(now),
        updated_at: Set(now),
    };

    let mut set_columns = vec![
        Column::GraphDataJson,
        Column::NodeCount,
        Column::EdgeCount,
        Column::ComputedAt,
        Column::UpdatedAt,
    ];
    if write_communities {
        set_columns.push(Column::CommunitiesJson);
    }

    WikiGraphCache::insert(am)
        .on_conflict(OnConflict::column(Column::VaultId).update_columns(set_columns).to_owned())
        .exec(db)
        .await
        .map(|_| ())
}

/// 仅更新社区检测结果（图谱数据已缓存，只补算社区）。
pub async fn save_cached_communities(
    db: &DatabaseConnection,
    vault_id: &str,
    communities: &LouvainResult,
) -> Result<(), DbErr> {
    let now = chrono::Utc::now().timestamp();
    let communities_json = serde_json::to_string(communities)
        .map_err(|e| DbErr::Custom(format!("序列化 communities 失败: {e}")))?;

    WikiGraphCache::update_many()
        .col_expr(Column::CommunitiesJson, Expr::value(communities_json))
        .col_expr(Column::UpdatedAt, Expr::value(now))
        .filter(Column::VaultId.eq(vault_id))
        .exec(db)
        .await
        .map(|_| ())
}

/// 失效缓存：notes 表有写入/更新/删除时调用。
pub async fn invalidate_cache(db: &DatabaseConnection, vault_id: &str) -> Result<(), DbErr> {
    WikiGraphCache::delete_by_id(vault_id.to_string()).exec(db).await.map(|_| ())
}
