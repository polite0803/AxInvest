// SPDX-License-Identifier: AGPL-3.0-only
//! 域工具定义 → 注册表实例（通用包装，零业务知识）。
//!
//! # 解决什么
//!
//! 业务域（股票 / 产业链等）的 MCP 工具定义由域侧经
//! [`crate::domain_tools::register_tool_definitions`] 注入；本模块把注入进来的
//! 定义批量包装成 `Tool` 实例注册进 `UnifiedToolRegistry`。
//!
//! 注册后 `state.rs::register_all_capabilities` 的通用逻辑会自动为每个工具派生
//! `tool:{name}` 护照（domain 由定义自带，`tool_ref` 指向自身），
//! DiscoverSkills / extra_tools 注入链路即刻生效，无需额外接线。
//!
//! # 为什么不再直接依赖域 crate
//!
//! 此前本模块（`astock_data.rs`）直接 `use axagent_astock_data::…` 取定义与执行，
//! 域整体迁出为插件时会把域 crate 钉在工具层里（PLAN §54.3 B5）。现在执行经
//! [`crate::domain_tools::find_executor`] 查已注册的执行器，工具层不再认识任何域。
//!
//! # 重名规避
//!
//! finance.rs 的 `api_tool!` 已注册 3 个同名工具（语义相同）：
//! `get_north_bound_flow` / `get_market_dragon_tiger` / `get_cls_flash`。
//! `ToolRegistry::register` 是 HashMap insert（重名静默覆盖），批量构建时显式
//! 跳过，保留 finance.rs 现实现。定义本身仍留在槽位里 —— 能力包白名单与
//! 审计豁免都按名字查，不该因为「注册表用哪份实现」而看不到这些名字。

use crate::{Tool, ToolCategory, ToolContext, ToolDomain, ToolError, ToolResult, domain_tools};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// 与 finance.rs 既有注册重名的域工具（保留 finance.rs 现实现，跳过注册）。
pub const REGISTRY_DUPLICATE_TOOLS: [&str; 3] =
    ["get_north_bound_flow", "get_market_dragon_tiger", "get_cls_flash"];

/// 域工具定义的薄包装实例。
///
/// 每个实例对应注入列表中的一项；执行统一委托该工具所属域的已注册执行器
/// （由 wiring 在域客户端就绪后注入）。
#[derive(Debug, Clone)]
pub struct DomainMcpTool {
    domain: String,
    name: String,
    description: String,
    schema: Value,
}

impl DomainMcpTool {
    /// 从 `(域标识, 工具定义 JSON)` 构建实例。
    fn from_definition(domain: String, def: &Value) -> Option<Self> {
        let name = def.get("name")?.as_str()?.to_string();
        if name.trim().is_empty() {
            return None;
        }
        let description = def.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let schema = def
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}}));
        Some(Self { domain, name, description, schema })
    }
}

#[async_trait]
impl Tool for DomainMcpTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> Value {
        self.schema.clone()
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Finance
    }

    fn is_concurrency_safe(&self) -> bool {
        // 执行器由域侧共享（如 astock 的 RateGate 内部串行化）
        true
    }

    fn is_read_only(&self) -> bool {
        !is_mutating_tool(&self.name)
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let executor = domain_tools::find_executor(&self.domain).ok_or_else(|| {
            ToolError::execution_failed(format!(
                "域 `{}` 的工具执行器未注册（wiring 缺失：启动时须调用 \
                 domain_tools::register_tool_executor）",
                self.domain
            ))
        })?;
        let out =
            executor.execute(&self.name, &input).await.map_err(ToolError::execution_failed)?;
        Ok(ToolResult::success(out))
    }
}

/// Dojo 变更类工具（创建/执行/修订计划）——非只读，其余全部只读。
///
/// 定义 JSON 目前不带只读标记（MCP schema 里没有 `readOnlyHint`），故按名字判。
/// 域侧若要改分类，应先给定义补字段，而不是在这里加域专属规则。
fn is_mutating_tool(name: &str) -> bool {
    name.starts_with("dojo_")
        && (name.contains("create") || name.contains("execute") || name.contains("revise"))
}

/// 从给定定义批量构建注册项（跳过与既有内置实现重名的工具）。
///
/// 纯函数：不读全局槽位，测试直接注入夹具，避免全局状态跨测试串扰。
pub fn instances_from(definitions: Vec<(String, Value)>) -> Vec<Arc<dyn Tool>> {
    definitions
        .into_iter()
        .filter_map(|(domain, def)| {
            let tool = DomainMcpTool::from_definition(domain, &def)?;
            (!REGISTRY_DUPLICATE_TOOLS.contains(&tool.name.as_str())).then_some(tool)
        })
        .map(|t| Arc::new(t) as Arc<dyn Tool>)
        .collect()
}

/// 从已注册的域工具定义批量构建注册项。
///
/// ⚠ 调用时机：必须在域侧 `register_tool_definitions` **之后**
/// （工具注册表构建时调用，见 `tools::builtin_tool_instances`）。
pub fn domain_mcp_tool_instances() -> Vec<Arc<dyn Tool>> {
    instances_from(domain_tools::all_definitions())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<(String, Value)> {
        vec![
            (
                "stock".to_string(),
                serde_json::json!({
                    "name": "get_stock_quote",
                    "description": "查询行情",
                    "inputSchema": {"type": "object", "properties": {"stock_code": {"type": "string"}}}
                }),
            ),
            ("stock".to_string(), serde_json::json!({"name": "dojo_execute_plan"})),
            ("stock".to_string(), serde_json::json!({"name": "get_north_bound_flow"})),
            ("stock".to_string(), serde_json::json!({"description": "无名字"})),
            ("stock".to_string(), serde_json::json!({"name": "  "})),
        ]
    }

    #[test]
    fn malformed_and_duplicate_definitions_are_skipped() {
        let instances = instances_from(fixture());
        assert_eq!(instances.len(), 2, "3 个合法定义里 1 个与 finance.rs 重名 ⇒ 2 个实例");
        assert!(instances.iter().any(|t| t.name() == "get_stock_quote"));
        assert!(
            instances.iter().all(|t| t.name() != "get_north_bound_flow"),
            "重名工具必须跳过，否则静默覆盖 finance.rs 现实现"
        );
    }

    #[test]
    fn definitions_metadata_passthrough() {
        let instances = instances_from(fixture());
        let quote = instances.iter().find(|t| t.name() == "get_stock_quote").unwrap();
        assert_eq!(quote.description(), "查询行情");
        assert_eq!(quote.input_schema()["properties"]["stock_code"]["type"], "string");
        assert_eq!(quote.domain(), ToolDomain::Finance);
        // 缺 inputSchema 时给空 object schema，而不是 None/panic
        let exec = instances.iter().find(|t| t.name() == "dojo_execute_plan").unwrap();
        assert_eq!(exec.input_schema()["type"], "object");
    }

    #[test]
    fn read_only_classification() {
        let instances = instances_from(fixture());
        let quote = instances.iter().find(|t| t.name() == "get_stock_quote").unwrap();
        assert!(quote.is_read_only(), "行情查询是只读");
        let exec = instances.iter().find(|t| t.name() == "dojo_execute_plan").unwrap();
        assert!(!exec.is_read_only(), "Dojo 计划执行是变更类");
    }

    #[tokio::test]
    async fn missing_executor_fails_loudly() {
        let instances = instances_from(vec![(
            "__fixture_domain_without_executor__".to_string(),
            serde_json::json!({"name": "x"}),
        )]);
        let tool = &instances[0];
        let ctx = ToolContext::new(".");
        let err = tool.call(serde_json::json!({}), &ctx).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("未注册"), "执行器缺失必须显式报错，不得静默：{msg}");
    }
}
