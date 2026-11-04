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

    // B8-0：前端资产随包安装 —— `pages[]` 声明的 entry 必须解析成**安装目录内**的既有文件。
    // 这里锁的是「打包脚本写出的清单与盘上资产一致」，不是资产内容本身（内容归 B8-2）。
    let listed = manager.list_plugin_pages().expect("页面列表投影应可读");
    let stock = listed
        .iter()
        .find(|p| p.id == outcome.plugin_id)
        .expect("已安装的股票包应出现在页面列表里");
    assert_eq!(stock.pages.len(), 1, "清单声明的页面数应与 pages[] 一致：{:?}", stock.pages);
    assert_eq!(stock.pages[0].id, "stock-home");

    let located = manager
        .resolve_page_asset(&outcome.plugin_id, "stock-home")
        .expect("bundle 页的 entry 应解析成安装根内的绝对路径");
    assert_eq!(located.entry, "pages/index.html");
    let page_path = std::path::PathBuf::from(&located.absolute_path);
    assert!(
        page_path.starts_with(&outcome.install_path),
        "页面资产必须落在安装目录内：{} vs {}",
        page_path.display(),
        outcome.install_path.display()
    );
    assert!(page_path.is_file(), "页面资产不存在：{}", page_path.display());
    let html = std::fs::read_to_string(&page_path).expect("页面资产应可读");
    assert!(html.contains("股票插件前端通道已打通"), "读到的不是探针页：{html}");

    // 先放 manager（析构时停 worker），再删临时目录（Windows 上占用中的 exe 删不掉）。
    drop(manager);
    market.undo();
}
