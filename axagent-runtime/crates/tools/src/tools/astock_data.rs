// SPDX-License-Identifier: AGPL-3.0-only
//! Astock 数据工具批量接入统一工具注册表（L1）。
//!
//! # 解决什么
//!
//! `axagent_astock_data::mcp_tools::stock_mcp_tools()` 定义了 57 个数据/计算工具
//! （search_stock / get_stock_quote / get_stock_financials / get_hot_stocks 等），
//! 此前只走"工作流 AgentNode ToolDef 白名单 + `execute_stock_mcp_tool` 命令"专用通道，
//! 从未注册进 UnifiedToolRegistry —— 护照索引里没有它们，DiscoverSkills 结构性搜不到，
//! 助手 agent（编排模式）既发现不了也调用不了。
//!
//! 本模块把定义列表批量包装为 `StockMcpTool` 实例注册进注册表：
//! 注册后 `state.rs::register_all_capabilities` 的通用逻辑会自动为每个工具派生
//! `tool:{name}` 护照（domain=Finance，tool_ref 指向自身），DiscoverSkills / extra_tools
//! 注入链路即刻生效，无需额外接线。
//!
//! # 分层合规
//!
//! tools crate 已依赖 `axagent_astock_data`（见 `global_state.rs` 的 client 槽位），
//! 执行直接复用 `mcp_tools::execute_mcp_tool`，与工作流 AgentNode 通道共享同一实现，
//! 不引入重复定义（AGENTS.md 规则 12）。
//!
//! # 重名规避
//!
//! finance.rs 的 `api_tool!` 已注册 3 个同名工具（语义相同）：
//! `get_north_bound_flow` / `get_market_dragon_tiger` / `get_cls_flash`。
//! `ToolRegistry::register` 是 HashMap insert（重名静默覆盖），
//! 批量构建时显式跳过，保留 finance.rs 现实现。

use crate::{Tool, ToolCategory, ToolContext, ToolDomain, ToolError, ToolResult, global_state};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// 与 finance.rs 既有注册重名的工具（保留 finance.rs 现实现，跳过注册）。
pub const FINANCE_DUPLICATE_TOOLS: [&str; 3] =
    ["get_north_bound_flow", "get_market_dragon_tiger", "get_cls_flash"];

/// Dojo 变更类工具（创建/执行/修订计划）——非只读，其余全部只读。
fn is_mutating_tool(name: &str) -> bool {
    name.starts_with("dojo_")
        && (name.contains("create") || name.contains("execute") || name.contains("revise"))
}

/// astock MCP 工具的薄包装实例。
///
/// 每个实例对应 `stock_mcp_tools()` 定义列表中的一项；执行统一委托
/// `mcp_tools::execute_mcp_tool`（与工作流 AgentNode / `execute_stock_mcp_tool`
/// 命令同一实现），client 经 `global_state` 槽位注入。
#[derive(Debug, Clone)]
pub struct StockMcpTool {
    name: String,
    description: String,
    schema: Value,
}

impl StockMcpTool {
    /// 从 MCP 工具定义 JSON（`{"name","description","inputSchema"}`）构建实例。
    fn from_definition(def: &Value) -> Option<Self> {
        let name = def.get("name")?.as_str()?.to_string();
        if name.trim().is_empty() {
            return None;
        }
        let description = def.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let schema = def
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}}));
        Some(Self { name, description, schema })
    }
}

#[async_trait]
impl Tool for StockMcpTool {
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
        // client 为 Arc 共享，内部自带 RateGate 串行化
        true
    }

    fn is_read_only(&self) -> bool {
        !is_mutating_tool(&self.name)
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let client = global_state::get_astock_client().ok_or_else(|| {
            ToolError::execution_failed(
                "AStockClient 未初始化（wiring 缺失：state.rs 须调用 set_astock_client）",
            )
        })?;
        let out = axagent_astock_data::mcp_tools::execute_mcp_tool(&client, &self.name, &input)
            .await
            .map_err(ToolError::execution_failed)?;
        Ok(ToolResult::success(out))
    }
}

/// 从 `stock_mcp_tools()` 定义批量构建注册项（跳过与 finance.rs 重名的工具）。
pub fn stock_mcp_tool_instances() -> Vec<Arc<dyn Tool>> {
    axagent_astock_data::mcp_tools::stock_mcp_tools()
        .iter()
        .filter_map(|def| {
            let tool = StockMcpTool::from_definition(def)?;
            (!FINANCE_DUPLICATE_TOOLS.contains(&tool.name.as_str())).then_some(tool)
        })
        .map(|t| Arc::new(t) as Arc<dyn Tool>)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instances_skip_finance_duplicates_and_cover_all_definitions() {
        let defs = axagent_astock_data::mcp_tools::stock_mcp_tools();
        let instances = stock_mcp_tool_instances();
        // 全量定义 - 3 个重名跳过
        assert_eq!(instances.len(), defs.len() - FINANCE_DUPLICATE_TOOLS.len());
        for dup in FINANCE_DUPLICATE_TOOLS {
            assert!(
                instances.iter().all(|t| t.name() != dup),
                "重名工具 {dup} 必须被跳过，否则静默覆盖 finance.rs 现实现"
            );
        }
    }

    #[test]
    fn definitions_metadata_passthrough() {
        let instances = stock_mcp_tool_instances();
        let fin = instances
            .iter()
            .find(|t| t.name() == "get_stock_financials")
            .expect("核心数据工具必须存在");
        assert!(fin.description().contains("财务"));
        let schema = fin.input_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"]["stock_code"]["type"], "string");
        // 护照域必须为 Finance（DiscoverSkills 检索与 F3 闸 domain 交集都依赖它）
        assert_eq!(fin.domain(), ToolDomain::Finance);
    }

    #[test]
    fn read_only_classification() {
        let instances = stock_mcp_tool_instances();
        let quote = instances.iter().find(|t| t.name() == "get_stock_quote").unwrap();
        assert!(quote.is_read_only(), "行情查询是只读");
        let exec_plan = instances.iter().find(|t| t.name() == "dojo_execute_plan").unwrap();
        assert!(!exec_plan.is_read_only(), "Dojo 计划执行是变更类");
    }

    #[test]
    fn malformed_definition_is_skipped() {
        assert!(
            StockMcpTool::from_definition(&serde_json::json!({"description": "无名字"})).is_none()
        );
        assert!(
            StockMcpTool::from_definition(&serde_json::json!({"name": "  "})).is_none(),
            "空名字定义必须被跳过"
        );
    }
}
