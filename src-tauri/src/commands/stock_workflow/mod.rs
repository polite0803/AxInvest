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
//! - sim_hook: 决策落库后的「仿真验证」挂钩（图示节点 sim-verify 的真执行体）
//! - misc: 导出和回测命令

pub mod core;
pub mod decision;
pub mod hooks;
pub mod misc;
pub mod reco_history;
pub mod reflection;
// M3：命中率统计只读命令（reflection_stats），消费 strategy_performance 聚合
pub mod reflection_stats;
pub mod rhai_bottleneck;
pub mod rhai_pm;
pub mod rhai_registry;
pub mod serenity;
pub mod sim_hook;

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
// [4 周期反思 / P2] 批量反思的筛选与配置：
// `batch-reflection` 与 `validate-decisions` 两个 cron 任务的配置结构，
// 由 `init/services.rs`（cron executor）与 `stock_analysis.rs`（create_*_cron）消费。
// 逐条 re-export（原因见下方 ⚠ 注释）。
// 注：`STALE_RUNNING_HOURS` / `reclaim_stale_running` 只在 reflection.rs 内部使用
// （`run_batch_reflection_inner` 自行调用），无跨模块消费者 ⇒ 不 re-export。
pub use reflection::BatchReflectionConfig;
pub use reflection::ReflectionFilter;
pub use reflection::ValidateDecisionsConfig;
// [实际行情] 反思的「结论 vs 当前实际行情」对比快照：
// 三个反思入口（手动命令 / Tauri 批量命令 / cron 批量）统一走它取数，
// 避免各入口各自造行情（历史缺陷正是「一条入口接了回测、另一条传 None」）。
//
// ⚠ 必须逐条 `pub use reflection::X;`，**禁止**写成 `pub use reflection::{X, Y};`
// 后者会被 build.rs 的 re-export 解析误判为「reflection 模块全部命令都已提到
// stock_workflow 层」，于是用父模块路径注册 reflection.rs 里所有 #[tauri::command]
// ⇒ 编译期报 `cannot find __cmd__run_batch_reflection in stock_workflow`，
// 错误信息完全指不到真因（详见 build.rs `parse_submodule_mod_rs` 的注释）。
// 另：`MarketSnapshot` 类型无需 re-export，用 `reflection::MarketSnapshot` 访问即可。
pub use reflection::compute_market_snapshot;

// Re-export public structs
