# 股票反思工作流四周期化基线审计

> 状态：active ｜ 日期：2026-09-23 ｜ 对应版本：v2.9.9


**日期：** 2026-09-23  
**用途：** 批次 0 基线。只记录当前契约与测试输入，不代表四周期反思已经实施。

## 1. 当前数据源

| 实体 | 当前四周期相关字段 | 当前语义 |
|---|---|---|
| `stock_analyses` | `horizon_price_map: Option<String>` | 四周期价位映射 JSON；旧记录为 `NULL` |
| `stock_analyses` | `horizon_decisions: Option<String>` | 四周期完整决策 JSON；旧记录为 `NULL` |
| `stock_analyses` | `decision_action` | 兼容用单值 action，不能被四周期 action 覆盖 |
| `stock_analyses` | `decision_time_horizon` | 兼容用主周期 |
| `stock_analyses` | `decision_expected_holding_days` | 兼容用主周期持有天数 |
| `stock_reflections` | `verdict` / `raw_return` / `alpha_return` / `holding_days` | 当前一条反思只有一组结果 |
| `strategy_performance` | `was_correct` / `return_pct` | 当前一条绩效只有一个确定性判定 |
| `strategy_performance` | `horizon_pnl_json` | 已存在的多时间点盈亏 JSON；当前不是四周期反思结果契约 |

## 2. 当前兼容边界

1. `stock_analyses.horizon_price_map` 和 `horizon_decisions` 可能为 `NULL`，消费端必须回退到 `decision_json` 和单值字段。
2. `stock_reflections` 的顶层 `verdict`、`raw_return`、`alpha_return`、`holding_days` 继续作为 legacy 主周期摘要。
3. `strategy_performance.was_correct` 只能表达一个兼容主周期结果，不能直接承载四个周期的真假值。
4. `reflection_stats` 当前通过 `original_analysis_id` 或时间窗关联 `stock_analyses`，再按单值 `decision_action` / `decision_time_horizon` 聚合。
5. 缺行情时当前链路使用 `market_snapshot=None`，不能生成伪造的 0% 收益作为四周期结果。

## 3. 基线输入

- 四周期完整决策：`src-tauri/crates/analysis-engine/tests/fixtures/stock_reflection_four_horizon_complete.json`
- 旧单值决策：`src-tauri/crates/analysis-engine/tests/fixtures/stock_reflection_legacy_single_horizon.json`
- 行情不可用：`src-tauri/crates/analysis-engine/tests/fixtures/stock_reflection_market_unavailable.json`

三份 fixture 仅用于后续批次的解析、成熟度、统计和回退测试。它们不应被当作生产数据库 seed，也不应绕过 migration。

## 4. 批次 0 基线结论

- 当前四周期决策输出已经存在，可以作为反思输入。
- 当前反思结果与绩效判定仍是单周期模型，需要新增版本化四周期结果 JSON。
- 当前 `strategy_performance.horizon_pnl_json` 可复用，但必须先定义新 schema，不能把现有 1d/3d/5d/10d/20d 序列直接解释成四周期结果。
- 在新增 schema、migration 和纯函数测试前，不进入真实工作流接线。
