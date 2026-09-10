//! 工作流驱动的股票分析 — 基于持久化 WorkflowTemplate + WorkEngine DAG 执行。
//!
//! 启动时种子化 stock-analysis 工作流模板到 workflow_templates 表，
//! 每次分析从模板加载 DAG 结构，注入实时行情数据，由 WorkEngine 并行执行。
//!
//! 子模块：
//! - decision: 决策质量预检、决策提取、重跑决策命令
//! - core: 股票分析工作流核心（run_stock_workflow, run_single_stock_analysis）
//! - reflection: 反思工作流（run_reflection_workflow, run_batch_reflection）
//! - serenity: Serenity 瓶颈筛选工作流
//! - reco_history: 推荐历史记录管理
//! - misc: 导出和回测命令

pub mod core;
pub mod decision;
pub mod hooks;
pub mod misc;
pub mod reco_history;
pub mod reflection;
pub mod rhai_pm;
pub mod serenity;

/// 构造严格模式工具权限：激活 AgentExecutor 的严格输出契约三道防线
/// （4f 尾部约束注入 / VERDICT 缺失兜底重试 / VERDICT tag 重构 + strict JSON 校验与降级）。
///
/// 2026-09-08 接线修复：历史上 `ExecutionState.tool_permissions` 从未被注入（恒 None），
/// 上述防线全部为死代码——分析师（a-market-analyst 等）漏发 `<!-- VERDICT -->` 标签时，
/// 无兜底重试、无降级 JSON，content 保持纯 Markdown，
/// analyst-brief.rhai 的 `content.verdict` 解析失败 → 摘要恒显「解析失败，数据不可用」。
/// 所有 stock workflow 执行入口（core/serenity/reflection）统一通过本函数启用。
pub(crate) fn strict_tool_permissions() -> std::sync::Arc<axagent_harness::tool::ToolPermissions> {
    std::sync::Arc::new(axagent_harness::tool::ToolPermissions {
        strict_mode: true,
        ..Default::default()
    })
}

// Re-export all #[tauri::command] functions so they remain accessible via stock_workflow::
pub use core::run_single_stock_analysis;
pub use reflection::run_reflection_workflow;
// Internal helpers called from outside the module
pub use reflection::run_batch_reflection_inner;

// Re-export public structs
