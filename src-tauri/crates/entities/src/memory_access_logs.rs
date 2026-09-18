// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "memory_access_logs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    #[sea_orm(indexed)]
    pub conversation_id: Option<String>,
    #[sea_orm(indexed)]
    pub namespace_id: String,
    pub memory_id: String,
    pub access_type: String,
    #[sea_orm(column_type = "Text", nullable)]
    pub query: Option<String>,
    #[sea_orm(column_type = "Text", nullable)]
    pub content_snippet: Option<String>,
    #[sea_orm(default_value = 0)]
    pub hit: i32,
    #[sea_orm(indexed)]
    #[sea_orm(default_value = 0)]
    pub created_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
