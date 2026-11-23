// SPDX-License-Identifier: AGPL-3.0-only

//! `bot_workflow_runs` —— 编排运行实例（docs/bot-integration/P3/MODEL-P3 §2.2）。
//!
//! 一次 run 对应一个线程（`requester_conversation_id`）；`context_json` 保存
//! inputs / outputs / skipped；步骤执行行复用 `bot_assignments`（按
//! `workflow_run_id` 关联），run 终态须等整棵子树终态（ADR-P3-2 ⑤）。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[sea_orm(table_name = "bot_workflow_runs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    #[sea_orm(indexed)]
    pub workflow_id: String,
    #[sea_orm(indexed)]
    #[sea_orm(default_value = "queued")]
    pub status: String,
    #[sea_orm(default_value = "person")]
    pub origin: String,
    pub requester_bot_id: Option<String>,
    pub requester_conversation_id: String,
    pub anchor_message_id: Option<String>,
    #[sea_orm(default_value = "{}")]
    pub context_json: String,
    #[sea_orm(default_value = 0)]
    pub completed_steps: i32,
    #[sea_orm(default_value = 0)]
    pub total_steps: i32,
    pub error_code: Option<String>,
    pub error_detail: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
