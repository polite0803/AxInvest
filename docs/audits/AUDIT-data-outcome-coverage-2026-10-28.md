# 取数状态信封：覆盖扩展（2026-10-28）

> 状态：active ｜ 日期：2026-10-28 ｜ 对应版本：v2.9.9


## 背景

`get_index_quotes` 在三条通道全部失败时曾返回 `Ok(vec![])`，LLM 只能看到「今天没有指数行情」，
报告板块与分析师会把取数故障当成真实结论消费。四态信封已在 `outcome.rs` 落地并接完
`get_index_quotes` 全链路，但 MCP 工具层其余核心接口仍是裸返回。

本轮把公共判据下沉到 `outcome.rs`，并把 MCP 工具层的 9 个核心接口改为信封。

## 改动

| 文件 | 改动 |
|---|---|
| `crates/astock-data/src/outcome.rs` | 新增 `Expect`、`empty_evaluation`、`outcome_from_vec`、`outcome_from_opt`（公共判据，MCP 层与日检共用） |
| `crates/astock-data/src/health_check.rs` | 删除本地 `Expect` / `empty_evaluation` / `from_vec` / `from_opt` 实现，改为复用 `outcome` |
| `crates/astock-data/src/mcp_tools.rs` | 新增 `envelope_vec` / `envelope_opt`；9 个工具分支改为信封 |
| `crates/analysis-engine/src/report.rs` | 仅格式化（既有 diff，非本次语义改动） |

### 新增信封的 9 个工具

| 工具 | 期望档 | 理由 |
|---|---|---|
| `get_stock_kline` | `MustHaveRows` | 收盘后应有 K 线 |
| `get_stock_money_flow` | `MustHaveRows` | 单对象维度，缺失即异常 |
| `get_hot_stocks` | `MustHaveRows` | 人气榜盘中即有数据 |
| `get_industry_ranking` | `MustHaveRows` | 行业横向排名应有值 |
| `search_news` | `MayBeEmpty` | 关键词可能确实无匹配 |
| `get_social_sentiment` | `MayBeEmpty` | 冷门股股吧可无帖子 |
| `get_cls_flash` | `MayBeEmpty` | 清淡时段可无快讯 |
| `get_north_bound_flow` | `MayBeEmpty` | 非交易日/暂停披露可无数据 |
| `get_market_dragon_tiger` | `MayBeEmpty` | 当日可无上榜股票 |

`get_stock_quote` 保持裸返回：单对象无「空集合」语义，归因由日检的 `Probe::Quote` 负责。

## 覆盖现状

- **MCP 工具层**：10 个工具带信封（含原有 `get_index_quotes`）
- **日级健康检查**：12 个核心接口（`CORE_INTERFACES`），交易日 15:00 后每小时 tick，落 `__interface_health__` 快照
- **报告层**：`parse_index_quotes` 兼容裸数组与信封（`status=None` 保持旧行为）
- **前端**：`IndexQuotesPanel` 四态各占一句、`InterfaceHealthPanel` 展示日检历史

## 验证

```
cargo check -p axagent-astock-data                     通过
cargo clippy -p axagent-astock-data --all-targets -- -D warnings   通过
cargo test  -p axagent-astock-data                     662 + 5 + 7 passed
cargo fmt --all --check                                通过
```

## 未做与原因

1. **`parse_index_quotes` 未泛化为通用解包器**：HTML 报告只有 `index_quotes` 一个变量消费信封；
   其余工具结果只进 Agent prompt，由 LLM 直接读 `{status,data,detail}`，没有第二个 Rust 消费者。
   等出现第二个报告变量再泛化，避免造一个当前无人调用的抽象。
2. **工具 `description` 未追加格式说明**：`status` / `detail` 字段名自解释；给 7 条描述各加一句
   会稀释原有语义说明。若实测 LLM 误按数组解析，再补一句。
3. **Agent prompt 模板未改**：`raw.combined` 里的这些工具结果现在是信封 JSON。LLM 能理解，
   但**需要观察一轮完整分析**确认报告质量未退化。
