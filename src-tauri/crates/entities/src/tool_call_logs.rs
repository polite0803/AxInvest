// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "tool_call_logs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    #[sea_orm(indexed)]
    pub conversation_id: Option<String>,
    pub trajectory_id: Option<String>,
    #[sea_orm(default_value = 0)]
    pub step_index: i32,
    #[sea_orm(indexed)]
    pub tool_name: String,
    #[sea_orm(column_type = "Text")]
    #[sea_orm(default_value = "{}")]
    pub arguments: String,
    #[sea_orm(column_type = "Text", nullable)]
    pub result: Option<String>,
    #[sea_orm(default_value = 0)]
    pub success: i32,
    #[sea_orm(default_value = 0)]
    pub duration_ms: u64,
    pub related_source_id: Option<String>,
    #[sea_orm(indexed)]
    #[sea_orm(default_value = 0)]
    pub created_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
