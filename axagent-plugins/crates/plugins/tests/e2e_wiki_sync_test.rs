// SPDX-License-Identifier: AGPL-3.0-only

//! Wiki 业务功能包（P6.3 第一片）的**真 bundle 安装 / 命令执行 / 禁用**端到端。
//!
//! 证明三件事：
//! 1. 真 bundle 安装并启用后，`wiki_sync_note_to_knowledge_base` 经 worker → 宿主
//!    `wiki.store` 写接缝全链可执行（宿主 provider 由替身承担，只测跨层链路）；
//! 2. 禁用后命令链不可用（不是静默成功，也不是挂起）；
//! 3. 恢复启用后仍可执行。
//!
//! 门控：`AXAGENT_WIKI_BUNDLE_DIR`（`build-plugin-bundle.mjs --plugin wiki` 的产物目录）；
//! CI 由 nextest staging 前置真实构建，缺失即失败，不把未执行的测试当成通过。

mod common;

use std::sync::Arc;

use axagent_harness::{
    SyncNoteToKnowledgeBaseOutcome, SyncNoteToKnowledgeBaseRequest, WikiStoreProvider,
    get_capability_registry,
};
use axagent_plugins::{PluginManager, PluginManagerConfig};
use serde_json::{Value, json};

fn sample_note(id: String) -> axagent_harness::note_dtos::Note {
    axagent_harness::note_dtos::Note {
        id,
        vault_id: "v1".into(),
        title: "测试笔记".into(),
        file_path: "a.md".into(),
        content: "# hi".into(),
        content_hash: "h".into(),
        author: "u".into(),
        page_type: None,
        tags: Vec::new(),
        source_refs: None,
        related_pages: None,
        quality_score: None,
        last_linted_at: None,
        last_compiled_at: None,
        compiled_source_hash: None,
        user_edited: false,
        user_edited_at: None,
        created_at: 1,
        updated_at: 2,
        is_deleted: false,
    }
}

/// `wiki.store` 的宿主替身：只回执同步结果，不碰 DB / 文件系统。
struct FixedWikiStore;

#[async_trait::async_trait]
impl WikiStoreProvider for FixedWikiStore {
    async fn create_note(
        &self,
        input: axagent_harness::note_dtos::CreateNoteInput,
    ) -> axagent_harness::core_error::Result<axagent_harness::note_dtos::Note> {
        let mut note = sample_note("note-created".into());
        note.title = input.title;
        note.vault_id = input.vault_id;
        note.file_path = input.file_path;
        note.content = input.content;
        Ok(note)
    }

    async fn list_notes(
        &self,
        vault_id: String,
    ) -> axagent_harness::core_error::Result<Vec<axagent_harness::note_dtos::Note>> {
        Ok(vec![sample_note(format!("note-in-{vault_id}"))])
    }

    async fn get_note_by_path(
        &self,
        _vault_id: String,
        file_path: String,
    ) -> axagent_harness::core_error::Result<axagent_harness::note_dtos::Note> {
        Ok(sample_note(format!("note-at-{file_path}")))
    }

    async fn get_note(
        &self,
        id: String,
    ) -> axagent_harness::core_error::Result<axagent_harness::note_dtos::Note> {
        Ok(sample_note(id))
    }

    async fn sync_note_to_knowledge_base(
        &self,
        request: SyncNoteToKnowledgeBaseRequest,
    ) -> axagent_harness::core_error::Result<SyncNoteToKnowledgeBaseOutcome> {
        Ok(SyncNoteToKnowledgeBaseOutcome {
            document_id: format!("doc-for-{}", request.note_id),
            index_queued: true,
        })
    }
}

#[test]
fn wiki_bundle_command_runs_through_host_seam_and_respects_disable() {
    let Ok(dir) = std::env::var("AXAGENT_WIKI_BUNDLE_DIR") else {
        eprintln!(
            "SKIP: 未设置 AXAGENT_WIKI_BUNDLE_DIR，跳过 wiki 功能包端到端。\
             \n      跑法：node axagent-plugins/scripts/build-plugin-bundle.mjs --plugin wiki \
             后指向 axagent-plugins/dist/plugins/wiki。"
        );
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    assert!(dir.join("plugin.json").is_file(), "产物目录里没有 plugin.json：{}", dir.display());
    assert!(dir.join("bin").is_dir(), "产物目录里没有 bin/：{}", dir.display());

    let runtime = tokio::runtime::Runtime::new().expect("端到端测试需要 tokio 运行时");
    let _entered = runtime.enter();
    let registry = get_capability_registry();
    assert!(registry.get_wiki_store().is_none(), "测试进程起点：wiki.store 应为空");
    let handle = registry
        .register_wiki_store(Arc::new(FixedWikiStore))
        .expect("wiki 同步替身应能注册到 wiki.store");

    let home = common::TempDir::new("wiki-bundle-home");
    let mut manager = PluginManager::new(PluginManagerConfig::new(&home.path));
    let outcome =
        manager.install(dir.to_str().expect("产物路径应是 UTF-8")).expect("wiki bundle 应能安装");
    manager.enable(&outcome.plugin_id).expect("enable 应成功");

    // ① 启用态：命令经 worker → 宿主 wiki.store 全链可执行。
    let output = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_sync_note_to_knowledge_base",
            &json!({ "noteId": "note-1", "knowledgeBaseId": "kb-1" }),
        )
        .expect("启用后 wiki 同步命令应可执行");
    let parsed: Value = serde_json::from_str(&output).expect("命令输出应是 JSON 文本");
    assert_eq!(parsed["documentId"], "doc-for-note-1", "宿主回执应逐字透出：{parsed:?}");
    assert_eq!(parsed["indexQueued"], true);

    // ② 禁用后：命令链不可用。
    manager.disable(&outcome.plugin_id).expect("disable 应成功");
    let err = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_sync_note_to_knowledge_base",
            &json!({ "noteId": "note-1", "knowledgeBaseId": "kb-1" }),
        )
        .expect_err("禁用后同名命令不得再执行");
    assert!(err.to_string().contains("未启用"), "错误应说明禁用态：{err}");

    // ③ 恢复启用后：链路仍可用。
    manager.enable(&outcome.plugin_id).expect("重新 enable 应成功");
    let output = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_sync_note_to_knowledge_base",
            &json!({ "noteId": "note-2", "knowledgeBaseId": "kb-1" }),
        )
        .expect("恢复后 wiki 同步命令应可执行");
    let parsed: Value = serde_json::from_str(&output).expect("恢复输出应是 JSON 文本");
    assert_eq!(parsed["documentId"], "doc-for-note-2");

    // ④ 第二条只读命令：同一真 bundle 的 `wiki_get_note` 经 wiki.store::get_note。
    let note = manager
        .execute_plugin_command(&outcome.plugin_id, "wiki_get_note", &json!({ "id": "note-9" }))
        .expect("启用后 wiki 读取命令应可执行");
    let note: Value = serde_json::from_str(&note).expect("读取输出应是 JSON 文本");
    assert_eq!(note["id"], "note-9");
    assert_eq!(note["title"], "测试笔记");

    let listed = manager
        .execute_plugin_command(&outcome.plugin_id, "wiki_list_notes", &json!({ "vaultId": "v1" }))
        .expect("wiki 列表命令应可执行");
    let listed: Value = serde_json::from_str(&listed).expect("列表输出应是 JSON 文本");
    assert_eq!(listed[0]["id"], "note-in-v1");

    let by_path = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_get_note_by_path",
            &json!({ "vaultId": "v1", "filePath": "a.md" }),
        )
        .expect("wiki 按路径读取命令应可执行");
    let by_path: Value = serde_json::from_str(&by_path).expect("按路径输出应是 JSON 文本");
    assert_eq!(by_path["id"], "note-at-a.md");

    let created = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_create_note",
            &json!({
                "input": {
                    "vaultId": "v1",
                    "title": "新笔记",
                    "filePath": "new.md",
                    "content": "# body",
                    "author": "u"
                }
            }),
        )
        .expect("wiki 创建命令应可执行");
    let created: Value = serde_json::from_str(&created).expect("创建输出应是 JSON 文本");
    assert_eq!(created["id"], "note-created");
    assert_eq!(created["title"], "新笔记");

    drop(manager);
    handle.undo();
}
