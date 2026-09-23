// SPDX-License-Identifier: AGPL-3.0-only

//! D5 外部依赖指标查询命令（审计报告 `AUDIT-codebase-review-roadmap-2026-09-19.md` §三 #8 / §四 D5）
//!
//! # 解决的问题
//!
//! `astock-data`（行情 vendor）与 `providers`（LLM 供应商）两条外部依赖此前
//! **只有日志、没有指标** —— 回答不了「近 24h 各供应商成功率 / 降级次数」，
//! 只能翻日志逐行数。本命令是那套指标的**唯一查询出口**。
//!
//! # 数据来源与口径
//!
//! 全部读数来自 `axagent_harness::dependency_metrics` 的**进程内**注册表：
//! - 埋点在两条外部依赖的**唯一收口**上（LLM：`execute_llm` / `execute_llm_stream`；
//!   vendor：`VendorHealthTracker::record_success` / `record_failure`），故无需各调用点配合；
//! - 口径细节（缓存命中不计入、429/空数据不计入、`Disabled` 冻结、
//!   **进程重启归零**）写在 `crates/harness/src/dependency_metrics.rs` 模块头，
//!   **读数前必须先看那里**，否则会把「本次进程启动以来」误读成「长期统计」。
//!
//! # 形态说明
//!
//! 与 `commands/agent_nudge.rs::get_invoke_metrics` 同形态：非 async、零参数、
//! 零 `AppState`（注册表是进程级静态，不挂在 state 上），`call_mode = Manual`。

use axagent_agent_macro::agent_command;
use axagent_harness::dependency_metrics::{DependencyMetricsSnapshot, snapshot};

/// 获取 LLM / 行情 vendor 的调用指标（累计值 + 近 24h 窗口）
///
/// 返回 `entries` 按 (kind, name) 排序，读数稳定可比；`successRate` 在
/// 「成功数 + 失败数 = 0」时为 `null`（**不是 1.0** —— 那会让「没数据」看起来像「全成功」）。
#[agent_command(domain = agent, safety = Safe, call_mode = Manual, description = "获取外部依赖（LLM/数据源）调用指标")]
#[tauri::command]
pub fn get_dependency_metrics() -> Result<DependencyMetricsSnapshot, String> {
    Ok(snapshot())
}
