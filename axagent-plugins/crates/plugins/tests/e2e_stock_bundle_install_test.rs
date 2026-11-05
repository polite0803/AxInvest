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
use common::{FixedQuoteProvider, FixedWorkflowRunProvider, TempDir};
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
    // 任务源取数走 `workflow.run` 的 `list_recent`（B8-1b 建通道 / #150 换取数口）⇒
    // 这条链要端到端跑通，必须把执行接缝也换成替身；真实引擎在测试进程里不存在。
    assert!(
        registry.get_workflow_run().is_none(),
        "测试进程起点：workflow.run 应为空（否则本文件与别的测试串了）"
    );
    let workflow_run = registry
        .register_workflow_run(Arc::new(FixedWorkflowRunProvider))
        .expect("工作流执行替身应能注册到 workflow.run");

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

    // B9-1：纯函数命令（证据权重）经同一 worker 通道执行，且**不依赖**行情接缝。
    let weights_output = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "compute_evidence_weights",
            &json!({
                "marketRegime": {
                    "regime": "sideways",
                    "confidence": 0.5,
                    "volatility": "normal",
                    "description": "e2e 固定环境"
                },
                "timeHorizon": "short",
                "analysts": [{ "analystId": "a-market-analyst", "stance": "中性" }]
            }),
        )
        .expect("权重命令应能经 worker 通道执行");
    let weights: Value = serde_json::from_str(&weights_output).expect("权重输出应是 JSON 文本");
    assert_eq!(weights["timeHorizon"], "short");
    assert!(
        weights["analystWeights"].as_array().is_some_and(|w| w.len() == 1),
        "输入一个分析师 ⇒ 一个权重：{weights}"
    );

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

    // B8-1b：任务中心任务源的**整条链** —— 清单声明 → 宿主投影 → 命名命令 → 宿主接缝取数 → 行记录。
    // 这四步任一漂了都会在真实安装里静默少一个源，所以在这里一次锁穿。
    let sources = manager.list_plugin_task_sources().expect("任务源列表投影应可读");
    let stock_sources = sources
        .iter()
        .find(|s| s.id == outcome.plugin_id)
        .expect("已安装且声明了 taskSources[] 的股票包应出现在任务源列表里");
    assert!(stock_sources.enabled, "enable 之后任务源投影的 enabled 应为 true");
    assert_eq!(
        stock_sources.task_sources.len(),
        1,
        "清单声明的任务源数应与 taskSources[] 一致：{:?}",
        stock_sources.task_sources
    );
    let declared = &stock_sources.task_sources[0];
    assert_eq!(declared.id, "stock-workflow-runs");
    assert_eq!(declared.command, "list_recent_workflow_runs");

    // 投影里的 `command` 必须真能执行 —— 否则任务中心会挂一个永远为空的标签。
    let rows_output = manager
        .execute_plugin_command(&outcome.plugin_id, &declared.command, &json!({}))
        .expect("清单声明的任务源取数命令应能经 worker 通道执行");
    let rows: Value = serde_json::from_str(&rows_output).expect("取数命令应返回 JSON 文本");
    let rows = rows.as_array().expect("任务源取数必须回数组（宿主泛型适配器按数组消费）");
    // #150 换取数口为 `list_recent` ⇒ 替身给「一条在跑 + 一条已结束」，两行都要能落到中心。
    // 少了后面那行，任务中心就只会显示在跑的批次（跑完即消失），耗时列也永远算不出来。
    assert_eq!(rows.len(), 2, "在跑 + 已结束各一行：{rows:?}");
    assert_eq!(rows[0]["nativeId"], "exec-1");
    assert_eq!(rows[0]["name"], "wf-1 · exec-1");
    // 引擎 `running` → 中心 `running`；拼法翻译（cancelled→canceled）在纯函数单测里锁。
    assert_eq!(rows[0]["state"], "running");
    assert_eq!(rows[0]["trigger"], "manual");
    assert_eq!(rows[0]["startedAt"], 1_700_000_000_000_i64);
    // 缺省项必须是**不出现**而不是 null：宿主按「字段在不在」决定耗时列与可否打开。
    for absent in ["finishedAt", "durationMs", "deepLink"] {
        assert!(rows[0].get(absent).is_none(), "在跑行的缺省字段 `{absent}` 不得进帧：{}", rows[0]);
    }
    assert_eq!(rows[0]["raw"]["currentNodeId"], "node-2", "引擎视图原样留在 raw");

    // 已结束那行：引擎 `partially_completed` → 中心 `succeeded`（有产出就不算失败），
    // 且带宿主盖的 `finishedAt` ⇒ 前端的耗时列由此可算（`durationMs` 不在插件侧算）。
    assert_eq!(rows[1]["nativeId"], "exec-0");
    assert_eq!(rows[1]["state"], "succeeded", "终态行的词表翻译：{}", rows[1]);
    assert_eq!(rows[1]["finishedAt"], 1_699_999_999_000_i64);
    for absent in ["durationMs", "deepLink"] {
        assert!(rows[1].get(absent).is_none(), "缺省字段 `{absent}` 不得进帧：{}", rows[1]);
    }

    // 先放 manager（析构时停 worker），再删临时目录（Windows 上占用中的 exe 删不掉）。
    drop(manager);
    market.undo();
    workflow_run.undo();
}
