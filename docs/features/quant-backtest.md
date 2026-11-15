# 量化回测与市场仿真

> 状态：active ｜ 日期：2026-11-04 ｜ 对应版本：v3.0.0 ｜ 功能点：quant-backtest

## 定位

量化能力由两条**互不依赖**的引擎组成，均由 wiring 层（`src-tauri/src/commands/`）对外暴露：

- **`crates/quant`**：日线级策略回测引擎。Rhai 脚本策略、A 股撮合规则、walk-forward 滚动验证、绩效指标对比。
- **`crates/market-sim`**：ABIDES 风格的离散事件市场仿真（DES）。中央限价订单簿、延迟矩阵、多类 Agent（做市商 / 动量 / 价值 / 噪声 / Rhai / 机构 / 事件驱动）。

两者按 Harness 依赖铁律各自**只依赖 `axagent-harness`**：`axagent-quant/crates/quant/Cargo.toml:13`、`axagent-quant/crates/market-sim/Cargo.toml:10`；market-sim 在 `axagent-quant/crates/market-sim/Cargo.toml:23-24` 明确说明「按铁律 5 不依赖 quant，本地 `MockMaCrossStrategy` 替代真实策略」。

## 当前行为

### Rhai 策略执行

- **AST 缓存**：每个 `RhaiStrategy` 实例构造时编译一次脚本，之后复用，避免每次 `on_bar` 重新编译。见 `axagent-quant/crates/quant/src/script.rs:39`、`axagent-quant/crates/quant/src/script.rs:71-72`。
- **收盘价增量缓存**：`closes_cache` 按 bar 递增追加，把历史上 O(n²) 的窗口取数降为 O(1)；检测到长度/内容不一致时整段重建。见 `axagent-quant/crates/quant/src/script.rs:75-80`、`axagent-quant/crates/quant/src/script.rs:131-153`。
- **线程模型**：`on_bar` 通过 `tokio::task::spawn_blocking` 提交，配合 `engine.blocking_lock()` 调用脚本函数，避免阻塞异步运行时。见 `axagent-quant/crates/quant/src/script.rs:186-201`。
- **沙箱限额**：`set_max_expr_depths(64, 64)`、`set_max_operations(20_000)`、`set_max_call_levels(32)`、`set_max_string_size(1_000_000)`（1 MB）、`set_max_array_size(10_000)`、`set_max_map_size(10_000)`。见 `axagent-quant/crates/quant/src/script.rs:241-249`。
- **输出抑制**：用 `on_print(|_| {})` 吞掉脚本 `print`，rhai 1.25 无 `disable_print`。见 `axagent-quant/crates/quant/src/script.rs:250-253`。
- **注入函数**：注册 `sma` / `ema` / `rsi` 与 harness 通用函数。见 `axagent-quant/crates/quant/src/script.rs:254-258`。

（对应版本：v2.8.4，`CHANGELOG.md:1835`）

### walk-forward 滚动验证

- `step = step_days.unwrap_or(test_days)`；当 `step == 0` 时直接返回 `QuantError::WalkForward("step_days must be > 0")`。这是防止 `cursor` 不推进、循环空转而形成 CPU 死锁的硬防护。见 `axagent-quant/crates/quant/src/walkforward.rs:204-210`。
- 多股票同日 train/test 数据泄漏检测。见 `axagent-quant/crates/quant/src/walkforward.rs:214-216`。

（对应版本：v2.8.7，`CHANGELOG.md:1244`；WF + DES 集成命令同版 `CHANGELOG.md:1247`）

### A 股撮合规则

默认参数集中在 `axagent-quant/crates/quant/src/matcher.rs:29-58`：

| 参数 | 默认值 | 说明 |
|---|---|---|
| `commission_rate` | `0.00025` | 佣金万 2.5 |
| `commission_min` | `5.0` | 单笔最低 5 元 |
| `stamp_tax_rate` | `0.0005` | 印花税万 5 |
| `slippage_rate` | `0.0005` | 滑点万 5 |
| `lot_size` | `100` | 100 股整手 |
| `t1_enforced` | `true` | T+1 |
| `limit_check` | `true` | 涨跌停校验 |

（对应版本：当前实现）

### 限价订单簿（LOB）

- 撮合优先级：价格优先 → 同价时间优先。买盘用 `BTreeMap<Reverse<Price>, _>` 实现降序，卖盘用 `BTreeMap<Price, _>` 升序；`order_index: HashMap<OrderId, Locator>` 支持 O(1) 撤单定位。见 `axagent-quant/crates/market-sim/src/orderbook.rs:1-19`。
- **成交历史上限**：`MAX_TRADE_HISTORY = 100_000`，超出丢弃最旧记录，防止长时仿真 OOM。见 `axagent-quant/crates/market-sim/src/orderbook.rs:31`。
- **自成交保护**：同 `agent_id` 的挂单不会与自身成交；命中时把该订单 rotate 到队尾并以 `self_trade_skips` 计数跳过，整档位全为自成交时跳出本档。见 `axagent-quant/crates/market-sim/src/orderbook.rs:463-480`。
- **涨跌停边界**：不在 LOB 层处理，由上层 Agent 在发单前拦截。见 `axagent-quant/crates/market-sim/src/orderbook.rs:19`。

（对应版本：v2.8.4，`CHANGELOG.md:1835`）

### 离散事件内核（DES）

- 执行模型：初始化 Agent（`on_init`）→ 优先级队列弹事件 → 推进虚拟时钟 → 投递消息 → 收集动作 → 调度新事件 → 到界停止。见 `axagent-quant/crates/market-sim/src/kernel.rs:1-22`。
- **panic 隔离**：`poisoned_agents: HashSet<String>` 记录已 panic 的 Agent，后续消息直接丢弃。见 `axagent-quant/crates/market-sim/src/kernel.rs:143-146`。
- `on_init` 用 `std::panic::catch_unwind(AssertUnwindSafe(...))` 包裹，panic 后写入毒化名单并返回空动作。见 `axagent-quant/crates/market-sim/src/kernel.rs:303-329`；`on_message` 同族隔离见 `axagent-quant/crates/market-sim/src/kernel.rs:357` 附近。

（对应版本：v2.8.7，`CHANGELOG.md:1244`）

### 多 Agent 仿真

Agent 模块清单见 `axagent-quant/crates/market-sim/src/agent/mod.rs:3-12`，`pub mod` 声明见 `:14-25`，对外 re-export 见 `:27-38`。共 12 个模块：`background`、`event_driven`、`exchange`、`institutional`、`market_maker`、`momentum`、`noise`、`quant_bridge`、`rhai`、`strategy`、`traits`、`value`。

- 仿真时钟上限：`max_time_ns = 600_000_000_000`（10 分钟）——防止多个 Agent `WakeupAfter` 自续约形成死循环。见 `axagent-quant/crates/market-sim/src/config.rs:11-42`。
- 其余默认：`seed = 42`、`stock_code = "000001"`、`reference_price = 1000`（10.00 元）、`tick_size = 1`、`default_latency_ns = 1_000_000`（1 ms）、`trace = false`。同处 `axagent-quant/crates/market-sim/src/config.rs:11-42`。

（对应版本：当前实现）

### Oracle 外生信号

- **Box-Muller 正态步进**：`u1`、`u2` 取自 `(0.0001..1.0)`，`normal = (-2 ln u1).sqrt() * cos(2π u2)`，再乘日波动率与 `dt_days.sqrt()`。见 `axagent-quant/crates/market-sim/src/oracle.rs:113-122`；`DriftOracle` 同构实现见同文件下方。
- **数值兜底**：`current_fv` 非有限或为负时记 warn 并重置为 `reference_price`，返回兜底 `fundamental_value = 1`，避免污染下游 Agent。见 `axagent-quant/crates/market-sim/src/oracle.rs:125-130`。
- **可复现性**：默认构造用 `StdRng::from_entropy()`；需要 Monte Carlo 可复现时使用 `with_seed` / `flash_crash_with_seed` / `high_volatility_with_seed`。见 `axagent-quant/crates/market-sim/src/oracle.rs:88-96`（`with_seed`）。

（对应版本：当前实现）

### 对外命令（后端）

**`src-tauri/src/commands/quant_backtest.rs`**

| 命令 | 行号 |
|---|---|
| `quant_backtest_run` | `src-tauri/src/commands/quant_backtest.rs:151` |
| `quant_run_get` | `src-tauri/src/commands/quant_backtest.rs:241` |
| `quant_strategies_list` | `src-tauri/src/commands/quant_backtest.rs:464` |
| `quant_strategy_register_rhai` | `src-tauri/src/commands/quant_backtest.rs:491` |
| `quant_metrics_compare` | `src-tauri/src/commands/quant_backtest.rs:570` |

请求/响应 DTO 定义在文件头部 `src-tauri/src/commands/quant_backtest.rs:1-80`。

**`src-tauri/src/commands/backtest_validation.rs`**

| 命令 | 行号 |
|---|---|
| `serenity_validation_summary` | `src-tauri/src/commands/backtest_validation.rs:987` |
| `create_decision_backtest_cron` | `src-tauri/src/commands/backtest_validation.rs:1415` |

`run_decision_backtest` / `list_decision_validations` / `compute_validation_report` 三条宿主命令已迁 T2（#324）：插件命名命令 `stock_run_decision_backtest` / `stock_list_decision_validations` / `stock_compute_validation_report` 经接缝 `stock.analysis` 同名 op 回宿主，实现体保留在本文件 —— `run_decision_backtest_inner`（`src-tauri/src/commands/backtest_validation.rs:178`）、`list_decision_validations`（`:845`）、`compute_validation_report`（`:1070`）。

（对应版本：当前实现）

### 因子信号契约

权威定义在 `axagent-plugins/crates/stock-core/src/factors.rs`（2026-11-08 随 #35 首刀整文件迁入 foundation 层 `axagent-stock-core`；`analysis-engine` 保留路径级 re-export，旧路径 `axagent_analysis_engine::factors::…` 不变）：

- 版本号：`FACTOR_VERSION = 1.0`，见 `axagent-plugins/crates/stock-core/src/factors.rs:36`。
- 窗口：`WINDOW = 20`，见 `:39`；最少 bar 数：`MIN_BARS = 21`（`rev20` 需要 n 与 n-21 两根，见 `:41-46`）。
- 四个因子字段：`chg1`（`:63`）、`dist_lo20`（`:65`）、`rev20`（`:67`）、`posshare20`（`:69`）。
- 序列化输出：四个因子 + `factor_version` 共 5 个键，见 `axagent-plugins/crates/stock-core/src/factors.rs:79-83`。

（对应版本：v2.9.9，`AUDIT-f-factor-signal-contract-2026-09-23.md`）

## 关键代码入口

**后端命令**

- `src-tauri/src/commands/quant_backtest.rs`、`src-tauri/src/commands/backtest_validation.rs`（见上表）。

**引擎**

- Rhai 执行器：`axagent-quant/crates/quant/src/script.rs`
- 滚动验证：`axagent-quant/crates/quant/src/walkforward.rs`
- 撮合器：`axagent-quant/crates/quant/src/matcher.rs`
- 订单簿：`axagent-quant/crates/market-sim/src/orderbook.rs`
- DES 内核：`axagent-quant/crates/market-sim/src/kernel.rs`
- 仿真配置：`axagent-quant/crates/market-sim/src/config.rs`
- Agent 集合：`axagent-quant/crates/market-sim/src/agent/mod.rs`
- Oracle：`axagent-quant/crates/market-sim/src/oracle.rs`
- 因子契约：`axagent-plugins/crates/stock-core/src/factors.rs`

**前端**

- 回测页面与策略矩阵：由 `BacktestPage` 承载（页面路由与 store 不在本次取证范围，未登记具体文件）。

**数据表 · 迁移**

- schema 迁移清单已清空（`MIGRATIONS = &[]`），建表由声明式引擎负责，见 `storage-migration.md`。回测结果表未在本次取证范围内定位。

## 配置与开关

- **仿真配置**：`SimConfig` 全字段与默认值见 `axagent-quant/crates/market-sim/src/config.rs:11-42`；`LatencyMatrix` 支持「精确 Agent 对 → 类型对 → 默认延迟」三级回退（同文件 `:44-97`）。
- **回测参数**：由命令 DTO 传入（`src-tauri/src/commands/quant_backtest.rs:1-80`），包含策略脚本、区间、初始资金等。
- **策略注册**：`quant_strategy_register_rhai`（`src-tauri/src/commands/quant_backtest.rs:491`）在运行时注册 Rhai 策略。
- **因子版本开关**：`factor_version` 随因子快照落库，用于区分不同版本的 IC 统计（`axagent-plugins/crates/stock-core/src/factors.rs:83`）。
- 无独立 feature flag：模块按 crate 编译期存在。

## 已知限制与坑

1. **`step_days = 0` 会直接报错**，不是静默默认——调用方必须显式给出正数步长，见 `axagent-quant/crates/quant/src/walkforward.rs:204-210`。
2. **沙箱 20 000 操作上限**可能误杀复杂脚本；脚本越复杂越应先拆分为多策略。
3. **脚本 `print` 被完全吞掉**，脚本内调试信息无输出（`axagent-quant/crates/quant/src/script.rs:250-253`）。
4. **自成交保护只 rotate 不撤单**：订单仍留在簿内，`self_trade_skips` 仅用于跳过与跳出（`axagent-quant/crates/market-sim/src/orderbook.rs:463-480`）。
5. **成交历史截断**：超过 100 000 条丢弃最旧记录，长回测的早期成交统计会失真（`axagent-quant/crates/market-sim/src/orderbook.rs:31`）。
6. **涨跌停不在 LOB 层**：若上层 Agent 未拦截，限价单可穿透涨跌停（`axagent-quant/crates/market-sim/src/orderbook.rs:19`）。
7. **Oracle 默认不可复现**：默认 `StdRng::from_entropy()`；需要稳定结果必须使用带 `seed` 的构造（`axagent-quant/crates/market-sim/src/oracle.rs:88-96`）。
8. **双重撮合语义**：quant 的 `matcher.rs` 是组合级日线撮合（含费用/整手/T+1），market-sim 的 `orderbook.rs` 是逐笔 LOB 撮合（不含费用），两套参数不可混用。
9. **旧文档「印花税 0.001」与代码不符**：实际 `stamp_tax_rate = 0.0005`（`axagent-quant/crates/quant/src/matcher.rs:52`）。
10. **决策回测存在因子快照缺口**：旧版命中率回测路径的 `factor_snapshot` 仍为 `None`，遇到 `None` 直接跳过 IC 计算；serenity 路径才注入因子。相关行见 `axagent-quant/crates/analysis-engine/src/hit_rate_backtest.rs:663`、`axagent-quant/crates/analysis-engine/src/hit_rate_backtest.rs:564`、`axagent-quant/crates/analysis-engine/src/serenity_hit_rate_backtest.rs:311`。

## 版本变更记录

| 版本 | 日期 | 变更 | 证据 |
|---|---|---|---|
| v2.8.4 | 2026-07-12 | 股票分析、量化回测、市场模拟全功能合入 | `CHANGELOG.md:1835` |
| v2.8.7 | 2026-08-02 | WalkForward 基线测试 + WF/DES 集成命令 | `CHANGELOG.md:1244` |
| v2.9.9 | 2026-09-23 | 移除 executionBridge，回测侧与真实交易桥解耦 | `CHANGELOG.md:482` |
| 未发布 | — | 按技术层拆分为 8 个 workspace 根，quant/market-sim 归 `axagent-quant` | `CHANGELOG.md:119` |

## 相关文档

| 文档 | 状态 | 版本 | 关联 |
|---|---|---|---|
| `AUDIT-baseline-comparison-2026-10-25.md` | active | v2.9.9 | 回测基线对比 |
| `AUDIT-factor-strategy-counterfactual-2026-10-25.md` | active | v2.9.9 | 因子策略反事实分析 |
| `AUDIT-sample-validity-2026-10-25.md` | active | v2.9.9 | 样本有效性 |
| `AUDIT-f-factor-signal-contract-2026-09-23.md` | active | v2.9.9 | 因子信号契约 |
| `AUDIT-metrics-definition-2026-09-20.md` | active | v2.9.7 | 绩效指标定义 |
| `PLAN-market-sim-value.md` | active | v2.9.7 | 市场仿真价值定位 |
| `PLAN-refactor.md` | 未标注 | v2.9.5 | 引擎重构计划 |
| `PLAN-unity-p2-runtime-permission-and-hybrid-classifier.md` | 未标注 | v2.9.5 | 运行时权限与混合分类器 |
| `PROPOSAL-cognitive-router-design.md` | 未标注 | v2.9.3 | 认知路由设计 |
| `PROPOSAL-afsim-axagent-distributed-architecture.md` | 未标注 | v1.4.4 | 分布式仿真架构 |
