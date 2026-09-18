// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "workflow_marketplace")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub template_id: String,
    pub author_id: String,
    pub name: String,
    pub description: Option<String>,
    pub category: String,
    #[sea_orm(default_value = "")]
    pub icon: String,
    pub tags: Option<String>,
    #[sea_orm(default_value = 0)]
    pub downloads: i64,
    #[sea_orm(default_value = 0.0)]
    pub rating_average: f64,
    #[sea_orm(default_value = 0)]
    pub rating_count: i32,
    #[sea_orm(default_value = false)]
    pub is_featured: bool,
    #[sea_orm(default_value = false)]
    pub is_verified: bool,
    #[sea_orm(default_value = true)]
    pub is_public: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
