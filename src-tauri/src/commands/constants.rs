// SPDX-License-Identifier: AGPL-3.0-only

//! 集中式常量定义
//!
//! 避免在代码中硬编码字符串字面量。
//!
//! ⚠ 本模块不再自行定义常量：`role` 的权威源是 `axagent_harness::constants::role`，
//! 此处仅 re-export（禁止重复定义，见 AGENTS.md 禁区 12）。
//! 调用方路径 `crate::commands::constants::role::<X>` 保持不变。

pub use axagent_harness::constants::domain_pack;
pub use axagent_harness::constants::role;
