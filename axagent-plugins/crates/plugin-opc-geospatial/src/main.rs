// SPDX-License-Identifier: AGPL-3.0-only
//! geospatial 域包 worker：6 条域专属命令。
//!
//! 命令实现只做「形状校验 + 固定域 id + 转发接缝」，工具逻辑留在宿主：
//! 每条工具命令走 `tool.set/exec`，自动化规则与实体批量校验走 `opc.store`。
//! 这是可执行 Rust 二进制，dist 中产出 worker 产物与 `worker://` 命令。
//! 命令→工具映射由 scripts/generate-opc-worker-source.mjs 从统一契约派生，勿手工改本文件。

use std::io::{StdinLock, StdoutLock};
use std::process::ExitCode;

use axagent_plugin_opc_core as core;
use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, CommandFault, CommandFrame, CommandOutcome, CommandReply,
    FrameRequest, FrameResponse, Peer, PluginDeclaration, error_codes, ops, serve,
};
use serde_json::{Value, json};

type WorkerPeer = Peer<StdinLock<'static>, StdoutLock<'static>>;
const PLUGIN_NAME: &str = "opc-geospatial";
/// 固定注入的域包 id：调用方不能通过命令入参越域触达别的域包规则。
const DOMAIN_ID: &str = "geospatial";
const CMD_RUN_RULES: &str = "opc_geospatial_run_automation_rules";

/// 域命令 → 宿主工具名。入参逐字透传，工具入参的真实 schema 由宿主负责校验。
const TOOL_COMMANDS: &[(&str, &str)] = &[
    ("opc_geospatial_opc_search_wiki", "OpcSearchWiki"),
    ("opc_geospatial_web_search", "WebSearch"),
    (
        "opc_geospatial_opc_create_content_asset",
        "OpcCreateContentAsset",
    ),
    ("opc_geospatial_opc_create_project", "OpcCreateProject"),
    ("opc_geospatial_file_write", "FileWrite"),
];

fn own_id_belongs_to_this_plugin(own_id: &str) -> bool {
    own_id == PLUGIN_NAME || own_id.starts_with(&format!("{PLUGIN_NAME}@"))
}

fn declaration() -> FrameResponse {
    FrameResponse::success(
        serde_json::to_value(PluginDeclaration {
            proto_version: AXAGENT_PLUGIN_PROTO_VERSION,
            subscribe: Vec::new(),
            // 两条出站接缝都要声明：少一条，宿主在 `call_seam` 入口即拒绝（worker.rs 的
            // `declared_calls` 闸），错误发生在取 provider 之前。
            calls: vec![
                core::OPC_STORE_SEAM.to_string(),
                core::TOOL_SET_SEAM.to_string(),
            ],
            capabilities: Vec::new(),
        })
        .expect("插件声明是可序列化的纯数据"),
    )
}

/// 工具型命令：入参是对象才能逐字作为工具 args 透传，形状闸在命令层完成。
fn call_tool(
    frame: &CommandFrame,
    call_chain: &[String],
    peer: &mut WorkerPeer,
    op: &str,
    tool_name: &str,
) -> Result<Value, (String, String)> {
    let args = core::require_object_input(&frame.input, op)?;
    core::call_builtin_tool(peer, tool_name, args, call_chain)
}

fn handle_command(request: &FrameRequest, peer: &mut WorkerPeer) -> FrameResponse {
    let frame = match CommandFrame::from_frame_request(request) {
        Ok(frame) => frame,
        Err(e) => {
            return FrameResponse::error(
                error_codes::PLUGIN_COMMAND_INVALID_ARGS,
                format!("命令帧解码失败：{e}"),
            );
        }
    };
    let own_id = std::env::var("AXAGENT_PLUGIN_ID").unwrap_or_default();
    if frame.plugin_id != own_id || !own_id_belongs_to_this_plugin(&own_id) {
        return CommandFault::for_frame(
            &frame,
            error_codes::PLUGIN_COMMAND_ID_MISMATCH,
            format!("帧/身份与 `{PLUGIN_NAME}` 不符"),
        )
        .into_frame_response();
    }
    let outcome = if frame.command == CMD_RUN_RULES {
        core::run_domain_automation_rules(DOMAIN_ID, &frame.input, &request.call_chain, peer)
    } else if let Some((name, tool)) = TOOL_COMMANDS
        .iter()
        .find(|(name, _)| *name == frame.command.as_str())
    {
        call_tool(&frame, &request.call_chain, peer, name, tool)
    } else {
        return CommandFault::for_frame(
            &frame,
            error_codes::PLUGIN_COMMAND_UNKNOWN,
            format!("opc-geospatial 插件没有命令 `{}`", frame.command),
        )
        .into_frame_response();
    };
    let outcome = match outcome {
        Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
        Err((code, message)) => {
            CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
        }
    };
    outcome
        .into_frame_response()
        .unwrap_or_else(|e| FrameResponse::error(error_codes::PLUGIN_COMMAND_FAILED, e.to_string()))
}

fn main() -> ExitCode {
    let result = serve(
        |request: &FrameRequest, peer: &mut WorkerPeer| match request.op.as_str() {
            ops::DESCRIBE => declaration(),
            ops::COMMAND => handle_command(request, peer),
            ops::SHUTDOWN => FrameResponse::success(json!({ "ok": true })),
            other => FrameResponse::error(
                error_codes::PLUGIN_UNKNOWN_OP,
                format!("opc-geospatial 插件未实现该操作：{other}"),
            ),
        },
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("opc-geospatial worker 退出：{e}");
            ExitCode::FAILURE
        }
    }
}
