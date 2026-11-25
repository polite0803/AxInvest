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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// 编排 run id；子孙继承父行值，纯委派链恒 NULL（P3）。
    pub workflow_run_id: Option<String>,
    /// 编排步骤 key；子孙继承父行值，纯委派链恒 NULL（P3）。
    pub step_key: Option<String>,
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
    /// P3 内部字段：父 assignment id（不暴露给 Tauri 入参 DTO 契约）。
    #[serde(default)]
    pub parent_assignment_id: Option<String>,
    /// P3 内部字段：链根 assignment id。
    #[serde(default)]
    pub root_assignment_id: Option<String>,
    /// P3 内部字段：编排 run id（步骤 assignment 落库用）。
    #[serde(default)]
    pub workflow_run_id: Option<String>,
    /// P3 内部字段：编排步骤 key。
    #[serde(default)]
    pub step_key: Option<String>,
    /// P3 内部字段：任务来源（person / bot / routine / workflow）。
    #[serde(default)]
    pub origin: Option<String>,
    /// P3 内部字段：回执落点（编排 run 线程）。
    #[serde(default)]
    pub requester_conversation_id_override: Option<String>,
}

fn default_assignment_mode() -> String {
    "dispatch".to_string()
}

// ───────────────────────────── P3 编排 DTO ─────────────────────────────
// 契约权威：docs/bot-integration/P3/IPC-P3.md §2（camelCase 序列化）。

fn default_step_timeout() -> i32 {
    300
}
fn default_on_failure() -> String {
    "abort".to_string()
}
fn default_max_concurrency() -> i32 {
    2
}
fn default_total_timeout() -> i32 {
    900
}

/// 编排步骤（`steps_json` 元素，与前端 TS 类型共用同一份契约）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotWorkflowStep {
    pub step_key: String,
    pub bot_id: String,
    pub prompt_template: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default = "default_step_timeout")]
    pub timeout_secs: i32,
    #[serde(default = "default_on_failure")]
    pub on_failure: String,
}

/// 编排定义 DTO（DB 的 steps_json 在命令层 parse 为 steps 数组）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotWorkflow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub owner_scope: String,
    pub department_id: Option<String>,
    pub steps: Vec<BotWorkflowStep>,
    pub max_concurrency: i32,
    pub total_timeout_secs: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBotWorkflow {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_owner_scope")]
    pub owner_scope: String,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub department_id: Option<Option<String>>,
    pub steps: Vec<BotWorkflowStep>,
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: i32,
    #[serde(default = "default_total_timeout")]
    pub total_timeout_secs: i32,
}

fn default_owner_scope() -> String {
    "instance".to_string()
}

/// `None` = 不改；`Some(None)` = 显式清空（P2-a 的 Option<Option<T>> 口径）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateBotWorkflowPatch {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub description: Option<Option<String>>,
    #[serde(default)]
    pub owner_scope: Option<String>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub department_id: Option<Option<String>>,
    #[serde(default)]
    pub steps: Option<Vec<BotWorkflowStep>>,
    #[serde(default)]
    pub max_concurrency: Option<i32>,
    #[serde(default)]
    pub total_timeout_secs: Option<i32>,
}

/// 单步输出（写入 run.context_json.outputs）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotWorkflowStepOutput {
    pub assignment_id: String,
    pub result_text: String,
}

/// run 上下文（inputs / outputs / skipped，整列 JSON 演进）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotWorkflowRunContext {
    #[serde(default)]
    pub inputs: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub outputs: std::collections::HashMap<String, BotWorkflowStepOutput>,
    #[serde(default)]
    pub skipped: Vec<String>,
}

/// 运行实例 DTO。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotWorkflowRun {
    pub id: String,
    pub workflow_id: String,
    pub workflow_name: String,
    pub status: String,
    pub origin: String,
    pub requester_bot_id: Option<String>,
    pub requester_conversation_id: String,
    pub anchor_message_id: Option<String>,
    pub context: BotWorkflowRunContext,
    pub completed_steps: i32,
    pub total_steps: i32,
    pub error_code: Option<String>,
    pub error_detail: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 新建 run 的内部输入（命令层构造，不直接暴露为 Tauri 入参）。
#[derive(Debug, Clone)]
pub struct CreateBotWorkflowRun {
    pub workflow_id: String,
    pub workflow_name: String,
    pub origin: String,
    pub requester_bot_id: Option<String>,
    pub requester_conversation_id: String,
    pub total_steps: i32,
    pub inputs: std::collections::HashMap<String, String>,
}

/// run 状态补丁（runner 单入口）。
#[derive(Debug, Clone, Default)]
pub struct BotWorkflowRunPatch {
    pub status: Option<String>,
    pub anchor_message_id: Option<String>,
    pub context: Option<BotWorkflowRunContext>,
    pub completed_steps: Option<i32>,
    pub error_code: Option<String>,
    pub error_detail: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

// ─────────────────────────── P4 ───────────────────────────
// 部门运行配额（docs/bot-integration/P4/MODEL-P4 §2、IPC-P4 §2）。

/// 部门运行配额 DTO。`is_default = true` 表示该部门没有配额行，字段值即 P3
/// 默认值（ADR-P4-1：无配额行的部门行为与 P3 逐项一致）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotDepartmentQuota {
    pub department_id: String,
    pub max_concurrency: i32,
    pub total_timeout_secs: i32,
    pub max_steps: i32,
    pub max_in_flight_per_depth: i32,
    pub step_timeout_secs: i32,
    pub max_concurrent_runs: i32,
    pub is_default: bool,
    pub updated_at: Option<i64>,
}

/// 写配额入参（全量 upsert，IPC-P4 §2）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotDepartmentQuotaInput {
    pub department_id: String,
    pub max_concurrency: i32,
    pub total_timeout_secs: i32,
    pub max_steps: i32,
    pub max_in_flight_per_depth: i32,
    pub step_timeout_secs: i32,
    pub max_concurrent_runs: i32,
}

/// P3 默认配额的唯一真值来源：六个上限逐项等于 P3 冻结常量
/// （`bot_workflow_runner::{MAX_CONCURRENCY,TOTAL_TIMEOUT_SECS,MAX_STEPS}`、
/// `bot_assignment_runner::{MAX_IN_FLIGHT_PER_DEPTH,DISPATCH_TIMEOUT_SECS}`，
/// 以及 `bot_workflow_steps` 的按步超时上限与 P3 并发 run 预算）。
/// 单测 `default_matches_p3_constants` 锁定二者一致，禁止各改一头。
pub mod quota_defaults {
    pub const MAX_CONCURRENCY: i32 = 2;
    pub const TOTAL_TIMEOUT_SECS: i32 = 900;
    pub const MAX_STEPS: i32 = 12;
    pub const MAX_IN_FLIGHT_PER_DEPTH: i32 = 2;
    pub const STEP_TIMEOUT_SECS: i32 = 300;
    pub const MAX_CONCURRENT_RUNS: i32 = 2;
}

impl BotDepartmentQuota {
    /// 无配额行时的 DTO：P3 默认值 + `is_default = true` + `updated_at = None`。
    pub fn defaults(department_id: &str) -> Self {
        Self {
            department_id: department_id.to_string(),
            max_concurrency: quota_defaults::MAX_CONCURRENCY,
            total_timeout_secs: quota_defaults::TOTAL_TIMEOUT_SECS,
            max_steps: quota_defaults::MAX_STEPS,
            max_in_flight_per_depth: quota_defaults::MAX_IN_FLIGHT_PER_DEPTH,
            step_timeout_secs: quota_defaults::STEP_TIMEOUT_SECS,
            max_concurrent_runs: quota_defaults::MAX_CONCURRENT_RUNS,
            is_default: true,
            updated_at: None,
        }
    }
}

/// 持久化待答请求 DTO（IPC-P4 §2）。`options` 由 `options_json` 解析而来，
/// 前端永不接触 JSON 字符串；`status` / `origin` 值域见 [`input_status`] /
/// [`input_origin`]。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotInputRequest {
    pub id: String,
    pub ask_id: String,
    pub conversation_id: String,
    pub bot_id: Option<String>,
    /// 预留列（D4=A 恒 None）。
    pub workflow_run_id: Option<String>,
    /// 预留列（D4=A 恒 None）。
    pub assignment_id: Option<String>,
    pub origin: String,
    pub question: String,
    pub options: Vec<String>,
    pub status: String,
    pub answer: Option<String>,
    pub answered_at: Option<i64>,
    pub expires_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// `list_bot_input_requests` 的过滤入参：四项全可选，省略即不过滤。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotInputRequestFilter {
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub bot_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub workflow_run_id: Option<String>,
}

/// 待答请求四态（MODEL-P4 §3）：`pending → answered | expired | cancelled`，
/// 终态不可回退。
pub mod input_status {
    pub const PENDING: &str = "pending";
    pub const ANSWERED: &str = "answered";
    pub const EXPIRED: &str = "expired";
    pub const CANCELLED: &str = "cancelled";

    /// 是否终态（answered / expired / cancelled）。
    pub fn is_terminal(status: &str) -> bool {
        matches!(status, ANSWERED | EXPIRED | CANCELLED)
    }
}

/// 待答请求来源（D4=A：交互线程一律 `person`，其余为预留）。
pub mod input_origin {
    pub const PERSON: &str = "person";
    pub const ROUTINE: &str = "routine";
    pub const WORKFLOW: &str = "workflow";
}
