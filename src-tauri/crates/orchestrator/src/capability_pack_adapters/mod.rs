// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 域包适配器模块 — 从 harness 重导出核心 trait
//!
//! `CapabilityPackAdapter` trait 和 `CapabilityPackAdapterRegistry` 的权威定义
//! 已迁移至 `axagent-harness::capability_pack_orchestration`。
//! 本模块仅保留重导出和 `BaseCapabilityPackAdapter`。

pub mod base_adapter;
pub mod types;

pub use axagent_harness::capability_pack_orchestration::{
    CapabilityPackAdapter, CapabilityPackAdapterRegistry,
};

// 导出基础适配器
pub use base_adapter::BaseCapabilityPackAdapter;
