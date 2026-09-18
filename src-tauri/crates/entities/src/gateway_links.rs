// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "gateway_links")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub name: String,
    pub link_type: String,
    pub endpoint: String,
    pub api_key_id: Option<String>,
    #[sea_orm(default_value = 1)]
    pub enabled: i32,
    #[sea_orm(default_value = "disconnected")]
    pub status: String,
    pub error_message: Option<String>,
    #[sea_orm(default_value = 1)]
    pub auto_sync_models: i32,
    #[sea_orm(default_value = 1)]
    pub auto_sync_skills: i32,
    pub last_sync_at: Option<i64>,
    pub latency_ms: Option<i64>,
    pub version: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::gateway_link_policies::Entity")]
    GatewayLinkPolicy,
    #[sea_orm(has_many = "super::gateway_link_activities::Entity")]
    GatewayLinkActivity,
}

impl Related<super::gateway_link_policies::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::GatewayLinkPolicy.def()
    }
}

impl Related<super::gateway_link_activities::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::GatewayLinkActivity.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
