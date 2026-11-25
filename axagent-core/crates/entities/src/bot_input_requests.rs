// SPDX-License-Identifier: AGPL-3.0-only

//! `bot_input_requests` —— 持久化待答请求（docs/bot-integration/P4/MODEL-P4.md §3）。
//!
//! P4 不改 assignment / run 状态机（ADR-P4-3）：待答生命周期独立成表
//! `pending → answered | expired | cancelled`，终态不可回退。`ask_id` 与
//! `agent-ask-user` 的 askId 一致，用于答复投递与去重；`UNIQUE(ask_id)` 走
//! `reconcile/extras.rs` 的 `MULTI_COL_INDEXES`。
//!
//! `workflow_run_id` / `assignment_id` 为无人值守预留列（D4=A 下恒 NULL），
//! `origin` 同样恒 `person`（ADR-P4-6）。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[sea_orm(table_name = "bot_input_requests")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 与 `agent-ask-user` 事件 / `agent_respond_ask` 的 askId 一致。
    pub ask_id: String,
    /// 所在 Bot 线程。
    #[sea_orm(indexed)]
    pub conversation_id: String,
    /// 线程归属 Bot（取 `conversations.bot_id`）；通用聊天为 NULL 且不落行。
    #[sea_orm(indexed)]
    pub bot_id: Option<String>,
    /// 预留（D4=A 恒 NULL）：无人值守轮的 run / assignment 关联。
    #[sea_orm(indexed)]
    pub workflow_run_id: Option<String>,
    #[sea_orm(indexed)]
    pub assignment_id: Option<String>,
    /// `person` / `routine` / `workflow`（D4=A 恒 `person`）。
    #[sea_orm(default_value = "person")]
    pub origin: String,
    /// 主问题文本（沿用桥的「只取第一个问题」）。
    pub question: String,
    /// 选项字符串数组的 JSON（命令层 parse 成 `options: string[]`）。
    #[sea_orm(default_value = "[]")]
    pub options_json: String,
    /// `pending` / `answered` / `expired` / `cancelled`。
    #[sea_orm(indexed, default_value = "pending")]
    pub status: String,
    pub answer: Option<String>,
    pub answered_at: Option<i64>,
    /// `created_at + 300_000`（ADR-P4-4）。
    pub expires_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
