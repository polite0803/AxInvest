// SPDX-License-Identifier: AGPL-3.0-only

//! `bot_assignments` —— Bot 委派合同（docs/bot-integration/P3/MODEL-P3 §2.3）。
//!
//! P3 放开多层委派 depth≤3：`root_id/parent_id/depth` 是运行期树边
//! （根 `root_id=id / parent_id=NULL / depth=1`，子行继承 root 并 `depth=parent+1`）；
//! `workflow_run_id/step_key` 沿树继承，标记编排步骤及其子孙。历史行两列为 NULL，
//! 语义 = 普通委派（声明式加列 + 可空，无 migration）。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[sea_orm(table_name = "bot_assignments")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 树根（自链第一单的 id；一层委派时 = 自身 id）。
    #[sea_orm(indexed)]
    pub root_id: String,
    /// 上游 assignment（根派发恒 NULL；子派发 = 发起该子轮的 assignment id）。
    #[sea_orm(indexed)]
    pub parent_id: Option<String>,
    /// 运行期树深度，取值域 1..3（MAX_DEPTH=3）。
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
    /// 编排 run id；子孙继承父行值，纯委派链恒 NULL。
    #[sea_orm(indexed)]
    pub workflow_run_id: Option<String>,
    /// `steps_json[].stepKey`；子孙继承父行值，纯委派链恒 NULL。
    pub step_key: Option<String>,
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
