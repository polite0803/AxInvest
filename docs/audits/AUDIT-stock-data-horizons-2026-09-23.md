# AUDIT：股票数据源多周期可用性（阶段0 结论 A）

> 状态：active ｜ 日期：2026-09-23 ｜ 对应版本：v2.9.9


> 关联：`PROPOSAL-stock-decision-four-horizon.md` 第 3 节（阶段 0）
> 日期：2026-09-23 · 类型：时点快照 · 结论：**A —— 多周期指标可用 → 阶段 2 评分层按多周期设计**

## 1. 结论

**多周期 K 线 / 指标可用**。`astock-data` 数据层已具备按 `period` 参数拉取任意周期 K 线的能力，
当前股票分析工作流只是「只用了日线」，并未受数据源能力限制。阶段 2 可放心按「多周期评分层」设计，
无需先新增数据源接口。

## 2. 可用性矩阵

| 周期 | 数据源是否支持 | 依据 | 备注 |
|---|---|---|---|
| 日线 `daily` | ✅ 可用（已使用） | `commands/stock_workflow/decision.rs:188`（500 根）、`hooks.rs:354`（30 根） | 现工作流默认周期 |
| 周线 `weekly` | ✅ 已映射 | `astock-data/src/vendors/eastmoney.rs:346-365` | 周期参数已透传上游 |
| 月线 `monthly` | ✅ 已映射 | 同上 | 同上 |
| 5/15/30/60 分钟线 | ✅ 主 vendor 已映射 | `eastmoney.rs:346-365` | ⚠ 浏览器 vendor 不支持 |
| 浏览器 vendor 分钟线 | ❌ 仅日/周/月 | `vendors/browser_eastmoney.rs:199-212` | 超短线需依赖主 vendor 或降级 |

指标层：`indicators.rs:44-78` 的 `IndicatorConfig` 已支持多周期均线 `ma_periods: [5,10,20,60]`
与多周期 RSI ⇒ 周/月维度的趋势/乖离/MACD/RSI/支撑/布林均可计算。

## 3. 关键链路

- 公共入口：`astock-data/src/lib.rs:1924-2004` —— `AStockClient::get_klines(code, period, limit)`
  **将 `period` 原样透传**给底层 vendor ⇒ 是否支持多周期取决于 vendor 实现。
- MCP：`mcp_tools.rs:974-1010` —— `get_stock_kline` 已接 `period` 参数，默认 `"daily"`。
- 股票分析决策校验：`decision.rs:188-240` 固定 `get_klines(code, "daily", 500)` —— 技术分析预检仅日线。
- 工作流增强：`hooks.rs:354-393` 固定 `get_klines(code, "daily", 30)` —— 日收益/波动率计算仅日线。

## 4. 超短线约束（阶段 2 需标注）

- 超短线（1-2 天）结论需要分钟级数据 ⇒ **仅在走主 vendor（eastmoney）时可用**；若运行环境走
  浏览器 vendor，需显式标注超短线置信度下限或降级为「单后验 + 档位映射（方案 B）」。
- 回溯长度需在阶段 2 抓取时按各周期确认（周/月线需要的绝对根数少，分钟线需较长回溯）。

## 5. 对阶段 1 的印证

阶段 1（多周期价位映射）不依赖阶段 0 结论，已独立实施——本审计佐证阶段 2 可按 **方案 A** 推进：

1. 评分层按多周期取多周期 MA / RSI 等指标，每周期产出独立 posterior。
2. 超短线走主 vendor 分钟线；浏览器 vendor 降级并标注置信度下限。