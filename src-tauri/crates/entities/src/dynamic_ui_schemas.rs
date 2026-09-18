// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "dynamic_ui_schemas")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub title: String,
    #[sea_orm(default_value = "")]
    pub description: String,
    #[sea_orm(column_type = "Text")]
    pub schema_json: String,
    #[sea_orm(indexed)]
    #[sea_orm(default_value = "custom")]
    pub category: String,
    #[sea_orm(default_value = "[]")]
    pub tags: String,
    #[sea_orm(default_value = "1.0.0")]
    pub version: String,
    #[sea_orm(default_value = 0)]
    pub is_builtin: i32,
    pub created_at: String,
    #[sea_orm(indexed)]
    pub updated_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
