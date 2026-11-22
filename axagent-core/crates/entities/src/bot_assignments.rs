// SPDX-License-Identifier: AGPL-3.0-only

//! `bot_assignments` —— Bot 委派合同（docs/bot-integration/P2/MODEL-P2B §2.3）。
//!
//! `root_id`/`parent_id`/`depth` 三列随表落地，但 P2-b 校验规则恒
//! `depth == 1`、`parent_id == NULL`（ADR-P2B-5：一层委派 + 防环，P3 解禁
//! 只改工具注册条件与赋值逻辑）。`executions`/`evidence` 列留 P3 加列
//! （声明式加列无迁移成本）。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[sea_orm(table_name = "bot_assignments")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 树根（自链第一单的 id；一层委派时 = 自身 id）。
    pub root_id: String,
    /// 上游 assignment（P2-b 恒 NULL，字段先落，Q1）。
    pub parent_id: Option<String>,
    /// P2-b 恒 1。
    #[sea_orm(default_value = 1)]
    pub depth: i32,
    /// `person` / `bot` / `routine`（例程发起，Q6）。
    #[sea_orm(default_value = "person")]
    pub origin: String,
    /// person 发起时 NULL。
    pub requester_bot_id: Option<String>,
    /// 回执落回的发起线程。
    pub requester_conversation_id: Option<String>,
    pub assignee_bot_id: String,
    /// `consult` / `dispatch`。
    #[sea_orm(default_value = "dispatch")]
    pub mode: String,
    pub task_text: String,
    /// `queued/running/completed/failed/cancelled` 五态（GAP G-B7）。
    #[sea_orm(indexed)]
    #[sea_orm(default_value = "queued")]
    pub status: String,
    /// completed 时的回执正文。
    pub result_text: Option<String>,
    /// failed 时的错误码（复用错误码族，IPC-P2B §4）。
    pub error_code: Option<String>,
    /// 自由文案。
    pub error_detail: Option<String>,
    /// 运行窗口（超时判定与展示）。
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
