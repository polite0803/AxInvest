// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "agent_sessions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    #[sea_orm(indexed)]
    pub conversation_id: String,
    pub cwd: Option<String>,
    #[sea_orm(default_value = 0)]
    pub workspace_locked: i32,
    pub permission_mode: String,
    pub runtime_status: String,
    pub sdk_context_json: Option<String>,
    pub sdk_context_backup_json: Option<String>,
    #[sea_orm(default_value = 0)]
    pub total_tokens: i64,
    #[sea_orm(default_value = 0.0)]
    pub total_cost_usd: f64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::conversations::Entity",
        from = "Column::ConversationId",
        to = "super::conversations::Column::Id",
        on_delete = "Cascade"
    )]
    Conversation,
}

impl ActiveModelBehavior for ActiveModel {}
