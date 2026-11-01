// SPDX-License-Identifier: AGPL-3.0-only

//! DynamicSubGraph — 从 harness 重导出
//!
//! `DynamicSubGraph` 和 `GeneratedSubGraph` 的权威定义
//! 已迁移至 `axagent-harness::capability_pack_orchestration::subgraph`。

pub use axagent_harness::capability_pack_orchestration::subgraph::{
    DynamicSubGraph, GeneratedSubGraph,
};
