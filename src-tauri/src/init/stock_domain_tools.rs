// SPDX-License-Identifier: AGPL-3.0-only

//! 股票域工具槽位注册（依赖倒置的**域侧**）。
//!
//! `tools` crate 只保留通用槽位（[`axagent_tools::domain_tools`]）；本模块是股票域
//! 的定义来源与执行器实现，把「工具层认识域 crate」翻转为「域侧向工具层注册」。
//! 域整体迁出为插件时，本文件随域一起走，工具层不需要改动（PLAN §54.3 B5）。
//!
//! 两段注册的时机不同（见 `domain_tools` 模块文档）：
//! - [`register_tool_definitions`]：纯静态 schema，**必须在工具注册表构建之前**调用
//!   （`AppState` 装配里紧跟 `tool.set` 接缝注册）；
//! - [`register_tool_executor`]：需要 `AStockClient`，客户端构造完成后调用。

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// 本域在 `domain_tools` 槽位里的标识。
pub const DOMAIN: &str = "stock";

/// 股票域的全部工具定义：astock 数据/计算工具 + 产业链映射工具。
///
/// 与 `tools` crate 拆分前的定义来源**逐字同源**（同一对 `mcp_tools()` 函数）。
pub fn stock_tool_definitions() -> Vec<Value> {
    axagent_astock_data::mcp_tools::stock_mcp_tools()
        .into_iter()
        .chain(axagent_analysis_engine::mcp_tools::industry_chain_mcp_tools())
        .collect()
}

/// 把股票域定义注册进工具层槽位（幂等；必须早于工具注册表构建）。
pub fn register_tool_definitions() {
    axagent_tools::domain_tools::register_tool_definitions(DOMAIN, stock_tool_definitions());
}

/// 注册股票域工具执行器（须在 `AStockClient` 构造完成后调用）。
pub fn register_tool_executor(client: Arc<axagent_astock_data::AStockClient>) {
    axagent_tools::domain_tools::register_tool_executor(Arc::new(StockToolExecutor { client }));
}

struct StockToolExecutor {
    client: Arc<axagent_astock_data::AStockClient>,
}

#[async_trait]
impl axagent_tools::domain_tools::DomainToolExecutor for StockToolExecutor {
    fn domain(&self) -> &str {
        DOMAIN
    }

    async fn execute(&self, tool_name: &str, input: &Value) -> Result<String, String> {
        axagent_astock_data::mcp_tools::execute_mcp_tool(&self.client, tool_name, input).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// 接线正控：槽位里的定义必须覆盖本轮真实撞限流的那只工具，且写类名单无幽灵条目。
    ///
    /// 定义来源倒置后，「豁免集合从哪来」不再由 `tools` crate 直接可见 ——
    /// 这条测试把它钉回**真实数据源**：改名 / 漏接一支数据源 / 幽灵写类名，都会在这里红。
    #[test]
    fn real_definitions_reach_the_auditor_through_the_slot() {
        let definitions = stock_tool_definitions();
        let declared: HashSet<String> = definitions
            .iter()
            .filter_map(|t| t.get("name").and_then(|v| v.as_str()).map(String::from))
            .collect();
        assert!(
            declared.len() >= 50,
            "股票域定义只有 {} 项 —— 预期 ≥ 50（astock 53 + 产业链 2；DojoSDK 9 条死声明已退役）。\
             先查是不是只抽到了其中一支数据源。",
            declared.len()
        );

        register_tool_definitions();

        // 本轮实测撞限流的那只（原文：`工具 'get_stock_margin_data' 调用过于频繁，
        // 最小间隔 200ms（当前距上次调用 48ms）`），以及新接线的质押工具。
        for must_have in ["get_stock_margin_data", "get_stock_pledge_data", "get_stock_kline"] {
            assert!(
                axagent_tools::audit::is_min_interval_exempt(must_have),
                "`{must_have}` 没进豁免集合 ⇒ 注册 → 派生链路断了（P1-2 假修复）"
            );
        }

        // 幽灵条目反查 + 写类剔除：两个断言缺一不可 —— 前者防「剔除打在空气上」，
        // 后者防「整包豁免」。
        for mutating in axagent_tools::audit::MUTATING_DATA_TOOL_NAMES {
            assert!(
                declared.contains(*mutating),
                "`{mutating}` 登记进 MUTATING_DATA_TOOL_NAMES，但数据源没声明过它 ⇒ 幽灵条目"
            );
            assert!(
                !axagent_tools::audit::is_min_interval_exempt(mutating),
                "写类工具 `{mutating}` 出现在豁免集合里 ⇒ 整包豁免的 bug"
            );
        }
    }

    /// 槽位按域打标：股票定义必须落在 `stock` 域下（执行器按域查找，标错就查不到）。
    #[test]
    fn definitions_are_tagged_with_stock_domain() {
        register_tool_definitions();
        let stock_defs: HashSet<String> = axagent_tools::domain_tools::all_definitions()
            .into_iter()
            .filter(|(domain, _)| domain == DOMAIN)
            .filter_map(|(_, def)| def.get("name").and_then(|v| v.as_str()).map(String::from))
            .collect();
        assert!(stock_defs.contains("get_stock_kline"));
        assert!(!stock_defs.is_empty());
    }
}
