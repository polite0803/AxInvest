// SPDX-License-Identifier: AGPL-3.0-only
//! accounting 域包 worker。
//!
//! 身份只接受 `opc-accounting` 或 `opc-accounting@*`。命令面只有发票列表 / 创建 /
//! 状态变更三条；其余会计命令（`list_customers` / `get_financial_report` / `record_kpi`
//! / `delete_invoice`）仍属公共底座，绝不复制到本 worker。

use std::io::{StdinLock, StdoutLock};
use std::process::ExitCode;

use axagent_plugin_opc_core as core;
use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, CommandFault, CommandFrame, CommandOutcome, CommandReply,
    FrameRequest, FrameResponse, Peer, PluginDeclaration, error_codes, ops, serve,
};
use serde_json::json;

type WorkerPeer = Peer<StdinLock<'static>, StdoutLock<'static>>;

const PLUGIN_NAME: &str = "opc-accounting";
const COMMAND_LIST_INVOICES: &str = "opc_list_invoices";
const COMMAND_CREATE_INVOICE: &str = "opc_create_invoice";
const COMMAND_TRANSITION_INVOICE: &str = "opc_transition_invoice";

fn own_id_belongs_to_this_plugin(own_id: &str) -> bool {
    own_id == PLUGIN_NAME || own_id.starts_with(&format!("{PLUGIN_NAME}@"))
}

fn declaration() -> FrameResponse {
    FrameResponse::success(
        serde_json::to_value(PluginDeclaration {
            proto_version: AXAGENT_PLUGIN_PROTO_VERSION,
            subscribe: Vec::new(),
            calls: vec![core::OPC_STORE_SEAM.to_string()],
            capabilities: Vec::new(),
        })
        .expect("插件声明是可序列化的纯数据"),
    )
}

fn handle_command(request: &FrameRequest, peer: &mut WorkerPeer) -> FrameResponse {
    let frame = match CommandFrame::from_frame_request(request) {
        Ok(frame) => frame,
        Err(e) => {
            return FrameResponse::error(
                error_codes::PLUGIN_COMMAND_INVALID_ARGS,
                format!("命令帧解码失败：{e}"),
            );
        },
    };
    let own_id = std::env::var("AXAGENT_PLUGIN_ID").unwrap_or_default();
    if frame.plugin_id != own_id {
        return CommandFault::for_frame(
            &frame,
            error_codes::PLUGIN_COMMAND_ID_MISMATCH,
            format!("帧绑定 `{}` 与本 worker 身份 `{own_id}` 不符，拒绝执行", frame.plugin_id),
        )
        .into_frame_response();
    }
    if !own_id_belongs_to_this_plugin(&own_id) {
        return CommandFault::for_frame(
            &frame,
            error_codes::PLUGIN_COMMAND_ID_MISMATCH,
            format!("worker 身份 `{own_id}` 不属于本二进制声明的插件 `{PLUGIN_NAME}`"),
        )
        .into_frame_response();
    }

    let outcome = match frame.command.as_str() {
        COMMAND_LIST_INVOICES => core::list_invoices(&frame.input, &request.call_chain, peer),
        COMMAND_CREATE_INVOICE => core::create_invoice(&frame.input, &request.call_chain, peer),
        COMMAND_TRANSITION_INVOICE => {
            core::transition_invoice(&frame.input, &request.call_chain, peer)
        },
        other => {
            return CommandFault::for_frame(
                &frame,
                error_codes::PLUGIN_COMMAND_UNKNOWN,
                format!("accounting 插件没有命令 `{other}`"),
            )
            .into_frame_response();
        },
    };
    let outcome = match outcome {
        Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
        Err((code, message)) => {
            CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
        },
    };
    outcome
        .into_frame_response()
        .unwrap_or_else(|e| FrameResponse::error(error_codes::PLUGIN_COMMAND_FAILED, e.to_string()))
}

fn main() -> ExitCode {
    let result = serve(|request: &FrameRequest, peer: &mut WorkerPeer| match request.op.as_str() {
        ops::DESCRIBE => declaration(),
        ops::COMMAND => handle_command(request, peer),
        ops::SHUTDOWN => FrameResponse::success(json!({ "ok": true })),
        other => FrameResponse::error(
            error_codes::PLUGIN_UNKNOWN_OP,
            format!("accounting 插件未实现该操作：{other}"),
        ),
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("accounting worker 退出：{e}");
            ExitCode::FAILURE
        },
    }
}
