// SPDX-License-Identifier: AGPL-3.0-only

//! content_media 域包的**真 bundle 安装 / 禁用 / 恢复**端到端。
//!
//! 命令形状已由 crate 内 `protocol.rs` 逐条钉住；本文件锁的是安装级链路：
//! 启用时命令可执行、禁用后整条命令链不可用且域包投影消失、恢复后仍可执行。
//! 门控：`AXAGENT_CONTENT_BUNDLE_DIR`；CI 由 nextest staging 前置。

mod common;

use std::sync::Arc;

use axagent_harness::{OpcStoreProvider, get_capability_registry};
use axagent_plugins::{PluginManager, PluginManagerConfig};
use serde_json::{Value, json};

/// content_media e2e 的宿主 `opc.store` 数据替身（不测 DB）。
struct FixedContentStore;

#[async_trait::async_trait]
impl OpcStoreProvider for FixedContentStore {
    async fn list_blog_posts(
        &self,
        _limit: Option<usize>,
        _offset: Option<usize>,
    ) -> axagent_harness::core_error::Result<axagent_harness::BlogPostListOutcome> {
        Ok(axagent_harness::BlogPostListOutcome {
            posts: vec![axagent_harness::BlogPostView {
                id: "post-1".into(),
                title: "第一篇".into(),
                slug: "first-post".into(),
                excerpt: "摘要".into(),
                tags: vec!["opc".into()],
                published: false,
                published_at: None,
                view_count: 0,
                created_at: 1_700_000_000,
                updated_at: 1_700_000_000,
            }],
            total: 1,
        })
    }

    async fn create_blog_post(
        &self,
        input: axagent_harness::CreateBlogPostInput,
    ) -> axagent_harness::core_error::Result<axagent_harness::BlogPostView> {
        Ok(axagent_harness::BlogPostView {
            id: "post-new".into(),
            title: input.title,
            slug: input.slug,
            excerpt: input.excerpt,
            tags: input.tags,
            published: false,
            published_at: None,
            view_count: 0,
            created_at: 1_700_000_000,
            updated_at: 1_700_000_000,
        })
    }

    async fn list_content_assets(
        &self,
    ) -> axagent_harness::core_error::Result<Vec<axagent_harness::ContentAssetSummaryView>> {
        Ok(vec![axagent_harness::ContentAssetSummaryView {
            id: "asset-1".into(),
            title: "文章".into(),
            content_type: "article".into(),
            status: "draft".into(),
            tags: vec!["opc".into()],
            created_at: 1_700_000_000,
        }])
    }

    async fn create_content_asset(
        &self,
        input: axagent_harness::CreateContentAssetRequest,
    ) -> axagent_harness::core_error::Result<axagent_harness::ContentAssetSummaryView> {
        Ok(axagent_harness::ContentAssetSummaryView {
            id: "asset-new".into(),
            title: input.title,
            content_type: input.content_type,
            status: "draft".into(),
            tags: input.tags.unwrap_or_default(),
            created_at: 1_700_000_000,
        })
    }

    async fn list_publish_schedules(
        &self,
    ) -> axagent_harness::core_error::Result<Vec<axagent_harness::PublishScheduleSummaryView>> {
        Ok(vec![axagent_harness::PublishScheduleSummaryView {
            id: "sched-1".into(),
            content_ref_type: "blog_post".into(),
            content_ref_id: "post-1".into(),
            status: "pending".into(),
            scheduled_at: 1_700_400_000,
            published_at: None,
        }])
    }

    async fn create_publish_schedule(
        &self,
        input: axagent_harness::CreatePublishScheduleRequest,
    ) -> axagent_harness::core_error::Result<axagent_harness::PublishScheduleSummaryView> {
        Ok(axagent_harness::PublishScheduleSummaryView {
            id: "sched-new".into(),
            content_ref_type: input.content_ref_type,
            content_ref_id: input.content_ref_id,
            status: "pending".into(),
            scheduled_at: input.scheduled_at,
            published_at: None,
        })
    }

    async fn delete_content_asset(
        &self,
        _id: String,
    ) -> axagent_harness::core_error::Result<axagent_harness::RemoveContentAssetOutcome> {
        Ok(axagent_harness::RemoveContentAssetOutcome::Removed)
    }

    async fn cancel_publish_schedule(
        &self,
        _id: String,
    ) -> axagent_harness::core_error::Result<axagent_harness::RemovePublishScheduleOutcome> {
        Ok(axagent_harness::RemovePublishScheduleOutcome::Removed)
    }

    async fn process_due_schedules(
        &self,
    ) -> axagent_harness::core_error::Result<Vec<axagent_harness::PublishScheduleSummaryView>> {
        Ok(vec![])
    }
}

#[test]
fn content_media_bundle_disable_and_restore_is_scoped_to_its_plugin() {
    let Ok(dir) = std::env::var("AXAGENT_CONTENT_BUNDLE_DIR") else {
        eprintln!(
            "SKIP: 未设置 AXAGENT_CONTENT_BUNDLE_DIR，跳过 content_media 禁用端到端。\
             \n      跑法：node axagent-plugins/scripts/build-plugin-bundle.mjs --plugin opc-content-media \
             后指向 axagent-plugins/dist/plugins/opc-content-media。"
        );
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    assert!(dir.join("plugin.json").is_file());
    assert!(dir.join("bin").is_dir());
    assert!(dir.join("domain_packs/content_media.json").is_file());

    let runtime = tokio::runtime::Runtime::new().expect("端到端测试需要 tokio 运行时");
    let _entered = runtime.enter();
    let registry = get_capability_registry();
    assert!(registry.get_opc_store().is_none(), "测试进程起点：opc.store 应为空");
    let handle = registry
        .register_opc_store(Arc::new(FixedContentStore))
        .expect("content_media 数据替身应能注册到 opc.store");

    let home = common::TempDir::new("content-bundle-home");
    let mut manager = PluginManager::new(PluginManagerConfig::new(&home.path));
    let outcome = manager
        .install_test_fixture(dir.to_str().expect("产物路径应是 UTF-8"))
        .expect("content_media bundle 应能安装");
    manager.enable(&outcome.plugin_id).expect("enable 应成功");

    // ① 关键命令链可用：blog 列表（嵌层回执）、blog 创建、两个只读列表、process_due。
    let posts = manager
        .execute_plugin_command(&outcome.plugin_id, "opc_list_blog_posts", &json!({}))
        .expect("启用后博客列表应可执行");
    let parsed: Value = serde_json::from_str(&posts).expect("博客输出应是 JSON 文本");
    assert_eq!(parsed[0]["slug"], "first-post", "opc.store 视图应逐字透出：{parsed:?}");
    let created = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "opc_create_blog_post",
            &json!({
                "title": "新文章",
                "slug": "new-post",
                "excerpt": "摘要",
                "content": "正文",
                "tags": ["opc"]
            }),
        )
        .expect("启用后创建博客应可执行");
    let parsed_created: Value = serde_json::from_str(&created).expect("创建输出应是 JSON 文本");
    assert_eq!(parsed_created["slug"], "new-post");
    let assets = manager
        .execute_plugin_command(&outcome.plugin_id, "opc_list_content_assets", &json!({}))
        .expect("启用后内容资产列表应可执行");
    let parsed_assets: Value = serde_json::from_str(&assets).expect("资产输出应是 JSON 文本");
    assert_eq!(parsed_assets[0]["contentType"], "article");
    let created_asset = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "opc_create_content_asset",
            &json!({
                "title": "新资产",
                "contentType": "article",
                "body": "正文",
                "tags": ["opc"]
            }),
        )
        .expect("启用后创建内容资产应可执行");
    let parsed_created_asset: Value =
        serde_json::from_str(&created_asset).expect("创建资产输出应是 JSON 文本");
    assert_eq!(parsed_created_asset["contentType"], "article");
    let removed_asset = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "opc_delete_content_asset",
            &json!({ "id": "asset-new" }),
        )
        .expect("启用后删除内容资产应可执行");
    assert_eq!(removed_asset, "removed", "删除结果应按 payload 透传");
    let schedules = manager
        .execute_plugin_command(&outcome.plugin_id, "opc_list_publish_schedules", &json!({}))
        .expect("启用后发布计划列表应可执行");
    let parsed_schedules: Value = serde_json::from_str(&schedules).expect("计划输出应是 JSON 文本");
    assert_eq!(parsed_schedules[0]["contentRefId"], "post-1");
    let created_schedule = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "opc_create_publish_schedule",
            &json!({
                "contentRefType": "content_asset",
                "contentRefId": "asset-new",
                "scheduledAt": 1_700_400_000
            }),
        )
        .expect("启用后创建发布计划应可执行");
    let parsed_created_schedule: Value =
        serde_json::from_str(&created_schedule).expect("创建计划输出应是 JSON 文本");
    assert_eq!(parsed_created_schedule["contentRefId"], "asset-new");
    let cancelled = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "opc_cancel_publish_schedule",
            &json!({ "id": "sched-new" }),
        )
        .expect("启用后取消发布计划应可执行");
    assert_eq!(cancelled, "removed", "取消结果应按 payload 透传");
    let due = manager
        .execute_plugin_command(&outcome.plugin_id, "opc_process_due_schedules", &json!({}))
        .expect("启用后处理到期计划应可执行");
    let parsed_due: Value = serde_json::from_str(&due).expect("处理到期输出应是 JSON 文本");
    assert_eq!(parsed_due.as_array().map(Vec::len), Some(0));

    // ② 禁用后：整条命令链不可用；域包投影消失。
    manager.disable(&outcome.plugin_id).expect("disable 应成功");
    let err = manager
        .execute_plugin_command(&outcome.plugin_id, "opc_list_blog_posts", &json!({}))
        .expect_err("禁用后同名命令不得再执行");
    assert!(err.to_string().contains("未启用"), "错误应说明禁用态：{err}");
    let packs = manager.list_plugin_domain_packs().expect("域包投影应可读");
    assert!(
        !packs.iter().any(|p| p.id == outcome.plugin_id),
        "禁用后 content_media 插件的域包投影不得出现：{packs:?}"
    );

    // ③ 恢复启用后：链路仍可用，页面资产可解析。
    manager.enable(&outcome.plugin_id).expect("重新 enable 应成功");
    let posts = manager
        .execute_plugin_command(&outcome.plugin_id, "opc_list_blog_posts", &json!({}))
        .expect("恢复后博客列表应可执行");
    let parsed: Value = serde_json::from_str(&posts).expect("恢复输出应是 JSON 文本");
    assert_eq!(parsed[0]["slug"], "first-post");
    let located = manager
        .resolve_page_asset(&outcome.plugin_id, "opc-content-media")
        .expect("恢复后页面资产应可解析");
    let page_path = std::path::PathBuf::from(&located.absolute_path);
    assert!(page_path.is_file(), "页面资产不存在：{}", page_path.display());
    let page_entry = std::fs::read_to_string(&page_path).expect("页面入口应可读");
    assert!(
        page_entry.contains("./bundle.js"),
        "content_media 页面必须加载真实 bundle.js，不得退回说明页：{}",
        page_path.display()
    );

    drop(manager);
    handle.undo();
}
