// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "stored_files")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    #[sea_orm(indexed)]
    pub hash: String,
    pub original_name: String,
    #[sea_orm(default_value = "application/octet-stream")]
    pub mime_type: String,
    pub size_bytes: i64,
    pub storage_path: String,
    #[sea_orm(indexed)]
    pub conversation_id: Option<String>,
    pub created_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::conversations::Entity",
        from = "Column::ConversationId",
        to = "super::conversations::Column::Id",
        on_delete = "SetNull"
    )]
    Conversation,
}

impl ActiveModelBehavior for ActiveModel {}
