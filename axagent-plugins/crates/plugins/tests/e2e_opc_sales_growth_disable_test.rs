// SPDX-License-Identifier: AGPL-3.0-only

//! sales_growth 域包的**真 bundle 安装 / 禁用 / 恢复**端到端。
//!
//! 判据与 accounting 同款：三条命令在启用时全链可用，禁用后整条命令链不可用且域包投影
//! 消失，恢复后仍可执行。门控：`AXAGENT_SALES_BUNDLE_DIR`；CI 由 nextest staging 前置。

mod common;

use std::sync::Arc;

use axagent_harness::{OpcStoreProvider, get_capability_registry};
use axagent_plugins::{PluginManager, PluginManagerConfig};
use serde_json::{Value, json};

/// sales_growth 三条命令的宿主 `opc.store` 数据替身（不测 DB）。
struct FixedSalesStore;

#[async_trait::async_trait]
impl OpcStoreProvider for FixedSalesStore {
    async fn create_customer(
        &self,
        _input: axagent_harness::CreateCustomerRequest,
    ) -> axagent_harness::core_error::Result<axagent_harness::CustomerSummaryView> {
        Ok(axagent_harness::CustomerSummaryView {
            id: "cust-new".into(),
            name: "张三".into(),
            email: "z@example.com".into(),
            phone: None,
            company: None,
            customer_type: "business".into(),
            country: None,
            region: None,
            city: None,
            address: None,
            latitude: None,
            longitude: None,
            source: None,
            notes: String::new(),
            status: "lead".into(),
            total_revenue: 0.0,
            invoice_count: 0,
            created_at: 1_700_000_000,
        })
    }

    async fn list_landing_pages(
        &self,
    ) -> axagent_harness::core_error::Result<Vec<axagent_harness::LandingPageSummaryView>> {
        Ok(vec![axagent_harness::LandingPageSummaryView {
            id: "lp-1".into(),
            title: "落地页".into(),
            slug: "lp-slug".into(),
            published: false,
            created_at: 1_700_000_000,
        }])
    }

    async fn list_contacts(
        &self,
    ) -> axagent_harness::core_error::Result<Vec<axagent_harness::ContactSummaryView>> {
        Ok(vec![axagent_harness::ContactSummaryView {
            id: "contact-1".into(),
            name: "李四".into(),
            email: "l@example.com".into(),
            source: "form".into(),
            read: false,
            created_at: 1_700_000_000,
        }])
    }
}

#[test]
fn sales_growth_bundle_disable_and_restore_is_scoped_to_its_plugin() {
    let Ok(dir) = std::env::var("AXAGENT_SALES_BUNDLE_DIR") else {
        eprintln!(
            "SKIP: 未设置 AXAGENT_SALES_BUNDLE_DIR，跳过 sales_growth 禁用端到端。\
             \n      跑法：node axagent-plugins/scripts/build-plugin-bundle.mjs --plugin opc-sales-growth \
             后指向 axagent-plugins/dist/plugins/opc-sales-growth。"
        );
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    assert!(dir.join("plugin.json").is_file());
    assert!(dir.join("bin").is_dir());
    assert!(dir.join("domain_packs/sales_growth.json").is_file());

    let runtime = tokio::runtime::Runtime::new().expect("端到端测试需要 tokio 运行时");
    let _entered = runtime.enter();
    let registry = get_capability_registry();
    assert!(registry.get_opc_store().is_none(), "测试进程起点：opc.store 应为空");
    let handle = registry
        .register_opc_store(Arc::new(FixedSalesStore))
        .expect("sales_growth 数据替身应能注册到 opc.store");

    let home = common::TempDir::new("sales-bundle-home");
    let mut manager = PluginManager::new(PluginManagerConfig::new(&home.path));
    let outcome = manager
        .install_test_fixture(dir.to_str().expect("产物路径应是 UTF-8"))
        .expect("sales_growth bundle 应能安装");
    manager.enable(&outcome.plugin_id).expect("enable 应成功");

    // ① 三条命令全链可用。
    let customer = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "opc_create_customer",
            &json!({ "name": "张三", "email": "z@example.com" }),
        )
        .expect("启用后创建客户应可执行");
    let parsed: Value = serde_json::from_str(&customer).expect("创建输出应是 JSON 文本");
    assert_eq!(parsed["invoiceNumber"].as_str(), None, "客户不是发票：{parsed:?}");
    assert_eq!(parsed["name"], "张三", "opc.store 视图应逐字透出：{parsed:?}");
    let pages = manager
        .execute_plugin_command(&outcome.plugin_id, "opc_list_landing_pages", &json!({}))
        .expect("启用后落地页列表应可执行");
    let parsed_pages: Value = serde_json::from_str(&pages).expect("落地页输出应是 JSON 文本");
    assert_eq!(parsed_pages[0]["slug"], "lp-slug");
    let contacts = manager
        .execute_plugin_command(&outcome.plugin_id, "opc_list_contacts", &json!({}))
        .expect("启用后联系人列表应可执行");
    let parsed_contacts: Value = serde_json::from_str(&contacts).expect("联系人输出应是 JSON 文本");
    assert_eq!(parsed_contacts[0]["email"], "l@example.com");

    // ② 禁用后：整条命令链不可用；域包投影消失。
    manager.disable(&outcome.plugin_id).expect("disable 应成功");
    let err = manager
        .execute_plugin_command(&outcome.plugin_id, "opc_list_contacts", &json!({}))
        .expect_err("禁用后同名命令不得再执行");
    assert!(err.to_string().contains("未启用"), "错误应说明禁用态：{err}");
    let packs = manager.list_plugin_domain_packs().expect("域包投影应可读");
    assert!(
        !packs.iter().any(|p| p.id == outcome.plugin_id),
        "禁用后 sales_growth 插件的域包投影不得出现：{packs:?}"
    );

    // ③ 恢复启用后：链路仍可用，页面资产可解析。
    manager.enable(&outcome.plugin_id).expect("重新 enable 应成功");
    let pages = manager
        .execute_plugin_command(&outcome.plugin_id, "opc_list_landing_pages", &json!({}))
        .expect("恢复后落地页列表应可执行");
    let parsed_pages: Value = serde_json::from_str(&pages).expect("恢复输出应是 JSON 文本");
    assert_eq!(parsed_pages[0]["slug"], "lp-slug");
    let located = manager
        .resolve_page_asset(&outcome.plugin_id, "opc-sales-growth")
        .expect("恢复后页面资产应可解析");
    let page_path = std::path::PathBuf::from(&located.absolute_path);
    assert!(page_path.is_file(), "页面资产不存在：{}", page_path.display());
    let page_entry = std::fs::read_to_string(&page_path).expect("页面入口应可读");
    assert!(
        page_entry.contains("./bundle.js"),
        "sales_growth 页面必须加载真实 bundle.js，不得退回说明页：{}",
        page_path.display()
    );

    drop(manager);
    handle.undo();
}
