# 审计：`verdict` 形态归一（value-investor 修复 + 全仓未归一面清点）

> 状态：active ｜ 日期：2026-10-23 ｜ 对应版本：v2.9.9


- 日期：2026-10-23
- 触发：运行日志 12 条 `Agent 节点 input_mapping 取不到值`（`risk-agg` / `risk-con` / `risk-neu` × `value-investor--{mid,long}.result.verdict.{bull,bear}_score`）
- 结论：**形态契约缺失**，不是路径写错一个字。已按方案 A 在 `value-verify.rhai` 落地归一（v175），并清出 4 项同族未归一问题。

---

## 1. 现象与直接原因（修正版）

三条风险偏好节点各 4 条映射解析为 `None`：

```
node_id=risk-agg target_key=value-investor--long_bear_score
source_key=value-investor--long.result.verdict.bear_score
```

映射由 `build_analyst_input_mapping`（`stock_analysis_setup/mod.rs:1296-1314`）生成，对 `value-investor` 走 `{node}.result` 前缀 + 统一拼 `.verdict.{bull,bear}_score`。

### 1.1 为什么 `.verdict.` 命中不了

链路逐段：

| 段 | 谁提供 | 是否成立 |
|---|---|---|
| `value-investor--long` | 引擎 `workflow.results` 的节点 id | ✅ |
| `.result` | `value-verify--long` 的 CodeNode 包装（`output_var` 按档同名覆写，`seed:4645-4675`） | ✅（前提是覆写节点跑完） |
| `.verdict` | agent 输出后处理是否把业务字段搬进 verdict | ⚠ **本次不成立** |
| `.bull_score` | value-investor 的提示词契约 | 顶层存在 |

第三段是缺口。`agent_executor.rs:2457-2572` 的「通用后处理」只覆盖两态：

```rust
let needs_verdict_tag_reconstruct = parsed_opt.is_none();          // A：非 JSON → 从 VERDICT 标签重构
let needs_flat_reconstruct = parsed_opt.as_ref().is_some_and(|v|    // B：verdict 存在且**非对象** → 全字段搬进 verdict
    v.is_object() && v.get("verdict").is_some_and(|vd| !vd.is_object()));
```

而 `value-investor.md` 的契约（`custom/value-investor.md:145-176`）是**扁平 JSON**：`verdict` 是方向枚举字符串、`bull_score`/`bear_score` 在顶层。若 LLM 恰好把 `verdict` 写成**对象（常见为空对象 `{}`，见 `AnalystReportCard.tsx:765` 的实证注释）**，两个分支都不触发 ⇒ 输出保持「verdict 空壳 + 打分在顶层」的**混合形态** ⇒ `.verdict.bull_score` 恒 `None`。

> 上一轮口头判断为「verdict 是字符串所以取不到」**不准确**：字符串形态会被分支 B 兜住。真正没被兜住的是**verdict 为对象/空对象、业务字段仍在顶层**的混合形态。

### 1.2 为什么只有 value-investor 出问题

其余节点都有兜底，且兜底方式不同：

| 节点 | 产出契约 | 被谁归一 | 消费路径 |
|---|---|---|---|
| `a-*` 分析师 | 正文 + `<!-- VERDICT: {...} -->` | 引擎分支 A（标签→嵌套） | `.content.verdict.*` |
| `trader` | 扁平 JSON（`trader_output` schema 顶层扁平） | 引擎分支 B（verdict 是字符串） | `.content.verdict.*` |
| `risk-convergence` | 正文 + VERDICT 标签 | 引擎分支 A | `.content.verdict.disagreement_score` |
| `debate-convergence` | 扁平 JSON，**无 verdict 字段** | 不需要（消费端也按扁平读） | `.content.consensus_score` |
| **`value-investor`** | 扁平/混合 JSON + CodeNode 原地覆写 | **无** | `.result.verdict.*` ❌ |

`value-investor` 是唯一「合法 JSON + 有 verdict 字段 + 业务字段在顶层」的组合，正好落在两个判据的缝里。

---

## 2. 落地改动（方案 A，v175）

### 2.1 `src-tauri/src/commands/value-verify.rhai`

新增 `normalize_verdict_shape()`：把扁平/混合形态的九个业务字段**镜像**进 `verdict` map，顶层字段一个不删。

- 镜像键：`verdict` / `bull_score` / `bear_score` / `confidence` / `bull_points` / `bear_points` + 本节点刚覆写的 `intrinsic_value_range` / `margin_of_safety` / `ideal_buy_price`
- 三类分支：`flat->nested`（原为标量或缺失）／`nested`（已是对象）／`nested(empty)`（空壳）
- **不覆盖已有值**：LLM 若本来就输出嵌套，尊重它的口径（覆盖等于抹平证据）
- 留痕：新增顶层 `verdict_shape` / `verdict_shape_mirrored`，并在 `valuation_audit` 追加一条 `field=verdict`
- 调用点：`output` 构造后、`valuation_conclusion` 注入前（解析失败分支同样经过）

首轮实现踩到一个自嵌套坑：`verdict` 已是 map 时再搬自身 ⇒ `{verdict:{verdict:{}}}`，被 `value_verify_unparsable_content_degrades_without_overwrite` 抓到，已加 `if k == "verdict" && was_map { continue; }`。

### 2.2 配套

| 文件 | 改动 |
|---|---|
| `stock_analysis_setup/mod.rs` | `build_analyst_input_mapping` 文档补「`.verdict.` 段成立的前置条件 = v175 归一」 |
| `seed_stock_analysis.rs` | `TEMPLATE_VERSION` 174 → **175** + 版本注记（`.rhai` 经 `include_str!` 嵌入节点 `code`，属模板内容，不换代版本门会挡住） |
| `rt-workflow/.../code_executor.rs` | 新增 `value_verify_normalizes_flat_verdict_shape_for_downstream_mapping`；两处审计长度断言 4 → 5（新增形态归一记录） |

---

## 3. 举一反三：其它未形态归一的问题

### F-1（P1，已修）引擎归一判据不覆盖「混合形态」

`agent_executor.rs:2458-2460`：`needs_flat_reconstruct` 只判 `verdict` **是否非对象**。缺失/对象两种情况下业务字段仍留在顶层，无人搬移。本次 value-investor 正是此格。

**修复（分支 C）**：新增 `needs_empty_verdict_merge` 判据 + `else if` 分支，把「`verdict` 是**空对象** ∧ 顶层仍有业务字段」的产物合并为 `{report, verdict:{…}}`，顶层字段不删。判据与改写落在 `axagent_harness::verdict_shape`（反思链共用同一份实现，避免两处判据漂移）。

**刻意不扩到其它形态**：`verdict` 是**非空**对象而另有字段在顶层时两处都有值，不构成 miss；没有 `verdict` 键的扁平产物（`debate-convergence`）是刻意设计。硬扩会把「猜出来的形态」写进引擎，属用启发式掩盖契约缺失。

### F-2（P1，已修）反思链对 trader 产出的读法与主链相反

| 位置 | 读法 |
|---|---|
| `stock_analysis_setup/mod.rs:1670-1672` | `sub-analysis.trader.content.action` / `.targetPrice` / `.confidence`（**顶层**） |
| `seed_stock_analysis.rs:6104-6120` | `trader.content.verdict.action` / `.targetPrice` / `.confidence`（**嵌套**） |

取证链（四跳，均为源码直读）：

1. `reflection.rs:468-480` — `sub-analysis` 变量由 `build_sub_analysis_from_snapshot()` 构造（不再重跑 DAG）；
2. `reflection.rs:3573-3602` — 逐条取快照的 `_raw.<node_id>`，并把 `content` **JSON 字符串解析成对象**；
3. `blackboard.rs:193-197` — `_raw.<node_id>` = **引擎后处理之后**的 `results` 原始输出 ⇒ `trader.content` 是 `{report, verdict:{action,...}}`（引擎分支 B 已把扁平字段搬进 verdict，见 §1.2）；
4. ⇒ `sub-analysis.trader.content.action` 在**顶层**取不到 ⇒ `reflection-comparator.rhai:141-175` 静默走兜底（`trader_conf=0.0`、`trader_act_str=""`）⇒ 反思对比缺失「交易员动作/目标价/置信度」三路输入。

反证（说明不是设计如此）：`reflection.rs:3636-3641` 的 `REFLECTION_MEMORY_PROTECTED_NODES` 注释明确把这些映射当作"必须保住、否则静默变空"的硬引用保护 —— 但它们本来就取不到。

**修复（入口镜像）**：`build_sub_analysis_from_snapshot` 在把 `content` 解析成对象后，调 `axagent_harness::verdict_shape::mirror_verdict_fields_to_top()` —— 把 `verdict` 内字段**镜像**到 `content` 顶层（verdict 保留、顶层已有值不覆盖）。三条映射恢复可读，而 `debate-convergence.content.consensus_score`（无 verdict 键）不受影响。体积增量由既有的 36k 字符预算裁剪兜住。

**为什么不改模板路径**：改成 `.content.verdict.*` 只是把缺陷**翻面** —— trader 若某次没输出 `verdict` 字段（引擎归一不触发、保持扁平），新路径同样 miss。镜像让两套读法同时成立，是这层契约缺位下唯一不赌形态的做法。

### F-3（P2，已修）Rust / TS 两份 `reconstruct*VerdictTag` 对字符串 verdict 产出不同

| 实现 | 修复前 | 修复后 |
|---|---|---|
| `analysis-engine/src/blackboard.rs:228` | `<!-- VERDICT: "看多" -->`（JSON 标量，无对象字段） | `<!-- VERDICT: {"verdict":"看多"} -->` |
| `lib/agentOutput.ts:805` | `<!-- VERDICT: 看多 -->`（**非法 JSON**） | 同上 |

**修复**：两侧都加「标量 → `{"verdict": <标量>}`」包装（Rust `verdict_tag_payload()` / TS `JSON.stringify({verdict})`），`__verdict_only` 逻辑保持不变。既有 TS 测试断言同步更新，Rust 新增 `scalar_verdict_is_wrapped_into_object_payload`。

### F-4（P3，已修）`flattenVerdictReport` 的运算符优先级偏离注释

`ValueAssessmentPanel.tsx:419` 原式：

```ts
if (k !== "report" && !(k in data) || data[k as keyof ValueReportData] === undefined) {
```

`&&` 先于 `||` ⇒ 实际语义是 `(k !== "report" && !(k in data)) || data[k] === undefined` —— `report` 例外会被 `||` 旁路。**修复**：补括号为 `k !== "report" && (!(k in data) || data[k] === undefined)`，与注释「只提升顶层没有的键」一致。

### F-5（信息）消费端已各自写兼容，反证契约长期缺位

| 位置 | 兼容内容 |
|---|---|
| `data-quality.rhai:68-80` | `confidence` 顶层 / `verdict.confidence` 两态 |
| `AnalystReportCard.tsx:738-746` | verdict 是对象时提升 `verdict/stance/confidence` |
| `AnalystReportGrid.tsx:527-533` | `rawVerdict.verdict` 取方向 |
| `DebatePanel.tsx:217-225` | 嵌套 verdict 保护 |
| `stock-analysis/utils.ts:36-37` | 嵌套 verdict 提升 |

**五处**消费端各写一遍兼容 ⇒ 说明「verdict 形态」这层契约从未被生产端保证。v175 只补了 value-investor 一支；正解是「生产端归一 + 消费端只读一种形态」，否则每新增一个专家就再写一份兼容。

---

## 4. 验证

```
cargo test -p axagent-rt-workflow --lib value_verify
running 5 tests
ok  value_verify_execute_declares_shared_output_var
ok  value_verify_compliant_output_is_left_intact
ok  value_verify_unparsable_content_degrades_without_overwrite
ok  value_verify_normalizes_flat_verdict_shape_for_downstream_mapping   ← 新增
ok  value_verify_hard_unavailable_overwrites_numeric_fields
test result: ok. 5 passed; 0 failed   EXIT=0
```

- 首轮结果 4 passed / 1 failed：`value_verify_unparsable_…` 报 `left: {"verdict":{}} right: {}` —— 空 map 分支把 `root["verdict"]`（容器自身）又搬了一次，自嵌套。已修（`if k == "verdict" && was_map { continue; }`），二轮全绿。
- 归一后自证字段：执行产物的 `result.verdict_shape`（`flat->nested` / `nested` / `nested(empty)`）+ `valuation_audit` 的 `field=verdict` 条目
- 生效前提：库版本必须重播种到 175（否则版本门 `existing.version >= 175 ⇒ skip`，脚本改动不进库）

## 5. 本轮修复清单（F-1 ~ F-4 全部落地）

| 项 | 文件 | 手法 |
|---|---|---|
| F-1 | `crates/harness/src/verdict_shape.rs`（新模块）+ `agent_executor.rs` 分支 C | 空 `verdict` 合并（顶层保留） |
| F-2 | `stock_workflow/reflection.rs` `build_sub_analysis_from_snapshot` | `verdict` → 顶层镜像 |
| F-3 | `analysis-engine/src/blackboard.rs` + `lib/agentOutput.ts` | 标量包成 `{"verdict": …}` |
| F-4 | `ValueAssessmentPanel.tsx` | 补括号 |

判据与改写集中在 `verdict_shape` 一个模块（F-1 / F-2 共用），杜绝「两处各写一份判据」的第二份实现。

## 6. 验证状态

| 面 | 命令 | 结果 |
|---|---|---|
| 前端单测 | `vitest run src/lib/__tests__/agentOutput.test.ts` | **55 passed** |
| 前端组件测 | `vitest run src/components/stock-analysis/__tests__` | **502 passed / 71 files** |
| 前端类型 | `tsc --noEmit` | **EXIT=0** |
| 前端格式 | `dprint check` | **EXIT=0** |
| Rust | `cargo test -p axagent-harness --lib verdict_shape` 等四条 | 见下方阻塞说明 |

⚠ Rust 侧验证被同机其它 cargo 进程（`cargo run` / `cargo test`，**非本会话启动**）持有的
`target/debug/.cargo-build-lock` 拦住（`拒绝访问 (os error 5)`）。已排队等锁后自动重跑，
结果回填本节 —— 在此之前 Rust 侧改动**未经编译验证**。

## 7. 遗留

1. **上线前提**：`TEMPLATE_VERSION=175` 需随下次启动重播种生效，之后再看一次
   `input_mapping 取不到值` 是否归零（判据：不再出现 `source_key` 以 `value-investor--` 开头的 12 条）
2. 引擎分支 C 的接线无集成测试（与既有分支 A/B 同状，均埋在 `execute()` 内、依赖真实 LLM）；
   判据侧由 `verdict_shape` 的 6 个单测覆盖
3. F-5（五处消费端各写双形态兼容）仍在 —— 契约归一后属可收敛项，是重构不是修 bug
4. `verdict_shape` 模块与 `value-verify.rhai` 的镜像键集合是**两份**清单（Rust 九键 / Rhai 九键），
   改一处需同步另一处（Rhai 沙箱无法引用 Rust 常量）
