# 行情与数据源

> 状态：active ｜ 日期：2026-11-04 ｜ 对应版本：v3.0.0 ｜ 功能点：market-data

## 定位

覆盖「把行情与个股数据取进来」这一层：多 vendor 适配与路由、跨市场（A 股 / 美股 / 港股）分流、抓取失败后的降级与健康回退、gzip 与编码异常处理、K 线缓存与复权、交易日历与非交易日闸门，以及收盘快照/市场主线的落库面。

不含：数据质量评分与 DQI 熔断（归 `data-quality-gates`）、决策与荐股口径（归 `stock-decision-chain` / `serenity-screening`）。`stock_pipeline` 只在其「取数并发随 vendor 健康度降档」这一面归本功能点，管道内的决策筛选逻辑不归。

## 当前行为

### 多源适配与统一契约

- 契约侧：`axagent-core/crates/harness/src/market_data.rs:266` 的 `trait MarketDataProvider`，行情入口 `get_quote` 在 `:268`，市场类型判定 `detect_market_type` 在 `:814`。
- 实现侧：`axagent-quant/crates/astock-data/src/lib.rs:6641` 的 `impl axagent_harness::market_data::MarketDataProvider for AStockClient`；`AStockClient` 结构在 `:373`，内含 moka L1 缓存字段（`:379`）。
- 路由表：`axagent-quant/crates/astock-data/src/lib.rs:125` 的 `struct VendorRouting`，`impl` 在 `:172`，默认路由 `default_routing()` 在 `:188`——按「数据面 × 市场」决定 vendor 顺序，quote 与 klines 走不同链。
- 个股数据面方法在 v3.0.0 的 `unreleased` 阶段扩到 25 个，且显式带 `as_of`（提交 `dd96731d6`、`0e55104b8`）；DTO 上收 harness 由 `e45649bd3` 完成。

（对应版本：v3.0.0，当前实现；数据面扩展见 CHANGELOG `[unreleased]`）

### 跨市场分流（A / US / HK）

- A 股代码归一与交易所分档在 `axagent-quant/crates/astock-data/src/code_form.rs:173`（注释明确「港股/美股标记不属于 A 股交易所，留给调用方的 HK/US 分支」）。
- 美股个股符号归一 `us_stock_symbol` 在 `axagent-quant/crates/astock-data/src/code_form.rs:108`：`AAPL` / `AAPL.US` / `105.AAPL` → `AAPL`，覆盖东财 secid 的三个美股市场位（`code_form.rs:113`）。
- 分流原因写在 `code_form.rs:104-106`：腾讯 `kline/kline` 对美股个股只回 2 根日线（实测 `usAAPL` / `usTSLA` / `usMSFT` 均为「2011-06-02 + 最新一日」），只能由新浪美股日线承接。
- 国际指数别名（`SPX` / `HSI` 等）与美股个股不是同一条通道，`code_form.rs:93` 明确「取数通道与个股不同，不可当个股处理」。

（对应版本：v3.0.0，当前实现）

### 抓取降级与健康回退

- 健康追踪：`axagent-quant/crates/astock-data/src/lib.rs:380` 注释「连续失败 3 次自动降级，5 分钟后恢复」，跟踪器类型来自 `vendor_health` 模块（`lib.rs:42`、`:57`）。
- 运行时探活：`check_vendor_health` 在 `axagent-quant/crates/astock-data/src/lib.rs:2401`；健康状态持久化在同文件 `:2360` 附近。
- 管道并发降档：`src-tauri/src/commands/stock_pipeline/core.rs:117` 的 `resolve_concurrency_with_vendor_health`，按健康供应商占比折半或退化为串行（单测 `:710`、`:728`、`:745` 锁住三档）。
- 部分成功不等于失败：`src-tauri/src/commands/stock_workflow/decision.rs:39` 的 `SourceCheck` 区分 `Ok` / `Partial` / `Failed`，聚合在 `:72`——`Partial` 拼原因、`Failed` 才计入缺失源（`:78`、`:103`）。

（对应版本：v3.0.0，当前实现）

### gzip 与编码异常

- emweb 财报接口经 CDN 后同一 URL 会随机返回 gzip 变体，不开自动解压则 `resp.json()` 解码失败、整链假性全灭：`axagent-quant/crates/astock-data/src/lib.rs:796-798`（L 轮，2026-09-28，002371 实证）；另一处客户端构造同样开 `gzip(true)` 在 `:659-660`。
- 历史事故记录：CHANGELOG v3.0.0「emweb 财报接口 gzip 变体炸穿解码——reqwest 开自动解压（L 轮，002371 实证）」。
- 「返回数据为空」按答案处理，不记红档取数失败：CHANGELOG v3.0.0「质押报表『返回数据为空』按答案处理，不再记成红档取数失败（as-of T18）」。

（对应版本：v3.0.0，当前实现）

### K 线缓存与复权

- L1：moka 缓存，1h idle 过期、4096 条上限（`axagent-quant/crates/astock-data/src/lib.rs:814-815`；结构字段 `:379`）。
- L2：`DiskCache`（`axagent-quant/crates/astock-data/src/disk_cache.rs:60`），默认容量 10_000 条（`:22`）、磁盘预算 64 MiB（`:34`）；replay 模式 TTL 由 `cache_set` 截到 1h（模块头 `:10`）。读写入口 `get` 在 `:169`、`get_with_expiry` 在 `:179`。
- 复权：K 线查询支持 `AdjType`（`axagent-quant/crates/astock-data/src/lib.rs:2937`）；vendor 支持复权时回传 `adj_factor`，不支持（sina/163）则由本地 `compute_adj_factors` + `apply_adjustment` 兜底（`:2957-2958`）。
- 缓存键必须带复权维度：`None`（不复权）与 `Forward` 不能共用键，否则不复权原始价会污染前复权请求（`lib.rs:1099-1102`，`v2` 版本号让旧 `fwd` 键整体失效）。

（对应版本：v3.0.0，当前实现）

### 交易日历与非交易日闸门

- 日历本体：`axagent-core/crates/harness/src/trading_calendar.rs:64` 的 `is_trading_day(&NaiveDate)`，`previous_trading_day` 在 `:172`；模块头 `:7` 说明它只是本地判据的薄封装，trait 里的 `is_trading_day(&self, &str)` 是网络查询语义，二者不得混用。
- 非交易日闸门：调度器在 `src-tauri/src/init/services.rs:2341` 起，只拦「本轮产物依赖当日行情」的任务；`batch-reflection` 等不受影响（`:2360`）。命中非交易日时**记 run 并标跳过**（`:2352`、`:2369`、`:2376`）。

（对应版本：v3.0.0，当前实现）

### 收盘快照与市场主线

- 收盘快照：`src-tauri/src/commands/market_close_sweep.rs:26` 的 `sweep_market_close`；全市场universe 刷新 `:96`；覆盖率视图 `:126`；历史回填 `:172`（默认 90 天，`:177`）。
- 市场主线：`src-tauri/src/commands/market_mainline.rs:59` 起是主线数据的落库/查询面（create/batch_upsert/update/archive/delete_by_date/get/list 系列，`:73`-`:182`）。
- 股票管道：`src-tauri/src/commands/stock_pipeline/core.rs:82` 的 `PipelineConfig`、`run_stock_pipeline_inner` 在 `:219`、命令入口 `:552`、历史 `:592`、明细 `:632`。

（对应版本：v3.0.0，当前实现）

## 关键代码入口

- 契约：`axagent-core/crates/harness/src/market_data.rs:266`（`MarketDataProvider`）、`:710`（`detect_market_type`）。
- 引擎/实现：`axagent-quant/crates/astock-data/src/lib.rs:373`（`AStockClient`）、`:6641`（trait 实现）、`:188`（默认路由）、`:2877`（复权 K 线）、`:2344`（健康探活）。
- 跨市场：`axagent-quant/crates/astock-data/src/code_form.rs:108`（美股符号）、`:173`（A 股交易所分档）。
- 缓存：`axagent-quant/crates/astock-data/src/disk_cache.rs:60`（`DiskCache`）。
- 后端命令：`src-tauri/src/commands/market_close_sweep.rs:26`、`src-tauri/src/commands/market_mainline.rs:59`、`src-tauri/src/commands/stock_pipeline/core.rs:552`。
- 前端：`src/components/stock-analysis/`（行情面板与个股数据面消费；具体组件按功能页挂载）。
- 数据表/迁移：行情快照与主线表由 `src-tauri/src/commands/market_close_sweep.rs` 与 `market_mainline.rs` 的 DAO 调用落库，迁移在 `axagent-infra/crates/migration/`。

## 配置与开关

- vendor 健康阈值：连续失败 3 次降级、5 分钟恢复（`axagent-quant/crates/astock-data/src/lib.rs:380`）；具体常量在 `axagent-quant/crates/astock-data/src/vendor_health.rs`。
- 缓存参数：L1 1h idle / 4096 条（`lib.rs:814-815`）；L2 10_000 条 / 64 MiB / replay TTL ≤1h（`disk_cache.rs:22`、`:34`、`:10`）。
- 路由开关：`VendorRouting::default_routing()`（`lib.rs:188`）是 vendor 顺序的唯一来源；`baidu_stock` 的所有引用已于 2026-10-01 摘除（`lib.rs:754` 注释）。
- 回填窗口：`TARGET_DAYS: u32 = 90`（`src-tauri/src/commands/market_close_sweep.rs:177`）。
- i18n：未发现独立 `market.*` 顶层段，行情文案分散在 `stockAnalysis.*` 与设置页（**待确认**：是否有未收口的前端硬编码）。

## 已知限制与坑

1. **腾讯对美股个股只有 2 根日线** —— 必须走新浪美股通道，判据在 `axagent-quant/crates/astock-data/src/code_form.rs:104-106`（实测 `usAAPL` / `usTSLA` / `usMSFT`）。
2. **前复权链可能双断** —— 腾讯 `fqkline` WAF + `push2his` RST 同时发生时 30s 预算必破（`axagent-quant/crates/astock-data/src/lib.rs:2911`）。
3. **复权缓存键曾混用** —— `None` 与 `Forward` 共用键会让不复权原始价污染前复权请求（`lib.rs:1099-1102`）；改键后必须带 `v2` 令旧键失效。
4. **emweb gzip 随机变体** —— 不开 `gzip(true)` 会整链假性全灭（`lib.rs:796-798`）。
5. **「返回数据为空」不是红档失败** —— 曾把它记成取数失败，见 CHANGELOG v3.0.0（as-of T18）。
6. **并发降档要读健康度** —— 直接用 `PipelineConfig.concurrency` 会在供应商大面积降级时把上游打满（`src-tauri/src/commands/stock_pipeline/core.rs:117`）。

## 版本变更记录

| 版本 | 日期 | 变更 | 证据 |
|---|---|---|---|
| v2.8.7 | 2026-08-02 | 完善行情数据源与估值报告逻辑；新增股吧数据源 | `CHANGELOG.md` v2.8.7 节「完善行情数据源与估值报告逻辑」「投资策略包 + 通知渠道 + 仪表盘报告 + 股吧数据源」 |
| v3.0.0 | 2026-10-31 | astock-data 行情源补强；as-of 取数修复；emweb gzip 自动解压；降级文案诚实化 | `CHANGELOG.md` v3.0.0 节「astock-data 行情源补强」「荐股信号质量收口 + 全市场发现 + as-of 取数修复」「emweb 财报接口 gzip 变体炸穿解码」「降级文案诚实化（as-of T5/T6）」 |
| v3.0.0（unreleased 累积） | 2026-11-01~03 | 行情 DTO 上收 harness（40 struct + KLinePeriod）；`MarketDataProvider` 扩 12 + 25 个数据面方法（显式 `as_of`） | `CHANGELOG.md` `[unreleased]` 节；提交 `e45649bd3`、`0e55104b8`、`dd96731d6` |

## 相关文档

| 文档 | 状态 | 版本 | 关联 |
|---|---|---|---|
| `AUDIT-data-outcome-coverage-2026-10-28.md` | active | v2.9.9 | 取数结果覆盖度审计 |
| `AUDIT-stock-live-data-2026-10-24.md` | done | v2.9.9 | 实盘取数链路 |
| `AUDIT-stock-data-horizons-2026-09-23.md` | active | v2.9.9 | 四周期数据面 |
| `PLAN-emweb-gzip-decode-failure.md` | active | v2.9.9 | gzip 解码缺陷修复 |
| `PLAN-live-kline-cache-calendar-storm.md` | active | v2.9.9 | K 线缓存 + 日历风暴 |
| `PLAN-industry-ranking-asof-gap.md` | active | v2.9.9 | 行业排名 as-of 缺口 |
| `PLAN-concept-block-change-pct-null.md` | active | v2.9.9 | 板块涨跌幅空值语义 |
| `PLAN-margin-designified-absent-semantics.md` | active | v2.9.9 | 融资标记缺失语义 |
| `PROPOSAL-advanced-rag-design.md` | 已确认 | v1.5.2 | 边缘：关键词命中，与行情取数弱关联（待复核归属） |
