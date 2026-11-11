// SPDX-License-Identifier: AGPL-3.0-only

//! 能力集清单 DTO
//!
//! 权威定义在 harness `capability`（#264 批 6 上收）：`CapabilityInventory` /
//! `CapabilityEntry`（含 `new` / `recalc_count` / `all_entries` / `Default`）。
//! 能力来源统一复用上游能力发现索引（`capability_indexer` 的能力护照），由接缝
// 实现层读取组装，不再本地扫描落库。

// `CapabilitySource`（承载形态 Tool/Skill/McpTool/Workflow/Agent）的权威定义在
// harness `capability_pack_registry`（#221 上收），harness 侧消歧名 `CapabilityCarrier`
// —— 与 harness::capability::CapabilitySource（护照注册来源 Builtin/Plugin）语义不同。
pub use axagent_harness::capability_pack_registry::CapabilityCarrier as CapabilitySource;

pub use axagent_harness::capability::{CapabilityEntry, CapabilityInventory};
