// SPDX-License-Identifier: AGPL-3.0-only

//! Loop 节点检查点持久化。
//!
//! 提供 save/load/delete 三个基本操作。底层表 `loop_checkpoints` 的 DDL 由
//! v100 建表，复合主键 `(execution_id, node_id)`；实体声明见
//! [`axagent_entities::loop_checkpoints`]。
//!
//! 调用方：LoopExecutor（写检查点 + 删除已完成检查点）、
//! WorkEngine::resume_loop_iteration（按 execution_id + node_id 读检查点）。
//!
//! ## 改造记录（2026-09-16）
//!
//! 原实现用 `Statement::from_sql_and_values` 手写双方言分支：SQLite 用 `?N`
//! 占位符 + `INSERT OR REPLACE`，PostgreSQL 用 `$N` + `ON CONFLICT ... DO UPDATE`
//! （PG 会把 `?1` 解析成 `?` 操作符 + integer，触发
//! "operator does not exist: ? integer"）。改走 SeaORM 实体后方言由 sea-query
//! 处理，双分支消失，且该表自此进入实体覆盖范围。

use axagent_entities::loop_checkpoints::{ActiveModel, Column, Entity as LoopCheckpoints};
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::util_fns::now_ts;
use axagent_harness::workflow_types::LoopCheckpoint;
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};

/// 写入或覆盖一个 Loop 节点的检查点。
///
/// UPSERT 语义：同一 (execution_id, node_id) 已存在则替换 payload_json 与
/// updated_at，其余列不变。
pub async fn save_loop_checkpoint(
    db: &DatabaseConnection,
    checkpoint: &LoopCheckpoint,
) -> Result<()> {
    let payload = serde_json::to_string(checkpoint)
        .map_err(|e| AxAgentError::Internal(format!("serialize LoopCheckpoint failed: {e}")))?;

    let am = ActiveModel {
        execution_id: Set(checkpoint.execution_id.clone()),
        node_id: Set(checkpoint.node_id.clone()),
        payload_json: Set(payload),
        updated_at: Set(now_ts() as i64),
    };

    LoopCheckpoints::insert(am)
        .on_conflict(
            OnConflict::columns([Column::ExecutionId, Column::NodeId])
                .update_columns([Column::PayloadJson, Column::UpdatedAt])
                .to_owned(),
        )
        .exec(db)
        .await
        .map(|_| ())
        .map_err(AxAgentError::Database)
}

/// 按 (execution_id, node_id) 读取 Loop 检查点。找不到时返回 Ok(None)。
pub async fn load_loop_checkpoint(
    db: &DatabaseConnection,
    execution_id: &str,
    node_id: &str,
) -> Result<Option<LoopCheckpoint>> {
    let row = LoopCheckpoints::find_by_id((execution_id.to_string(), node_id.to_string()))
        .one(db)
        .await
        .map_err(AxAgentError::Database)?;

    let Some(row) = row else {
        return Ok(None);
    };
    let cp: LoopCheckpoint = serde_json::from_str(&row.payload_json)
        .map_err(|e| AxAgentError::Internal(format!("deserialize LoopCheckpoint failed: {e}")))?;
    Ok(Some(cp))
}

/// 删除指定 (execution_id, node_id) 的 Loop 检查点。
/// 在 Loop 整体完成（或用户取消）后调用，清理磁盘。
pub async fn delete_loop_checkpoint(
    db: &DatabaseConnection,
    execution_id: &str,
    node_id: &str,
) -> Result<()> {
    LoopCheckpoints::delete_by_id((execution_id.to_string(), node_id.to_string()))
        .exec(db)
        .await
        .map(|_| ())
        .map_err(AxAgentError::Database)
}

/// 删除指定 execution 的所有 Loop 检查点。
/// 在 cancel/reset 时调用，避免脏数据遗留。
pub async fn delete_loop_checkpoints_for_execution(
    db: &DatabaseConnection,
    execution_id: &str,
) -> Result<()> {
    LoopCheckpoints::delete_many()
        .filter(Column::ExecutionId.eq(execution_id))
        .exec(db)
        .await
        .map(|_| ())
        .map_err(AxAgentError::Database)
}
