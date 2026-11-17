# 模拟持仓与组合风控

> 状态：active ｜ 日期：2026-11-04 ｜ 对应版本：v3.0.0 ｜ 功能点：paper-portfolio

## 定位

「模拟持仓」是一套**虚拟组合簿记**：把决策/资讯/截图诊断产生的标的按事件日价格记入 `paper_positions`，用最新行情价计算浮动与已实现盈亏，供观察后市表现。它不连接真实账户、不产生委托，也**不是**回测引擎——与 `quant` 的撮合、`market-sim` 的订单簿没有数据通路。

模块自述见 `axagent-quant/crates/analysis-engine/src/paper_portfolio.rs:14`：「本模块：管理**虚拟**组合（`paper_portfolios` / `paper_positions` 表）」。

## 当前行为

### 组合生命周期

引擎提供完整的组合增删改查与归档：

| 能力 | 函数 | 行号 |
|---|---|---|
| 创建组合 | `create_portfolio` | `axagent-quant/crates/analysis-engine/src/paper_portfolio.rs:146` |
| 列出组合 | `list_portfolios` | `axagent-quant/crates/analysis-engine/src/paper_portfolio.rs:167` |
| 读取组合 | `get_portfolio` | `axagent-quant/crates/analysis-engine/src/paper_portfolio.rs:179` |
| 关闭组合 | `close_portfolio` | `axagent-quant/crates/analysis-engine/src/paper_portfolio.rs:187` |
| 归档组合 | `archive_portfolio` | `axagent-quant/crates/analysis-engine/src/paper_portfolio.rs:202` |
| 组合详情（含盈亏） | `get_portfolio_detail` | `axagent-quant/crates/analysis-engine/src/paper_portfolio.rs:303` |
| 活跃组合列表（含盈亏） | `list_active_portfolios_detail` | `axagent-quant/crates/analysis-engine/src/paper_portfolio.rs:323` |

- 组合来源有三类，对应实体字段 `source_event`、`source_news_id`、`source_screenshot_diagnosis_id`（`axagent-core/crates/entities/src/paper_portfolios.rs:23-28`）：
  - 手工创建（`create_portfolio`）；
  - 由资讯创建（`create_portfolio_from_news`，`axagent-quant/crates/analysis-engine/src/paper_portfolio.rs:422`）；
  - 由截图诊断创建（`create_portfolio_from_screenshot_diagnosis`，`axagent-quant/crates/analysis-engine/src/paper_portfolio.rs:441`）。
- 状态字段 `status` 与关闭时间 `closed_at` 见 `axagent-core/crates/entities/src/paper_portfolios.rs:31-35`。

（对应版本：v2.8.7，`CHANGELOG.md:1253`）

### 仓位簿记

| 能力 | 函数 | 行号 |
|---|---|---|
| 加仓（虚拟建仓） | `add_position` | `axagent-quant/crates/analysis-engine/src/paper_portfolio.rs:217` |
| 列出持仓 | `list_positions` | `axagent-quant/crates/analysis-engine/src/paper_portfolio.rs:243` |
| 平仓 | `close_position` | `axagent-quant/crates/analysis-engine/src/paper_portfolio.rs:255` |
| 全平 | `close_all_positions` | `axagent-quant/crates/analysis-engine/src/paper_portfolio.rs:272` |

- **持仓表结构**（`axagent-core/crates/entities/src/paper_positions.rs:12-44`）：`id`（`:16`）、`portfolio_id`（`:19`）、`symbol`（`:22`）、`market`（`:25`）、`entry_price`（`:27`）、`entry_date`（`:29`）、`quantity`（`:31`）、`exit_price`（`:33`）、`exit_date`（`:35`）、`status`（`:38`）、`note`（`:40`）、`created_at`（`:42`）、`updated_at`（`:44`）。
- **盈亏计算**：引擎返回 `PositionWithPnl`，字段为 `unrealized_pnl` / `unrealized_pnl_pct` / `realized_pnl` / `realized_pnl_pct`（`axagent-quant/crates/analysis-engine/src/paper_portfolio.rs:106-118`）；组合级汇总为 `total_unrealized_pnl` / `total_realized_pnl` / `total_return_pct`（`:136-140`）。
- **平仓校验**：`close_position` 会校验 `exit_price` 合法性，避免 0 或负数导致已实现盈亏异常（`axagent-quant/crates/analysis-engine/src/paper_portfolio.rs:278`）。

（对应版本：v2.8.7，`CHANGELOG.md:1253`）

### 决策与仓位落库

- 组合模板与决策输出由 `src-tauri/src/commands/portfolio-mgr.rhai` 承载，输出字段含 `positionPct`、`action`。
- 模板版本常量 `TEMPLATE_VERSION = 194` 定义在 `src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs:1643`。
- 决策字段提取与落库在 `src-tauri/src/commands/stock_workflow/decision.rs:909-915`；落库目标为 `stock_analyses` 的决策字段（`decision_json`、`decision_action`、`decision_position_pct`、`decisions_by_horizon`），实体定义见 `axagent-core/crates/entities/src/stock_analyses.rs:32-49`。
- 语义：Rhai 决策产出的是**建议仓位与动作**，写入分析记录；模拟持仓是另一条独立链路，按需由决策结果显式建仓（`add_position`），二者不自动联动。

（对应版本：当前实现）

### 风控门

- **限额默认值**（`axagent-quant/crates/analysis-engine/src/position_limits.rs:28-95`）：

| 限额 | 默认值 |
|---|---|
| `max_single_stock_pct` | `20.0`（单票上限 20%） |
| `max_sector_exposure_pct` | `40.0`（行业暴露上限 40%） |
| `max_total_positions` | `10`（持仓总数上限 10） |

- **风险等级收紧**：Low / Medium / High 逐级收紧限额；极高风险直接禁止新开仓。构造入口 `from_risk_str` / `panel_effective` 见同文件 `:28-95`。
- **风控裁决函数**：`portfolio_risk_gate`（`axagent-quant/crates/analysis-engine/src/portfolio_formula.rs:680-750`）：
  - 单票超 `max_single_stock_pct` ⇒ 拒绝或裁剪；
  - 行业暴露超 `max_sector_exposure_pct` ⇒ 拒绝或裁剪；
  - 总持仓数超 `max_total_positions` ⇒ 拒绝新开仓；
  - 看空目标价比例阈值 `BEARISH_TARGET_PRICE_RATIO = 0.85`；
  - 风险等级为高/极高 ⇒ `forbid_new_position`；
  - 权重归一化由 `normalize_position_weights` 完成。
- **Rhai 风控门**：`src-tauri/src/commands/portfolio-risk-gate.rhai:1-40` 直接调用 Rust 的 `portfolio_risk_gate`，把裁决结果回传给工作流。

（对应版本：当前实现）

### 对外命令（后端）

全部命令在 `src-tauri/src/commands/paper_portfolio.rs`，统一返回 `Result<T, String>`：

| 命令 | 行号 |
|---|---|
| `paper_portfolio_create` | `src-tauri/src/commands/paper_portfolio.rs:28` |
| `paper_portfolio_list` | `src-tauri/src/commands/paper_portfolio.rs:43` |
| `paper_portfolio_get` | `src-tauri/src/commands/paper_portfolio.rs:58` |
| `paper_portfolio_close` | `src-tauri/src/commands/paper_portfolio.rs:79` |
| `paper_portfolio_archive` | `src-tauri/src/commands/paper_portfolio.rs:94` |
| `paper_portfolio_add_position` | `src-tauri/src/commands/paper_portfolio.rs:109` |
| `paper_portfolio_close_position` | `src-tauri/src/commands/paper_portfolio.rs:124` |
| `paper_portfolio_close_all_positions` | `src-tauri/src/commands/paper_portfolio.rs:139` |
| `paper_portfolio_list_active_details` | `src-tauri/src/commands/paper_portfolio.rs:158` |

（对应版本：当前实现）

### 与回测 / 市场仿真的边界

| 维度 | 模拟持仓（paper-portfolio） | 量化回测（quant） | 市场仿真（market-sim） |
|---|---|---|---|
| 价格来源 | 最新行情价（用于计算 PnL） | 日线 bar 序列 | 逐笔限价订单簿 |
| 撮合 | 无撮合，按事件价直接记账 | `matcher.rs` 组合级撮合 | `orderbook.rs` 逐笔撮合 |
| 费用 | 无佣金/印花税/滑点建模 | 佣金/印花税/滑点/整手/T+1 | 无费用（仅 LOB 规则） |
| 资金 | 无现金字段，不存在现金扣减 | 初始资金 + 组合权益 | 无账户概念，Agent 自行记账 |
| 交易桥 | **无**（`execution_bridge` 已移除） | 无 | 无 |

- `paper_portfolios` 实体只有 `id` / `name` / `source_*` / `status` / `created_at` / `closed_at`（`axagent-core/crates/entities/src/paper_portfolios.rs:15-35`），**不含 `initial_cash` 或 `description`**——因此不存在「初始资金扣减」这一语义。
- `execution_bridge` 模块与订单/撤单命令已不存在；`CHANGELOG.md:482` 记录 v2.9.9「移除 executionBridge」。当前模拟持仓**不与任何真实交易通道连接**。

（对应版本：v2.9.9，`CHANGELOG.md:482`）

## 关键代码入口

**后端命令**

- `src-tauri/src/commands/paper_portfolio.rs`（9 条命令，见上表）
- 风控门：`src-tauri/src/commands/portfolio-risk-gate.rhai`
- 组合模板：`src-tauri/src/commands/portfolio-mgr.rhai`

**引擎**

- 组合/持仓引擎：`axagent-quant/crates/analysis-engine/src/paper_portfolio.rs`
- 限额模型：`axagent-quant/crates/analysis-engine/src/position_limits.rs`
- 风控裁决：`axagent-quant/crates/analysis-engine/src/portfolio_formula.rs`

**前端**

- 模拟持仓面板组件（`PaperPortfolioDashboard`）与 `paperPortfolioStore` 存在于 `src/components/`、`src/stores/feature/` 下，本次未逐路径取证。

**数据表 · 迁移**

- `axagent-core/crates/entities/src/paper_portfolios.rs`（表 `paper_portfolios`，`:15`）
- `axagent-core/crates/entities/src/paper_positions.rs`（表 `paper_positions`，`:12`）
- 建表由声明式 schema 收敛负责（迁移清单已清空），见 `storage-migration.md`。

## 配置与开关

- **组合来源**：`source_event` 三选一（手工 / `source_news_id` / `source_screenshot_diagnosis_id`），见 `axagent-core/crates/entities/src/paper_portfolios.rs:23-28`。
- **风控限额**：`max_single_stock_pct = 20.0`、`max_sector_exposure_pct = 40.0`、`max_total_positions = 10`，并按风险等级动态收紧（`axagent-quant/crates/analysis-engine/src/position_limits.rs:28-95`）。
- **看空阈值**：`BEARISH_TARGET_PRICE_RATIO = 0.85`（`axagent-quant/crates/analysis-engine/src/portfolio_formula.rs:680-750`）。
- **模板版本**：`TEMPLATE_VERSION = 194`（`src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs:1643`）。
- **无 feature flag**：模块随 `analysis-engine` 编译期存在。

## 已知限制与坑

1. **不存在现金模型**：组合表无 `initial_cash`，持仓表无 `cost_price` / `current_price` 落库；浮动盈亏是读取时按最新价计算，不持久化（`axagent-core/crates/entities/src/paper_portfolios.rs:15-35`、`axagent-core/crates/entities/src/paper_positions.rs:12-44`）。
2. **不计交易成本**：佣金、印花税、滑点全部未建模，模拟收益天然高于含费回测。
3. **无撮合与成交概率**：`add_position` 按给定价直接记账，不判断盘口深度，也不受涨跌停约束。
4. **决策与建仓不自动联动**：Rhai 决策只写 `stock_analyses` 的决策字段，是否建仓需显式调用 `paper_portfolio_add_position`。
5. **交易桥已移除**：`execution_bridge` 不存在，`CHANGELOG.md:482` v2.9.9 移除；不要按旧文档寻找订单/撤单命令。
6. **平仓价必须为正**：`close_position` 校验 `exit_price`，0 或负数会被拒（`axagent-quant/crates/analysis-engine/src/paper_portfolio.rs:278`）。
7. **限额是「面板有效值」**：`position_limits` 的默认值会被风险等级覆盖，排查拒单时先看当前风险等级而非只看默认 20%/40%/10。
8. **归档 ≠ 关闭**：`archive_portfolio` 只改状态，不结算持仓；结算语义在 `close_portfolio` / `close_all_positions`。

## 版本变更记录

| 版本 | 日期 | 变更 | 证据 |
|---|---|---|---|
| v2.8.7 | 2026-08-02 | DojoAgents P0 阶段对齐 — 跨市场 / 主线 / 模拟组合 / 截图诊断 | `CHANGELOG.md:1253` |
| v2.8.7 | 2026-08-02 | 补 paper / reading_list 三枚错误码到 `error_code.rs` 及多语言翻译 | `CHANGELOG.md:1415` |
| v2.9.9 | 2026-09-23 | 移除 `executionBridge`，模拟持仓与真实交易通道彻底解耦 | `CHANGELOG.md:482` |

## 相关文档

暂无。`docs/INDEX.md` 中「模拟持仓」分组当前挂靠文档数为 0，未凑数登记。
