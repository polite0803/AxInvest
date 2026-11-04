// SPDX-License-Identifier: AGPL-3.0-only

//! 首个真实插件能力（`plugin-stock` 的 `get_stock_quote`）在**真实宿主路径**上的端到端验证：
//! `LoadedPlugin::load`（spawn + describe + 注册 `tool.set`）→ 宿主 `Tool::call`
//! → worker 回调 `market.quote` → 宿主 provider → `content` 回到宿主。
//!
//! 与 `plugin-stock/tests/protocol.rs` 的分工：那边「测试自扮宿主」锁帧形状；本文件走
//! 宿主侧真实实现（`RemoteToolSet` 解析 `list`、`HostPeer` 校验 `declared_calls`），
//! 只把行情 provider 换成固定值替身（真实 provider 要联网取数，不适合进单测）。
//!
//! 门控：`AXAGENT_STOCK_WORKER_BIN=<worker 可执行文件绝对路径>`。为什么用环境变量：
//! `plugin-stock` 是另一个 package，`CARGO_BIN_EXE_*` 只对本 package 的 bin 生效；本机
//! `CARGO_TARGET_DIR` 又指向 `~/.cargo/target-axagent`，写死 `target/` 会找不到。
//! 取路径的推荐方式：`node scripts/build-plugin-bundle.mjs` 后指向
//! `dist/plugins/stock/bin/axagent-plugin-stock(.exe)`。

use std::path::PathBuf;
use std::sync::Arc;

use axagent_harness::{
    AdjType, KLine, MarketDataProvider, Result as HarnessResult, StockQuote, StockSearchResult,
    ToolCategory, ToolContext, get_capability_registry,
};
use axagent_plugin_proto::CommandFrame;
use axagent_plugins::{LoadedPlugin, PluginWorkerConfig};
use serde_json::json;

/// 固定行情：只验证「宿主 provider 的数据能经插件回到宿主」，不验证真实取数。
struct FixedQuoteProvider;

#[async_trait::async_trait]
impl MarketDataProvider for FixedQuoteProvider {
    async fn get_quote(&self, stock_code: &str) -> HarnessResult<StockQuote> {
        Ok(StockQuote {
            code: stock_code.to_string(),
            name: "测试股".to_string(),
            price: 1500.25,
            pre_close: 1490.0,
            open: 1495.0,
            high: 1510.0,
            low: 1488.0,
            volume: 1_000_000.0,
            amount: 1_500_000_000.0,
            change_pct: 0.68,
            turnover_rate: 0.5,
            pe: Some(12.3),
            pb: Some(1.8),
            total_mv: Some(1.0e11),
            circulating_mv: None,
            limit_up: None,
            limit_down: None,
            is_st: false,
            timestamp: "2026-11-04T15:00:00+08:00".to_string(),
        })
    }

    async fn get_klines(
        &self,
        _stock_code: &str,
        _period: &str,
        _limit: u32,
        _adj_type: Option<AdjType>,
    ) -> HarnessResult<Vec<KLine>> {
        Ok(Vec::new())
    }

    async fn search_stock(&self, _keyword: &str) -> HarnessResult<Vec<StockSearchResult>> {
        Ok(Vec::new())
    }
}

fn worker_bin() -> Option<PathBuf> {
    match std::env::var("AXAGENT_STOCK_WORKER_BIN") {
        Ok(path) if !path.trim().is_empty() => Some(PathBuf::from(path)),
        _ => {
            eprintln!(
                "SKIP: 未设置 AXAGENT_STOCK_WORKER_BIN，跳过真实宿主 × 真实 worker 的端到端测试。\
                 \n      本测试**未通过，也未被验证** —— 跳过仅表示没有可用产物。\
                 \n      跑法：node scripts/build-plugin-bundle.mjs，再指向 dist/plugins/stock/bin/。"
            );
            None
        },
    }
}

#[test]
fn host_loads_stock_worker_and_executes_the_tool_through_market_quote() {
    let Some(bin) = worker_bin() else { return };
    assert!(bin.is_file(), "AXAGENT_STOCK_WORKER_BIN 指向的不是文件：{}", bin.display());

    // `set_declaration` 用 `Handle::try_current()` 取运行时；不进运行时上下文 ⇒ 装载虽成功，
    // 但 worker 回调 `market.quote` 会以「无运行时」失败。故装载前必须先进入。
    let runtime = tokio::runtime::Runtime::new().expect("端到端测试需要 tokio 运行时");
    let _entered = runtime.enter();

    let registry = get_capability_registry();
    assert!(registry.get_tool_set().is_none(), "测试进程起点：tool.set 应为空");
    assert!(registry.get_market_data().is_none(), "测试进程起点：market.quote 应为空");
    let market = registry
        .register_market_data(Arc::new(FixedQuoteProvider))
        .expect("行情替身应能注册到 market.quote");

    let loaded = LoadedPlugin::load(PluginWorkerConfig::new("plugin.stock", &bin))
        .expect("真实 worker 应完成 spawn + describe + tool.set 注册");
    assert!(
        loaded.declaration().calls.iter().any(|seam| seam == "market.quote"),
        "插件必须声明出站接缝，否则宿主会拒掉回调：{:?}",
        loaded.declaration().calls
    );

    // 宿主侧消费方零改动：经既有接缝 getter 取到的就是远程门面。
    let tool_set = registry.get_tool_set().expect("tool.set 应已入表");
    let tool = tool_set
        .tools()
        .into_iter()
        .find(|tool| tool.name() == "get_stock_quote")
        .expect("插件贡献的 get_stock_quote 应出现在工具集里");
    assert_eq!(tool.category(), ToolCategory::Integration);
    assert_eq!(
        tool.input_schema()["required"][0],
        "stock_code",
        "宿主看到的 inputSchema 必须带 required.stock_code：{}",
        tool.input_schema()
    );

    let result = runtime
        .block_on(tool.call(json!({ "stock_code": "600519" }), &ToolContext::new(".")))
        .expect("真实链路应执行成功：宿主 → worker → market.quote → worker → 宿主");
    let content: serde_json::Value =
        serde_json::from_str(&result.content).expect("content 应是行情 JSON 字符串");
    assert_eq!(content["price"], 1500.25, "行情必须原样透出：{content:?}");
    assert_eq!(content["code"], "600519");
    assert_eq!(
        result.metadata.as_ref().and_then(|m| m.get("stock_code")).and_then(|v| v.as_str()),
        Some("600519"),
        "metadata.stock_code 必须透出：{:?}",
        result.metadata
    );

    // T2 命令面：同一条 worker 连接上再走一次命令帧（宿主侧真实入口）。
    let reply = loaded
        .execute_command(&CommandFrame::new(
            "plugin.stock",
            "stock_get_quote",
            json!({ "stock_code": "000001" }),
            5_000,
        ))
        .expect("命令帧全链路应成功");
    assert_eq!(reply.plugin_id, "plugin.stock", "回执必须回填宿主绑定的身份");
    assert_eq!(reply.command, "stock_get_quote");
    assert_eq!(reply.value["price"], 1500.25, "命令面行情必须原样透出：{:?}", reply.value);

    // 卸载 = 可逆回滚：句柄撤销后接缝不得残留 provider。
    drop(loaded);
    assert!(registry.get_tool_set().is_none(), "drop 后应回滚 tool.set 注册");
    // `EffectHandle` 没有 `Drop` 实现（只有显式 `undo()`），别把「drop 变量」当撤销。
    market.undo();
    assert!(registry.get_market_data().is_none(), "行情替身句柄 undo 后应回滚");
}
