// SPDX-License-Identifier: AGPL-3.0-only

//! 公共 `opc` bundle 的真安装 / 命令 / 禁用 / 恢复端到端。
//!
//! 与 14 个垂直域不同，公共 bundle 是共享 worker 与共享 Tool 的 owner：
//! 本测试只锁它自己的命名命令经 `opc.store` 接缝可用、禁用后不可用、恢复后可用，
//! 不把垂直域的命令算到它头上。门控 `AXAGENT_OPC_BUNDLE_DIR`，未设置时打印 SKIP。

mod common;

use std::sync::Arc;

use axagent_harness::{OpcStoreProvider, get_capability_registry};
use axagent_plugins::{PluginManager, PluginManagerConfig};
use serde_json::json;

struct FixedCommonOpcStore;

#[async_trait::async_trait]
impl OpcStoreProvider for FixedCommonOpcStore {
    async fn market_list(&self) -> axagent_harness::core_error::Result<serde_json::Value> {
        Ok(json!({ "items": [{ "id": "pack-1", "name": "示例能力包" }] }))
    }
}

#[test]
fn common_opc_bundle_install_command_disable_and_restore() {
    let Ok(dir) = std::env::var("AXAGENT_OPC_BUNDLE_DIR") else {
        eprintln!(
            "SKIP: 未设置 AXAGENT_OPC_BUNDLE_DIR，跳过公共 opc bundle 启停端到端。\n\
             跑法：node axagent-plugins/scripts/build-plugin-bundle.mjs --plugin opc 后指向产物目录。"
        );
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    assert!(dir.join("plugin.json").is_file());
    assert!(dir.join("bin").is_dir());
    assert!(dir.join("pages/index.html").is_file());

    let runtime = tokio::runtime::Runtime::new().expect("端到端测试需要 tokio 运行时");
    let _entered = runtime.enter();
    let registry = get_capability_registry();
    assert!(registry.get_opc_store().is_none(), "测试进程起点：opc.store 应为空");
    let handle = registry
        .register_opc_store(Arc::new(FixedCommonOpcStore))
        .expect("公共 opc 数据替身应能注册到 opc.store");

    let home = common::TempDir::new("common-opc-bundle-home");
    let mut manager = PluginManager::new(PluginManagerConfig::new(&home.path));
    let outcome = manager
        .install(dir.to_str().expect("产物路径应是 UTF-8"))
        .expect("公共 opc bundle 应能安装");
    assert!(
        outcome.plugin_id.starts_with("opc@"),
        "目录安装的公共 bundle 应保持 `name@external` 形态：{}",
        outcome.plugin_id
    );
    manager.enable(&outcome.plugin_id).expect("enable 应成功");

    let output = manager
        .execute_plugin_command(&outcome.plugin_id, "opc_market_list", &json!({}))
        .expect("启用后公共市场列表命令应可执行");
    assert!(output.contains("pack-1"), "命令输出应逐字透出接缝数据：{output}");
    let located = manager
        .resolve_page_asset(&outcome.plugin_id, "opc-home")
        .expect("启用后公共页面资产应可解析");
    assert!(std::path::PathBuf::from(located.absolute_path).is_file());

    manager.disable(&outcome.plugin_id).expect("disable 应成功");
    let err = manager
        .execute_plugin_command(&outcome.plugin_id, "opc_market_list", &json!({}))
        .expect_err("禁用后公共命令不得再执行");
    assert!(err.to_string().contains("未启用"), "错误应说明禁用态：{err}");
    assert!(
        manager.resolve_page_asset(&outcome.plugin_id, "opc-home").is_err(),
        "禁用后公共页面资产不得再解析"
    );

    manager.enable(&outcome.plugin_id).expect("重新 enable 应成功");
    let output = manager
        .execute_plugin_command(&outcome.plugin_id, "opc_market_list", &json!({}))
        .expect("恢复后公共市场列表命令应可执行");
    assert!(output.contains("pack-1"), "恢复后输出应逐字透出接缝数据：{output}");

    drop(manager);
    handle.undo();
}
