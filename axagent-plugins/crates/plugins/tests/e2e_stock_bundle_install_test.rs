// SPDX-License-Identifier: AGPL-3.0-only

//! 真安装路径端到端：`PluginManager::install`（拷贝整目录）→ `enable`
//! （`resolve_worker_program` 解析清单里的**相对** `worker.program` + 从 manifest
//! `permissions` 推沙箱）→ `execute_plugin_command`。
//!
//! 与 `e2e_stock_tool_test.rs` 的分工：那个从 `PluginWorkerConfig` + 绝对路径直接 spawn，
//! 锁的是「帧 + 宿主分派」；本文件锁的是**安装/启用链路**（相对路径、权限闸、清单命令表）。
//!
//! 门控：`AXAGENT_STOCK_BUNDLE_DIR=<打包产物目录>`（含 `plugin.json` 与 `bin/`）。
//! 取法：`node scripts/build-plugin-bundle.mjs` 后指向 `dist/plugins/stock`。

mod common;

use std::sync::Arc;

use axagent_harness::get_capability_registry;
use axagent_plugins::{PluginManager, PluginManagerConfig};
use common::{FixedQuoteProvider, TempDir};
use serde_json::{Value, json};

fn bundle_dir() -> Option<std::path::PathBuf> {
    match std::env::var("AXAGENT_STOCK_BUNDLE_DIR") {
        Ok(path) if !path.trim().is_empty() => Some(std::path::PathBuf::from(path)),
        _ => {
            eprintln!(
                "SKIP: 未设置 AXAGENT_STOCK_BUNDLE_DIR，跳过「安装 → 启用 → 命令」端到端测试。\
                 \n      本测试**未通过，也未被验证** —— 跳过仅表示没有可用产物。\
                 \n      跑法：node scripts/build-plugin-bundle.mjs，再指向 dist/plugins/stock。"
            );
            None
        },
    }
}

#[test]
fn install_enable_and_execute_a_command_from_the_staged_bundle() {
    let Some(dir) = bundle_dir() else { return };
    assert!(dir.join("plugin.json").is_file(), "产物目录里没有 plugin.json：{}", dir.display());
    assert!(dir.join("bin").is_dir(), "产物目录里没有 bin/：{}", dir.display());

    // 加载/回调都要运行时上下文（`set_declaration` 用 `Handle::try_current()`）。
    let runtime = tokio::runtime::Runtime::new().expect("端到端测试需要 tokio 运行时");
    let _entered = runtime.enter();

    let registry = get_capability_registry();
    assert!(registry.get_market_data().is_none(), "测试进程起点：market.quote 应为空");
    let market = registry
        .register_market_data(Arc::new(FixedQuoteProvider))
        .expect("行情替身应能注册到 market.quote");

    let home = TempDir::new("stock-bundle-home");
    let mut manager = PluginManager::new(PluginManagerConfig::new(&home.path));

    let outcome = manager
        .install(dir.to_str().expect("产物路径应是 UTF-8"))
        .expect("install 应成功：清单合法、整目录（含 bin/）被拷贝进安装根");
    // 拷贝必须带走 worker 产物 —— 只拷清单会让 enable 在 resolve_worker_program 处失败。
    assert!(
        outcome.install_path.join("bin").is_dir(),
        "安装目录里没有 bin/：{}",
        outcome.install_path.display()
    );

    manager
        .enable(&outcome.plugin_id)
        .expect("enable 应成功：相对 worker.program 可解析，且清单声明了 subprocess_execution");

    let output = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "stock_get_quote",
            &json!({ "stock_code": "600519" }),
        )
        .expect("清单里声明的命令应能经 worker 通道执行");

    let parsed: Value = serde_json::from_str(&output).expect("命令输出应是 JSON 文本");
    assert_eq!(parsed["price"], 1500.25, "行情必须原样透出：{parsed:?}");
    assert_eq!(parsed["code"], "600519");

    // 先放 manager（析构时停 worker），再删临时目录（Windows 上占用中的 exe 删不掉）。
    drop(manager);
    market.undo();
}
