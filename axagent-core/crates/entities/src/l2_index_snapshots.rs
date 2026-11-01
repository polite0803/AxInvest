// SPDX-License-Identifier: AGPL-3.0-only

//! `l2_index_snapshots` —— L2 索引快照元数据（**侧车库，`disk-cache` 自持 SQLite 文件**）。
//!
//! 背景与「一模块一实体」契约说明见 `l2_search_results.rs`。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "l2_index_snapshots")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub snapshot_id: String,
    #[sea_orm(default_value = 0)]
    pub file_count: i32,
    #[sea_orm(default_value = 0)]
    pub definition_count: i32,
    /// 快照文件路径
    #[sea_orm(column_type = "Text")]
    pub snapshot_path: String,
    /// 创建时间（epoch 秒）
    #[sea_orm(default_value = 0)]
    pub created_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
