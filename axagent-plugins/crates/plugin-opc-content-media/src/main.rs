// SPDX-License-Identifier: AGPL-3.0-only
//! content_media 域包 worker：八条内容/发布计划命令。

use std::io::{StdinLock, StdoutLock};
use std::process::ExitCode;

use axagent_plugin_opc_core as core;
use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, CommandFault, CommandFrame, CommandOutcome, CommandReply,
    FrameRequest, FrameResponse, Peer, PluginDeclaration, error_codes, ops, serve,
};
use serde_json::json;

type WorkerPeer = Peer<StdinLock<'static>, StdoutLock<'static>>;
const PLUGIN_NAME: &str = "opc-content-media";

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
    if frame.plugin_id != own_id || !own_id_belongs_to_this_plugin(&own_id) {
        return CommandFault::for_frame(
            &frame,
            error_codes::PLUGIN_COMMAND_ID_MISMATCH,
            format!("帧/身份与 `{PLUGIN_NAME}` 不符"),
        )
        .into_frame_response();
    }
    let outcome = match frame.command.as_str() {
        "opc_list_blog_posts" => core::blog_posts(&frame.input, &request.call_chain, peer),
        "opc_publish_blog_post" => core::publish_blog_post(&frame.input, &request.call_chain, peer),
        "opc_delete_blog_post" => core::delete_blog_post(&frame.input, &request.call_chain, peer),
        "opc_create_blog_post" => core::create_blog_post(&frame.input, &request.call_chain, peer),
        "opc_list_content_assets" => {
            core::list_rows_no_args(&request.call_chain, peer, core::OPC_STORE_LIST_CONTENT_ASSETS)
        },
        "opc_list_publish_schedules" => core::list_rows_no_args(
            &request.call_chain,
            peer,
            core::OPC_STORE_LIST_PUBLISH_SCHEDULES,
        ),
        "opc_update_content_asset" => {
            core::update_content_asset(&frame.input, &request.call_chain, peer)
        },
        "opc_delete_content_asset" => core::id_command(
            &frame.input,
            &request.call_chain,
            peer,
            "opc_delete_content_asset",
            core::OPC_STORE_DELETE_CONTENT_ASSET,
        ),
        "opc_cancel_publish_schedule" => core::id_command(
            &frame.input,
            &request.call_chain,
            peer,
            "opc_cancel_publish_schedule",
            core::OPC_STORE_CANCEL_PUBLISH_SCHEDULE,
        ),
        "opc_process_due_schedules" => core::list_rows_no_args(
            &request.call_chain,
            peer,
            core::OPC_STORE_PROCESS_DUE_SCHEDULES,
        ),
        other => {
            return CommandFault::for_frame(
                &frame,
                error_codes::PLUGIN_COMMAND_UNKNOWN,
                format!("content_media 插件没有命令 `{other}`"),
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
            format!("content_media 插件未实现该操作：{other}"),
        ),
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("content_media worker 退出：{e}");
            ExitCode::FAILURE
        },
    }
}
