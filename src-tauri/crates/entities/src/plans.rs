// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "plans")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub conversation_id: String,
    pub user_message_id: String,
    pub title: String,
    /// JSON-encoded Vec<PlanStep>
    #[sea_orm(default_value = "[]")]
    pub steps_json: String,
    #[sea_orm(default_value = "draft")]
    pub status: String,
    /// 执行授权位（P0-A）：0=未授权，1=已授权。
    ///
    /// **与 `status` 生命周期解耦**：`status` 表达「计划自身走到哪一步」
    /// （draft/reviewing/executing/completed/...），本字段表达「用户是否批准执行」。
    /// `plan_execute` 以本字段为执行判据，而非 `status`。
    ///
    /// 写入者受限：仅 `commands/plan.rs::AUTHORIZED_BY_WHITELIST`
    /// （user/system/api/ui/automation）可写入，**模型路径不得写入**。
    #[sea_orm(default_value = 0)]
    pub execution_authorized: i32,
    /// 授权时间（毫秒时间戳）。未授权为 None。
    pub authorized_at: Option<i64>,
    /// 授权来源（白名单值）。未授权为 None。
    pub authorized_by: Option<String>,
    #[sea_orm(default_value = 1)]
    pub is_active: i32,
    pub created_under_strategy: Option<String>,
    pub reason: Option<String>,
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

impl Related<super::conversations::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Conversation.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
