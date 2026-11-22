// SPDX-License-Identifier: AGPL-3.0-only

//! SDK 契约钉版冒烟（2026-11-22）。
//!
//! 为什么宿主侧要有这个测试：17 个业务插件迁出主仓后，`plugin-opc-core` /
//! `plugin-testkit` 留在宿主 workspace，作为 SDK 权威快照
//! （`scripts/generate-plugin-independent-repos.mjs` 从这里同步到各独立仓）。
//! 宿主自己的 `cargo test` 因此是「宿主与插件两侧用同一份 SDK」的本地证据：
//! 接缝名或命令帧信封被手改，先在这里红，而不是等某个插件仓的 CI 红。
//!
//! 只钉两处最容易被手改的东西：
//!   · OPC 接缝名（`opc.store` / `tool.set` / `exec`）—— 宿主 `worker.rs` 的分派字面量必须同值；
//!   · `command_frame` 造出的命令帧形状 —— 宿主按同一信封（`op=command` + args 四字段）解包。

use axagent_plugin_opc_core::{OPC_STORE_SEAM, TOOL_SET_EXEC, TOOL_SET_SEAM};
use axagent_plugin_proto::ops;
use axagent_plugin_testkit::command_frame;
use serde_json::json;

#[test]
fn opc_seam_names_are_pinned() {
    assert_eq!(OPC_STORE_SEAM, "opc.store");
    assert_eq!(TOOL_SET_SEAM, "tool.set");
    assert_eq!(TOOL_SET_EXEC, "exec");
}

#[test]
fn testkit_command_frame_matches_host_envelope() {
    let frame = command_frame(
        "opc-accounting@builtin",
        OPC_STORE_SEAM,
        json!({ "op": "list_invoices" }),
        30_000,
    );

    assert_eq!(frame.op, ops::COMMAND, "命令帧的操作名固定为 command");

    let args = serde_json::to_value(&frame.args).expect("args 必须可序列化");
    assert_eq!(args["plugin_id"], json!("opc-accounting@builtin"));
    assert_eq!(args["command"], json!(OPC_STORE_SEAM));
    assert_eq!(args["input"], json!({ "op": "list_invoices" }));
    assert_eq!(args["timeout_ms"], json!(30_000_u64));
}
