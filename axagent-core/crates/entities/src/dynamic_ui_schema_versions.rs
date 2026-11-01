// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "dynamic_ui_schema_versions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i64,
    #[sea_orm(indexed)]
    pub schema_id: String,
    pub version: String,
    pub title: String,
    #[sea_orm(default_value = "")]
    pub description: String,
    #[sea_orm(column_type = "Text")]
    pub schema_json: String,
    #[sea_orm(default_value = "custom")]
    pub category: String,
    #[sea_orm(default_value = "[]")]
    pub tags: String,
    #[sea_orm(default_value = "")]
    pub change_log: String,
    pub created_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
