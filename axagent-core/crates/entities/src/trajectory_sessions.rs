// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "trajectory_sessions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub title: String,
    #[sea_orm(default_value = "web")]
    pub platform: String,
    #[sea_orm(default_value = "default")]
    pub user_id: String,
    #[sea_orm(default_value = "unknown")]
    pub model: String,
    #[sea_orm(default_value = "")]
    pub system_prompt: String,
    pub created_at: String,
    #[sea_orm(indexed)]
    pub updated_at: String,
    pub parent_session_id: Option<String>,
    #[sea_orm(default_value = 0)]
    pub token_input: i64,
    #[sea_orm(default_value = 0)]
    pub token_output: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
