// SPDX-License-Identifier: AGPL-3.0-only

//! #320：运行时工具面净变化 0 的**端到端实测**（销 `PLAN-shell-plus-plugins.md` §176 未做③）。
//!
//! §176/#311 批把四条宿主纯智能体工具（get_portfolio_risk / compare_trade_with_analysis /
//! scan_sector_coherence / set_quote_watch_priority）迁成插件命名命令，「运行时净变化 0」
//! 当时只按 #146 通道推演。本文件用**真 bundled 产物**实测插件半边的三跳：
//! ① 清单声明的四条命令经 `agent_command_specs` 投影进智能体命令面 catalog —— 名字面
//!    是 `stock_*` 短名（`worker://` 前缀是清单 `command` 路由键，不是命令面名字），
//!    授权档位（三条 read-only + 一条 workspace-write）与 inputSchema 原样带出；
//! ② 四条命令各自经真 worker 进程 + 命令帧**真的执行**（帧往返，不是进程内直调分派函数），
//!    出参形状按各接缝替身的固定值钉住；
//! ③ 写命令 `bg` 别名必须在插件侧归一成 `background`（接缝严格 serde，替身断言收到的
//!    是归一后档位）。
//!
//! 宿主半边（宿主 inventory 注册表无四条宿主名 + 合并口零剔除）在 `src-tauri` 的
//! `commands::agent::command_bridge` 测试区（`retired_stock_agent_tools_absent_from_host_registry_and_worker_aliases_survive_merge`）。
//!
//! 门控：`AXAGENT_STOCK_BUNDLE_DIR=<打包产物目录>`（同 `e2e_stock_bundle_install_test`）。
//! 取法：`node axagent-plugins/scripts/build-plugin-bundle.mjs` 后指向 `axagent-plugins/dist/plugins/stock`。

mod common;

use std::sync::Arc;

use axagent_harness::{
    AsOfContext, PositionSummary, TradePredictionComparison, WatchOutcome, WatchPriority,
    get_capability_registry,
};
use axagent_plugins::{PluginManager, PluginManagerConfig};
use common::{FixedQuoteProvider, TempDir};
use serde_json::{Value, json};

fn bundle_dir() -> Option<std::path::PathBuf> {
    match std::env::var("AXAGENT_STOCK_BUNDLE_DIR") {
        Ok(path) if !path.trim().is_empty() => Some(std::path::PathBuf::from(path)),
        _ => {
            eprintln!(
                "SKIP: 未设置 AXAGENT_STOCK_BUNDLE_DIR，跳过四条智能体命令端到端测试。\
                 \n      本测试**未通过，也未被验证** —— 跳过仅表示没有可用产物。\
                 \n      跑法：node axagent-plugins/scripts/build-plugin-bundle.mjs，再指向 axagent-plugins/dist/plugins/stock。"
            );
            None
        },
    }
}

/// trading.engine 替身：`get_positions` 回一条样例持仓（风险指标按它算）；
/// `compare_trade` 回正面 `Some`（透明透传断言）。其余方法吃 trait 默认 `Err`。
struct FixedTradingEngine;

#[async_trait::async_trait]
impl axagent_harness::TradingEngineProvider for FixedTradingEngine {
    async fn get_positions(
        &self,
        _as_of: Option<AsOfContext>,
    ) -> axagent_harness::core_error::Result<Vec<PositionSummary>> {
        Ok(vec![PositionSummary {
            stock_code: "600519".to_string(),
            stock_name: "贵州茅台".to_string(),
            total_shares: 100,
            avg_cost: 1_480.0,
            current_price: Some(1_500.0),
            market_value: Some(150_000.0),
            unrealized_pnl: Some(2_000.0),
            unrealized_pnl_pct: Some(1.35),
            total_realized_pnl: 500.0,
            sector_name: Some("白酒".to_string()),
        }])
    }

    async fn compare_trade(
        &self,
        trade_id: &str,
    ) -> axagent_harness::core_error::Result<Option<TradePredictionComparison>> {
        assert_eq!(
            trade_id, "trade-1",
            "插件应把命令面的 camelCase `tradeId` 解出后按接缝参数透传"
        );
        Ok(Some(TradePredictionComparison {
            analysis_action: "hold".to_string(),
            analysis_target: Some(1_560.0),
            analysis_stop: Some(1_420.0),
            actual_price: 1_500.0,
            target_deviation_pct: -3.85,
        }))
    }
}

/// quote.watcher 替身：`watch` 回 `applied = 1`（未截断 ⇒ 插件 fail-closed 闸不触发）。
struct FixedWatcher;

#[async_trait::async_trait]
impl axagent_harness::QuoteWatcherProvider for FixedWatcher {
    async fn watch(
        &self,
        codes: &[String],
        priority: WatchPriority,
        replace: bool,
    ) -> axagent_harness::core_error::Result<WatchOutcome> {
        assert_eq!(codes, ["600519".to_string()], "单码入参应原样透传");
        assert_eq!(
            priority,
            WatchPriority::Background,
            "命令层传的 `bg` 必须已被插件 `watch_priority_arg` 归一成 background"
        );
        assert!(!replace);
        Ok(WatchOutcome { requested: 1, applied: 1 })
    }
}

#[test]
fn four_agent_commands_project_into_catalog_and_execute_over_real_worker_frames() {
    let Some(dir) = bundle_dir() else { return };
    assert!(dir.join("plugin.json").is_file(), "产物目录里没有 plugin.json：{}", dir.display());

    // 加载/回调都要运行时上下文（与 bundle_install_test 同口径）。
    let runtime = tokio::runtime::Runtime::new().expect("端到端测试需要 tokio 运行时");
    let _entered = runtime.enter();

    // 四条命令的三处接缝替身：trading.engine（两条读）、market.quote（板块扫描取行情）、
    // quote.watcher（唯一写命令）。workflow.run 本批四条用不到，不注册。
    let registry = get_capability_registry();
    let market = registry
        .register_market_data(Arc::new(FixedQuoteProvider))
        .expect("行情替身应能注册到 market.quote");
    let trading = registry
        .register_trading_engine(Arc::new(FixedTradingEngine))
        .expect("交易替身应能注册到 trading.engine");
    let watcher = registry
        .register_quote_watcher(Arc::new(FixedWatcher))
        .expect("监视替身应能注册到 quote.watcher");

    let home = TempDir::new("stock-agent-cmds-home");
    let mut manager = PluginManager::new(PluginManagerConfig::new(&home.path));
    let outcome = manager
        .install(dir.to_str().expect("产物路径应是 UTF-8"))
        .expect("install 应成功：清单合法、整目录（含 bin/）被拷贝进安装根");
    manager.enable(&outcome.plugin_id).expect("enable 应成功");

    // ── ① catalog 面：四条命令投影进智能体命令面（B10-1 通道的真实数据） ──
    let specs = manager.agent_command_specs().expect("投影插件命令");
    let by_name = |name: &str| {
        specs
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("智能体命令面缺 `{name}`：实际 {specs:?}"))
    };
    let portfolio = by_name("stock_get_portfolio_risk");
    let compare = by_name("stock_compare_trade_with_analysis");
    let scan = by_name("stock_scan_sector_coherence");
    let watch = by_name("stock_set_quote_watch_priority");
    assert!(
        specs.iter().all(|s| !s.name.starts_with("worker://")),
        "命令面名字应是清单 `name` 短名，`worker://` 只是路由键：{specs:?}"
    );
    // 授权档位原样带出：三条读 + 一条写（写档位错标 read-only 会让确认弹窗形同虚设）。
    assert_eq!(portfolio.required_permission.as_deref(), Some("read-only"));
    assert_eq!(compare.required_permission.as_deref(), Some("read-only"));
    assert_eq!(scan.required_permission.as_deref(), Some("read-only"));
    assert_eq!(
        watch.required_permission.as_deref(),
        Some("workspace-write"),
        "唯一写命令必须带写档位"
    );
    // 参数面跟着穿：compare 的 tradeId 是 required；写命令不声明 enum（`bg` 别名宽容）。
    let compare_schema = compare.input_schema.as_ref().expect("compare 声明了 inputSchema");
    assert_eq!(
        compare_schema["required"].as_array().map(|r| r[0].as_str()),
        Some(Some("tradeId")),
        "实际 {compare_schema}"
    );
    let watch_schema = watch.input_schema.as_ref().expect("watch 声明了 inputSchema");
    assert!(
        watch_schema["properties"].get("priority").is_some_and(|p| p.get("enum").is_none()),
        "priority 不得加 enum（会把 `bg` 别名宽容变成 IPC 拒收）：{watch_schema}"
    );

    // ── ② 帧往返：四条命令逐条真执行（真 worker 进程 + 命令帧 + 插件分派臂 + 接缝替身）──
    let output = manager
        .execute_plugin_command(&outcome.plugin_id, "stock_get_portfolio_risk", &json!({}))
        .expect("组合风险命令应经 worker 通道执行");
    let risk: Value = serde_json::from_str(&output).expect("输出应是 JSON 文本");
    assert!(
        risk.get("totalPositions").is_some() && risk["riskLevel"].is_string(),
        "风险指标应按 PortfolioRiskMetrics 的 camelCase 形状出参：{risk}"
    );
    assert_eq!(
        risk["totalPositions"], 1,
        "替身只给一条持仓 ⇒ 持仓数必须是 1（证明执行链真的穿过接缝取了数）"
    );

    let output = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "stock_compare_trade_with_analysis",
            &json!({ "tradeId": "trade-1" }),
        )
        .expect("交易对比命令应经 worker 通道执行");
    let comparison: Value = serde_json::from_str(&output).expect("输出应是 JSON 文本");
    assert_eq!(comparison["analysisAction"], "hold", "接缝出参应原样透传：{comparison}");
    assert_eq!(comparison["targetDeviationPct"], -3.85);

    let output = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "stock_scan_sector_coherence",
            &json!({ "conceptIds": ["concept_ai"] }),
        )
        .expect("板块联动扫描命令应经 worker 通道执行");
    let reports: Value = serde_json::from_str(&output).expect("输出应是 JSON 文本");
    let reports = reports.as_array().expect("扫描结果应是报告数组");
    assert!(
        !reports.is_empty(),
        "样本本体里 concept_ai 有成员 ⇒ 至少一条报告（空数组说明行情或索引链路断了）"
    );

    let output = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "stock_set_quote_watch_priority",
            &json!({ "stockCode": "600519", "priority": "bg" }),
        )
        .expect("监控优先级命令应经 worker 通道执行（替身已断言 `bg` 被归一）");
    assert_eq!(
        output.parse::<Value>().expect("输出应是 JSON 文本"),
        Value::Null,
        "applied=1 时出参保持宿主原命令的 `()`（JSON null）：{output}"
    );

    // 先放 manager（析构时停 worker），再放接缝替身、删临时目录。
    drop(manager);
    watcher.undo();
    trading.undo();
    market.undo();
}
