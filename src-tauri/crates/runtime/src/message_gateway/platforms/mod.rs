// SPDX-License-Identifier: AGPL-3.0-only

//! Re-exported from axagent-rt-messaging。
//!
//! 真正的 `MESSAGE_CALLBACK` static、setter/getter、PlatformAdapter trait
//! 都在 `axagent_rt_messaging::message_gateway::platforms`。
//! 本模块仅作 re-export 转发，避免双 crate 复制。

// [2026-09-03] 本目录下 8 个 dingtalk/discord/... stub（各 4 行，纯 pub use 转发）
// 属冗余转发层：上方 glob 已全量 re-export 同一批符号，权威在 rt-messaging。
// 未声明（不参与编译），是否删除待用户裁决。
pub use axagent_rt_messaging::message_gateway::platforms::*;
