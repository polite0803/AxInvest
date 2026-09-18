// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "conversations")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub title: String,
    pub model_id: String,
    pub provider_id: String,
    pub system_prompt: Option<String>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<i64>,
    pub top_p: Option<f64>,
    pub frequency_penalty: Option<f64>,
    #[sea_orm(default_value = 0)]
    pub search_enabled: i32,
    pub search_provider_id: Option<String>,
    pub thinking_budget: Option<i64>,
    #[sea_orm(default_value = "[]")]
    pub enabled_mcp_server_ids: String,
    #[sea_orm(default_value = "[]")]
    pub enabled_knowledge_base_ids: String,
    #[sea_orm(default_value = "[]")]
    pub enabled_memory_namespace_ids: String,
    #[sea_orm(default_value = "[]")]
    pub enabled_wiki_ids: String,
    #[sea_orm(default_value = 0)]
    pub message_count: i32,
    pub created_at: i64,
    #[sea_orm(indexed)]
    pub updated_at: i64,
    #[sea_orm(default_value = 0)]
    pub is_pinned: i32,
    #[sea_orm(default_value = 0)]
    pub is_archived: i32,
    #[sea_orm(default_value = "{}")]
    pub workspace_snapshot_json: String,
    pub active_branch_id: Option<String>,
    pub active_artifact_id: Option<String>,
    #[sea_orm(default_value = 0)]
    pub research_mode: i32,
    #[sea_orm(default_value = 0)]
    pub context_compression: i32,
    pub category_id: Option<String>,
    pub parent_conversation_id: Option<String>,
    #[sea_orm(default_value = "chat")]
    pub mode: String,
    pub work_strategy: Option<String>,
    pub scenario: Option<String>,
    #[sea_orm(default_value = "[]")]
    pub enabled_skill_ids: String,
    pub agent_profile_id: Option<String>,
    pub workflow_template_id: Option<String>,
    #[sea_orm(default_value = "conversation")]
    pub session_type: String,
    pub workflow_status: Option<String>,
    #[sea_orm(indexed)]
    #[sea_orm(default_value = "none")]
    pub memory_status: String,
    pub last_memory_extracted_at: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::messages::Entity")]
    Messages,
}

impl Related<super::messages::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Messages.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
