// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "artifacts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    #[sea_orm(indexed)]
    pub conversation_id: String,
    #[sea_orm(default_value = "draft")]
    pub kind: String,
    pub title: String,
    #[sea_orm(column_type = "Text")]
    #[sea_orm(default_value = "")]
    pub content: String,
    #[sea_orm(default_value = "markdown")]
    pub format: String,
    #[sea_orm(indexed)]
    #[sea_orm(default_value = 0)]
    pub pinned: i32,
    pub updated_at: String,
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
