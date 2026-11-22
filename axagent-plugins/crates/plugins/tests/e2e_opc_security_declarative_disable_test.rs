// SPDX-License-Identifier: AGPL-3.0-only

//! security 域 worker 的**真 bundle 启停**端到端。
//!
//! 本域已统一为 Rust worker，但这里只锁与传输形态无关的共性面：
//! 启用后页面可解析、页面读取随包域数据、域包投影可见；
//! 禁用后两者消失；恢复后仍可解析。命令转发由 worker 域通用 E2E 覆盖。
//! 门控：`AXAGENT_SECURITY_BUNDLE_DIR`；CI 由 nextest staging 前置。

mod common;

use axagent_harness::get_capability_registry;
use axagent_plugins::{PluginManager, PluginManagerConfig};

#[test]
fn security_declarative_bundle_disable_and_restore_is_scoped_to_its_plugin() {
    let Ok(dir) = std::env::var("AXAGENT_SECURITY_BUNDLE_DIR") else {
        eprintln!(
            "SKIP: 未设置 AXAGENT_SECURITY_BUNDLE_DIR，跳过 security bundle 启停端到端。\
             \n      跑法：node axagent-plugins/scripts/build-plugin-bundle.mjs --plugin opc-security \
             后指向 axagent-plugins/dist/plugins/opc-security。"
        );
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    assert!(dir.join("plugin.json").is_file());
    assert!(dir.join("pages/index.html").is_file());
    assert!(dir.join("domain_packs/security.json").is_file());

    let runtime = tokio::runtime::Runtime::new().expect("端到端测试需要 tokio 运行时");
    let _entered = runtime.enter();
    let registry = get_capability_registry();
    assert!(registry.get_opc_store().is_none(), "security E2E 不应依赖宿主 opc.store");
    let _guard = registry.clone();

    let home = common::TempDir::new("security-bundle-home");
    let mut manager = PluginManager::new(PluginManagerConfig::new(&home.path));
    let outcome = manager
        .install(dir.to_str().expect("产物路径应是 UTF-8"))
        .expect("security bundle 应能安装");
    manager.enable(&outcome.plugin_id).expect("enable 应成功");

    // ① 启用态：页面资产可解析、域包投影可见。
    let located = manager
        .resolve_page_asset(&outcome.plugin_id, "opc-security")
        .expect("启用后页面资产应可解析");
    let page_path = std::path::PathBuf::from(&located.absolute_path);
    assert!(page_path.starts_with(&outcome.install_path), "页面资产必须落在安装根内");
    assert!(page_path.is_file());
    let html = std::fs::read_to_string(&page_path).expect("页面资产应可读");
    assert!(html.contains("AX_OPC_DOMAIN_DATA"), "页面必须读取随包域数据：{html}");
    assert!(html.contains("OPC 域包"), "读到的不是 OPC 域包页：{html}");
    let domain_data_path = page_path.parent().expect("页面资产应有父目录").join("domain-data.js");
    let domain_data = std::fs::read_to_string(&domain_data_path).expect("随包域数据应可读");
    assert!(domain_data.contains("\"id\": \"security\""), "域数据必须属于 security：{domain_data}");
    assert!(
        domain_data.contains("\"name\": \"安全合规完整流程\""),
        "域数据必须是真实域包投影：{domain_data}"
    );
    let packs = manager.list_plugin_domain_packs().expect("域包投影应可读");
    let pack =
        packs.iter().find(|p| p.id == outcome.plugin_id).expect("启用后 security 域包投影应可见");
    assert!(pack.enabled);
    assert_eq!(pack.domain_packs.len(), 1);
    assert_eq!(pack.domain_packs[0].id, "security");
    assert_eq!(pack.domain_packs[0].transport_owner, "opc-security");
    assert!(!pack.domain_packs[0].commands.is_empty(), "worker 域包必须声明自己的命令面");

    // ② 禁用态：页面与域包投影都消失；不得引入命令不可用的说法（本来零命令）。
    manager.disable(&outcome.plugin_id).expect("disable 应成功");
    let err = manager
        .resolve_page_asset(&outcome.plugin_id, "opc-security")
        .expect_err("禁用后页面资产不得再解析");
    assert!(
        err.to_string().contains("禁用") || err.to_string().contains("未启用"),
        "错误应说明禁用态：{err}"
    );
    let packs = manager.list_plugin_domain_packs().expect("禁用后投影应可读");
    assert!(
        !packs.iter().any(|p| p.id == outcome.plugin_id),
        "禁用后 security 域包投影不得出现：{packs:?}"
    );

    // ③ 恢复启用：两者都回来。
    manager.enable(&outcome.plugin_id).expect("重新 enable 应成功");
    let located = manager
        .resolve_page_asset(&outcome.plugin_id, "opc-security")
        .expect("恢复后页面资产应可解析");
    let page_path = std::path::PathBuf::from(&located.absolute_path);
    assert!(page_path.is_file());
    assert!(
        page_path.parent().expect("页面资产应有父目录").join("domain-data.js").is_file(),
        "恢复后随包域数据也应可用"
    );
    let packs = manager.list_plugin_domain_packs().expect("恢复后投影应可读");
    assert!(packs.iter().any(|p| p.id == outcome.plugin_id));

    drop(manager);
}
