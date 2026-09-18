// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "search_citations")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    #[sea_orm(indexed)]
    pub conversation_id: String,
    #[sea_orm(indexed)]
    pub message_id: String,
    pub title: String,
    pub url: String,
    pub snippet: Option<String>,
    pub provider_id: String,
    #[sea_orm(default_value = 0)]
    pub rank: i32,
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
