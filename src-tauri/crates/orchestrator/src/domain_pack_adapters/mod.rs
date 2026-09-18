// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 域包适配器模块 — 从 harness 重导出核心 trait
//!
//! `DomainPackAdapter` trait 和 `DomainPackAdapterRegistry` 的权威定义
//! 已迁移至 `axagent-harness::domain_pack_orchestration`。
//! 本模块仅保留重导出和 `BaseDomainPackAdapter`。

pub mod base_adapter;
pub mod types;

pub use axagent_harness::domain_pack_orchestration::{
    DomainPackAdapter, DomainPackAdapterRegistry,
};

// 导出基础适配器
pub use base_adapter::BaseDomainPackAdapter;
