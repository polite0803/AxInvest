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

/// `wiki.store` 的宿主替身：只回执同步结果，不碰 DB / 文件系统。
struct FixedWikiStore;

#[async_trait::async_trait]
impl WikiStoreProvider for FixedWikiStore {
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

    drop(manager);
    handle.undo();
}
