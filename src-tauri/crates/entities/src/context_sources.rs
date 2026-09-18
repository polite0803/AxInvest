// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "context_sources")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    #[sea_orm(indexed)]
    pub conversation_id: String,
    #[sea_orm(indexed)]
    pub message_id: Option<String>,
    pub source_type: String,
    pub ref_id: String,
    pub title: String,
    #[sea_orm(default_value = 1)]
    pub enabled: i32,
    #[sea_orm(column_type = "Text", nullable)]
    pub summary: Option<String>,
    /// 多文档协同：JSON 数组字符串，如 `["doc1","doc2"]`；
    /// NULL 或空数组表示不限制（检索整个容器）
    #[sea_orm(column_type = "Text", nullable)]
    pub doc_ids_json: Option<String>,
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
