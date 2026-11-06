// SPDX-License-Identifier: AGPL-3.0-only

//! 域包工具白名单映射（由 `commands/opc_workflows/capability_pack.rs` 下沉）
//!
//! 三个 `*_tool_defs` 把「域包声明的工具名」映射成 `ToolDef`（含 description / parameters）。
//! 归位到 tools crate 的原因：它们依赖 `axagent_tools::tools::*` 的工具实现。
//!
//! 业务域工具（股票 / 产业链）的定义来源已倒置到 [`crate::domain_tools`] 槽位 ——
//! 本文件不再直接依赖域 crate（PLAN §54.3 B5）。

use std::sync::Arc;

// ── 业务域工具白名单（P4-2：金融域包吃 astock-data 工具链）────────

/// 从**已注册的域工具定义**匹配工具名 → ToolDef 列表。
///
/// 定义由域侧在启动早期经 [`crate::domain_tools::register_tool_definitions`] 注入
/// （股票 + 产业链见 `src-tauri/src/init/stock_domain_tools.rs`）。本 crate 不再
/// 直接依赖域 crate（PLAN §54.3 B5），只做「名字 → ToolDef」的通用映射。
/// 工具已由 init/services.rs ToolResolver 接通执行路径，工作流 AgentNode
/// 只要 exposed_tools 含工具名即可调用。
///
/// ⚠ 调用前提：该域的定义已注册。未注册时返回空 —— 与「域未安装」同义。
pub fn domain_tool_defs(names: &[String]) -> Vec<axagent_harness::workflow_types::ToolDef> {
    let mut out = Vec::new();
    for (_domain, tool) in crate::domain_tools::all_definitions() {
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
/// ⚠️ `OpcSendNotification` **已退役**（缺陷 #184，2026-11 本批）：它的 `call` 走
/// `opc.rs` 的 `OPC_NOTIFY_TX.get().expect("OPC notify tx not initialized")`，而
/// `set_opc_notify_tx` 全仓零调用者 ⇒ 注册即必 panic；同时 `OpcNotification` **零接收端**
/// ⇒ 「补个生产者」只会把必 panic 换成必失败（任务 #115 对 `alert_tx` 的同一判据）。
/// 故实现体已从 `opc.rs` 删除，本白名单也不再为它产出 ToolDef —— 域包若仍声明该名字，
/// 这里会**静默跳过**（不产出 ToolDef），而不是产出一个必失败的工具面。
/// （2026-11-07 任务 #208 已把 31 处活声明全部清理，剩余零命中；本白名单的跳过分支保留为兜底。）
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
/// 与 domain_tool_defs / opc_tool_defs 并列，构成完整工具注入白名单。
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
