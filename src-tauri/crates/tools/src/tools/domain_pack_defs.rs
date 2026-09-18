// SPDX-License-Identifier: AGPL-3.0-only

//! 域包工具白名单映射（由 `commands/opc_workflows/domain_pack.rs` 下沉）
//!
//! 三个 `*_tool_defs` 把「域包声明的工具名」映射成 `ToolDef`（含 description / parameters）。
//! 归位到 tools crate 的原因：它们依赖 `axagent_tools::tools::*` 的工具实现 ——
//! 而 tools **已经依赖** analysis-engine，反向会成环（见 PLAN §12.3）。

use std::sync::Arc;

// ── 股票工具白名单（P4-2：金融域包吃 astock-data 工具链）────────

/// 从 astock-data stock_mcp_tools 匹配工具名 → ToolDef 列表。
/// 工具已由 init/services.rs ToolResolver 接通执行路径（execute_mcp_tool），
/// 工作流 AgentNode 只要 exposed_tools 含工具名即可调用。
pub fn stock_tool_defs(names: &[String]) -> Vec<axagent_harness::workflow_types::ToolDef> {
    let mut out = Vec::new();
    for tool in axagent_astock_data::mcp_tools::stock_mcp_tools() {
        let Some(name) = tool.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        if !names.iter().any(|n| n == name) {
            continue;
        }
        let description = tool.get("description").and_then(|v| v.as_str()).map(|s| s.to_string());
        // parameters：把 inputSchema json 转 ToolDef.parameters（JsonSchema）
        let parameters =
            tool.get("inputSchema").and_then(|v| serde_json::from_value(v.clone()).ok());
        out.push(axagent_harness::workflow_types::ToolDef {
            name: name.to_string(),
            description,
            parameters,
        });
    }
    out
}

// ── OPC 工具白名单（一人公司业务：内容营销/电商等域包吃 Opc 工具链）────

/// 从 tools crate 内置 OPC 工具匹配工具名 → ToolDef 列表。
///
/// 工具已在 `crate::tools::register_all` 的「OPC 业务工具」段注册进
/// `UnifiedToolRegistry`，`init/services.rs` ToolResolver 的 `known` 分支即可接通
/// 执行路径，工作流 AgentNode 只要 `tools` / `exposed_tools` 含工具名即可调用。
///
/// ⚠️ `OpcSendNotification` 是唯一例外：它**不在注册表里**（其 `call` 依赖的
/// `set_opc_notify_tx` 零调用者，注册即必 panic），所以这里给它保留 ToolDef
/// 只会让 LLM 看到一个调用必失败的工具。修好 panic 后再把两边同时补上。
pub fn opc_tool_defs(names: &[String]) -> Vec<axagent_harness::workflow_types::ToolDef> {
    use crate::Tool;
    let candidates: Vec<Arc<dyn Tool>> = vec![
        std::sync::Arc::new(crate::tools::opc::OpcListInvoicesTool),
        std::sync::Arc::new(crate::tools::opc::OpcCreateInvoiceTool),
        std::sync::Arc::new(crate::tools::opc::OpcTransitionInvoiceTool),
        std::sync::Arc::new(crate::tools::opc::OpcListCustomersTool),
        std::sync::Arc::new(crate::tools::opc::OpcCreateCustomerTool),
        std::sync::Arc::new(crate::tools::opc::OpcListProjectsTool),
        std::sync::Arc::new(crate::tools::opc::OpcCreateProjectTool),
        std::sync::Arc::new(crate::tools::opc::OpcAddMilestoneTool),
        std::sync::Arc::new(crate::tools::opc::OpcGetDashboardTool),
        std::sync::Arc::new(crate::tools::opc::OpcListLandingPagesTool),
        std::sync::Arc::new(crate::tools::opc::OpcListBlogPostsTool),
        std::sync::Arc::new(crate::tools::opc::OpcCreateLandingPageTool),
        std::sync::Arc::new(crate::tools::opc::OpcCreateBlogPostTool),
        std::sync::Arc::new(crate::tools::opc::OpcListContactsTool),
        std::sync::Arc::new(crate::tools::opc::OpcSendNotificationTool),
        std::sync::Arc::new(crate::tools::opc::OpcRecordKpiTool),
        std::sync::Arc::new(crate::tools::opc::OpcListKpisTool),
        std::sync::Arc::new(crate::tools::opc::OpcSearchWikiTool),
        std::sync::Arc::new(crate::tools::opc::OpcGetFinancialReportTool),
        // 下面 3 项是「域包模板已经用 `td("…")` 引用过、但此前不在本白名单里」的工具。
        // 不补的后果不是「跑不起来」（`td()` 只有名字，LLM 仍看得到），而是
        // LLM 拿不到 description / parameters，参数质量下降。
        // 其余 5 个（`OpcListContentAssets` / `OpcUpdateContentAsset` /
        // `OpcDeleteContentAsset` / `OpcCancelPublishSchedule` / `OpcProcessDueSchedules`）
        // 已在注册表里，但**没有任何域包模板引用它们**，故不放进本 ToolDef 白名单：
        // 一次性铺开只会无谓扩大 LLM 可见工具面。要用时随模板声明一并加。
        std::sync::Arc::new(crate::tools::opc::OpcCreateContentAssetTool),
        std::sync::Arc::new(crate::tools::opc::OpcCreatePublishScheduleTool),
        std::sync::Arc::new(crate::tools::opc::OpcListPublishSchedulesTool),
    ];
    let mut out = Vec::new();
    for tool in candidates {
        if !names.iter().any(|n| n == tool.name()) {
            continue;
        }
        // parameters：把 input_schema()（serde_json::Value）转 ToolDef.parameters（JsonSchema）
        let parameters = serde_json::from_value(tool.input_schema()).ok();
        out.push(axagent_harness::workflow_types::ToolDef {
            name: tool.name().to_string(),
            description: Some(tool.description().to_string()),
            parameters,
        });
    }
    out
}

// ── 通用本机工具白名单（P1-1：software_dev 等域包声明 FileRead/Bash/Grep 等）──

/// 从 tools crate 内置通用工具匹配工具名 → ToolDef 列表。
/// 与 stock_tool_defs / opc_tool_defs 并列，构成完整工具注入白名单。
pub fn local_tool_defs(names: &[String]) -> Vec<axagent_harness::workflow_types::ToolDef> {
    use crate::Tool;
    let candidates: Vec<Arc<dyn Tool>> = vec![
        std::sync::Arc::new(crate::tools::file_read::FileReadTool),
        std::sync::Arc::new(crate::tools::file_write::FileWriteTool),
        std::sync::Arc::new(crate::tools::file_edit::FileEditTool),
        std::sync::Arc::new(crate::tools::bash::BashTool),
        std::sync::Arc::new(crate::tools::grep::GrepTool),
        std::sync::Arc::new(crate::tools::glob::GlobTool),
        std::sync::Arc::new(crate::tools::file_system::ListDirectoryTool),
        std::sync::Arc::new(crate::tools::web_search::WebSearchTool),
    ];
    let mut out = Vec::new();
    for tool in candidates {
        if !names.iter().any(|n| n == tool.name()) {
            continue;
        }
        let parameters = serde_json::from_value(tool.input_schema()).ok();
        out.push(axagent_harness::workflow_types::ToolDef {
            name: tool.name().to_string(),
            description: Some(tool.description().to_string()),
            parameters,
        });
    }
    out
}
