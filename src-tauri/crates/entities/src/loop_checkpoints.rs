// SPDX-License-Identifier: AGPL-3.0-only

//! Loop 节点检查点表（DDL 由 v100 建表）。
//!
//! 一行 = 某次工作流执行中某个 Loop 节点的一份可恢复检查点。复合主键
//! `(execution_id, node_id)`。
//!
//! 写入方：`LoopExecutor`（每轮迭代写检查点、Loop 完成后删检查点）、
//! `WorkEngine::resume_loop_iteration`（按 execution_id + node_id 读回）。
//! 持久化入口为 [`axagent_dao::repo::loop_checkpoint`]，历史实现用原生 SQL
//! 手写双方言分支，已改为走本实体。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "loop_checkpoints")]
pub struct Model {
    /// 工作流执行 ID
    #[sea_orm(primary_key, auto_increment = false)]
    pub execution_id: String,
    /// Loop 节点 ID
    #[sea_orm(primary_key, auto_increment = false)]
    pub node_id: String,
    /// 检查点内容（`LoopCheckpoint` 的 JSON 序列化）
    pub payload_json: String,
    /// 写入时间戳（秒）
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
