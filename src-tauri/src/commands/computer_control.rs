// SPDX-License-Identifier: AGPL-3.0-only

//! 电脑控制命令面（无障碍树语义化阶段3）。
//!
//! 观察走 [`find_ui_elements`] 查询系统无障碍树快照，操作走语义动作命令，
//! 全链路不接受屏幕坐标；键盘仅保留 [`press_key`] 纯键盘注入（按键 + 修饰键 + 可选文本）。
//! 平台实现由 `axagent_kit::ui_tree::platform_ui_tree_provider()` 按编译目标提供。

use axagent_agent_macro::agent_command;
use axagent_harness::ui_tree::{
    UiActionKind, UiKeyModifier, UiNodeId, UiScrollIntent, UiTreeProvider, UiTreeQuery,
};
use axagent_kit::permission;

use crate::commands::error::{ErrorCategory, ErrorResponse};

/// 构造 current 平台的无障碍树 provider。
///
/// 每次命令调用构造一个新实例：平台 provider 只持有原子代次与轻量句柄表，
/// 不持有跨调用的 D-Bus / COM 连接，避免长连接生命周期绑定到 tokio runtime。
fn ui_tree_provider() -> std::sync::Arc<dyn UiTreeProvider> {
    axagent_kit::ui_tree::platform_ui_tree_provider()
}

fn map_ui_error(e: impl std::fmt::Display) -> String {
    String::from(ErrorResponse::from_error(e, ErrorCategory::Unrecoverable))
}

/// 授予 computer_control 权限（由前端授权流程调用）。
/// 实际状态保存在 `axagent_kit::permission`，命令与 AI 工具路径共用同一闸门。
#[agent_command(domain = computer_control, safety = Caution, call_mode = Manual, description = "授予电脑控制权限")]
#[tauri::command]
pub fn grant_computer_control_permission() {
    permission::grant_computer_control();
}

/// 撤销 computer_control 权限。
#[agent_command(domain = computer_control, safety = Dangerous, call_mode = Manual, description = "撤销电脑控制权限")]
#[tauri::command]
pub fn revoke_computer_control_permission() {
    permission::revoke_computer_control();
}

/// 查询当前 computer_control 权限状态。
#[agent_command(domain = computer_control, safety = Safe, call_mode = Manual, description = "查询电脑控制权限状态")]
#[tauri::command]
pub fn is_computer_control_granted() -> bool {
    permission::is_computer_control_granted()
}

// 说明：所有 computer_control 敏感操作的实际权限校验已下沉到
// `axagent_kit::permission::*` 内部（ensure_computer_control_granted），
// 命令 / AI 工具（ComputerUseTool）共用同一 C4 闸门，避免任一入口绕过授权。

/// 查询无障碍树快照（role / name / value / states / supportedActions）。
#[agent_command(domain = computer_control, safety = Safe, call_mode = Manual, description = "查询无障碍树")]
#[tauri::command]
pub async fn find_ui_elements(
    query: UiTreeQuery,
) -> Result<axagent_harness::ui_tree::UiTreeSnapshot, String> {
    ui_tree_provider().query(query).await.map_err(map_ui_error)
}

/// 激活语义节点（按钮 / 菜单项等）。
#[agent_command(domain = computer_control, safety = Dangerous, call_mode = Manual, description = "激活UI元素")]
#[tauri::command]
pub async fn invoke_ui_element(id: UiNodeId) -> Result<(), String> {
    ui_tree_provider().invoke(&id).await.map_err(map_ui_error)
}

/// 设置语义节点的值（输入框文本等）。
#[agent_command(domain = computer_control, safety = Dangerous, call_mode = Manual, description = "设置UI元素值")]
#[tauri::command]
pub async fn set_ui_element_value(id: UiNodeId, value: String) -> Result<(), String> {
    ui_tree_provider().set_value(&id, &value).await.map_err(map_ui_error)
}

/// 聚焦语义节点。
#[agent_command(domain = computer_control, safety = Caution, call_mode = Manual, description = "聚焦UI元素")]
#[tauri::command]
pub async fn focus_ui_element(id: UiNodeId) -> Result<(), String> {
    ui_tree_provider().focus(&id).await.map_err(map_ui_error)
}

/// 对语义节点执行动作（Toggle / Select / Expand / Collapse / ScrollIntoView / Scroll）。
#[agent_command(domain = computer_control, safety = Dangerous, call_mode = Manual, description = "执行UI元素语义动作")]
#[tauri::command]
pub async fn perform_ui_element_action(
    id: UiNodeId,
    action: UiActionKind,
    scroll_intent: Option<UiScrollIntent>,
) -> Result<(), String> {
    ui_tree_provider().perform(&id, action, scroll_intent).await.map_err(map_ui_error)
}

/// 纯键盘注入（无坐标）：按键 + 修饰键 + 可选文本。
#[agent_command(domain = computer_control, safety = Dangerous, call_mode = Manual, description = "按键")]
#[tauri::command]
pub async fn press_key(
    key: String,
    modifiers: Vec<UiKeyModifier>,
    text: Option<String>,
) -> Result<(), String> {
    ui_tree_provider().press_key(&key, &modifiers, text.as_deref()).await.map_err(map_ui_error)
}
