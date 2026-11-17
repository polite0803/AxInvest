// SPDX-License-Identifier: AGPL-3.0-only

//! ComputerUseTool —— 无障碍树语义化桌面控制工具。
//!
//! 观察走系统无障碍树（query），操作走语义节点句柄（invoke / set_value / focus /
//! perform），键盘仅保留纯键盘注入（press_key）。全链路零坐标、零屏幕像素读取。

use crate::{PermissionResult, Tool, ToolCategory, ToolContext, ToolError, ToolResult};
use async_trait::async_trait;
use axagent_harness::ui_tree::{
    UiActionKind, UiKeyModifier, UiNodeId, UiScrollIntent, UiTreeProvider, UiTreeQuery,
};
use serde_json::Value;

/// 平台 provider 工厂与命令层共用同一实现。
fn provider() -> std::sync::Arc<dyn UiTreeProvider> {
    axagent_kit::ui_tree::platform_ui_tree_provider()
}

fn parse_node_id(input: &Value, name: &str) -> Result<UiNodeId, ToolError> {
    let id = &input["nodeId"];
    let backend = id["backend"]
        .as_str()
        .ok_or_else(|| ToolError::invalid_input_for(name, "nodeId.backend 缺失（字符串）"))?;
    let raw_id = id["rawId"]
        .as_str()
        .ok_or_else(|| ToolError::invalid_input_for(name, "nodeId.rawId 缺失（字符串）"))?;
    let session_gen = id["sessionGen"]
        .as_u64()
        .ok_or_else(|| ToolError::invalid_input_for(name, "nodeId.sessionGen 缺失（非负整数）"))?;
    Ok(UiNodeId { backend: backend.to_string(), raw_id: raw_id.to_string(), session_gen })
}

fn parse_modifiers(input: &Value) -> Vec<UiKeyModifier> {
    input["modifiers"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| match v.as_str()? {
                    "ctrl" => Some(UiKeyModifier::Ctrl),
                    "alt" => Some(UiKeyModifier::Alt),
                    "shift" => Some(UiKeyModifier::Shift),
                    "meta" => Some(UiKeyModifier::Meta),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

pub struct ComputerUseTool;

#[async_trait]
impl Tool for ComputerUseTool {
    fn name(&self) -> &str {
        "ComputerUse"
    }
    fn description(&self) -> &str {
        "通过系统无障碍树控制桌面：查询语义节点、激活/设值/聚焦/执行语义动作、纯键盘注入。零坐标。"
    }
    fn input_schema(&self) -> Value {
        let node_id_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "backend": { "type": "string", "description": "节点后端标识（来自查询结果）" },
                "rawId": { "type": "string", "description": "平台原始节点 id（不透明）" },
                "sessionGen": { "type": "integer", "description": "查询代次（来自查询结果）" }
            },
            "required": ["backend", "rawId", "sessionGen"]
        });
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["query", "invoke", "set_value", "focus", "perform", "press_key"],
                    "description": "要执行的操作"
                },
                "query": {
                    "type": "object",
                    "description": "query 操作的查询条件（空对象 = 查全部）",
                    "properties": {
                        "application": { "type": "string" },
                        "windowTitle": { "type": "string" },
                        "role": { "type": "string" },
                        "nameContains": { "type": "string" },
                        "maxDepth": { "type": "integer" },
                        "includeOffscreen": { "type": "boolean" }
                    }
                },
                "nodeId": node_id_schema,
                "value": { "type": "string", "description": "set_value 操作的目标值" },
                "semanticAction": {
                    "type": "string",
                    "enum": ["invoke", "setValue", "focus", "toggle", "select", "expand", "collapse", "scrollIntoView", "scroll"],
                    "description": "perform 操作的语义动作"
                },
                "scrollIntent": {
                    "type": "string",
                    "enum": ["up", "down", "left", "right"]
                },
                "key": { "type": "string", "description": "press_key 的按键名（如 enter / tab / a）" },
                "text": { "type": "string", "description": "press_key 的可选文本注入" },
                "modifiers": {
                    "type": "array",
                    "items": { "type": "string", "enum": ["ctrl", "alt", "shift", "meta"] },
                    "description": "press_key 的修饰键列表"
                }
            },
            "required": ["action"]
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Desktop
    }
    fn is_concurrency_safe(&self) -> bool {
        false
    }
    fn is_destructive(&self) -> bool {
        true
    }

    fn check_permissions(&self, _input: &Value, _ctx: &ToolContext) -> PermissionResult {
        PermissionResult::Ask("桌面控制需要用户确认。".into())
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let name = self.name();
        // SECURITY (C4): AI 工具路径与 Tauri 命令、内置 MCP 描述符共用同一授权闸门。
        axagent_kit::permission::ensure_computer_control_granted()
            .map_err(|e| ToolError::execution_failed_for(name, e))?;
        let action = input["action"].as_str().unwrap_or("query");

        let output = match action {
            "query" => {
                let query: UiTreeQuery =
                    serde_json::from_value(input["query"].clone()).map_err(|e| {
                        ToolError::invalid_input_for(name, format!("query 参数无效: {e}"))
                    })?;
                let snapshot = provider()
                    .query(query)
                    .await
                    .map_err(|e| ToolError::execution_failed_for(name, e.to_string()))?;
                serde_json::to_string(&snapshot)
                    .map_err(|e| ToolError::execution_failed_for(name, e.to_string()))?
            },
            "invoke" => {
                let id = parse_node_id(&input, name)?;
                provider()
                    .invoke(&id)
                    .await
                    .map_err(|e| ToolError::execution_failed_for(name, e.to_string()))?;
                format!("已激活节点: {}:{}", id.backend, id.raw_id)
            },
            "set_value" => {
                let id = parse_node_id(&input, name)?;
                let value = input["value"].as_str().ok_or_else(|| {
                    ToolError::invalid_input_for(name, "set_value 需要 value 参数")
                })?;
                provider()
                    .set_value(&id, value)
                    .await
                    .map_err(|e| ToolError::execution_failed_for(name, e.to_string()))?;
                format!("已设置节点值: {}:{}", id.backend, id.raw_id)
            },
            "focus" => {
                let id = parse_node_id(&input, name)?;
                provider()
                    .focus(&id)
                    .await
                    .map_err(|e| ToolError::execution_failed_for(name, e.to_string()))?;
                format!("已聚焦节点: {}:{}", id.backend, id.raw_id)
            },
            "perform" => {
                let id = parse_node_id(&input, name)?;
                let semantic = input["semanticAction"].as_str().ok_or_else(|| {
                    ToolError::invalid_input_for(name, "perform 需要 semanticAction 参数")
                })?;
                let action_kind: UiActionKind =
                    serde_json::from_value(serde_json::Value::String(semantic.to_string()))
                        .map_err(|e| {
                            ToolError::invalid_input_for(name, format!("semanticAction 无效: {e}"))
                        })?;
                let scroll_intent: Option<UiScrollIntent> = input
                    .get("scrollIntent")
                    .filter(|v| !v.is_null())
                    .map(|v| serde_json::from_value(v.clone()))
                    .transpose()
                    .map_err(|e| {
                        ToolError::invalid_input_for(name, format!("scrollIntent 无效: {e}"))
                    })?;
                provider()
                    .perform(&id, action_kind, scroll_intent)
                    .await
                    .map_err(|e| ToolError::execution_failed_for(name, e.to_string()))?;
                format!("已对节点执行 {semantic}: {}:{}", id.backend, id.raw_id)
            },
            "press_key" => {
                let key = input["key"]
                    .as_str()
                    .ok_or_else(|| ToolError::invalid_input_for(name, "press_key 需要 key 参数"))?;
                let text = input["text"].as_str();
                provider()
                    .press_key(key, &parse_modifiers(&input), text)
                    .await
                    .map_err(|e| ToolError::execution_failed_for(name, e.to_string()))?;
                format!("已注入按键: {key}")
            },
            _ => {
                return Err(ToolError::invalid_input_for(
                    name,
                    format!(
                        "未知操作: {action}。支持: query, invoke, set_value, focus, perform, press_key"
                    ),
                ));
            },
        };

        Ok(ToolResult::success(output))
    }
}
