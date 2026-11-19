// SPDX-License-Identifier: AGPL-3.0-only

//! wiki 业务功能包 worker 的真进程帧测试。
//!
//! 锁三件事：① 身份白名单（`wiki` / `wiki@*`，别的逻辑名被拒）；
//! ② 出站接缝声明（`wiki.store`）；
//! ③ 命令面 camelCase → 接缝入参 camelCase 的逐字映射，以及缺参拒收。

use std::io::{BufReader, BufWriter};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use axagent_plugin_proto::{
    FrameRequest, FrameResponse, ResponseKind, error_codes, ops, read_inbound, write_json_frame,
};
use axagent_plugin_testkit::{
    assert_command_value, command_frame, declaration_of, describe_frame, shutdown_frame,
};

const PLUGIN_ID: &str = "wiki@bundled";

struct Worker {
    child: Child,
    stdin: Option<BufWriter<ChildStdin>>,
    stdout: BufReader<ChildStdout>,
}

impl Worker {
    fn spawn_as(plugin_id: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_axagent-plugin-wiki"))
            .env("AXAGENT_PLUGIN_ID", plugin_id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("wiki worker 应能启动");
        let stdin = BufWriter::new(child.stdin.take().expect("stdin 已设为 piped"));
        let stdout = BufReader::new(child.stdout.take().expect("stdout 已设为 piped"));
        Self { child, stdin: Some(stdin), stdout }
    }

    fn send(&mut self, request: &FrameRequest) {
        write_json_frame(self.stdin.as_mut().expect("写端仍打开"), request).expect("写帧应成功");
    }

    fn recv_request(&mut self) -> FrameRequest {
        read_inbound(&mut self.stdout)
            .expect("读帧应成功")
            .as_request()
            .cloned()
            .expect("期望 worker → 宿主的出站请求")
    }

    fn send_response(&mut self, response: &FrameResponse) {
        write_json_frame(self.stdin.as_mut().expect("写端仍打开"), response).expect("写帧应成功");
    }

    fn recv_response(&mut self) -> FrameResponse {
        read_inbound(&mut self.stdout)
            .expect("读帧应成功")
            .as_response()
            .cloned()
            .expect("期望 worker 回执")
    }

    fn shutdown(&mut self) {
        self.send(&shutdown_frame());
        assert_eq!(self.recv_response().kind, ResponseKind::Success);
        drop(self.stdin.take());
        assert!(self.child.wait().expect("等待 worker 退出").success());
    }
}

fn command(plugin_id: &str, name: &str, input: serde_json::Value) -> FrameRequest {
    command_frame(plugin_id, name, input, 5_000)
}

fn sample_note(id: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "vaultId": "v1",
        "title": "测试笔记",
        "filePath": "a.md",
        "content": "# hi",
        "contentHash": "h",
        "author": "u",
        "pageType": null,
        "tags": [],
        "sourceRefs": null,
        "relatedPages": null,
        "qualityScore": null,
        "lastLintedAt": null,
        "lastCompiledAt": null,
        "compiledSourceHash": null,
        "userEdited": false,
        "userEditedAt": null,
        "createdAt": 1,
        "updatedAt": 2,
        "isDeleted": false,
    })
}

#[test]
fn describe_declares_only_the_wiki_store_seam() {
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&describe_frame().with_id("req-describe"));
    let response = worker.recv_response();
    assert_eq!(response.request_id.as_deref(), Some("req-describe"));
    let declaration = declaration_of(&response);
    assert_eq!(declaration.calls, vec!["wiki.store".to_string()]);
    worker.shutdown();
}

#[test]
fn foreign_plugin_ids_are_rejected() {
    let mut worker = Worker::spawn_as("opc@bundled");
    worker.send(&command(
        "opc@bundled",
        "wiki_sync_note_to_knowledge_base",
        serde_json::json!({ "noteId": "n1", "knowledgeBaseId": "kb1" }),
    ));
    let response = worker.recv_response();
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_COMMAND_ID_MISMATCH));
    worker.shutdown();
}

#[test]
fn sync_command_goes_through_the_host_wiki_store_seam() {
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_sync_note_to_knowledge_base",
        serde_json::json!({ "noteId": "note-1", "knowledgeBaseId": "kb-1" }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.op, ops::CALL_SEAM);
    assert_eq!(call.args["seam"], "wiki.store");
    assert_eq!(call.args["op"], "sync_note_to_knowledge_base");
    // 接缝入参是嵌一层的 `request`，键名 camelCase（禁区 13）。
    assert_eq!(call.args["args"]["request"]["noteId"], "note-1");
    assert_eq!(call.args["args"]["request"]["knowledgeBaseId"], "kb-1");
    // 插件侧不传正文 / 标题 / 路径 —— 数据主权在宿主。
    assert!(call.args["args"]["request"].get("content").is_none());
    assert!(call.args["args"]["request"].get("sourcePath").is_none());

    // 宿主回执原样透出（documentId / indexQueued）。
    worker.send_response(
        &FrameResponse::success(serde_json::json!({
            "documentId": "doc-1",
            "indexQueued": true,
        }))
        .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    let response = worker.recv_response();
    assert_command_value(
        &response,
        &serde_json::json!({ "documentId": "doc-1", "indexQueued": true }),
    );
    worker.shutdown();
}

#[test]
fn missing_ids_are_rejected_before_calling_the_host() {
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_sync_note_to_knowledge_base",
        serde_json::json!({ "knowledgeBaseId": "kb-1" }),
    ));
    let response = worker.recv_response();
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_COMMAND_INVALID_ARGS));
    worker.shutdown();

    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_sync_note_to_knowledge_base",
        serde_json::json!({ "noteId": "   ", "knowledgeBaseId": "kb-1" }),
    ));
    let response = worker.recv_response();
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_COMMAND_INVALID_ARGS));
    worker.shutdown();
}

#[test]
fn get_note_reads_via_the_host_seam_without_write_permission() {
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(PLUGIN_ID, "wiki_get_note", serde_json::json!({ "id": "note-9" })));
    let call = worker.recv_request();
    assert_eq!(call.op, ops::CALL_SEAM);
    assert_eq!(call.args["seam"], "wiki.store");
    assert_eq!(call.args["op"], "get_note");
    // 标量键（无 request 外壳），与 get_collection_info 同形态。
    assert_eq!(call.args["args"]["id"], "note-9");

    let note = sample_note("note-9");
    worker.send_response(
        &FrameResponse::success(note.clone())
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    let response = worker.recv_response();
    assert_command_value(&response, &note);
    worker.shutdown();
}

#[test]
fn list_and_get_by_path_are_read_only_seam_calls() {
    // ① 列表：标量 vaultId。
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(PLUGIN_ID, "wiki_list_notes", serde_json::json!({ "vaultId": "v1" })));
    let call = worker.recv_request();
    assert_eq!(call.args["seam"], "wiki.store");
    assert_eq!(call.args["op"], "list_notes");
    assert_eq!(call.args["args"]["vaultId"], "v1");
    let notes = serde_json::json!([sample_note("note-1")]);
    worker.send_response(
        &FrameResponse::success(notes.clone())
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    let response = worker.recv_response();
    assert_command_value(&response, &notes);
    worker.shutdown();

    // ② 按路径：两个标量键都是 camelCase。
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_get_note_by_path",
        serde_json::json!({ "vaultId": "v1", "filePath": "dir/a.md" }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.args["op"], "get_note_by_path");
    assert_eq!(call.args["args"]["vaultId"], "v1");
    assert_eq!(call.args["args"]["filePath"], "dir/a.md");
    let note = sample_note("note-2");
    worker.send_response(
        &FrameResponse::success(note.clone())
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    let response = worker.recv_response();
    assert_command_value(&response, &note);
    worker.shutdown();
}

#[test]
fn search_notes_is_a_read_only_seam_call() {
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_notes_search",
        serde_json::json!({ "vaultId": "v1", "query": "测试", "topK": 5 }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.args["seam"], "wiki.store");
    assert_eq!(call.args["op"], "search_notes");
    assert_eq!(call.args["args"]["vaultId"], "v1");
    assert_eq!(call.args["args"]["query"], "测试");
    assert_eq!(call.args["args"]["topK"], 5);

    let result = serde_json::json!([{
        "note": sample_note("note-search"),
        "snippet": "…测试…",
        "score": 0.9,
    }]);
    worker.send_response(
        &FrameResponse::success(result.clone())
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    assert_command_value(&worker.recv_response(), &result);
    worker.shutdown();

    // 可选 topK 缺失 / null 也合法，透传为 null。
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_notes_search",
        serde_json::json!({ "vaultId": "v1", "query": "测试", "topK": null }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.args["op"], "search_notes");
    assert!(call.args["args"]["topK"].is_null());
    worker.send_response(
        &FrameResponse::success(serde_json::json!([]))
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    assert_command_value(&worker.recv_response(), &serde_json::json!([]));
    worker.shutdown();
}

#[test]
fn import_and_export_commands_map_to_the_host_wiki_store_seam() {
    // ① 导入 Obsidian vault：两个标量键，camelCase。
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_import_obsidian_vault",
        serde_json::json!({ "wikiId": "w1", "vaultPath": "/tmp/vault" }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.args["seam"], "wiki.store");
    assert_eq!(call.args["op"], "import_obsidian_vault");
    assert_eq!(call.args["args"]["wikiId"], "w1");
    assert_eq!(call.args["args"]["vaultPath"], "/tmp/vault");
    let stats = serde_json::json!({ "imported": 2, "failed": 0, "skipped": 1 });
    worker.send_response(
        &FrameResponse::success(stats.clone())
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    assert_command_value(&worker.recv_response(), &stats);
    worker.shutdown();

    // ② 导入 KNOWLEDGE.md：filePath 可选，缺失时透传 null。
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_import_knowledge_md",
        serde_json::json!({ "wikiId": "w1" }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.args["op"], "import_knowledge_md");
    assert_eq!(call.args["args"]["wikiId"], "w1");
    assert!(call.args["args"]["filePath"].is_null());
    let stats = serde_json::json!({ "imported": 1, "failed": 0, "skipped": 0, "total": 4 });
    worker.send_response(
        &FrameResponse::success(stats.clone())
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    assert_command_value(&worker.recv_response(), &stats);
    worker.shutdown();

    // ③ 导出 Markdown：wikiId + outputPath。
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_export_markdown",
        serde_json::json!({ "wikiId": "w1", "outputPath": "/tmp/out" }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.args["op"], "export_markdown");
    assert_eq!(call.args["args"]["outputPath"], "/tmp/out");
    let stats = serde_json::json!({ "exported": 3, "failed": 0 });
    worker.send_response(
        &FrameResponse::success(stats.clone())
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    assert_command_value(&worker.recv_response(), &stats);
    worker.shutdown();

    // ④ 导出 HTML：同上。
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_export_html",
        serde_json::json!({ "wikiId": "w1", "outputPath": "/tmp/out" }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.args["op"], "export_html");
    assert_eq!(call.args["args"]["wikiId"], "w1");
    worker.send_response(
        &FrameResponse::success(serde_json::json!({ "exported": 1, "failed": 0 }))
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    assert_command_value(
        &worker.recv_response(),
        &serde_json::json!({ "exported": 1, "failed": 0 }),
    );
    worker.shutdown();

    // ⑤ 单条笔记导出 HTML：noteId + outputPath，回执是路径字符串。
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_note_export_html",
        serde_json::json!({ "noteId": "n1", "outputPath": "/tmp/out/n1.pdf" }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.args["op"], "export_note_html");
    assert_eq!(call.args["args"]["noteId"], "n1");
    assert_eq!(call.args["args"]["outputPath"], "/tmp/out/n1.pdf");
    worker.send_response(
        &FrameResponse::success(serde_json::json!({ "path": "/tmp/out/n1.html" }))
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    assert_command_value(
        &worker.recv_response(),
        &serde_json::json!({ "path": "/tmp/out/n1.html" }),
    );
    worker.shutdown();
}

#[test]
fn import_and_export_commands_reject_missing_args() {
    // 缺 vaultPath ⇒ 插件侧拒收，不触达宿主。
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_import_obsidian_vault",
        serde_json::json!({ "wikiId": "w1" }),
    ));
    let response = worker.recv_response();
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_COMMAND_INVALID_ARGS));
    worker.shutdown();

    // 缺 outputPath ⇒ 同样拒收。
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(PLUGIN_ID, "wiki_export_html", serde_json::json!({ "wikiId": "w1" })));
    let response = worker.recv_response();
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_COMMAND_INVALID_ARGS));
    worker.shutdown();

    // 缺 noteId ⇒ 同样拒收。
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_note_export_html",
        serde_json::json!({ "outputPath": "/tmp/out/n1.html" }),
    ));
    let response = worker.recv_response();
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_COMMAND_INVALID_ARGS));
    worker.shutdown();
}

#[test]
fn create_note_is_a_write_seam_call_with_camel_case_input() {
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    let input = serde_json::json!({
        "vaultId": "v1",
        "title": "新笔记",
        "filePath": "new.md",
        "content": "# body",
        "author": "u",
    });
    worker.send(&command(PLUGIN_ID, "wiki_create_note", serde_json::json!({ "input": input })));
    let call = worker.recv_request();
    assert_eq!(call.args["seam"], "wiki.store");
    assert_eq!(call.args["op"], "create_note");
    assert_eq!(call.args["args"]["input"]["vaultId"], "v1");
    assert_eq!(call.args["args"]["input"]["filePath"], "new.md");
    assert_eq!(call.args["args"]["input"]["content"], "# body");

    let note = sample_note("note-new");
    worker.send_response(
        &FrameResponse::success(note.clone())
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    let response = worker.recv_response();
    assert_command_value(&response, &note);
    worker.shutdown();
}

#[test]
fn update_and_delete_notes_are_write_seam_calls() {
    // ① 更新：id 标量 + input 对象。
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    let input = serde_json::json!({ "title": "改标题", "content": "# 新" });
    worker.send(&command(
        PLUGIN_ID,
        "wiki_update_note",
        serde_json::json!({ "id": "note-3", "input": input }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.args["op"], "update_note");
    assert_eq!(call.args["args"]["id"], "note-3");
    assert_eq!(call.args["args"]["input"]["title"], "改标题");
    let note = sample_note("note-3");
    worker.send_response(
        &FrameResponse::success(note.clone())
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    let response = worker.recv_response();
    assert_command_value(&response, &note);
    worker.shutdown();

    // ② 删除：只有 id；宿主回执是空对象。
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(PLUGIN_ID, "wiki_delete_note", serde_json::json!({ "id": "note-3" })));
    let call = worker.recv_request();
    assert_eq!(call.args["op"], "delete_note");
    assert_eq!(call.args["args"]["id"], "note-3");
    worker.send_response(
        &FrameResponse::success(serde_json::json!({}))
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    let response = worker.recv_response();
    assert_command_value(&response, &serde_json::json!({}));
    worker.shutdown();
}

#[test]
fn get_note_links_is_a_read_only_seam_call() {
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_get_note_links",
        serde_json::json!({ "noteId": "note-1" }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.args["seam"], "wiki.store");
    assert_eq!(call.args["op"], "get_note_links");
    assert_eq!(call.args["args"]["noteId"], "note-1");
    let links = serde_json::json!([{
        "id": 1,
        "vaultId": "v1",
        "sourceNoteId": "note-1",
        "targetNoteId": "note-2",
        "linkText": "目标",
        "linkType": "wiki",
        "createdAt": 1
    }]);
    worker.send_response(
        &FrameResponse::success(links.clone())
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    let response = worker.recv_response();
    assert_command_value(&response, &links);
    worker.shutdown();
}

#[test]
fn get_note_backlinks_is_a_read_only_seam_call() {
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_get_note_backlinks",
        serde_json::json!({ "noteId": "note-1" }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.args["seam"], "wiki.store");
    assert_eq!(call.args["op"], "get_note_backlinks");
    assert_eq!(call.args["args"]["noteId"], "note-1");
    let backlinks = serde_json::json!([{
        "noteId": "note-9",
        "title": "来源笔记",
        "snippets": ["前 [[目标]] 后"]
    }]);
    worker.send_response(
        &FrameResponse::success(backlinks.clone())
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    let response = worker.recv_response();
    assert_command_value(&response, &backlinks);
    worker.shutdown();
}

#[test]
fn sync_note_links_is_a_write_seam_call_with_camel_case_input() {
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_sync_note_links",
        serde_json::json!({
            "vaultId": "v1",
            "sourceNoteId": "note-1",
            "links": [["note-2", "目标", "wiki"]],
        }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.op, ops::CALL_SEAM);
    assert_eq!(call.args["seam"], "wiki.store");
    assert_eq!(call.args["op"], "sync_note_links");
    // 接缝入参嵌在 `request` 下，字段 camelCase；链接三元组逐字透传（禁区 13）。
    assert_eq!(call.args["args"]["request"]["vaultId"], "v1");
    assert_eq!(call.args["args"]["request"]["sourceNoteId"], "note-1");
    assert_eq!(
        call.args["args"]["request"]["links"],
        serde_json::json!([["note-2", "目标", "wiki"]])
    );
    // 宿主返回 `()` ⇒ 序列化成 null，插件不包装额外结构。
    worker.send_response(
        &FrameResponse::success(serde_json::Value::Null)
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    let response = worker.recv_response();
    assert!(response.kind == ResponseKind::Success, "宿主空回执应透出成功：{response:?}");
    worker.shutdown();
}

#[test]
fn wiki_template_commands_map_to_the_host_wiki_store_seam() {
    // ① 列表是读 op：标量 wikiId，不额外包装。
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(PLUGIN_ID, "wiki_list_templates", serde_json::json!({ "wikiId": "w1" })));
    let call = worker.recv_request();
    assert_eq!(call.args["seam"], "wiki.store");
    assert_eq!(call.args["op"], "list_wiki_templates");
    assert_eq!(call.args["args"]["wikiId"], "w1");
    let templates = serde_json::json!([{
        "id": "tpl-1", "wikiId": "w1", "name": "模板", "description": null,
        "content": "# body", "pageType": null, "isBuiltin": false,
        "createdAt": 1, "updatedAt": 2,
    }]);
    worker.send_response(
        &FrameResponse::success(templates.clone())
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    assert_command_value(&worker.recv_response(), &templates);
    worker.shutdown();

    // ② 创建是写 op：整个 input 透传，字段保持 camelCase。
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_create_template",
        serde_json::json!({
            "input": {
                "wikiId": "w1", "name": "模板", "description": "说明",
                "content": "# body", "pageType": "note", "isBuiltin": false,
            }
        }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.args["op"], "create_wiki_template");
    assert_eq!(call.args["args"]["input"]["wikiId"], "w1");
    assert_eq!(call.args["args"]["input"]["pageType"], "note");
    worker.send_response(
        &FrameResponse::success(templates[0].clone())
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    assert_command_value(&worker.recv_response(), &templates[0]);
    worker.shutdown();

    // ③ 更新是写 op：id 留在 input 内，与宿主权威 DTO 同形。
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_update_template",
        serde_json::json!({ "input": { "id": "tpl-1", "name": "新名" } }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.args["op"], "update_wiki_template");
    assert_eq!(call.args["args"]["input"]["id"], "tpl-1");
    assert_eq!(call.args["args"]["input"]["name"], "新名");
    worker.send_response(
        &FrameResponse::success(templates[0].clone())
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    assert_command_value(&worker.recv_response(), &templates[0]);
    worker.shutdown();

    // ④ 删除是写 op：id 标量透传，宿主空回执透出成功。
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(PLUGIN_ID, "wiki_template_delete", serde_json::json!({ "id": "tpl-1" })));
    let call = worker.recv_request();
    assert_eq!(call.args["op"], "delete_wiki_template");
    assert_eq!(call.args["args"]["id"], "tpl-1");
    worker.send_response(
        &FrameResponse::success(serde_json::Value::Null)
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    assert_command_value(&worker.recv_response(), &serde_json::Value::Null);
    worker.shutdown();

    // ⑤ 缺 id 必须先在插件侧拒收，不触达宿主。
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(PLUGIN_ID, "wiki_template_delete", serde_json::json!({})));
    let response = worker.recv_response();
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_COMMAND_INVALID_ARGS));
    worker.shutdown();
}

#[test]
fn graph_cache_commands_map_to_read_only_host_seam_calls() {
    let graph = serde_json::json!({
        "nodes": [{ "id": "n1", "title": "节点", "type": "note" }],
        "edges": [],
        "danglingEdges": { "totalEdges": 0, "dangling": 0, "missingSourceOnly": 0, "missingTargetOnly": 0, "missingBoth": 0, "sampleEdgeIds": [] },
        "unresolvedTypes": [],
        "unresolvedRelations": [],
    });
    let communities = serde_json::json!({
        "communities": { "n1": 1 },
        "entityCommunities": null,
        "colorPalette": [],
    });

    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_get_graph_cached",
        serde_json::json!({ "wikiId": "w1" }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.args["seam"], "wiki.store");
    assert_eq!(call.args["op"], "get_cached_graph");
    assert_eq!(call.args["args"]["wikiId"], "w1");
    worker.send_response(
        &FrameResponse::success(graph.clone())
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    assert_command_value(&worker.recv_response(), &graph);
    worker.shutdown();

    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_get_graph_communities_cached",
        serde_json::json!({ "wikiId": "w1" }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.args["seam"], "wiki.store");
    assert_eq!(call.args["op"], "get_cached_communities");
    assert_eq!(call.args["args"]["wikiId"], "w1");
    worker.send_response(
        &FrameResponse::success(communities.clone())
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    assert_command_value(&worker.recv_response(), &communities);
    worker.shutdown();
}

#[test]
fn graph_cache_commands_reject_missing_wiki_id() {
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(PLUGIN_ID, "wiki_get_graph_cached", serde_json::json!({})));
    let response = worker.recv_response();
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_COMMAND_INVALID_ARGS));
    worker.shutdown();
}

#[test]
fn note_version_commands_map_to_host_wiki_store_seam() {
    let versions = serde_json::json!([{
        "id": 7,
        "wikiId": "w1",
        "noteId": "n1",
        "title": "旧标题",
        "content": "旧正文",
        "contentHash": "h",
        "author": "u",
        "createdAt": 123,
    }]);

    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(PLUGIN_ID, "wiki_note_versions", serde_json::json!({ "noteId": "n1" })));
    let call = worker.recv_request();
    assert_eq!(call.args["seam"], "wiki.store");
    assert_eq!(call.args["op"], "list_note_versions");
    assert_eq!(call.args["args"]["noteId"], "n1");
    worker.send_response(
        &FrameResponse::success(versions.clone())
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    assert_command_value(&worker.recv_response(), &versions);
    worker.shutdown();

    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_note_get_version",
        serde_json::json!({ "versionId": 7 }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.args["op"], "get_note_version");
    assert_eq!(call.args["args"]["versionId"], 7);
    worker.send_response(
        &FrameResponse::success(versions[0].clone())
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    assert_command_value(&worker.recv_response(), &versions[0]);
    worker.shutdown();

    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_note_restore_version",
        serde_json::json!({ "noteId": "n1", "versionId": 7 }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.args["op"], "restore_note_version");
    assert_eq!(call.args["args"]["noteId"], "n1");
    assert_eq!(call.args["args"]["versionId"], 7);
    worker.send_response(
        &FrameResponse::success(sample_note("n1"))
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    assert_command_value(&worker.recv_response(), &sample_note("n1"));
    worker.shutdown();
}

#[test]
fn note_version_commands_reject_invalid_args() {
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(PLUGIN_ID, "wiki_note_versions", serde_json::json!({})));
    assert_eq!(
        worker.recv_response().code.as_deref(),
        Some(error_codes::PLUGIN_COMMAND_INVALID_ARGS)
    );
    worker.shutdown();

    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_note_get_version",
        serde_json::json!({ "versionId": 0 }),
    ));
    assert_eq!(
        worker.recv_response().code.as_deref(),
        Some(error_codes::PLUGIN_COMMAND_INVALID_ARGS)
    );
    worker.shutdown();
}

#[test]
fn template_and_daily_note_commands_map_to_host_wiki_store_seam() {
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_note_create_from_template",
        serde_json::json!({ "vaultId": "v1", "templateId": "t1", "title": "自定义标题" }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.args["seam"], "wiki.store");
    assert_eq!(call.args["op"], "create_note_from_template");
    assert_eq!(call.args["args"]["vaultId"], "v1");
    assert_eq!(call.args["args"]["templateId"], "t1");
    assert_eq!(call.args["args"]["title"], "自定义标题");
    worker.send_response(
        &FrameResponse::success(sample_note("note-tpl"))
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    assert_command_value(&worker.recv_response(), &sample_note("note-tpl"));
    worker.shutdown();

    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_note_create_from_template",
        serde_json::json!({ "vaultId": "v1", "templateId": "t1", "title": null }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.args["op"], "create_note_from_template");
    assert!(call.args["args"]["title"].is_null());
    worker.send_response(
        &FrameResponse::success(sample_note("note-tpl-null"))
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    assert_command_value(&worker.recv_response(), &sample_note("note-tpl-null"));
    worker.shutdown();

    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_create_daily_note",
        serde_json::json!({ "vaultId": "v1" }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.args["op"], "create_daily_note");
    assert_eq!(call.args["args"]["vaultId"], "v1");
    worker.send_response(
        &FrameResponse::success(sample_note("note-daily"))
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    assert_command_value(&worker.recv_response(), &sample_note("note-daily"));
    worker.shutdown();
}

#[test]
fn template_and_daily_note_commands_reject_invalid_args() {
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(
        PLUGIN_ID,
        "wiki_note_create_from_template",
        serde_json::json!({ "vaultId": "v1", "templateId": "t1", "title": 7 }),
    ));
    assert_eq!(
        worker.recv_response().code.as_deref(),
        Some(error_codes::PLUGIN_COMMAND_INVALID_ARGS)
    );
    worker.shutdown();

    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(PLUGIN_ID, "wiki_create_daily_note", serde_json::json!({})));
    assert_eq!(
        worker.recv_response().code.as_deref(),
        Some(error_codes::PLUGIN_COMMAND_INVALID_ARGS)
    );
    worker.shutdown();
}

#[test]
fn unknown_commands_are_rejected() {
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&command(PLUGIN_ID, "wiki_not_a_command", serde_json::json!({})));
    let response = worker.recv_response();
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_COMMAND_UNKNOWN));
    worker.shutdown();
}
