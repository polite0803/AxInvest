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
    NodeError, NodeExecutorTrait, NodeOutput, check_cancellation_or_pause, error_code,
};
use async_trait::async_trait;
use axagent_harness::tool::ToolContext;
use axagent_harness::workflow_types::WorkflowNode;
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
        // ── 取消/暂停检查：避免已取消/暂停的 Workflow 继续执行工具并消耗资源 ──
        check_cancellation_or_pause(context).await?;

        let WorkflowNode::Tool(tool_node) = node else {
            return Err(NodeError::type_mismatch(
                "tool".to_string(),
                super::node_type_name(node).to_string(),
            ));
        };

        // 解析输入映射
        //
        // 使用全限定 `super::resolve_var_path`（共享宽松版），**不用 `use` 导入**：
        // 本项目有「本地定义 vs 导入同名静默遮蔽」的先例，全限定调用可彻底规避。
        // 与旧的文件内私有严格版相比，共享版会穿透 result/content 这类 JSON 字符串
        // 包裹（见 executors/mod.rs:106-164），因此 `<节点>.content.<字段>` /
        // `<节点>.result.<字段>` 形式的 input_mapping 由此变为可解析。
        let resolved_args: serde_json::Value =
            tool_node.config.input_mapping.iter().fold(serde_json::json!({}), |mut acc, (k, v)| {
                let resolved = super::resolve_var_path(v, &context.variables);
                acc[k] = resolved.unwrap_or(serde_json::Value::Null);
                acc
            });

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

        // ── Dry Run 短路 ──
        // 单步调试模式下不执行真实工具调用（避免 MCP 副作用、外部 API 请求），
        // 返回模拟执行结果。工具名与参数保留以供下游节点识别节点配置。
        if context.dry_run {
            tracing::info!("[ToolExecutor] dry_run 模式：工具 '{}' 短路返回模拟结果", tool_name);
            return Ok(NodeOutput {
                output: serde_json::json!({
                    "tool_name": tool_name,
                    "result": "[DRY RUN] 工具模拟执行结果",
                    "args": resolved_args,
                    "dry_run": true,
                    "node_id": node.base_id(),
                }),
                output_var: Some(tool_node.config.output_var.clone()),
                control: None,
            });
        }

        // ── 1. 优先走 ToolRegistry 中心化路径（含权限/审计/脱敏）──
        // 注意：不通过 find() 守卫——ToolRegistry 自身负责处理未知工具名。
        // CapturingRegistry 等测试桩不注册具体工具（find() 返回 None），
        // 但 execute_tool() 为合法调用路径，不应被跳过。
        if let Some(ref tool_registry) = context.tool_registry {
            tracing::info!("[ToolExecutor] 工具 '{tool_name}' 通过 ToolRegistry 中心化路径执行");

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
                output: attach_investigation(
                    tool_name,
                    serde_json::json!({
                        "tool_name": tool_name,
                        "result": result.content,
                        "truncated": result.truncated,
                        "is_error": result.is_error,
                        "node_id": node.base_id(),
                    }),
                ),
                output_var: Some(tool_node.config.output_var.clone()),
                control: None,
            });
        }

        // ── 2. 回退：查找回调（多路注册 → fallback → 未配置） ──
        let cb: Option<ToolCallback> = context
            .callbacks
            .as_ref()
            .and_then(|cbs| cbs.tool_handlers.get(tool_name).cloned())
            .or_else(|| context.callbacks.as_ref().and_then(|cbs| cbs.tool_fallback.clone()));

        let output = if let Some(ref cb) = cb {
            tracing::info!("[ToolExecutor] 工具 '{tool_name}' 通过 ToolResolver 回调路径执行");
            cb(tool_name.clone(), resolved_args.clone()).await.map_err(|e| {
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
            output: attach_investigation(
                tool_name,
                serde_json::json!({
                    "tool_name": tool_name,
                    "result": output,
                    "node_id": node.base_id(),
                }),
            ),
            output_var: Some(tool_node.config.output_var.clone()),
            control: None,
        })
    }
}

/// 执行后自检（设计文档 §6）：对工具结果做轻量级结构校验。
///
/// 仅针对"状态变更 / 外部副作用"类工具：若结果串携带错误信号（error/failed/
/// exception/permission denied 等），视为疑似失败，返回需要人工调查的原因；
/// 否则返回 None。
///
/// 该信号会写入节点输出 `needs_investigation` 字段，供 Scheduler 汇入 ⑦ 长时报告，
/// 并让后台任务置为 `needs_investigation` 状态交由人工复核。
fn attach_investigation(tool_name: &str, mut output: serde_json::Value) -> serde_json::Value {
    const SIDE_EFFECT_TOOLS: &[&str] = &[
        "write_file",
        "create_file",
        "edit_file",
        "apply_diff",
        "execute_command",
        "bash",
        "shell",
        "webhookSend",
        "email",
        "notification",
        "httpRequest",
        "databaseQuery",
        "fileOperation",
    ];

    if SIDE_EFFECT_TOOLS.contains(&tool_name) {
        let text = output
            .get("result")
            .and_then(|v| v.as_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        let suspicious = text.contains("error")
            || text.contains("failed")
            || text.contains("exception")
            || text.contains("traceback")
            || text.contains("permission denied")
            || text.contains("not found");
        if suspicious && let Some(obj) = output.as_object_mut() {
            obj.insert("needs_investigation".to_string(), serde_json::json!(true));
            obj.insert(
                "investigation_reason".to_string(),
                serde_json::json!(format!(
                    "工具 '{}' 执行后自检发现疑似失败信号，需人工复核",
                    tool_name
                )),
            );
        }
    }

    output
}

// ── 测试：ToolNode 转调共享宽松版 resolver 后的行为契约 ──
//
// 本模块置于文件**最末尾**，避免 `clippy::items_after_module`。
// 契约来源：`executors/mod.rs:106-164`（共享宽松版）＋ 2026-09-09「终值不 auto_parse」修复。
// 对照物：`condition_executor.rs` 的严格版仍保留（其 `None` 语义是另一套已登记契约，不随本次统一改变）。
#[cfg(test)]
mod resolve_var_path_unified_tests {
    use serde_json::{Value, json};
    use std::collections::HashMap;

    /// ① AgentNode 生产者：`content` 是 **JSON 字符串**（而非 JSON 对象）时，
    /// `<节点>.content.<字段>` 必须能穿透 —— 这正是本次统一所修复的能力。
    /// 旧的文件内私有严格版在此返回 `None`：它对 String 执行 `current.get(part)?`
    /// 会直接失败并整体 return None（不走 fallback）。
    #[test]
    fn toolnode_penetrates_json_string_content() {
        let mut vars: HashMap<String, Value> = HashMap::new();
        vars.insert(
            "lc-conceive".to_string(),
            json!({
                "role": "assistant",
                "content": "{\"persona\":\"网络小说老手\",\"genre\":\"novel\"}",
            }),
        );

        assert_eq!(
            super::super::resolve_var_path("lc-conceive.content.persona", &vars),
            Some(json!("网络小说老手")),
            "content 为 JSON 字符串时应可穿透取到字段"
        );
    }

    /// ② ToolNode 生产者：`result` 是 JSON 字符串，其内部又嵌一层 `content` 包裹，
    /// 于是消费路径由三段 `<工具>.result.<字段>` 变成四段 `<工具>.result.content.<字段>`。
    /// 四段路径需要连续两次字符串穿透：`result` 字符串 → 对象 → `content` 字符串 → 对象。
    #[test]
    fn toolnode_penetrates_result_content_four_segments() {
        let mut vars: HashMap<String, Value> = HashMap::new();
        vars.insert(
            "t-extract".to_string(),
            json!({
                "tool_name": "narrative_chapter_instructions",
                "result": "{\"content\":{\"chapter_text\":\"第一章 起风\"}}",
                "is_error": false,
            }),
        );

        assert_eq!(
            super::super::resolve_var_path("t-extract.result.content.chapter_text", &vars),
            Some(json!("第一章 起风")),
            "四段路径应能连续穿透 result 与 content 两层 JSON 字符串包裹"
        );

        // 对照组：三段路径 `<工具>.result.<字段>` 在 `result` 字符串本身即带该字段时同样可解析。
        let mut vars_flat: HashMap<String, Value> = HashMap::new();
        vars_flat.insert(
            "t-risk".to_string(),
            json!({ "tool_name": "risk_scan", "result": "{\"totalScore\":42}" }),
        );
        assert_eq!(
            super::super::resolve_var_path("t-risk.result.totalScore", &vars_flat),
            Some(json!(42)),
            "三段路径穿透 result 字符串后取字段"
        );
    }

    /// ③ 平键**不**做 auto_parse（`executors/mod.rs:119-125` 的显式契约）。
    /// ToolNode 的 `input_mapping` 常以平键读取 `stock_code` 这类纯数字字符串；
    /// 若被 auto_parse 成 Number，下游字符串参数会被破坏。
    #[test]
    fn flat_key_not_auto_parsed() {
        let mut vars: HashMap<String, Value> = HashMap::new();
        vars.insert("stock_code".to_string(), json!("600036"));
        vars.insert("enabled".to_string(), json!("true"));

        let code = super::super::resolve_var_path("stock_code", &vars).expect("平键应直查命中");
        assert!(
            matches!(code, Value::String(_)),
            "单段平键 stock_code 必须原样返回字符串，实际得到：{code:?}"
        );
        assert_eq!(code, json!("600036"));

        let flag = super::super::resolve_var_path("enabled", &vars).expect("平键应直查命中");
        assert!(
            matches!(flag, Value::String(_)),
            "单段平键 enabled 不得被 auto_parse 成 Bool，实际得到：{flag:?}"
        );

        // fallback 分支（root 不是节点 ID，整路径直查）同样保持不 auto_parse。
        let mut vars_dotted: HashMap<String, Value> = HashMap::new();
        vars_dotted.insert("a.b".to_string(), json!("100"));
        let dotted = super::super::resolve_var_path("a.b", &vars_dotted).expect("fallback 应命中");
        assert!(
            matches!(dotted, Value::String(_)),
            "fallback 分支不得 auto_parse，实际得到：{dotted:?}"
        );
    }

    /// ④ 终值**不** auto_parse（2026-09-09 显式修复）。
    /// `<节点>.content` 这类以字符串字段收尾的路径，必须把内容的**字符串本身**交回
    /// 调用方 —— data-quality / portfolio-mgr / pace-calc 的 Rhai 消费端契约是
    /// 「ToolNode 输出为 JSON 字符串，脚本内自行 json_parse」，且用
    /// `type_of(x) == "string"` 做分支判定。若终值被 parse 成 map/array，
    /// 这些分支会全部失效、因子信号恒 0。
    #[test]
    fn final_value_not_auto_parsed() {
        let mut vars: HashMap<String, Value> = HashMap::new();
        vars.insert(
            "lc-draft-agent".to_string(),
            json!({
                "role": "assistant",
                "content": "{\"chapter_text\":\"第一章\"}",
            }),
        );

        let final_value =
            super::super::resolve_var_path("lc-draft-agent.content", &vars).expect("终值应命中");
        assert!(
            matches!(final_value, Value::String(_)),
            "终值必须是字符串本身，不得被 parse 成对象，实际得到：{final_value:?}"
        );
        assert_eq!(final_value, json!("{\"chapter_text\":\"第一章\"}"));
    }
}
