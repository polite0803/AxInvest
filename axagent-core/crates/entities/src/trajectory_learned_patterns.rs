// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "trajectory_learned_patterns")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub pattern: String,
    #[sea_orm(indexed)]
    pub pattern_type: String,
    #[sea_orm(default_value = 0)]
    pub success: i32,
    #[sea_orm(default_value = 0)]
    pub failure: i32,
    pub last_used: String,
    pub created_at: String,
    pub metadata: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
