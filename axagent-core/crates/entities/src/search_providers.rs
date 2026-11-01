// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "search_providers")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub name: String,
    #[sea_orm(default_value = "tavily")]
    pub provider_type: String,
    pub endpoint: Option<String>,
    pub api_key_ref: Option<String>,
    #[sea_orm(indexed)]
    #[sea_orm(default_value = 1)]
    pub enabled: i32,
    pub region: Option<String>,
    pub language: Option<String>,
    pub safe_search: Option<i64>,
    #[sea_orm(default_value = 10)]
    pub result_limit: i32,
    #[sea_orm(default_value = 5000)]
    pub timeout_ms: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
