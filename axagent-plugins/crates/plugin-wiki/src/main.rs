// SPDX-License-Identifier: AGPL-3.0-only
//! Wiki 业务功能包 worker（P6.3 第一片）。
//!
//! 当前命令面覆盖笔记创建、更新、删除、列表、读取、出链、反链、模板读写，
//! 同步到知识库、缓存图谱 / 社区检测，以及笔记版本历史读写面。
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
const COMMAND_SYNC_NOTE_LINKS: &str = "wiki_sync_note_links";
const COMMAND_LIST_NOTES: &str = "wiki_list_notes";
const COMMAND_GET_NOTE_BACKLINKS: &str = "wiki_get_note_backlinks";
const COMMAND_GET_NOTE_LINKS: &str = "wiki_get_note_links";
const COMMAND_GET_NOTE: &str = "wiki_get_note";
const COMMAND_GET_NOTE_BY_PATH: &str = "wiki_get_note_by_path";
const COMMAND_SEARCH_NOTES: &str = "wiki_notes_search";
const COMMAND_IMPORT_OBSIDIAN_VAULT: &str = "wiki_import_obsidian_vault";
const COMMAND_IMPORT_KNOWLEDGE_MD: &str = "wiki_import_knowledge_md";
const COMMAND_EXPORT_MARKDOWN: &str = "wiki_export_markdown";
const COMMAND_EXPORT_HTML: &str = "wiki_export_html";
const COMMAND_NOTE_EXPORT_HTML: &str = "wiki_note_export_html";
const COMMAND_LIST_TEMPLATES: &str = "wiki_list_templates";
const COMMAND_CREATE_TEMPLATE: &str = "wiki_create_template";
const COMMAND_UPDATE_TEMPLATE: &str = "wiki_update_template";
const COMMAND_DELETE_TEMPLATE: &str = "wiki_template_delete";
const COMMAND_GET_GRAPH_CACHED: &str = "wiki_get_graph_cached";
const COMMAND_GET_GRAPH_COMMUNITIES_CACHED: &str = "wiki_get_graph_communities_cached";
const COMMAND_GET_GRAPH: &str = "wiki_get_graph";
const COMMAND_DETECT_COMMUNITIES: &str = "wiki_detect_communities";
const COMMAND_INVALIDATE_GRAPH_CACHE: &str = "wiki_invalidate_graph_cache";
const COMMAND_LIST_NOTE_VERSIONS: &str = "wiki_note_versions";
const COMMAND_GET_NOTE_VERSION: &str = "wiki_note_get_version";
const COMMAND_RESTORE_NOTE_VERSION: &str = "wiki_note_restore_version";
const COMMAND_NOTE_FROM_TEMPLATE: &str = "wiki_note_create_from_template";
const COMMAND_CREATE_DAILY_NOTE: &str = "wiki_create_daily_note";
const COMMAND_UPDATE_WIKI: &str = "wiki_update_wiki";
const COMMAND_REBUILD_INDEX: &str = "wiki_rebuild_index";
const COMMAND_REPAIR_GRAPH: &str = "wiki_repair_graph";
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

/// 取必填正整数版本 id；缺失 / 非整数 / 非正数一律拒收。
fn require_i64(input: &Value, key: &str) -> Result<i64, (String, String)> {
    match input.get(key) {
        Some(Value::Number(number))
            if number.is_i64() && number.as_i64().is_some_and(|v| v > 0) =>
        {
            Ok(number.as_i64().expect("已确认是 i64"))
        },
        _ => Err((
            error_codes::PLUGIN_COMMAND_INVALID_ARGS.to_string(),
            format!("命令需要正整数参数 `{key}`"),
        )),
    }
}

/// 取可选标题：缺失 / `null` 表示由宿主用模板名兜底；非字符串一律拒收。
fn optional_title(input: &Value) -> Result<Option<String>, (String, String)> {
    match input.get("title") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(Some(value.clone())),
        _ => Err((
            error_codes::PLUGIN_COMMAND_INVALID_ARGS.to_string(),
            "命令 `wiki_note_create_from_template` 的可选参数 `title` 必须是非空字符串或 null"
                .into(),
        )),
    }
}

/// 取可选字符串参数；缺失 / `null` 返回 `None`（保持原值），非字符串报参数非法。
fn optional_str(
    input: &Value,
    key: &str,
    command: &str,
) -> Result<Option<String>, (String, String)> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        _ => Err((
            error_codes::PLUGIN_COMMAND_INVALID_ARGS.to_string(),
            format!("命令 `{command}` 的可选参数 `{key}` 必须是字符串或 null"),
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
        COMMAND_GET_NOTE_BACKLINKS => {
            let note_id = match require_str(&frame.input, "noteId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            seam_call("get_note_backlinks", json!({ "noteId": note_id }))
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
        COMMAND_SEARCH_NOTES => {
            let vault_id = match require_str(&frame.input, "vaultId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            let query = match require_str(&frame.input, "query") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            let top_k = match frame.input.get("topK") {
                None | Some(Value::Null) => None,
                Some(Value::Number(number)) if number.is_u64() => {
                    Some(number.as_u64().unwrap() as usize)
                },
                _ => {
                    return CommandFault::for_frame(
                        &frame,
                        error_codes::PLUGIN_COMMAND_INVALID_ARGS.to_string(),
                        "命令 `wiki_notes_search` 的可选参数 `topK` 必须是非负整数或 null",
                    )
                    .into_frame_response();
                },
            };
            seam_call("search_notes", json!({ "vaultId": vault_id, "query": query, "topK": top_k }))
        },
        COMMAND_IMPORT_OBSIDIAN_VAULT => {
            let wiki_id = match require_str(&frame.input, "wikiId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            let vault_path = match require_str(&frame.input, "vaultPath") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            seam_call(
                "import_obsidian_vault",
                json!({ "wikiId": wiki_id, "vaultPath": vault_path }),
            )
        },
        COMMAND_IMPORT_KNOWLEDGE_MD => {
            let wiki_id = match require_str(&frame.input, "wikiId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            // `filePath` 可选：缺失 / null 表示由宿主用默认路径（KNOWLEDGE.md）。
            let file_path = match frame.input.get("filePath") {
                None | Some(Value::Null) => None,
                Some(Value::String(value)) if !value.trim().is_empty() => Some(value.clone()),
                _ => {
                    return CommandFault::for_frame(
                        &frame,
                        error_codes::PLUGIN_COMMAND_INVALID_ARGS.to_string(),
                        "命令 `wiki_import_knowledge_md` 的可选参数 `filePath` 必须是非空字符串或 null",
                    )
                    .into_frame_response();
                },
            };
            seam_call("import_knowledge_md", json!({ "wikiId": wiki_id, "filePath": file_path }))
        },
        COMMAND_EXPORT_MARKDOWN => {
            let wiki_id = match require_str(&frame.input, "wikiId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            let output_path = match require_str(&frame.input, "outputPath") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            seam_call("export_markdown", json!({ "wikiId": wiki_id, "outputPath": output_path }))
        },
        COMMAND_EXPORT_HTML => {
            let wiki_id = match require_str(&frame.input, "wikiId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            let output_path = match require_str(&frame.input, "outputPath") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            seam_call("export_html", json!({ "wikiId": wiki_id, "outputPath": output_path }))
        },
        COMMAND_NOTE_EXPORT_HTML => {
            let note_id = match require_str(&frame.input, "noteId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            let output_path = match require_str(&frame.input, "outputPath") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            seam_call("export_note_html", json!({ "noteId": note_id, "outputPath": output_path }))
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
        COMMAND_SYNC_NOTE_LINKS => {
            let vault_id = match require_str(&frame.input, "vaultId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            let source_note_id = match require_str(&frame.input, "sourceNoteId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            let links = match frame.input.get("links") {
                Some(Value::Array(links)) => links.clone(),
                _ => {
                    return CommandFault::for_frame(
                        &frame,
                        error_codes::PLUGIN_COMMAND_INVALID_ARGS.to_string(),
                        format!("命令 `{COMMAND_SYNC_NOTE_LINKS}` 需要数组参数 `links`"),
                    )
                    .into_frame_response();
                },
            };
            seam_call(
                "sync_note_links",
                json!({
                    "request": {
                        "vaultId": vault_id,
                        "sourceNoteId": source_note_id,
                        "links": links,
                    }
                }),
            )
        },
        COMMAND_LIST_TEMPLATES => {
            let wiki_id = match require_str(&frame.input, "wikiId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            seam_call("list_wiki_templates", json!({ "wikiId": wiki_id }))
        },
        COMMAND_CREATE_TEMPLATE => {
            let input = match frame.input.get("input") {
                Some(value @ Value::Object(_)) => value.clone(),
                _ => {
                    return CommandFault::for_frame(
                        &frame,
                        error_codes::PLUGIN_COMMAND_INVALID_ARGS.to_string(),
                        "命令 `wiki_create_template` 需要对象参数 `input`",
                    )
                    .into_frame_response();
                },
            };
            seam_call("create_wiki_template", json!({ "input": input }))
        },
        COMMAND_UPDATE_TEMPLATE => {
            let input = match frame.input.get("input") {
                Some(value @ Value::Object(_)) => value.clone(),
                _ => {
                    return CommandFault::for_frame(
                        &frame,
                        error_codes::PLUGIN_COMMAND_INVALID_ARGS.to_string(),
                        "命令 `wiki_update_template` 需要对象参数 `input`",
                    )
                    .into_frame_response();
                },
            };
            seam_call("update_wiki_template", json!({ "input": input }))
        },
        COMMAND_DELETE_TEMPLATE => {
            let id = match require_str(&frame.input, "id") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            seam_call("delete_wiki_template", json!({ "id": id }))
        },
        COMMAND_GET_GRAPH_CACHED => {
            let wiki_id = match require_str(&frame.input, "wikiId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            seam_call("get_cached_graph", json!({ "wikiId": wiki_id }))
        },
        COMMAND_GET_GRAPH_COMMUNITIES_CACHED => {
            let wiki_id = match require_str(&frame.input, "wikiId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            seam_call("get_cached_communities", json!({ "wikiId": wiki_id }))
        },
        COMMAND_GET_GRAPH => {
            let wiki_id = match require_str(&frame.input, "wikiId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            seam_call("get_graph", json!({ "wikiId": wiki_id }))
        },
        COMMAND_DETECT_COMMUNITIES => {
            let wiki_id = match require_str(&frame.input, "wikiId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            seam_call("detect_communities", json!({ "wikiId": wiki_id }))
        },
        COMMAND_INVALIDATE_GRAPH_CACHE => {
            let wiki_id = match require_str(&frame.input, "wikiId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            seam_call("invalidate_graph_cache", json!({ "wikiId": wiki_id }))
        },
        COMMAND_LIST_NOTE_VERSIONS => {
            let note_id = match require_str(&frame.input, "noteId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            seam_call("list_note_versions", json!({ "noteId": note_id }))
        },
        COMMAND_GET_NOTE_VERSION => {
            let version_id = match require_i64(&frame.input, "versionId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            seam_call("get_note_version", json!({ "versionId": version_id }))
        },
        COMMAND_RESTORE_NOTE_VERSION => {
            let note_id = match require_str(&frame.input, "noteId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            let version_id = match require_i64(&frame.input, "versionId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            seam_call("restore_note_version", json!({ "noteId": note_id, "versionId": version_id }))
        },
        COMMAND_NOTE_FROM_TEMPLATE => {
            let vault_id = match require_str(&frame.input, "vaultId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            let template_id = match require_str(&frame.input, "templateId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            let title = match optional_title(&frame.input) {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            seam_call(
                "create_note_from_template",
                json!({ "vaultId": vault_id, "templateId": template_id, "title": title }),
            )
        },
        COMMAND_CREATE_DAILY_NOTE => {
            let vault_id = match require_str(&frame.input, "vaultId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            seam_call("create_daily_note", json!({ "vaultId": vault_id }))
        },
        COMMAND_UPDATE_WIKI => {
            let id = match require_str(&frame.input, "id") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            // 命令面键名沿用宿主原 `update_wiki`（`id` + 四个可选字段，全部 camelCase）；
            // 接缝侧包一层 `request`，缺省 / null 字段一律透传 null ⇒ 宿主保持原值。
            let name = match optional_str(&frame.input, "name", COMMAND_UPDATE_WIKI) {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            let description = match optional_str(&frame.input, "description", COMMAND_UPDATE_WIKI) {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            let embedding_provider =
                match optional_str(&frame.input, "embeddingProvider", COMMAND_UPDATE_WIKI) {
                    Ok(value) => value,
                    Err((code, message)) => {
                        return CommandFault::for_frame(&frame, code, message)
                            .into_frame_response();
                    },
                };
            let knowledge_base_id =
                match optional_str(&frame.input, "knowledgeBaseId", COMMAND_UPDATE_WIKI) {
                    Ok(value) => value,
                    Err((code, message)) => {
                        return CommandFault::for_frame(&frame, code, message)
                            .into_frame_response();
                    },
                };
            seam_call(
                "update_wiki",
                json!({
                    "request": {
                        "wikiId": id,
                        "name": name,
                        "description": description,
                        "embeddingProvider": embedding_provider,
                        "knowledgeBaseId": knowledge_base_id,
                    }
                }),
            )
        },
        COMMAND_REBUILD_INDEX => {
            let wiki_id = match require_str(&frame.input, "wikiId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            // fire-and-forget：宿主清库后立即返回，逐篇进度走 `wiki-note-indexed` 事件。
            seam_call("rebuild_index", json!({ "wikiId": wiki_id }))
        },
        COMMAND_REPAIR_GRAPH => {
            let wiki_id = match require_str(&frame.input, "wikiId") {
                Ok(value) => value,
                Err((code, message)) => {
                    return CommandFault::for_frame(&frame, code, message).into_frame_response();
                },
            };
            seam_call("repair_graph", json!({ "wikiId": wiki_id }))
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
