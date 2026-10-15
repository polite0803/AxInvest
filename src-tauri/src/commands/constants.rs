// SPDX-License-Identifier: AGPL-3.0-only

//! 集中式常量定义
//!
//! 避免在代码中硬编码字符串字面量。
//!
//! ⚠ 本模块的常量权威源分两类：
//! - `role` / `capability_pack` 的权威源是 `axagent_harness::constants`，
//!   此处仅 re-export（禁止重复定义，见 AGENTS.md 禁区 12）。
//!   调用方路径 `crate::commands::constants::role::<X>` 保持不变。
//! - 内置定时任务的 `task_type` 在此直接定义：它们被任务中心白名单、cron
//!   seed、执行分支与任务模块横向共享，留在任一业务命令模块都会逼出
//!   `crate::commands::<业务模块>::` 横向互调（分层护栏规则 2），故按
//!   共享基础设施常量收口到这里。

pub use axagent_harness::constants::capability_pack;
pub use axagent_harness::constants::role;

/// 妖股漏检核查定时任务的 task_type
/// （与前端任务中心白名单、`init/services.rs` 执行分支逐字对齐）。
pub const MOVER_RECALL_TASK_TYPE: &str = "mover-recall";

/// 进化计划定时聚合任务的 task_type
/// （与前端任务中心白名单、`init/services.rs` 执行分支逐字对齐）。
pub const EVOLUTION_PROPOSAL_TASK_TYPE: &str = "evolution-proposal";

/// 收盘组合指标日更任务的 task_type
/// （与前端任务中心白名单、`init/services.rs` 执行分支逐字对齐）。
pub const PORTFOLIO_METRICS_TASK_TYPE: &str = "portfolio-metrics";

/// pending 交易意图自动过期任务的 task_type
/// （与前端任务中心白名单、`init/services.rs` 执行分支逐字对齐）。
pub const INTENT_EXPIRY_TASK_TYPE: &str = "intent-expiry";
