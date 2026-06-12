// SPDX-License-Identifier: AGPL-3.0-only

//! 工具执行器 —— 解析 ToolNodeConfig 后通过注入的回调或 ToolRegistry 调用 MCP 工具。
//!
//! 默认无回调时返回清晰的"需要注入"错误，避免静默失败。
//!
//! 调用优先级：
//!   1. `context.tool_registry.execute_tool()` — 中心化路径（权限/限流/脱敏集成）
//!   2. `context.callbacks.tool_handlers` 按 tool_name 精确匹配（多路注册）
//!   3. `context.callbacks.tool_fallback` 旧版全局回调（兼容）

use crate::work_engine::execution_state::ExecutionState;
use crate::work_engine::node_executor_trait::{
    NodeError, NodeExecutorTrait, NodeOutput, error_code,
};
use async_trait::async_trait;
use axagent_core::workflow_types::WorkflowNode;
use axagent_harness::tool::ToolContext;
use std::pin::Pin;
use std::sync::Arc;
use tracing;

pub type ToolCallback = Arc<
    dyn Fn(
            String,
            serde_json::Value,
        )
            -> Pin<Box<dyn std::future::Future<Output = Result<serde_json::Value, String>> + Send>>
        + Send
        + Sync,
>;

pub struct ToolExecutor;

impl ToolExecutor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ToolExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl NodeExecutorTrait for ToolExecutor {
    fn node_type(&self) -> &'static str {
        "tool"
    }

    async fn execute(
        &self,
        node: &WorkflowNode,
        context: &ExecutionState,
    ) -> Result<NodeOutput, NodeError> {
        let WorkflowNode::Tool(tool_node) = node else {
            return Err(NodeError::type_mismatch(
                "tool".to_string(),
                super::node_type_name(node).to_string(),
            ));
        };

        // 解析输入映射
        let mut resolved_args: serde_json::Value =
            tool_node
                .config
                .input_mapping
                .iter()
                .fold(serde_json::json!({}), |mut acc, (k, v)| {
                    let resolved = resolve_var_path(v, context);
                    acc[k] = resolved.unwrap_or(serde_json::Value::Null);
                    acc
                });

        // ── 注入模板参数：将 workflow template variables 作为 _template_vars
        //    传递给工具函数，使 scoring/valuation/indicator 等层面可以
        //    读取用户在设置面板中配置的参数（权重、阈值、周期等）。
        if let Some(template_vars) = collect_template_vars(context) {
            resolved_args["_template_vars"] = template_vars;
        }

        let tool_name = &tool_node.config.tool_name;

        // ── 权限校验（基于 ExecutionState.tool_permissions） ──
        if let Some(ref perms) = context.tool_permissions {
            if perms.forbidden_tools.iter().any(|t| t == tool_name) {
                let reason = format!("权限拒绝: 工具 '{tool_name}' 在禁止调用列表中");
                tracing::warn!("{reason}");
                return Err(NodeError::exec_failed(error_code::TOOL_CALL_FAILED, reason));
            }
            if let Some(ref allowed) = perms.allowed_tools
                && !allowed.iter().any(|t| t == tool_name)
            {
                let reason = format!("权限拒绝: 工具 '{tool_name}' 不在允许调用列表中");
                tracing::warn!("{reason}");
                return Err(NodeError::exec_failed(error_code::TOOL_CALL_FAILED, reason));
            }
        }

        // ── 1. 优先走 ToolRegistry 中心化路径 ──
        if let Some(ref tool_registry) = context.tool_registry {
            tracing::warn!("[ToolExecutor] 工具 '{tool_name}' 通过 ToolRegistry 中心化路径执行");

            let mut tool_ctx =
                ToolContext::new(".").with_conversation(context.execution_id.clone());
            // 附加权限
            if let Some(ref perms) = context.tool_permissions {
                tool_ctx.permissions = Some(perms.clone());
            }

            let result = tool_registry
                .execute_tool(tool_name, resolved_args.clone(), &tool_ctx)
                .await
                .map_err(|e| {
                    NodeError::exec_failed(
                        error_code::TOOL_CALL_FAILED,
                        format!("ToolRegistry 调用失败: {e}"),
                    )
                })?;

            return Ok(NodeOutput {
                output: serde_json::json!({
                    "tool_name": tool_name,
                    "result": result.content,
                    "truncated": result.truncated,
                    "is_error": result.is_error,
                    "node_id": node.base_id(),
                }),
                output_var: Some(tool_node.config.output_var.clone()),
            });
        }

        // ── 2. 回退：查找回调（多路注册 → fallback → 未配置） ──
        let cb: Option<ToolCallback> = context
            .callbacks
            .as_ref()
            .and_then(|cbs| cbs.tool_handlers.get(tool_name).cloned())
            .or_else(|| {
                context
                    .callbacks
                    .as_ref()
                    .and_then(|cbs| cbs.tool_fallback.clone())
            });

        let output = if let Some(ref cb) = cb {
            tracing::warn!(
                "[ToolExecutor] 工具 '{tool_name}' 通过回调路径执行（ToolRegistry 未配置）"
            );
            cb(tool_name.clone(), resolved_args.clone())
                .await
                .map_err(|e| {
                    NodeError::exec_failed(
                        error_code::TOOL_CALL_FAILED,
                        format!("Tool call failed: {e}"),
                    )
                })?
        } else {
            return Err(NodeError::exec_failed(
                error_code::TOOL_CALL_FAILED,
                format!(
                    "工具 '{}' 未注册，请通过 WorkEngine::register_tool_handler() 注册或注入 ToolRegistry",
                    tool_name
                ),
            ));
        };

        Ok(NodeOutput {
            output: serde_json::json!({
                "tool_name": tool_name,
                "result": output,
                "node_id": node.base_id(),
            }),
            output_var: Some(tool_node.config.output_var.clone()),
        })
    }
}

fn resolve_var_path(path: &str, context: &ExecutionState) -> Option<serde_json::Value> {
    super::resolve_var_path(path, &context.variables)
}

/// 从 ExecutionState 中收集所有模板变量（非节点输出），用于注入工具函数的 _template_vars。
///
/// 判断逻辑：如果一个变量 key 不以节点 ID 前缀（如 t- / a- / d- / s- / j-/ m-/ r- ）开头，
/// 且不在已知的系统变量列表中，则视为模板变量。工具函数通过 `input["_template_vars"]`
/// 读取用户在设置面板中配置的权重 / 阈值 / 周期等参数。
///
/// 实现复用 `super::var_filter` 的反集（`is_data_var` 的否集）。
fn collect_template_vars(context: &ExecutionState) -> Option<serde_json::Value> {
    let vars: serde_json::Map<String, serde_json::Value> = context
        .variables
        .iter()
        .filter(|(key, _)| !super::is_data_var(key))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    if vars.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(vars))
    }
}
