// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "agent_profiles")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[sea_orm(default_value = "general")]
    pub category: String,
    #[sea_orm(default_value = "🤖")]
    pub icon: String,
    #[sea_orm(column_name = "agent_role")]
    pub agent_role: Option<String>,
    #[sea_orm(default_value = "builtin")]
    pub source: String,
    pub tags: Option<String>,
    #[sea_orm(column_name = "suggested_provider_id")]
    pub suggested_provider_id: Option<String>,
    #[sea_orm(column_name = "suggested_model_id")]
    pub suggested_model_id: Option<String>,
    #[sea_orm(column_name = "suggested_temperature")]
    pub suggested_temperature: Option<f64>,
    #[sea_orm(column_name = "suggested_max_tokens")]
    pub suggested_max_tokens: Option<i64>,
    #[sea_orm(column_name = "search_enabled")]
    pub search_enabled: Option<bool>,
    #[sea_orm(column_name = "recommend_permission_mode")]
    pub recommend_permission_mode: Option<String>,
    #[sea_orm(column_name = "recommended_tools")]
    pub recommended_tools: Option<String>,
    #[sea_orm(column_name = "disallowed_tools")]
    pub disallowed_tools: Option<String>,
    #[sea_orm(column_name = "recommended_workflows")]
    pub recommended_workflows: Option<String>,
    #[sea_orm(default_value = 0)]
    pub sort_order: i32,
    #[sea_orm(default_value = 1)]
    pub is_enabled: i32,
    pub expert_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::agency_experts::Entity",
        from = "Column::ExpertId",
        to = "super::agency_experts::Column::Id",
        on_delete = "SetNull"
    )]
    AgencyExpert,
}

impl ActiveModelBehavior for ActiveModel {}
