// SPDX-License-Identifier: AGPL-3.0-only

//! Bot 归属层共享 DTO（docs/bot-integration/MODEL.md / IPC.md）。
//!
//! 权威定义在 harness：命令层、DAO 层都经 `axagent_harness::types::*` 引用，
//! 不在各自 crate 里复制一份（AGENTS.md 禁区 12）。

use serde::{Deserialize, Serialize};

use super::provider_model::deserialize_double_option;

/// Bot 记录（输出 DTO，camelCase 序列化给前端）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bot {
    pub id: String,
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub soul: String,
    pub soul_hash: String,
    pub soul_drift: bool,
    pub icon: String,
    pub color: Option<String>,
    pub section: String,
    pub model_id: Option<String>,
    pub provider_id: Option<String>,
    pub effort: Option<String>,
    pub workdir: Option<String>,
    pub approval_mode: String,
    pub agent_profile_id: Option<String>,
    pub is_enabled: bool,
    pub sort_order: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 新建 Bot 的输入。全部可选字段都带 `#[serde(default)]`：
/// 前端省略键与显式传 `null` 等价（Tauri 按缺失处理，行为一致）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBot {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub soul: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub section: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub workdir: Option<String>,
    #[serde(default)]
    pub approval_mode: Option<String>,
    #[serde(default)]
    pub agent_profile_id: Option<String>,
}

/// 局部更新补丁：`None` = 不改，`Some(None)` = 显式置空（与 IPC.md 的
/// `UpdateBotInput` 的 `Option<Option<T>>` 语义一一对应）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateBotPatch {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub title: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub description: Option<Option<String>>,
    #[serde(default)]
    pub soul: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub color: Option<Option<String>>,
    #[serde(default)]
    pub section: Option<String>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub model_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub provider_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub effort: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub workdir: Option<Option<String>>,
    #[serde(default)]
    pub approval_mode: Option<String>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub agent_profile_id: Option<Option<String>>,
    #[serde(default)]
    pub is_enabled: Option<bool>,
    #[serde(default)]
    pub sort_order: Option<i32>,
}

// ───────────────────────── P2-b：部门与委派（IPC-P2B §2） ─────────────────────────

/// 部门记录（输出 DTO）。`bot_ids` 由命令层 join 成员表得出。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotDepartment {
    pub id: String,
    pub name: String,
    pub description: String,
    pub chief_bot_id: Option<String>,
    pub bot_ids: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 新建部门输入。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBotDepartment {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub chief_bot_id: Option<String>,
}

/// 更新部门补丁：`None` = 不改，`Some(None)` = 显式置空（Chief 可摘除）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateBotDepartmentPatch {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub description: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub chief_bot_id: Option<Option<String>>,
}

/// 委派合同（输出 DTO，与 `bot_assignments` 实体一一对应；状态五态）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotAssignment {
    pub id: String,
    pub root_id: String,
    pub parent_id: Option<String>,
    pub depth: i32,
    /// `person` / `bot` / `routine`。
    pub origin: String,
    pub requester_bot_id: Option<String>,
    pub requester_conversation_id: Option<String>,
    pub assignee_bot_id: String,
    /// 冗余展示名（命令层 join bots 表回填，部门删除后仍可读）。
    pub assignee_name: String,
    /// `consult` / `dispatch`。
    pub mode: String,
    pub task_text: String,
    /// `queued/running/completed/failed/cancelled`。
    pub status: String,
    pub result_text: Option<String>,
    pub error_code: Option<String>,
    pub error_detail: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 发起委派输入（`create_bot_assignment` 命令 + consult 工具 runner 共用 DTO）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBotAssignment {
    pub assignee_bot_id: String,
    /// 1..20000 字符。
    pub task_text: String,
    /// `consult` / `dispatch`；命令层只接受 `dispatch`（IPC-P2B §2.3）。
    #[serde(default = "default_assignment_mode")]
    pub mode: String,
    #[serde(default)]
    pub requester_conversation_id: Option<String>,
    #[serde(default)]
    pub requester_bot_id: Option<String>,
}

fn default_assignment_mode() -> String {
    "dispatch".to_string()
}
