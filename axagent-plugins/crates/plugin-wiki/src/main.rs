// SPDX-License-Identifier: AGPL-3.0-only
//! Wiki 业务功能包 worker（P6.3 第一片）。
//!
//! 当前命令面覆盖笔记创建、更新、删除、列表、读取、出链，以及同步到知识库。
//! 它**不查库、不读文件**，只把命令参数转成宿主 `wiki.store` 接缝的入参；
//! 笔记正文、markdown 落盘、知识库文档行与索引入队全部在宿主 provider 内完成
//!（数据主权在宿主）。
//!
//! 身份只接受 `wiki` 或 `wiki@*`；别的逻辑名（如 `opc@bundled`）在本 worker 被拒。

use std::io::{StdinLock, StdoutLock};
use std::process::ExitCode;

use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, CommandFault, CommandFrame, CommandOutcome, CommandReply,
    FrameRequest, FrameResponse, Peer, PluginDeclaration, error_codes, ops, serve,
};
use serde_json::{Value, json};

type WorkerPeer = Peer<StdinLock<'static>, StdoutLock<'static>>;

const PLUGIN_NAME: &str = "wiki";
const COMMAND_CREATE_NOTE: &str = "wiki_create_note";
const COMMAND_UPDATE_NOTE: &str = "wiki_update_note";
const COMMAND_DELETE_NOTE: &str = "wiki_delete_note";
const COMMAND_SYNC_NOTE: &str = "wiki_sync_note_to_knowledge_base";
const COMMAND_LIST_NOTES: &str = "wiki_list_notes";
const COMMAND_GET_NOTE_LINKS: &str = "wiki_get_note_links";
const COMMAND_GET_NOTE: &str = "wiki_get_note";
const COMMAND_GET_NOTE_BY_PATH: &str = "wiki_get_note_by_path";
/// 宿主提供的出站接缝；少声明一条 = 运行时被拒。
const WIKI_STORE_SEAM: &str = "wiki.store";
const WIKI_STORE_SYNC_NOTE: &str = "sync_note_to_knowledge_base";

fn own_id_belongs_to_this_plugin(own_id: &str) -> bool {
    own_id == PLUGIN_NAME || own_id.starts_with(&format!("{PLUGIN_NAME}@"))
}

fn declaration() -> FrameResponse {
    FrameResponse::success(
        serde_json::to_value(PluginDeclaration {
            proto_version: AXAGENT_PLUGIN_PROTO_VERSION,
            subscribe: Vec::new(),
            calls: vec![WIKI_STORE_SEAM.to_string()],
            capabilities: Vec::new(),
        })
        .expect("插件声明是可序列化的纯数据"),
    )
}

/// 取必填字符串；缺失 / 非字符串 / 全空白一律拒收（不静默补默认值）。
fn require_str(input: &Value, key: &str) -> Result<String, (String, String)> {
    match input.get(key) {
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(value.clone()),
        _ => Err((
            error_codes::PLUGIN_COMMAND_INVALID_ARGS.to_string(),
            format!("命令 `{COMMAND_SYNC_NOTE}` 需要非空字符串参数 `{key}`"),
        )),
    }
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

    let mut seam_call = |op: &str, args: Value| -> Result<Value, (String, String)> {
        match peer.call_seam(WIKI_STORE_SEAM, op, args, request.call_chain.clone()) {
            Ok(reply) => match reply.kind {
                axagent_plugin_proto::ResponseKind::Success => {
                    Ok(reply.value.unwrap_or(Value::Null))
                },
                axagent_plugin_proto::ResponseKind::Error => Err((
                    reply.code.unwrap_or_else(|| error_codes::PLUGIN_COMMAND_FAILED.to_string()),
                    reply
                        .message
                        .unwrap_or_else(|| "宿主 wiki.store 接缝返回错误但未说明".to_string()),
                )),
            },
            Err(e) => Err((
                error_codes::PLUGIN_COMMAND_FAILED.to_string(),
                format!("wiki.store 接缝调用失败：{e}"),
            )),
        }
    };
    let outcome = match frame.command.as_str() {
        COMMAND_LIST_NOTES => {
            let vault_id = match require_str(&frame.input, "vaultId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            seam_call("list_notes", json!({ "vaultId": vault_id }))
        },
        COMMAND_GET_NOTE_LINKS => {
            let note_id = match require_str(&frame.input, "noteId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            seam_call("get_note_links", json!({ "noteId": note_id }))
        },
        COMMAND_GET_NOTE => {
            let id = match require_str(&frame.input, "id") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            seam_call("get_note", json!({ "id": id }))
        },
        COMMAND_GET_NOTE_BY_PATH => {
            let vault_id = match require_str(&frame.input, "vaultId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            let file_path = match require_str(&frame.input, "filePath") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            seam_call("get_note_by_path", json!({ "vaultId": vault_id, "filePath": file_path }))
        },
        COMMAND_CREATE_NOTE => {
            let input = match frame.input.get("input") {
                Some(value @ Value::Object(_)) => value.clone(),
                _ => {
                    return CommandFault::for_frame(
                        &frame,
                        error_codes::PLUGIN_COMMAND_INVALID_ARGS.to_string(),
                        "命令 `wiki_create_note` 需要对象参数 `input`",
                    )
                    .into_frame_response();
                },
            };
            seam_call("create_note", json!({ "input": input }))
        },
        COMMAND_UPDATE_NOTE => {
            let id = match require_str(&frame.input, "id") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            let input = match frame.input.get("input") {
                Some(value @ Value::Object(_)) => value.clone(),
                _ => {
                    return CommandFault::for_frame(
                        &frame,
                        error_codes::PLUGIN_COMMAND_INVALID_ARGS.to_string(),
                        "命令 `wiki_update_note` 需要对象参数 `input`",
                    )
                    .into_frame_response();
                },
            };
            seam_call("update_note", json!({ "id": id, "input": input }))
        },
        COMMAND_DELETE_NOTE => {
            let id = match require_str(&frame.input, "id") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            seam_call("delete_note", json!({ "id": id }))
        },
        COMMAND_SYNC_NOTE => {
            let note_id = match require_str(&frame.input, "noteId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            let knowledge_base_id = match require_str(&frame.input, "knowledgeBaseId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            // 命令面 camelCase（禁区 13）；接缝入参同样 camelCase，宿主 provider
            // 反序列化成契约 DTO。id 之外的字段插件侧不传（数据主权在宿主）。
            seam_call(
                WIKI_STORE_SYNC_NOTE,
                json!({ "request": { "noteId": note_id, "knowledgeBaseId": knowledge_base_id } }),
            )
        },
        other => {
            return CommandFault::for_frame(
                &frame,
                error_codes::PLUGIN_COMMAND_UNKNOWN,
                format!("wiki 插件没有命令 `{other}`"),
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
            format!("wiki 插件未实现该操作：{other}"),
        ),
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("wiki worker 退出：{e}");
            ExitCode::FAILURE
        },
    }
}
