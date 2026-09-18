// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "tool_executions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    #[sea_orm(indexed)]
    pub conversation_id: String,
    #[sea_orm(indexed)]
    pub message_id: Option<String>,
    #[sea_orm(indexed)]
    pub server_id: String,
    pub tool_name: String,
    #[sea_orm(default_value = "pending")]
    pub status: String,
    pub input_preview: Option<String>,
    pub output_preview: Option<String>,
    pub error_message: Option<String>,
    pub duration_ms: Option<i64>,
    pub created_at: String,
    pub approval_status: Option<String>,
    pub skill_steps_json: Option<String>,
    pub depends_on: Option<String>,
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
