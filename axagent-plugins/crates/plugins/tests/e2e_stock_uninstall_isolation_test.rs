// SPDX-License-Identifier: AGPL-3.0-only
//!
//! stock 真 bundle 卸载隔离：与 security 声明式插件并存时卸载 stock，验证：
//! 1. stock 页面、命令与 worker 都不可用；
//! 2. stock 安装目录被删除；
//! 3. security 页面与域包投影不受影响。
//!
//! 门控：`AXAGENT_STOCK_BUNDLE_DIR` 与 `AXAGENT_SECURITY_BUNDLE_DIR`；CI 前置会真实 staging
//! 两个产物，缺失即失败，不把未执行的隔离测试当成通过。

mod common;

use std::sync::Arc;

use axagent_harness::get_capability_registry;
use axagent_plugins::{PluginManager, PluginManagerConfig};
use common::{FixedQuoteProvider, TempDir};
use serde_json::json;

fn bundled_dir(env: &str) -> std::path::PathBuf {
    let value =
        std::env::var(env).unwrap_or_else(|_| panic!("未设置 {env}；隔离测试必须使用真 bundle"));
    let dir = std::path::PathBuf::from(value);
    assert!(dir.join("plugin.json").is_file(), "{env} 缺少 plugin.json：{}", dir.display());
    dir
}

#[test]
fn uninstalling_stock_leaves_security_plugin_available() {
    let stock_dir = bundled_dir("AXAGENT_STOCK_BUNDLE_DIR");
    let security_dir = bundled_dir("AXAGENT_SECURITY_BUNDLE_DIR");

    let runtime = tokio::runtime::Runtime::new().expect("卸载隔离测试需要 tokio 运行时");
    let _entered = runtime.enter();
    let registry = get_capability_registry();
    let market = registry
        .register_market_data(Arc::new(FixedQuoteProvider))
        .expect("stock 行情替身应能注册");

    let home = TempDir::new("stock-uninstall-isolation");
    let mut manager = PluginManager::new(PluginManagerConfig::new(&home.path));

    let stock = manager
        .install(stock_dir.to_str().expect("stock 路径应是 UTF-8"))
        .expect("stock bundle 应能安装");
    let security = manager
        .install(security_dir.to_str().expect("security 路径应是 UTF-8"))
        .expect("security bundle 应能安装");
    manager.enable(&stock.plugin_id).expect("stock enable 应成功");
    manager.enable(&security.plugin_id).expect("security enable 应成功");

    manager
        .execute_plugin_command(
            &stock.plugin_id,
            "stock_get_quote",
            &json!({ "stockCode": "600519" }),
        )
        .expect("卸载前 stock 命令应可执行");
    assert!(manager.plugin_worker_is_running(&stock.plugin_id));
    assert!(
        manager.resolve_page_asset(&stock.plugin_id, "stock-home").is_ok(),
        "卸载前 stock 页面应可解析"
    );

    let before_security = manager.resolve_page_asset(&security.plugin_id, "opc-security");
    assert!(before_security.is_ok(), "卸载前 security 页面应可解析");

    let stock_install_path = stock.install_path.clone();
    manager.uninstall(&stock.plugin_id).expect("stock bundle 应能卸载");

    assert!(!manager.plugin_worker_is_running(&stock.plugin_id), "卸载后 stock worker 不应运行");
    assert!(!stock_install_path.exists(), "卸载后 stock 安装目录应删除");
    assert!(
        manager
            .execute_plugin_command(
                &stock.plugin_id,
                "stock_get_quote",
                &json!({ "stockCode": "600519" })
            )
            .is_err(),
        "卸载后 stock 命令必须失败"
    );
    assert!(
        manager.resolve_page_asset(&stock.plugin_id, "stock-home").is_err(),
        "卸载后 stock 页面必须不可解析"
    );
    assert!(
        manager.resolve_page_asset(&security.plugin_id, "opc-security").is_ok(),
        "卸载 stock 不得影响 security 页面"
    );
    let packs = manager.list_plugin_domain_packs().expect("域包投影应可读");
    assert!(packs.iter().any(|pack| pack.id == security.plugin_id), "security 域包投影应保留");

    drop(manager);
    market.undo();
}
