// SPDX-License-Identifier: AGPL-3.0-only

//! accounting 域包的**真 bundle 安装 / 禁用 / 恢复**端到端。
//!
//! 与单测的分工：`manager::domain_pack_tests` 只证明投影位消失；本测试走真打包产物，
//! 证明「启用时三条命令经 worker 通道可执行、禁用后整条命令链不可用、恢复后仍可执行」。
//! 门控：`AXAGENT_ACCOUNTING_BUNDLE_DIR`（`build-plugin-bundle.mjs --plugin opc-accounting`
//! 的产物目录），未设置时打印 SKIP，不代表通过。

mod common;

use std::sync::Arc;

use axagent_harness::{OpcStoreProvider, get_capability_registry};
use axagent_plugins::{PluginManager, PluginManagerConfig};
use serde_json::{Value, json};

/// accounting 三条命令的宿主 `opc.store` 数据替身（仅跨层链路，不测 DB）。
struct FixedAccountingStore;

#[async_trait::async_trait]
impl OpcStoreProvider for FixedAccountingStore {
    async fn list_invoices(
        &self,
        _query: axagent_harness::InvoiceListQuery,
    ) -> axagent_harness::core_error::Result<Vec<axagent_harness::InvoiceSummaryView>> {
        Ok(vec![axagent_harness::InvoiceSummaryView {
            id: "inv-1".into(),
            customer_id: "cust-1".into(),
            invoice_number: "INV-001".into(),
            status: "draft".into(),
            total: 100.0,
            currency: "CNY".into(),
            due_at: None,
            created_at: 1_700_000_000,
        }])
    }

    async fn create_invoice(
        &self,
        _input: axagent_harness::CreateInvoiceRequest,
    ) -> axagent_harness::core_error::Result<axagent_harness::InvoiceSummaryView> {
        Ok(axagent_harness::InvoiceSummaryView {
            id: "inv-new".into(),
            customer_id: "cust-1".into(),
            invoice_number: "INV-002".into(),
            status: "draft".into(),
            total: 120.0,
            currency: "CNY".into(),
            due_at: None,
            created_at: 1_700_000_000,
        })
    }

    async fn transition_invoice(
        &self,
        _id: String,
        _target: String,
    ) -> axagent_harness::core_error::Result<axagent_harness::TransitionInvoiceOutcome> {
        Ok(axagent_harness::TransitionInvoiceOutcome::NotFound)
    }
}

#[test]
fn accounting_bundle_disable_and_restore_is_scoped_to_its_plugin() {
    let Ok(dir) = std::env::var("AXAGENT_ACCOUNTING_BUNDLE_DIR") else {
        eprintln!(
            "SKIP: 未设置 AXAGENT_ACCOUNTING_BUNDLE_DIR，跳过 accounting 禁用端到端。\
             \n      跑法：node axagent-plugins/scripts/build-plugin-bundle.mjs --plugin opc-accounting \
             后指向 axagent-plugins/dist/plugins/opc-accounting。"
        );
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    assert!(dir.join("plugin.json").is_file());
    assert!(dir.join("bin").is_dir());
    assert!(dir.join("domain_packs/accounting.json").is_file());

    let runtime = tokio::runtime::Runtime::new().expect("端到端测试需要 tokio 运行时");
    let _entered = runtime.enter();
    let registry = get_capability_registry();
    assert!(registry.get_opc_store().is_none(), "测试进程起点：opc.store 应为空");
    let handle = registry
        .register_opc_store(Arc::new(FixedAccountingStore))
        .expect("accounting 数据替身应能注册到 opc.store");

    let home = common::TempDir::new("accounting-bundle-home");
    let mut manager = PluginManager::new(PluginManagerConfig::new(&home.path));
    let outcome = manager
        .install(dir.to_str().expect("产物路径应是 UTF-8"))
        .expect("accounting bundle 应能安装");
    manager.enable(&outcome.plugin_id).expect("enable 应成功");

    // ① 三条命令全链可用。
    let rows = manager
        .execute_plugin_command(&outcome.plugin_id, "opc_list_invoices", &json!({}))
        .expect("启用后发票列表应可执行");
    // `execute_plugin_command` 的出参是 worker 回执 `value` 的 JSON 文本（不是信封）：
    // accounting 的列表命令逐字透出 `Vec<InvoiceSummaryView>` 裸数组。
    let parsed: Value = serde_json::from_str(&rows).expect("命令输出应是 JSON 文本");
    assert_eq!(parsed[0]["invoiceNumber"], "INV-001", "opc.store 视图应逐字透出：{parsed:?}");
    let created = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "opc_create_invoice",
            &json!({ "customerId": "cust-1", "lineItems": [{ "description": "咨询", "quantity": 1 }] }),
        )
        .expect("启用后创建发票应可执行");
    let parsed_created: Value = serde_json::from_str(&created).expect("创建输出应是 JSON 文本");
    assert_eq!(parsed_created["invoiceNumber"], "INV-002");
    let transitioned = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "opc_transition_invoice",
            &json!({ "id": "inv-1", "status": "paid" }),
        )
        .expect("启用后状态变更应可执行");
    assert_eq!(transitioned, "not_found", "缺行态走 payload：{transitioned}");

    // ② 禁用后：整条命令链不可用；宿主 opc.store 与页面投影不再受该插件贡献。
    manager.disable(&outcome.plugin_id).expect("disable 应成功");
    let err = manager
        .execute_plugin_command(&outcome.plugin_id, "opc_list_invoices", &json!({}))
        .expect_err("禁用后同名命令不得再执行");
    assert!(err.to_string().contains("未启用"), "错误应说明禁用态：{err}");
    let packs = manager.list_plugin_domain_packs().expect("域包投影应可读");
    assert!(
        !packs.iter().any(|p| p.id == outcome.plugin_id),
        "禁用后 accounting 插件的域包投影不得出现：{packs:?}"
    );

    // ③ 恢复启用后：链路仍可用（不是一次性只能跑一轮）。
    manager.enable(&outcome.plugin_id).expect("重新 enable 应成功");
    let rows = manager
        .execute_plugin_command(&outcome.plugin_id, "opc_list_invoices", &json!({}))
        .expect("恢复后发票列表应可执行");
    let parsed: Value = serde_json::from_str(&rows).expect("恢复输出应是 JSON 文本");
    assert_eq!(parsed[0]["invoiceNumber"], "INV-001");
    let located = manager
        .resolve_page_asset(&outcome.plugin_id, "opc-accounting")
        .expect("恢复后页面资产应可解析");
    let page_path = std::path::PathBuf::from(&located.absolute_path);
    assert!(page_path.is_file(), "页面资产不存在：{}", page_path.display());

    drop(manager);
    handle.undo();
}
