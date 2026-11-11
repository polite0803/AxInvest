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

// B7b：`REGISTRY_DUPLICATE_TOOLS` 已删除 —— 原先跳过的是 finance.rs 里同名的三个
// api_tool；那三个工具改为域工具定义后，域侧必须注册它们，跳过即「工具消失」。

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

/// 变更类工具（非只读）——按名字查审计层的写类名单，其余全部只读。
///
/// 定义 JSON 目前不带只读标记（MCP schema 里没有 `readOnlyHint`），故按名字判。
/// 名单与限流豁免同源（[`crate::audit::MUTATING_DATA_TOOL_NAMES`]）：此前这里是
/// 一套 `dojo_*` 前缀启发式，与审计名单各说各话 —— 名单里的 `optimize_attention_weights`
/// 在工具层反被判成「只读」。域侧若要改分类，应先给定义补字段，而不是在这里加域专属规则。
fn is_mutating_tool(name: &str) -> bool {
    crate::audit::MUTATING_DATA_TOOL_NAMES.contains(&name)
}

/// 从给定定义批量构建注册项（跳过与既有内置实现重名的工具）。
///
/// 纯函数：不读全局槽位，测试直接注入夹具，避免全局状态跨测试串扰。
pub fn instances_from(definitions: Vec<(String, Value)>) -> Vec<Arc<dyn Tool>> {
    definitions
        .into_iter()
        .filter_map(|(domain, def)| DomainMcpTool::from_definition(domain, &def))
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

    /// 写类夹具名：取审计名单第 0 项，不在测试里重抄一个会随退役腐烂的名字。
    fn mutating_fixture_name() -> &'static str {
        crate::audit::MUTATING_DATA_TOOL_NAMES
            .first()
            .copied()
            .expect("审计写类名单为空 ⇒ 本夹具无处取名，改用显式名字或恢复名单")
    }

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
            ("stock".to_string(), serde_json::json!({"name": mutating_fixture_name()})),
            ("stock".to_string(), serde_json::json!({"name": "get_north_bound_flow"})),
            ("stock".to_string(), serde_json::json!({"description": "无名字"})),
            ("stock".to_string(), serde_json::json!({"name": "  "})),
        ]
    }

    #[test]
    fn malformed_definitions_are_skipped() {
        let instances = instances_from(fixture());
        assert_eq!(instances.len(), 3, "5 个定义里 2 个畸形（无名字 / 空白名字）⇒ 3 个实例");
        assert!(instances.iter().any(|t| t.name() == "get_stock_quote"));
        // B7b：`REGISTRY_DUPLICATE_TOOLS` 删除后，原先与 finance.rs 重名的三个工具
        // 必须由域侧注册（跳过即工具消失）。
        assert!(
            instances.iter().any(|t| t.name() == "get_north_bound_flow"),
            "重名跳过机制已退役：同名工具必须由域侧注册，否则该工具整体消失"
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
        let exec = instances.iter().find(|t| t.name() == mutating_fixture_name()).unwrap();
        assert_eq!(exec.input_schema()["type"], "object");
    }

    #[test]
    fn read_only_classification() {
        let instances = instances_from(fixture());
        let quote = instances.iter().find(|t| t.name() == "get_stock_quote").unwrap();
        assert!(quote.is_read_only(), "行情查询是只读");
        let exec = instances.iter().find(|t| t.name() == mutating_fixture_name()).unwrap();
        assert!(!exec.is_read_only(), "审计名单登记的写类工具必须判非只读");
        // 反向钉桩：分类只看审计名单，不再认 `dojo_*` + create/execute/revise 前缀启发式
        // （该启发式已随 DojoSDK 死通道退役；留着会让名单外的名字各说各话）。
        let legacy = instances_from(vec![(
            "stock".to_string(),
            serde_json::json!({"name": "dojo_execute_plan"}),
        )]);
        assert!(legacy[0].is_read_only(), "前缀启发式已退役：名单外的名字一律判只读");
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
