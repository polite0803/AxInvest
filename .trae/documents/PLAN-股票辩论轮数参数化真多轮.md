# 股票多空辩论轮数参数化 + 真多轮

## Context（背景与目标）

用户报告：股票分析工作流「第二、第三轮辩论没有运行就直接进入风险评估」。

根因（已确认）：模板 seed 里 `debate_max_rounds` 在
`src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs:2033`
被**硬编码为 1**；且它是从模板变量 `debate_rounds`（定义于
`seed_variables.rs:20-40`）读取的，但 `seed_stock_analysis.rs:4246` 又用
`force_variable_value(..., json!(1))` 把该变量**强制覆写为 1**。两层共同导致
用户无论在前端把「多空辩论轮数」设成几，实际只跑 1 轮。

过去的 `v48`（2026-09-14）之所以改 1 轮，是因为当时引擎对辩论容器体内
`Completed` 的辩手节点「复用上次输出、不重跑 LLM」（`engine/mod.rs:6342-6366`），
多轮是「假多轮」。但这只对**同一批节点被多次轮播**成立。

本次目标：把轮数改为**从股票分析设置参数 `debate_rounds` 读取**（去掉硬编码与强制覆写），
并且 N>1 时**每一轮都真跑 LLM**、产出不同内容。

## 核心洞察：seed 的节点模型本身已支持真多轮（引擎零改动）

关键实证（subagent 核实，含 `dag_store.rs:130-315`、`debate_executor.rs:115-198`）：

- seed 在 `seed_stock_analysis.rs:2115` 起**按轮展开独立的实体 AgentNode** `bull-rN` / `bear-rN`
  （每轮不同 id、不同专家 persona：R1=bull/bear-researcher、R2=bull-r2/bear-r2、R3=bull-r3/bear-r3，
  专家 md 文件均在仓内），且每轮通过 `context_sources` **精确引用前一轮的实际输出**
  （`bull-r2` 引用 `bull-r1`/`bear-r1`+`analyst-brief`，见 `:2182-2188`、`:2218-2226`）。
- 容器 `debate-bull-bear` 的 `debater_steps` 是全部 distinct 节点的展平
  （`:2086-2088`）。`build_debate_body_dispatch`（`engine/mod.rs:6242`）按 step_id 驱动，
  **每个 distinct 节点只被驱动一次**（后续因 `Completed` 被复用短路，但那是「防止同一节点重复执行」，
  对 distinct 模型是正确的）。
- 因此：**只要 `debate_max_rounds = N`，就会展开 2N 个 distinct 节点，每个各自真跑一次 LLM**，
  且每轮输入因 `context_sources` + 不同 expert persona 而真实不同 —— **这就是真多轮**。

> 结论：本次改动**不动引擎**。`build_debate_body_dispatch` 的复用分支、`debate_executor`、
> `swarm_executor` 全部保持原样，回归面最小。

## 改动点

### 1. `seed_stock_analysis.rs:2033` — 从变量而不是硬编码 `1`

```rust
let debate_max_rounds: usize = 1;
```
改为：在建辩论节点前从 `old_variables`/默认值解析出 `debate_rounds`。

建议：抽一个共享 helper（放在 `merge_variable_values` / `force_variable_value` 附近，
`stock_analysis_setup/mod.rs`），解析旧变量 JSON 中 `name=="debate_rounds"` 的
`.value.as_u64()`，未命中则用常量默认（值与 `seed_variables.rs` 的 `debate_rounds` 默认一致）。
保持与最终 `variables_val` 的取值同源，避免「解析用一套、落库存一套」漂移。

### 2. `seed_stock_analysis.rs:4246` — 删除强制覆写

```rust
let variables_val = force_variable_value(&variables_val, "debate_rounds", serde_json::json!(1));
```
删除。`merge_variable_values`（`:4235-4240` 覆盖旧版变量迁移时）与 `RENAME_MAP`
（`stock_analysis_setup/mod.rs:1236`，旧 `analysis_maxDebateRounds` → 新 `debate_rounds`）
会保留用户在前端设置的值。删除后，用户设成多少，重建后就能真跑多少轮。

### 3. `seed_stock_analysis.rs:120` — `TEMPLATE_VERSION` 升版

当前为 62。本改动属于模板语义变更（默认轮数变、节点展开数变），而种子版本门是
`existing.version >= TEMPLATE_VERSION` 即跳过（`:855`）。**必须 +1（取 63）** 才会强制重种
覆盖 DB 旧模板，否则「改了等于没改」。代价：覆盖 DB 中对 nodes/edges 的手工调整——这是
本种子固有的、已在 `seed_variables.rs:31-32` 记录过的换版成本；用户自定义变量由
`merge_variable_values` 保留。

### 4. `seed_variables.rs:20-40` — 默认值与文案

- `debate_rounds` 默认 `value` 改为意图默认（建议 3），并更新 `description`：
  移除「多轮为假多轮 / v48 固定 1」的过时字面，改为说明现在会真跑 N 轮。
- 同步确认 `harness/workflow_types.rs:1306` 的 `default_debate_rounds()` 兜底值
  （当前 1）是否需要与新默认对齐（三处「同名散值」曾踩过坑，见其注释）。

## ⚠️ 约束与回归检查

1. **别把 `max_rounds` 改成 `debate_max_rounds`**：`DebateNode.config.max_rounds`（`:2089`）
   保持 1。轮次语义由 distinct 节点承载；若容器 `max_rounds = N`，`debate_executor` 会对
   整批 `debater_steps` 做 N 遍轮播，其中 N-1 遍被 `Completed` 复用短路 → 回到假多轮浪费。
   保持 1 = 单遍把 2N 个 distinct 节点全驱动一次。
2. **收敛检测**：`debate_executor` 的 `check_round_convergence` 只在 `round>0` 触发
   （`:186`）。`max_rounds=1` 下不触发；收敛语义由 seed 的 `debate-convergence` agent 节点
   （`:2245` 起，context_sources 动态遍历 `1..=debate_max_rounds`）承担，语义不降级。
3. **请求体体积（回归需盯）**：N 增大 → distinct 辩手节点增多 → 链尾 `bear-r{debate_max_rounds}`
   （`:2250/:2301` 依赖锚点）平均请求体上升，历史曾达 85.8KB 撞 provider 头超时档 15s
   （601166 实证）。建议前端将轮数上限限制在 ≤3（用户原诉求即 2/3 轮），避免无限扩大。
4. **负控测试不受影响**：`debater_round_refs_are_parameterized`
   （`seed_consistency_tests.rs:454`）只查边的端点是否为**字面量硬编码** `bull-rN/bear-rN`。
   seed 端点全用 `format!("bear-r{debate_max_rounds}")` 派生，改从变量读 `debate_max_rounds`
   仍是参数化形态，**不触发**。只需回归跑它确认自证计数（`blocks/calls/key_hits >= 1`）仍满足。

## 建议新增/调整的测试

- `seed_consistency_tests.rs` 增一个不变量断言：模拟 `debate_max_rounds = N` 时，
  展开的 `bull-r1..bull-rN` / `bear-r1..bear-rN` 各恰 N 对、`debater_steps` 长度 = 2N、
  且容器 `max_rounds == 1`（防回归到 `max_rounds = N` 的冗余轮播）。类同现有源码级提取风格，
  从 `seed_stock_analysis.rs` 源码抓 `for round in 0..debate_max_rounds` 与 debater_steps
  `flat_map` 的参数化形态判定。
- 若可行，补一条端到端断言：`debate_rounds=3` 重建后，下游（`debate-convergence` / `value-investor`）
  拿到的是 `bear-r3` 输出而非 `bear-r1`。

## 验证

1. `cargo fmt --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `__TAURI_WORKSPACE__=true cargo test --workspace`（重点看 `seed_consistency_tests`）
4. 端到端（需 Tauri 环境）：
   - 更新 DB 中已 seed 的模板（新 `TEMPLATE_VERSION` 触发重种）；
   - 在前端「多空辩论轮数」设成 3 → 触发重建 → 跑一次股票分析；
   - 观察执行记录：应出现 bull-r1/bear-r1/bull-r2/bear-r2/bull-r3/bear-r3 各一次真执行，
     且 `debate-convergence`/`value-investor` 消费 `bear-r3` 输出。

## 涉及文件

- `src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs`（:120 / :2033 / :2086-2089 / :4246）
- `src-tauri/src/commands/stock_analysis_setup/seed_variables.rs`（:20-40）
- `src-tauri/src/commands/stock_analysis_setup/mod.rs`（新增轮数解析 helper，或复用/扩展既有 merge/force helper）
- `src-tauri/src/commands/stock_analysis_setup/seed_consistency_tests.rs`（新增不变量断言）
- `src-tauri/crates/harness/src/workflow_types.rs:1306`（如有需要，兜底默认对齐）

## 明确不改

- `crates/rt-workflow/src/work_engine/engine/mod.rs:6342-6366`（复用分支）—— 保持不动
- `crates/rt-workflow/src/work_engine/executors/debate_executor.rs` —— 保持不动
- `crates/rt-workflow/src/work_engine/executors/swarm_executor.rs` —— 保持不动