# PLAN：荐股链反思优化闭环（R-E 从「只报数」升级为「回写通路」）

## 0. 结论与目标

荐股链是纯规则打分（`recommender/strategies/` 5 风格×4 周期），反思对它的优化形态**不是**改 prompt /
改专家结论（那是分析链的通路），而是把「荐股自己的实现结果」回写成 18 个子策略消费的
**置信权重与先验**。目标：把 Phase R-E 的逐档 rank IC（`recommender/ic.rs`，现只报数不回写）
接入权重通路，形成可证伪的闭环；同时修掉两处会让闭环失真的既有缺陷（D1/D2）。

## 1. 现状盘点（三条通路 + 两个缺陷）

### 通路 ①「胜率线」——已连通，但归属错位（B1）

```
分析链回测 backtest_analysis（stock_analysis.rs:1891）
  → record_performance(:1924，strategy_id = map_action_to_strategy_id 的伪风格，:1876)
  → strategy_performance 表
  → evolution_drift::recalc_and_persist（weight_decay::compute_adjusted_weights，Beta平滑+样本饱和+EWMA）
  → strategy_weight_history
  → load_reco_served_vars 覆盖 reco_strategy_weights（stock_analysis.rs:4492-4503）
  → parse_strategy_weights（recommender/mod.rs:182）按 (style, period) 缩放 confidence
```

### 通路 ②「力度线」——半环，终点是面板

```
荐股落库 persist_reco_response_picks（stock_analysis.rs:4522）
  → run_decision_backtest → decision_validations（backtest_validation.rs:331，含 hit_outcome / t_plus_n_price）
  → reco_ic_stats 命令（stock_analysis.rs:4669）→ ic::aggregate_reco_ic（逐 (风格,档位) rank IC + 半衰期）
  → ❌ 只渲染进 IC 面板，无任何写入方
```

### 通路 ③「先验线」——已连通且口径正确

`reco_horizon_prior`（stock_analysis.rs:4451）与分析链共用
`reflection_stats::build_hitrate_stats` → `horizon_prior::horizon_prior_map` →
`scoring::blend_confidence` logit 合成（recommender/scoring.rs:44）。本计划不动这条线。

### 缺陷 D1（归属错位）

`strategy_performance` 的写入方只有分析链：`backtest_analysis` 用
`map_action_to_strategy_id`（stock_analysis.rs:1876）把**决策 action** 映射成
trend/reversion/value/capital/watchlist 这组**与荐股 Style 同名但语义无关**的
strategy_id；`stock_workflow/reflection.rs:771` 同理。后果：B1 覆盖进 `reco_strategy_weights`
的是「分析链决策在买卖观望上的胜率」，却被荐股链当成「趋势策略在超短档的预测力」消费。
荐股链**自己**的实现样本（decision_validations，逐 pick 带真 style×period）从未参与权重重算。

### 缺陷 D2（入口分裂）

消费 `reco_strategy_weights` 的三条入口走了两套口径：

| 入口 | 变量来源 | 是否消费演化权重 |
|---|---|---|
| `recommend_stocks`（:4555） | `load_reco_served_vars` | ✅ |
| `recommend_stocks_all_periods`（:4602） | `load_reco_served_vars` | ✅ |
| cron 荐股扫描（init/services.rs:2301-2325） | 裸 `extract_template_vars` | ❌ 只用模板静态值 |

定时荐股（通知 + 落库的主生产者）从不消费闭环产物 ⇒ 闭环生效后大部分票仍是出厂权重算的。

## 2. 设计原则

- **P1 非对称回写（防噪声标定噪声）**：负 IC / 低胜率档位**降权**；正 IC 不提权（基线 1.0 即上限）。
  rank IC 单格样本门槛 n≥8（`reflection_stats::IC_MIN_SAMPLE`），24 格里大量格常年不足 ⇒ 提权通路
  会先被噪声点亮。若 Q1 拍板要提权，提权上限也须显著低于降权幅度。
- **P2 样本门闸三态**：`ic_status ∈ {ok, insufficient_ic_samples, degenerate_variance}`（ic.rs:49）
  ⇒ 只有 ok 且 IC<0 的格降权；不足/退化 ⇒ 保持基线并在呈现层**显式声明「该格未被闭环校准」**，
  不得让「没校准」在 UI 上与「校准过且良好」同形（结构性缺席不歧义）。
- **P3 归属分离**：荐股权重只从荐股自己的样本推导（decision_validations 的真 style×period）；
  分析链胜率经通路 ③（逐档先验）影响荐股，二者不再互相冒名。D1 由此收口。
- **P4 复用既有基元**（禁区 12）：胜率平滑用 `weight_decay::compute_adjusted_weights` 的
  Beta(1,1)+log 饱和+EWMA 三件套（或其配置克隆），IC 聚合用 `ic::aggregate_reco_ic`；
  名目别名收口用 `style_matrix::db_style_aliases`（serenity↔bottleneck，ic.rs:111）。
- **P5 全程留痕**：每次回写落 `strategy_weight_history`（trigger 用 `"reco-loop"`，与
  cron/manual/rule 区分）；荐股落库时 `reco_strategy_weights_snapshot`（:4509）已自动快照实际消费值。

## 3. 阶段划分

### Phase A｜数据面：荐股样本进权重输入（修 D1 的前半）

- 新增构造：从 `decision_validations`（hit_outcome 已实现者）生成
  `(style, period, was_correct, exit_at)` 行，喂给 `weight_decay`——不落 `strategy_performance`
  （避免与分析链伪风格行同表混码），走独立 recalc 分支。
- `was_correct` 判定复用 `hit_rate_backtest::validate_pick` 的 hit/miss 口径（hit⇒1，miss/false_hit⇒0，
  partial/insufficient 不入样本），不写第四份判定。

### Phase B｜计算面：胜率×IC 合成为档位权重（闭环核心）

- 新纯函数（建议 `recommender/ic.rs` 或独立 `reco_loop.rs`）：

  ```
  reco_cell_weight(cell_ic, cell_winrate, cfg) -> f64   // 每 (style, period) 一格
  规则：
    ic_status != ok            ⇒ 1.0（基线，标 not_calibrated）
    rank_ic < 0                ⇒ 降权，下限 0.05（沿用 weight_decay clamp）
    胜率线（A 段样本，n≥饱和参考）⇒ EWMA 平滑进该格，alpha 复用 WeightDecayConfig
    serenity/bottleneck 别名   ⇒ 合并样本后再算，一格一个权重
  ```
- 写 `strategy_weight_history`（trigger=`"reco-loop"`），键空间与
  `parse_strategy_weights` 的 `"{style}_{period}"` 对齐（mod.rs:193-217）。

### Phase C｜消费面：三入口统一 + 权重换源（修 D2 + D1 后半）

- cron 扫描（init/services.rs:2301）改走 `load_reco_served_vars`——**纯缺陷修复，无裁定点**。
- `load_reco_served_vars` 的 B1 覆盖源：`load_current_weights`（分析伪风格线）→ 改为只取
  trigger=`"reco-loop"` 的记录（Q3 拍板项；先切源后闭环生效，避免闭环上线当天放大 D1）。

### Phase D｜呈现面：IC 面板显式化闭环状态

- `RecoIcCell` / 前端矩阵每格新增闭环态：`未校准(样本不足) | 未校准(IC退化) | 已降权(x→y) | 基线`，
  与既有「按设计不出票」（contract_cells 理由码）三态并列不混排（禁区：缺席分句）。
- DTO 新增字段带 `#[serde(rename_all = "camelCase")]` + 序列化测试照 ic.rs:312 模式；
  locales 11 语言补齐（zh-CN 为源）。

### Phase E｜证伪面：闭环有效性回测 + 参数五点对账

- 开关 `reco_ic_gate`（三值 `off|shadow|on`，进 seed_variables，出厂 **shadow**）：
  shadow = 计算+落 history+面板可读，但 `load_reco_served_vars` 不消费。
- A/B：as-of 回放同一区间，`reco_ic_gate=off` vs `on`，对比四档命中率与逐格 rank IC；
  判据=降权格的实现胜率确实劣于未校准格，否则闭环不转正。
- 新可调参数按五点对账登记（seed_variables / 面板 / served_vars / 快照 / 门禁统计）。

## 4. 裁定项（请逐条拍板）

| # | 问题 | 推荐 | 备选 |
|---|---|---|---|
| Q1 | 回写方向 | 非对称：只降不升（P1） | 对称双向，升上限 ≤1.5 |
| Q2 | 生效方式 | shadow 起步，A/B 过判据转 on | 直接 on |
| Q3 | 分析链伪风格权重是否退出荐股消费 | 退出（P3，荐股权重只认荐股样本） | 保留 B1 现状、reco-loop 叠加其上 |
| Q4 | cron 入口统一（D2） | 纳入本期（纯缺陷） | —— |
| Q5 | 胜率线 lookback | 按档对齐验证窗（2/5/28/90 天） | 沿用 weight_decay 30 天固定窗 |

## 5. 验证与门禁

- 阶段内：`cargo check` + 定点 `cargo clippy -p analysis-engine --all-targets -- -D warnings`；
  纯函数单测覆盖（样本不足不回写、负 IC 降权、NaN/±INF 防御、别名合并、键空间往返）。
- 全阶段完成后一次全量 CI 口径：`cargo fmt --all --check`、
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`、`__TAURI_WORKSPACE__=true cargo test --workspace`。
- 前端：`npm run typecheck` + `npm run lint:eslint`（oxlint）+ Vitest。
- 实机：tauri dev 手动 `manual_recalc_strategy_weights` 与回放 A/B，核对面板闭环态与 history 留痕。

## 6. 边界（本期不做）

- 不动通路 ③ 先验线；不做子策略**内部阈值**（RSI 参数等）的自动寻优（那是参数级进化，超出
  「反思→档位权重」闭环范围）；不动 `ScoringWeights` 四因子常量（scoring.rs:27，无逐格样本支撑）。
