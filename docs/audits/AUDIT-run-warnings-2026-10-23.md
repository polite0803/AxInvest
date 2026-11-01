# 审计：10-23 运行日志 WARN 分类与定性

- 输入：`C:\Users\polit\Documents\PowerShell.txt`（1238 行，单次运行 10:25:15 ~ 10:35:xx，含多个 execution）
- 方法：按 `(level, 模块, 消息模板)` 聚合 + 按 `execution_id` 对齐时间线 + 源码核对
- 规模：**WARN 116 种 / 0 ERROR**；其中 `strict_mode enabled` 60 条属正常提示

---

## 0. 结论摘要

| 类别 | 条数 | 定性 | 严重度 | 状态 |
|---|---|---|---|---|
| A `input_mapping 取不到值` | 39 | **依赖未满足**（同 execution 内上游尚未完成 / 无等待边） | **P0** | ✅ v176 修（见 §1.5） |
| A′ `V57 自动补默认 unit` | 5 | 与 A 同族（`a_*_raw` / `risk_*` 变量未注入） | P1 | 部分（时序类降噪，见 §1.6） |
| B 供应商 504 / fallback 切换 | 40 + 39 + 9 + 1 | 外部（agnes-3.0-flash 网关挂起），但暴露超时/重试配置 | P1（外部） | 未动 |
| C `未知断言类型 exists` | 1 | **模板与执行器契约不一致**，断言恒 false | P2 | ✅ v176 修（含 `actual` 路径订正） |
| D `take_asof_degradation_report 访问失败` | 1 | task-local 不在 scope 内 ⇒ 降级记录丢失 | P3 | 未动 |
| E `portfolio-risk-gate 缺 action` | 1 | 回落分支生效（设计内），但门可能从未生效 | P2（待查） | 未动 |
| F `通知渠道未装配（已注册 0 个）` | 6 | 配置缺失（未配通知渠道） | 配置 | 未动 |
| G 数据源降级（eastmoney / tencent） | 4 | 外部源不稳定 | 外部 | 未动 |
| H 噪音（诊断 WARN 打印全量 JSON） | 6 | `code_executor` 诊断日志级别过高 | P3（噪音） | 未动 |

**最重要的一条**：v175 的「verdict 形态归一」只覆盖 A 类的**后半段**（上游已完成、但形态取不到）。
本次日志的 39 条里，**主因是链路时序 / 缺边**；v176 才是针对主因的修复。

---

## 1. A 类：`input_mapping 取不到值`（39 条）

按 `(引用节点 ← 源路径)` 聚合：

| 次数 | 引用节点 | 源路径 |
|---|---|---|
| 4 | `risk-agg` | `value-investor--{mid,long}.result.verdict.{bull,bear}_score` |
| 4 | `risk-con` | 同上 |
| 4 | `risk-neu` | 同上 |
| 2 | `debate-convergence--long` | `value-investor--long.result.verdict.*` |
| 2 | `debate-convergence--mid` | `value-investor--mid.result.verdict.*` |
| 2 | `risk-{agg,con,neu}` | `a-lockup--short.content.verdict.{bull,bear}_score` |
| 2 | `debate-convergence--short` | `a-lockup--short.content.verdict.*` |
| 1 | `trader` | `risk-convergence.content.verdict.disagreement_score` |

### 1.1 铁证：不是形态，是「求值时上游还没产出」

同一 execution `edf041e7-…` 内的时间线：

```
10:25:15.248  dispatch value-investor--mid
10:25:15.266  dispatch risk-agg / risk-con / risk-neu
10:25:15.276  risk-con  求值 input_mapping  ⇒ value-investor--mid 尚不存在（WARN）
10:26:18.623  value-investor--mid completed（30315ms）
10:27:51.018  value-investor--long  completed
```

⇒ risk 三节点在 `value-investor--mid` 完成**前 63 秒**就算完了 prompt（两者同属 `edf041e7`，可直接比对）。

`a-lockup--short` 那 8 条不能这么比：它的 completed 记录属于**另一个 execution**（`484c9a34`，10:32:46），
与 10:26:52 那次 miss 不在同一执行里 —— 日志混了多个 execution，跨 execution 对齐是无效推理。
但引用关系同型（见 1.2），结论不变。

### 1.2 为什么没有等：**没有边，也没有自动依赖推导**

`seed_stock_analysis.rs` 里 `risk-agg/con/neu` 的出边只有：

```rust
for rid in &["risk-agg", "risk-con", "risk-neu"] {
    edges.push(edge(&format!("e-{rid}-agg-risk"), rid, "agg-risk"));
}
```

`grep 'edge(&format!("e-value'` **零命中** ⇒ value 链与 risk 链之间没有任何依赖边。
而 risk 三节点的 `input_mapping` 由 `build_analyst_input_mapping(&tiered)` 统一生成（含 `value-investor--{mid,long}` 两档），`context_sources` 却**故意不含** value-investor（`seed:4877`）。

引擎侧也没有「按 `input_mapping` 的 `source_key` 首段推导软依赖」的机制 —— `context.variables` 是**全量快照**
（`value-verify` 那次 `variables keys count=456` 可证），所以只要上游**已完成**就取得到；
拿不到的唯一原因就是**调度时它还没跑完**。

⇒ 属性：**声明了依赖但没有建立等待关系**。v133 把映射改成「全档 23 条」时没有同步补边，
而引擎不会替你等 ⇒ 每次运行的成败取决于并行时序（偶发、看似随机）。

### 1.3 与 v175 的关系（必须说清）

| 场景 | v175 形态归一生效？ |
|---|---|
| 上游**已完成**、产物是扁平/混合形态 | ✅ 生效（把打分搬进 `verdict`） |
| 上游**尚未完成**（本次 39 条的主体） | ❌ 无效 —— 变量根本不在 `context.variables` |
| `a-lockup--short` 整档实例缺失/晚到 | ❌ 无效 |

⇒ 修 A 类要动**图**（补边或删无用映射），不是动归一逻辑。

### 1.4 修复（v176，2026-10-23）

| 动作 | 对象 | 依据 |
|---|---|---|
| **补边** | `value-verify--{mid,long}` → `risk-{agg,con,neu}`（6 条） | risk 与 value 同属 `debate-convergence` 下游**兄弟**（`e-convergence-p-risk-assess` vs `e-convergence-{vi_id}`）⇒ 不成环；`seeded_main_graph_is_acyclic` 按 Kahn 复核 |
| **删键** | `debate-convergence--{tier}` 的 value 两档键（`build_analyst_input_mapping(..., &[VALUE_INVESTOR_ID])`） | value-investor 是辩论的**下游**，补边必成环（v143 现网「Cycle detected」，`seed:3933-3941`）⇒ 拓扑上永不可满足，删键是语义诚实 |
| **降噪** | `agent_executor` 的 miss 分级 | `source_key` 首段不在 `context.variables` ⇒ 上游未产出（时序）记 **info**；在 variables 但下钻失败（形态/字段名错）仍 **warn** |
| **门禁** | 新增 `input_mapping_sources_are_ancestors`（`mod.rs` version_gate_tests） | `input_mapping` 首段若引用节点 id，必须是祖先或 `context_sources` 成员 —— 本次 39 条里绝大多数能在**不运行**时被抓出 |

未补的部分：risk 的 `context_sources` **不**加 value 报告正文 —— 风险视角的价值维度本就由 `t-valuation`（算法结论）承担，加全文会让 prompt 多 2~3k 字且与算法口径重复。

### 1.5 A′ 的处置边界

`a_*_raw` / `risk_drawdown_depth` 等未注入变量属**同类**（映射声明了、产出点缺失或晚到），
但它们的源节点在图里**是祖先**（故新门禁放行），属调度时序 ⇒ 由 §1.5 的日志分级降噪处理，
不改映射。若后续确认某键**结构上**无产出点，再按「删键」处置。

### 1.6 A′ `V57 自动补默认 unit`（原始读数）

```
["a_fundamentals_raw","a_lockup_raw","a_policy_raw","a_research_raw","a_sector_raw"]  ×2 组
["a_hot_money_raw","a_lockup_raw","a_sentiment_raw"] / ["a_policy_raw","a_sector_raw","a_sentiment_raw"]
["risk_drawdown_depth","risk_window_days"]
```

`analyst-brief` 的 `input_mapping` 键 = `{短名}__{tier}`（`analyst_short_key`，`seed:8627`），
未注入 = 对应分析师实例在该档未完成/不存在；`risk_drawdown_depth` / `risk_window_days`
是 `risk-level` 节点的输入，声明了但无产出点。**与 A 同族**，属另一批缺边/缺源。

---

## 2. B 类：供应商 504（外部，但需要配置层应对）

```
40  [SSE-DIAG] 响应头超时：上游网关挂起 … timeout_secs=10/15
40  [execute_llm_stream] LLM_CALL_FAILED: OpenAI API error 504 …
39  Agent LLM stream chunk 错误，已切换 fallback 流继续
 9  Agent LLM 返回空内容 (round 1/2~1/3, output_mode=Text)   ← 8 个分析师 + a-catalyst--long
 1  Node failed value-investor--mid … 已尝试回退模型仍失败 → Retrying attempt=1/5
```

- 全部集中在 `agnes-3.0-flash` → 回退 `agnes-2.5-flash`；属**供应商侧**（网关 504），不是本地缺陷。
- 但两类后果值得治：
  1. **空内容 9 次**：模型返回 200 但正文为空，走「注入强制总结指令」重试（8 条）。这是**质量**信号，建议按节点统计空返回率，超阈值切模型。
  2. **一次真失败**：`value-investor--mid` 首次失败（30s）后重试成功，整链多花 ~1 分钟。`retry.max_retries=5` 与「上游未完成就求值」叠加，会放大 A 类问题。

---

## 3. C 类：`未知断言类型 exists`（P2，真 bug）

`validation_executor.rs:60-137` 实现了 `expression / json_schema / not_null / non_empty / contains`，
`_` 分支记 false 并 warn。而模板里 `v-validate` 用的正是 `exists`：

```rust
// seed_stock_analysis.rs:5409-5416
assertions: vec![ValidationAssertion {
    assertion_type: "exists".into(),
    actual: Some("t-risk.output".into()),
    ...
}],
on_fail: "skip".into(),
```

⇒ 断言**恒 false** ⇒ `v-validate` **永远 Skipped** ⇒ 这个「结果完整性校验」自 v10 起从未真正生效。

**已修（v176，两处必须一起改，否则是假修复）**：

1. `validation_executor.rs` 的 `match` 增加分支：

   ```rust
   "exists" => resolve_var_path(path, context).is_some(),
   ```

   语义与 `not_null` 的区别：`exists` 只要求路径能解析出值（含 null）。

2. `seed:5409` 的 `actual` 路径 `t-risk.output` → **`t-risk.result`** —— ToolNode 的输出形态是
   `{tool_name, result, node_id}`（`tool_executor.rs:142-155`），**没有** `output` 字段。
   只补 ① 不补 ② ⇒ 断言依旧恒 false（"改了个永不生效的判据"式假修复）。

⚠ 行为变化：`v-validate` 从「恒 Skipped」变成「t-risk 有产物即 pass」。它没有出边（唯一入边是
`e-t-risk-v-validate`），不影响其它节点调度。

---

## 4. D 类：`take_asof_degradation_report 访问失败: task-local value not set`（P3）

`as_of.rs:410-418` 注释即写明「必须在 `with_degradation_log` scope 内调用，否则返回空 Vec」。
本次在 scope 外调用 ⇒ **本次运行的 as-of 降级记录被丢弃**（不影响数值，但 replay 审计少一段证据）。
需定位调用点是否在 `tokio::spawn`（task-local 不跨 task）。

---

## 5. E 类：`portfolio-risk-gate 结果缺少 action 字段，回落 portfolio-mgr`（P2，待查）

`decision.rs:977-988`：`portfolio-risk-gate` 有 `action` 就优先采用，否则回落 `portfolio-mgr`。
但 `portfolio-risk-gate.rhai` 本身**会输出** `action`（`:127` / `:140` / `:157` 读 `pm_result["action"]`）。

⇒ 需要确认这次是「gate 产物被 CodeNode 包装、`unwrap_node_output` 未解到 `result.action`」还是
「gate 走了无 action 的早退分支」。**若长期回落，风控门等于没生效**（与 F-2 同族：契约形态不一致）。

---

## 6. F 类：`[monitor_emitter] 通知渠道未装配（已注册 0 个）`（配置）

6 条，含义：价格告警/监控事件**不会外推**，只写前端事件 + `price_alerts` 落库。
属**未配置**（设置面板里没启用任何通知渠道），不是缺陷；但会让「无人值守告警」失效。

---

## 7. G 类：数据源降级（外部）

```
2  [asof] tencent 600178 合成的 quote 无成交额（该源日 K 线不含此字段）⇒ 不作为可用结果，试下一源
2  [eastmoney] 连接被掐断且无代理 (push2his.eastmoney.com/...)
1  [eastmoney] 连接被掐断且无代理 (.../kline)
2  [asof] eastmoney_push2 get_money_flow 取数失败
```

前两条是**判据正确**（缺成交额不用）而非缺陷；eastmoney 连接被掐属网络/风控，建议确认是否需代理配置。

---

## 8. H 类：日志噪音（P3）

| 来源 | 问题 |
|---|---|
| `code_executor` 的 `input_mapping snapshot: {...}` | 以 **WARN** 打印整份输入（单行可达数千字符），一次运行 6 条，淹没真警告 |
| `strict_mode enabled (output_mode=…)` ×60 | 每节点一次，属正常提示，建议 INFO |
| `V57 自动补默认 unit` | 有诊断价值，但与 A 类同因，宜汇总为一条 |

---

## 9. 本轮已执行（v176）

| # | 动作 | 落点 |
|---|---|---|
| 1 | **A 类补边**：`value-verify--{mid,long}` → `risk-{agg,con,neu}`（6 条） | `seed_stock_analysis.rs`（value-verify 构造段） |
| 2 | **A 类删键**：`debate-convergence--{tier}` 排除 value-investor 两档键 | `build_analyst_input_mapping(..., &[VALUE_INVESTOR_ID])` |
| 3 | **A 类降噪**：miss 分级（上游未产出 ⇒ info；路径下钻失败 ⇒ warn） | `agent_executor.rs` input_mapping 注入段 |
| 4 | **C 类**：补 `exists` 断言 + `t-risk.output` → `t-risk.result` | `validation_executor.rs` + `seed:5409` |
| 5 | **门禁**：`input_mapping_sources_are_ancestors`（祖先/context_sources 可达性） | `mod.rs` version_gate_tests |
| 6 | 换代 `TEMPLATE_VERSION` 175 → **176** | `seed_stock_analysis.rs` |

### 仍未处理（需外部条件或另行裁决）

- **B**（供应商 504）：外部；若要治，是「按节点统计空返回率 + 超阈值切模型」，不是改代码一行的事。
- **D**（as_of task-local）：需定位调用点是否在 `tokio::spawn` 外（task-local 不跨 task）。
- **E**（risk-gate 缺 action）：需核对 `unwrap_node_output` 对该节点产物的解包路径 —— 若长期回落，风控门等于不存在。
- **F / G / H**：配置、外部源、日志级别。

### 复核判据（下次运行）

1. 日志中 `input_mapping 取不到值` 的 **warn** 级应只剩「源节点在 variables 但字段错」那条通道；
   `value-investor--*` / `a-lockup--*` 这类应以 **info** 出现（时序）或彻底消失（补边生效）。
2. `v-validate` 状态从 `skipped` 变为 `completed`（前提：t-risk 成功）。
3. 库版本应为 176（否则 v175/v176 的图与脚本改动都不进库）。
