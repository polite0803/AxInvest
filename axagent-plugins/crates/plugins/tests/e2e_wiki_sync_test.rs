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
    SyncNoteLinksRequest, SyncNoteToKnowledgeBaseOutcome, SyncNoteToKnowledgeBaseRequest,
    WikiStoreProvider, get_capability_registry,
    wiki_dtos::{CreateWikiTemplateInput, NoteVersion, UpdateWikiTemplateInput, WikiTemplate},
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

    async fn search_notes(
        &self,
        vault_id: String,
        query: String,
        _top_k: Option<usize>,
    ) -> axagent_harness::core_error::Result<Vec<axagent_harness::rag_config::NoteSearchResult>>
    {
        let mut note = sample_note("note-search".into());
        note.vault_id = vault_id;
        note.title = query;
        Ok(vec![axagent_harness::rag_config::NoteSearchResult {
            note,
            snippet: "…搜索…".into(),
            score: 0.9,
        }])
    }

    async fn import_obsidian_vault(
        &self,
        _wiki_id: String,
        _vault_path: String,
    ) -> axagent_harness::core_error::Result<axagent_harness::wiki_dtos::ImportStats> {
        Ok(axagent_harness::wiki_dtos::ImportStats { imported: 2, failed: 0, skipped: 1 })
    }

    async fn import_knowledge_md(
        &self,
        _wiki_id: String,
        _file_path: Option<String>,
    ) -> axagent_harness::core_error::Result<axagent_harness::wiki_dtos::KnowledgeMdImportStats>
    {
        Ok(axagent_harness::wiki_dtos::KnowledgeMdImportStats {
            imported: 1,
            failed: 0,
            skipped: 0,
            total: 4,
        })
    }

    async fn export_markdown(
        &self,
        _wiki_id: String,
        _output_path: String,
    ) -> axagent_harness::core_error::Result<axagent_harness::wiki_dtos::ExportStats> {
        Ok(axagent_harness::wiki_dtos::ExportStats { exported: 3, failed: 0 })
    }

    async fn export_html(
        &self,
        _wiki_id: String,
        _output_path: String,
    ) -> axagent_harness::core_error::Result<axagent_harness::wiki_dtos::ExportStats> {
        Ok(axagent_harness::wiki_dtos::ExportStats { exported: 2, failed: 0 })
    }

    async fn export_note_html(
        &self,
        _note_id: String,
        output_path: String,
    ) -> axagent_harness::core_error::Result<axagent_harness::wiki_dtos::NoteExportPath> {
        Ok(axagent_harness::wiki_dtos::NoteExportPath {
            path: output_path.replace(".pdf", ".html"),
        })
    }

    async fn get_note_links(
        &self,
        _note_id: String,
    ) -> axagent_harness::core_error::Result<Vec<axagent_harness::types::NoteLink>> {
        Ok(Vec::new())
    }

    async fn get_note_backlinks(
        &self,
        note_id: String,
    ) -> axagent_harness::core_error::Result<Vec<axagent_harness::types::BacklinkInfo>> {
        Ok(vec![axagent_harness::types::BacklinkInfo {
            note_id: note_id.clone(),
            title: "来源笔记".into(),
            snippets: vec![format!("指向 {note_id}")],
        }])
    }

    async fn get_note(
        &self,
        id: String,
    ) -> axagent_harness::core_error::Result<axagent_harness::note_dtos::Note> {
        Ok(sample_note(id))
    }

    async fn update_note(
        &self,
        id: String,
        input: axagent_harness::note_dtos::UpdateNoteInput,
    ) -> axagent_harness::core_error::Result<axagent_harness::note_dtos::Note> {
        let mut note = sample_note(id);
        if let Some(title) = input.title {
            note.title = title;
        }
        if let Some(content) = input.content {
            note.content = content;
        }
        Ok(note)
    }

    async fn delete_note(&self, _id: String) -> axagent_harness::core_error::Result<()> {
        Ok(())
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

    async fn sync_note_links(
        &self,
        _request: SyncNoteLinksRequest,
    ) -> axagent_harness::core_error::Result<()> {
        Ok(())
    }

    async fn list_wiki_templates(
        &self,
        wiki_id: String,
    ) -> axagent_harness::core_error::Result<Vec<WikiTemplate>> {
        Ok(vec![WikiTemplate {
            id: "tpl-1".into(),
            wiki_id,
            name: "模板".into(),
            description: None,
            content: "# body".into(),
            page_type: None,
            is_builtin: false,
            created_at: 1,
            updated_at: 2,
        }])
    }

    async fn create_wiki_template(
        &self,
        input: CreateWikiTemplateInput,
    ) -> axagent_harness::core_error::Result<WikiTemplate> {
        Ok(WikiTemplate {
            id: "tpl-created".into(),
            wiki_id: input.wiki_id,
            name: input.name,
            description: input.description,
            content: input.content,
            page_type: input.page_type,
            is_builtin: input.is_builtin,
            created_at: 1,
            updated_at: 2,
        })
    }

    async fn update_wiki_template(
        &self,
        input: UpdateWikiTemplateInput,
    ) -> axagent_harness::core_error::Result<WikiTemplate> {
        Ok(WikiTemplate {
            id: input.id,
            wiki_id: "w1".into(),
            name: input.name.unwrap_or_else(|| "模板".into()),
            description: input.description,
            content: input.content.unwrap_or_else(|| "# body".into()),
            page_type: input.page_type,
            is_builtin: false,
            created_at: 1,
            updated_at: 3,
        })
    }

    async fn delete_wiki_template(&self, _id: String) -> axagent_harness::core_error::Result<()> {
        Ok(())
    }

    async fn get_cached_graph(
        &self,
        wiki_id: String,
    ) -> axagent_harness::core_error::Result<axagent_harness::graph_dtos::GraphData> {
        Ok(axagent_harness::graph_dtos::GraphData::new(
            vec![axagent_harness::graph_dtos::GraphNode {
                id: format!("note-{wiki_id}"),
                title: "图谱节点".to_string(),
                node_type: "note".to_string(),
                tags: Vec::new(),
                link_count: 0,
                backlink_count: 0,
                path: String::new(),
            }],
            Vec::new(),
        ))
    }

    async fn get_cached_communities(
        &self,
        _wiki_id: String,
    ) -> axagent_harness::core_error::Result<axagent_harness::louvain_dtos::LouvainResult> {
        Ok(axagent_harness::louvain_dtos::LouvainResult {
            communities: std::collections::HashMap::from([("note-w1".to_string(), 1)]),
            cohesion_scores: std::collections::HashMap::new(),
            community_sizes: std::collections::HashMap::new(),
            top_nodes: std::collections::HashMap::new(),
            modularity: 0.0,
            num_communities: 1,
            entity_communities: None,
            color_palette: axagent_harness::louvain_dtos::LouvainResult::default_palette(),
        })
    }

    async fn list_note_versions(
        &self,
        note_id: String,
    ) -> axagent_harness::core_error::Result<Vec<NoteVersion>> {
        Ok(vec![NoteVersion {
            id: 7,
            wiki_id: "w1".into(),
            note_id,
            title: "旧标题".into(),
            content: "旧正文".into(),
            content_hash: "h".into(),
            author: "u".into(),
            created_at: 123,
        }])
    }

    async fn get_note_version(
        &self,
        version_id: i64,
    ) -> axagent_harness::core_error::Result<NoteVersion> {
        Ok(NoteVersion {
            id: version_id,
            wiki_id: "w1".into(),
            note_id: "n1".into(),
            title: "旧标题".into(),
            content: "旧正文".into(),
            content_hash: "h".into(),
            author: "u".into(),
            created_at: 123,
        })
    }

    async fn restore_note_version(
        &self,
        note_id: String,
        _version_id: i64,
    ) -> axagent_harness::core_error::Result<axagent_harness::note_dtos::Note> {
        Ok(sample_note(note_id))
    }

    async fn create_note_from_template(
        &self,
        vault_id: String,
        template_id: String,
        title: Option<String>,
    ) -> axagent_harness::core_error::Result<axagent_harness::note_dtos::Note> {
        let mut note = sample_note(format!("note-from-{template_id}"));
        note.vault_id = vault_id;
        note.title = title.unwrap_or_else(|| "模板笔记".into());
        Ok(note)
    }

    async fn create_daily_note(
        &self,
        vault_id: String,
    ) -> axagent_harness::core_error::Result<axagent_harness::note_dtos::Note> {
        let mut note = sample_note("note-daily".into());
        note.vault_id = vault_id;
        note.title = "今日笔记".into();
        Ok(note)
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
    let outcome = manager
        .install_test_fixture(dir.to_str().expect("产物路径应是 UTF-8"))
        .expect("wiki bundle 应能安装");
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

    let searched = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_notes_search",
            &json!({ "vaultId": "v1", "query": "测试", "topK": 5 }),
        )
        .expect("wiki 搜索命令应可执行");
    let searched: Value = serde_json::from_str(&searched).expect("搜索输出应是 JSON 文本");
    assert_eq!(searched[0]["note"]["id"], "note-search");
    assert_eq!(searched[0]["note"]["title"], "测试");
    assert_eq!(searched[0]["snippet"], "…搜索…");
    assert_eq!(searched[0]["score"], 0.9);

    let imported = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_import_obsidian_vault",
            &json!({ "wikiId": "w1", "vaultPath": "/tmp/vault" }),
        )
        .expect("wiki 导入命令应可执行");
    let imported: Value = serde_json::from_str(&imported).expect("导入输出应是 JSON 文本");
    assert_eq!(imported, json!({ "imported": 2, "failed": 0, "skipped": 1 }));

    let imported_md = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_import_knowledge_md",
            &json!({ "wikiId": "w1" }),
        )
        .expect("wiki KNOWLEDGE.md 导入命令应可执行");
    let imported_md: Value =
        serde_json::from_str(&imported_md).expect("KNOWLEDGE.md 导入输出应是 JSON 文本");
    assert_eq!(imported_md, json!({ "imported": 1, "failed": 0, "skipped": 0, "total": 4 }));

    let exported_md = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_export_markdown",
            &json!({ "wikiId": "w1", "outputPath": "/tmp/out" }),
        )
        .expect("wiki Markdown 导出命令应可执行");
    let exported_md: Value = serde_json::from_str(&exported_md).expect("导出输出应是 JSON 文本");
    assert_eq!(exported_md, json!({ "exported": 3, "failed": 0 }));

    let exported_html = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_export_html",
            &json!({ "wikiId": "w1", "outputPath": "/tmp/out" }),
        )
        .expect("wiki HTML 导出命令应可执行");
    let exported_html: Value =
        serde_json::from_str(&exported_html).expect("导出输出应是 JSON 文本");
    assert_eq!(exported_html, json!({ "exported": 2, "failed": 0 }));

    let exported_note = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_note_export_html",
            &json!({ "noteId": "n1", "outputPath": "/tmp/out/n1.pdf" }),
        )
        .expect("wiki 笔记导出命令应可执行");
    let exported_note: Value =
        serde_json::from_str(&exported_note).expect("导出输出应是 JSON 文本");
    assert_eq!(exported_note, json!({ "path": "/tmp/out/n1.html" }));

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

    let links = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_get_note_links",
            &json!({ "noteId": "note-created" }),
        )
        .expect("wiki 出链命令应可执行");
    let links: Value = serde_json::from_str(&links).expect("出链输出应是 JSON 文本");
    assert_eq!(links, json!([]));
    assert_eq!(created["title"], "新笔记");

    let backlinks = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_get_note_backlinks",
            &json!({ "noteId": "note-created" }),
        )
        .expect("wiki 反链命令应可执行");
    let backlinks: Value = serde_json::from_str(&backlinks).expect("反链输出应是 JSON 文本");
    assert_eq!(
        backlinks,
        json!([{ "noteId": "note-created", "title": "来源笔记", "snippets": ["指向 note-created"] }])
    );

    let synced = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_sync_note_links",
            &json!({
                "vaultId": "v1",
                "sourceNoteId": "note-created",
                "links": [["note-9", "目标", "wiki"]],
            }),
        )
        .expect("wiki 链接同步命令应可执行");
    let synced: Value = serde_json::from_str(&synced).expect("链接同步输出应是 JSON 文本");
    assert!(
        synced.is_null() || synced.as_object().is_some_and(|o| o.is_empty()),
        "链接同步回执应为 null 或空对象：{synced:?}"
    );

    let updated = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_update_note",
            &json!({ "id": "note-created", "input": { "title": "改过的标题" } }),
        )
        .expect("wiki 更新命令应可执行");
    let updated: Value = serde_json::from_str(&updated).expect("更新输出应是 JSON 文本");
    assert_eq!(updated["title"], "改过的标题");

    let deleted = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_delete_note",
            &json!({ "id": "note-created" }),
        )
        .expect("wiki 删除命令应可执行");
    let deleted: Value = serde_json::from_str(&deleted).expect("删除输出应是 JSON 文本");
    // 单位回执：宿主返回 `()` ⇒ 序列化后是 `null`（插件侧不包装任何结构）。
    assert!(
        deleted.is_null() || deleted.as_object().is_some_and(|o| o.is_empty()),
        "删除回执应为 null 或空对象：{deleted:?}"
    );

    let listed_templates = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_list_templates",
            &json!({ "wikiId": "w1" }),
        )
        .expect("wiki 模板列表命令应可执行");
    let listed_templates: Value =
        serde_json::from_str(&listed_templates).expect("模板列表输出应是 JSON 文本");
    assert_eq!(listed_templates[0]["id"], "tpl-1");

    let created_template = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_create_template",
            &json!({
                "input": {
                    "wikiId": "w1", "name": "新模板", "description": null,
                    "content": "# body", "pageType": null, "isBuiltin": false,
                }
            }),
        )
        .expect("wiki 模板创建命令应可执行");
    let created_template: Value =
        serde_json::from_str(&created_template).expect("模板创建输出应是 JSON 文本");
    assert_eq!(created_template["id"], "tpl-created");
    assert_eq!(created_template["name"], "新模板");

    let updated_template = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_update_template",
            &json!({ "input": { "id": "tpl-1", "name": "改过的模板" } }),
        )
        .expect("wiki 模板更新命令应可执行");
    let updated_template: Value =
        serde_json::from_str(&updated_template).expect("模板更新输出应是 JSON 文本");
    assert_eq!(updated_template["id"], "tpl-1");
    assert_eq!(updated_template["name"], "改过的模板");

    let deleted_template = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_template_delete",
            &json!({ "id": "tpl-1" }),
        )
        .expect("wiki 模板删除命令应可执行");
    let deleted_template: Value =
        serde_json::from_str(&deleted_template).expect("模板删除输出应是 JSON 文本");
    assert!(
        deleted_template.is_null() || deleted_template.as_object().is_some_and(|o| o.is_empty()),
        "模板删除回执应为 null 或空对象：{deleted_template:?}"
    );

    let graph = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_get_graph_cached",
            &json!({ "wikiId": "w1" }),
        )
        .expect("wiki 图谱读命令应可执行");
    let graph: Value = serde_json::from_str(&graph).expect("图谱输出应是 JSON 文本");
    assert_eq!(graph["nodes"][0]["id"], "note-w1");

    let communities = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_get_graph_communities_cached",
            &json!({ "wikiId": "w1" }),
        )
        .expect("wiki 社区读命令应可执行");
    let communities: Value = serde_json::from_str(&communities).expect("社区输出应是 JSON 文本");
    assert_eq!(communities["communities"]["note-w1"], 1);

    let versions = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_note_versions",
            &json!({ "noteId": "n1" }),
        )
        .expect("wiki 版本列表命令应可执行");
    let versions: Value = serde_json::from_str(&versions).expect("版本列表输出应是 JSON 文本");
    assert_eq!(versions[0]["id"], 7);

    let version = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_note_get_version",
            &json!({ "versionId": 7 }),
        )
        .expect("wiki 指定版本命令应可执行");
    let version: Value = serde_json::from_str(&version).expect("版本输出应是 JSON 文本");
    assert_eq!(version["id"], 7);

    let restored = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_note_restore_version",
            &json!({ "noteId": "n1", "versionId": 7 }),
        )
        .expect("wiki 版本恢复命令应可执行");
    let restored: Value = serde_json::from_str(&restored).expect("恢复结果应是 JSON 文本");
    assert_eq!(restored["id"], "n1");

    let from_template = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_note_create_from_template",
            &json!({ "vaultId": "v1", "templateId": "t1", "title": "自定义标题" }),
        )
        .expect("wiki 从模板创建笔记命令应可执行");
    let from_template: Value =
        serde_json::from_str(&from_template).expect("模板创建结果应是 JSON 文本");
    assert_eq!(from_template["id"], "note-from-t1");
    assert_eq!(from_template["title"], "自定义标题");

    let daily = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "wiki_create_daily_note",
            &json!({ "vaultId": "v1" }),
        )
        .expect("wiki 每日笔记命令应可执行");
    let daily: Value = serde_json::from_str(&daily).expect("每日笔记结果应是 JSON 文本");
    assert_eq!(daily["id"], "note-daily");

    drop(manager);
    handle.undo();
}
