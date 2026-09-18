// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "retrieval_hits")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    #[sea_orm(indexed)]
    pub conversation_id: String,
    #[sea_orm(indexed)]
    pub message_id: String,
    #[sea_orm(indexed)]
    pub knowledge_base_id: String,
    pub document_id: String,
    pub chunk_ref: String,
    #[sea_orm(default_value = 0.0)]
    pub score: f64,
    #[sea_orm(column_type = "Text")]
    pub preview: String,
    /// 用户反馈：'positive' / 'negative' / 'irrelevant' / NULL
    pub feedback: Option<String>,
    /// 反馈时间戳（Unix 秒）
    pub feedback_at: Option<i64>,
    /// 是否在最终回复中被引用（0/1）
    #[sea_orm(default_value = 0)]
    pub used_in_response: i32,
    /// 重排后分数（可选，用于对比原始 score）
    pub score_after_rerank: Option<f64>,
    /// 创建时间戳（Unix 秒）
    #[sea_orm(default_value = 0)]
    pub created_at: i64,
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
    #[sea_orm(
        belongs_to = "super::knowledge_bases::Entity",
        from = "Column::KnowledgeBaseId",
        to = "super::knowledge_bases::Column::Id",
        on_delete = "Cascade"
    )]
    KnowledgeBase,
}

impl ActiveModelBehavior for ActiveModel {}
