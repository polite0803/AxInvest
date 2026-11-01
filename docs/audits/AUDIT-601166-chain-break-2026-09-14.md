# AUDIT — 601166 决策链断裂（`bear-r3` 单点失败掐断全链）

- **日期**：2026-09-14
- **标的**：601166 兴业银行
- **运行**：`workflow_executions.id = fc9103dd-0c98-469f-bcff-591d680c409f`（15:59:58 → 16:08:09，491 秒）
- **落库**：`stock_analyses.id = 4310fada-8f47-4a3d-8a14-167dda56e8ef`，`decision_action = "数据缺失"`
- **性质**：**引擎缺陷（P0）**，首次触发；外部 LLM 抖动只是导火索

---

## 0. 一句话结论

`debate-bull-bear` 的**链尾辩手 `bear-r3`** 撞上上游 LLM 504：
它的 prompt 因 `contextSources` 逐节点累积而**天然全局最大（实测 85.8 KB）**，
恰好把 `header_timeout_secs` 公式推入 15s 档（`10 + ⌊body/64KB⌋ × 5`），而上游此时正处第 2 波拥堵；
该失败**既没有重试机会、也没有写入节点执行记录**，却作为 `Failed` 留在内存里；
主图「停滞检测」据 `upstream ∈ {Skipped, Failed}` 把**全部 27 个下游节点级联标为 `Skipped`**，
于是**风险分析、估值、组合管理、风控门、落库全链未执行**，执行被判 `partially_completed` 提前结束。
落库只留 `"数据缺失"` 与一句 `portfolio-mgr 节点未完成`，**真因完全丢失**。

**关键澄清 1**：本次断链与代码改动、与模板 v47 改动**均无因果关系**（证据见 §4）。
**关键澄清 2**：`bear-r3` 失败**不是随机抖动** —— 它是链上唯一必然最脆弱的一环（证据见 §3 层 1）。
**关键澄清 3**：`fail-closed` 级联跳过是**期望行为**；用户已裁决「下游不该继续，但应告知错误」，
故要修的是**告知层**与**重试机会**，不是容错策略（见 §6）。

---

## 1. 现象（落库实证）

```
status            = completed        ← 注意：db 层标 completed
decision_action   = "数据缺失"
decision_position = 0
decision_reasoning= 组合管理节点未产出有效决策（数据缺失，非观望结论）。
                    原因：portfolio-mgr 节点未完成（状态非 Completed）
blackboard len    = 601802 字符      ← 有数据，但只到 t-institutional-visits
```

黑板块的 key 中**没有任何** `t-valuation` / `portfolio-mgr` / `portfolio-risk-gate` / `risk-*` 节点条目。

---

## 2. 断点定位（可复算）

| 量 | 值 |
|---|---|
| 模板 `stock-analysis` 节点总数 | **62** |
| 本次运行实际产生的 node_executions | **39** |
| 差集 | **29** |
| `bear-r3` 的下游闭包（模板 BFS） | **27** |
| 差集 = bear-r3 自身(1) + 下游(27) + `sim-verify`(1) | **29 ✅ 精确吻合，零剩余** |

`bear-r3` 的 4 条出边（模板 `edges` 实证）：

```
bear-r3 -> debate-convergence
bear-r3 -> value-investor
bear-r3 -> p-risk-assess      ← 用户所说「风险分析」
bear-r3 -> t-scoring
```

`p-risk-assess` 及其下游 `risk-agg` / `risk-con` / `risk-neu` / `agg-risk` / `risk-convergence` **整族未执行** —— 这正是「连风险分析都过不去」的直接原因。

**缺失清单（29）**：`bear-r3`、`debate-convergence`、`value-investor`、`p-risk-assess`、`risk-agg`、`risk-con`、`risk-neu`、`agg-risk`、`risk-convergence`、`t-scoring`、`t-valuation`、`t-risk`、`raw-data`、`cls-risk-level`、`v-validate`、`data-quality`、`research-mgr`、`trader`、`portfolio-mgr`、`portfolio-risk-gate`、`regime-weights`、`rule-check`、`quality-gate`、`quality-fallback`、`decision-explainer`、`sim-verify`、`notify-result`、`store-result`、`end-output`

运行时间线（相对 t0，秒）：

```
120.3 -> 169.5  [completed] bull-r1
169.5 -> 223.2  [completed] bear-r1
223.2 -> 264.6  [completed] bull-r2
264.6 -> 320.4  [completed] bear-r2
320.4 -> 367.1  [completed] bull-r3
367.1 -> ~457.6 [ 缺失   ] bear-r3        ← 断点（无 node_execution 记录）
120.3 -> 457.6  [completed] debate-bull-bear (337222ms)
                          ↑ 之后无任何节点被调度，执行即结束
```

---

## 3. 根因链（6 层，含代码行号）

### 层 1 — 触发：为什么**必然**是 `bear-r3`（外部 × 结构，非随机）

`bear-r3` 的报错：

```
UNSUPPORTED_PROVIDER: Agent LLM stream error:
{"category":"retryable","code":"LLM_CALL_FAILED",
 "detail":"OpenAI API error 504: response headers not received within 15s.
           The upstream gateway is likely hung."}
```

该错误被记入 `debate-bull-bear` 的 `output.consensus.entries[].output.error`（`agent: "bear-r3"`）。

**它不是抖动，而是整条链上唯一必然最脆弱的一环。** 四步证据：

**1a. 辩论链的 prompt 是单调递增的** —— `contextSources` 逐节点累积前一个辩手的输出。
日志（`~/.axagent/session_logs/agnes-2.5-flash.jsonl`）逐条实测每次 dispatch 的字节数：

| 记录 | system chars/B | user chars/B | 对应辩手 | 增量 = 前一节点输出 |
|---|---|---|---|---|
| [102][103] | 4356 / 9865 | 4700 / 9034 | `bull-r1` | — |
| [104][105] | 7513 / 17398 | 8136 / 16794 | `bear-r1` | +3426 ✓ |
| [106][107] | 11036 / 25089 | 12023 / 24893 | `bull-r2` | +3877 ✓ |
| [108][109] | 13381 / 30595 | 14704 / 30652 | `bear-r2` | +2671 ✓ |
| [110][111] | 15707 / 35634 | 17386 / 36412 | `bull-r3` | +2672 ✓ |
| **[112][113]** | **18965 / 43328** | **21057 / 44513** | **`bear-r3`** | +3661 ✓ |
| [114][115] | 18965 / 43328 | 21057 / 44513 | `bear-r3` 第 2 次 dispatch | hash 完全相同 |

增量与前一节点输出长度**逐一吻合**（6/6）⇒ 这是结构性规律，不是巧合。
`bear-r3` 在链尾 ⇒ **请求体全局最大 ≈ 43328 + 44513 = 87841 B（85.8 KB）**。

**1b. 请求体积直接决定超时档位**（`providers/src/openai.rs:992-998`）：

```rust
// 固定 10s 的问题（2026-09-09/10 实测）：大 prompt 请求（聚合节点
// portfolio-mgr 308KB / candidate-mapper 149KB / data-quality 142KB）的
// 上游 prefill/排队时间远超 10s，头超时对其必然触发……
// 故改为随请求体积缩放：base 10s + 每 64KB 递增 5s，封顶 60s。
let header_timeout_secs: u64 = (10 + (body_size / (64 * 1024)) as u64 * 5).min(60);
```

代入实测值：`⌊87841 / 65536⌋ = 1` ⇒ `10 + 5 = **15s**` ✓
**公式反推的 15s 与报错文本的 15s 完全一致 —— 双向验证成立。**

各辩手的档位（按实测字节数）：

| 辩手 | body ≈ | 档位 | 结果 |
|---|---|---|---|
| `bull-r1` | 18.5 KB | 10s | ✅ |
| `bear-r1` | 33.4 KB | 10s | ✅ |
| `bull-r2` | 48.8 KB | 10s | ✅ |
| `bear-r2` | 59.8 KB | 10s | ✅（**刚卡在 64KB 门槛下方 4.2KB**） |
| `bull-r3` | 70.4 KB | **15s** | ✅ |
| `bear-r3` | **85.8 KB** | **15s** | ❌ **超时** |

⇒ 阈值模型是**线性**的（每 +64KB 只多给 5s），而上游 prefill 复杂度随 prompt 增长是**超线性**的。
89KB 中文 prompt ≈ 3 万 token 量级，15s 对它是**边界值**——所以它失败而 70KB 的 `bull-r3` 成功。

**1c. 上游在 16:06 处于第 2 波拥堵**（同一次运行内的对照）：
16:01 已有第 1 波同款 504 打在 `a-policy`、`a-market-analyst` 上（`a-policy` 08:01:16.675 失败 → 08:02:00.956 重试成功）。
**主图节点重试成功了，容器体节点没有重试机会**（见层 2）。

**1d. 90.5 秒 = 2 次 dispatch × 每次 3 次内部重试 × 15s**：
`debate-bull-bear` 窗口 337s − 5 个成功辩手串行 246.8s = **90.5s**；`⌈90.5/15⌉ = 6` 次尝试 = 2 × 3。
（2 次 dispatch 的来源见层 5 新增小节「假收敛」。）

**1e. 模板声明的超时对这条链完全无效**：
`bear-r3.streamChunkTimeoutSecs = 300`，但 300s 是 **body 流分片超时**，而 504 发生在**响应头阶段**——
真正决定成败的是上面那个 15s 的 `header_timeout_secs`。**声明值与实际生效值相差 20 倍，且没有任何提示。**

### 层 2 — 放大器：容器体子节点**没有** retry / timeout（本项目缺陷）

| 路径 | 代码位置 | 重试 | 超时包装 |
|---|---|---|---|
| 主图节点 | `rt-workflow/src/work_engine/engine/mod.rs:4531-4535` | ✅ 有 | ✅ `tokio::time::timeout(node_timeout, …)` |
| **容器体子节点** | `engine/mod.rs:6273` | ❌ **无** | ❌ **无** |

```rust
// mod.rs:6273（容器体路径）
let dispatch_result = engine.dispatcher.read().await.dispatch(&node, &ctx).await;
//                                                  ↑ 裸调用，无 timeout、无 retry
```

而模板里**已声明**重试：

```
bear-r3  retry={"enabled":true,"maxRetries":1,...}  timeout=null  continueOnFail=false  parentId=debate-bull-bear
```

⇒ **声明未接线**。后果对照（同一次运行内）：

- `a-market-analyst`（主图）第 1 次 504 → **重试 → 成功**
- `bear-r3`（容器体）504 → **无重试 → 直接 Failed → 杀死整条链**

### 层 3 — 灾难化：fail-open 与 fail-closed 语义冲突（本项目缺陷）

**容器侧 fail-open**（`executors/debate_executor.rs:126-145`）：

```rust
match dispatch_fn(step_id.clone(), round_ctx).await {
    Ok(output) => { round_results.insert(step_id.clone(), output.output); },
    Err(e) => {
        tracing::warn!("Debate debater '{}' failed in round {}: {}", step_id, round, e);
        round_results.insert(step_id.clone(),
            serde_json::json!({ "error": e.to_string(), "round": round }));   // 吞掉，继续
    },
}
// 结尾硬编码： "status": "completed"   ← 无论几个辩手失败
```

**主图侧 fail-closed**（`engine/mod.rs:2820-2870` 停滞检测的级联跳过）：

```rust
let has_blocked = wf.node_states.values().any(|s| matches!(s.status, Pending | Ready));
if has_blocked {
    loop {  // 迭代式传播，直到不动点
        let keys_to_skip = ...filter_map(|(key, state)| {
            if !matches!(state.status, Pending | Ready) { return None; }
            let upstream_terminal = wf.edges.iter().all(|e| {
                if e.target != *key { return true; }
                matches!(wf.node_states.get(&e.source).map(|s| &s.status),
                         Some(NodeStatus::Skipped | NodeStatus::Failed))   // ← 判据
            });
            upstream_terminal.then(|| key.clone())
        })...;
        for key in keys_to_skip { state.status = NodeStatus::Skipped; }
    }
    wf.status = WorkflowStatus::PartiallyCompleted;   // mod.rs:2868
}
```

**`bear-r3` 的双重身份是冲突根源**：它既是 `debate-bull-bear` 的容器体子节点（容器容忍其失败），
**又是主图节点**（带着 4 条出边）。容器按前者语义忽略失败，主图按后者语义判定下游全灭。

⇒ **一次子节点失败 = 27 个下游节点全部 `Skipped`。**

**终态判定**（`mod.rs:1726-1746`）：

```rust
let all_ok = ...all(|s| matches!(s.status, Completed | Skipped));
let any_skipped = ...any(|s| s.status == Skipped);
if all_ok && any_skipped { workflow.status = PartiallyCompleted; }   // ← 命中
```

主循环随后据此 `break`（`mod.rs:4588-4599`），**执行结束**。

### 层 4 — 不可观测：失败路径**不落库**（本项目缺陷）

`build_debate_body_dispatch`（`engine/mod.rs:6085-6395`）两条路径**不对称**：

| 路径 | 状态更新 | `record_node_execution` |
|---|---|---|
| Ok（6276-6351） | `Completed` | ✅ **有**（`status: "completed"`） |
| **Err（6353-6391）** | `Failed`（仅内存） | ❌ **无** |

```rust
Err(e) => {
    tracing::warn!("Debate/Swarm body node failed");     // 仅 warn
    engine.update_node_status_for_execution(..., NodeStatus::Failed, ...).await.ok();  // 仅内存
    Err(e)                                                // ⚠️ 没有 record_node_execution
}
```

⇒ `bear-r3` 在 `node_executions` 里**完全不存在**：既不是 completed，也不是 failed。
**前端、审计、AI 三方都看不到「bear-r3 失败」这件事。**

这是本次误判的直接原因 —— 用 `node_executions` 判断「节点是否执行过」在本项目**不可靠**。

### 层 5 — 用户可见后果

`portfolio-mgr`（`code` 节点）从未执行 ⇒ 落库走「组合管理节点未产出有效决策」分支
⇒ `decision_action = "数据缺失"`、`position = 0`。

### 层 6 — 旁路发现：**「3 轮辩论」其实是 1 轮 + 1 份复制 + 假收敛**

追查「为什么 90.5s 里 bear-r3 被 dispatch 了 2 次」时发现的独立缺陷。

`debate_executor.rs` 是 `for round in 0..max_rounds { for step in debater_steps { dispatch } }`，
`max_rounds = 3`、6 个辩手 ⇒ 本该 18 次调用。而运行记录里每个成功辩手**只有 1 条**，
第 2 轮输出与第 1 轮**字节级完全相同**（3877/2672/3426/2671/3661 逐一相同）。

原因是 09-08 的一项修复（`mod.rs:6185-6224`）——**修得对，但产生了副作用**：

```rust
// 容器体（Debate/Swarm/Loop）按轮次重复驱动同一批 body 节点……
// 处理：Completed 且已有输出 → 直接复用返回（不重跑 LLM）；
//       其他终态（Failed/Skipped）或无输出 → 重置为 Ready 重跑。
Some(NodeStatus::Completed) => {
    if let Some(existing) = wf.results.get(&node_id).cloned() {
        return Ok(NodeOutput { output: existing, ... });   // ← round 1 直接返回 round 0 的对象
    }
}
Some(s) if s.is_terminal() => { st.status = NodeStatus::Ready; }   // ← Failed 会重跑
```

⇒ round 1 里 5 个成功辩手**复用跳过（0 秒）**，只有 `bear-r3`（Failed 终态）被重置重跑
⇒ 这才有了「2 次 dispatch × 45s = 90.5s」。

**而收敛检测（`executors/mod.rs:210-215`，`matching/total >= 0.80` 即收敛）拿到的是同一份对象**
⇒ 相似度恒为 1.0 ⇒ `5/6 = 0.833 ≥ 0.80` ⇒ **必然判定收敛** ⇒ `total_rounds = 2`。

**结论**：`max_rounds = 3` 的辩论，实际是「1 轮真实辩论 + 1 轮复制品 + 假收敛」。
且注释已说明根因——辩手的 `contextSources` 不引用 `__debate_history__` / `__debate_round__`
（`bear-r3` 只引用 `["bull-r1","bear-r1","bull-r2","bear-r2","bull-r3","analyst-brief"]`），
所以第 2 轮的 prompt 与第 1 轮**完全相同**，重跑本身确实无信息增量（这也解释了日志里 [112][113] 与 [114][115] 的 hash 相同）。

⇒ 二选一：**要么**让辩手消费轮次变量（真正多轮），**要么**把 `max_rounds` 老实设成 1（省一次 45s 的必败重跑）。

---

## 4. 与「修改」的关系（澄清，含时间线实证）

用户判断「你修改完后重跑就过不去了」。**该判断在时间上成立，在因果上不成立**：

| 时间（09-14） | 事件 | 证据 |
|---|---|---|
| 01:26:42 | **全天最后一次 Rust 源码修改**（`analysis-engine/src/opc/industry_config.rs`）；引擎文件最后修改为 **09-13 22:58** | 文件 mtime 全量扫描 |
| **11:01:17** | 模板 `stock-analysis` → **v47** 写入 | `workflow_templates.updated_at = 1789354877286`；快照 `workflow_template_versions` = 41 条 |
| 11:01:14 | 运行 001313 成功走完全链 | `650e8e42`：`has_bear3/has_pm/has_tval` 全 true |
| **14:42:58** | **601698 运行成功走完全链** | `26115a5c`：同上全 true |
| **15:59:58 → 16:08:09** | **601166 运行 → 断链** | `fc9103dd`：三项全 false |

**两条独立否证**：

1. **模板 v47 ≠ 断链原因**。快照 diff 实证：v47 相对上一版**只新增一个节点 `sim-verify`**，
   未增删任何边，`debate-bull-bear` / `bear-r3` / `bull-r3` 定义**逐字段相同**。
   且 v47 写入后 **14:42 的 601698 走完了全链**。
2. **代码改动 ≠ 断链原因**。`11:01 → 15:59` 区间内**没有任何 `.rs` 文件被修改**。

⇒ 触发因素是 **15:59 那次运行时 `bear-r3` 的 LLM 504**。
⇒ 但**引擎缺陷是既有的**（代码形态早于 09-13 22:58），只是**今天第一次被触发**（全库 `数据缺失` 仅此 1 条，历史 0 条）。

**附带**：`sim-verify` 是 `enabled: false` 且不连边的「图示节点」（实际执行走决策落库后的后端挂钩 `sim_hook::spawn_simulation_after_decision`），
它未出现在 `node_executions` 属**设计如此**，不是缺陷。但它在停滞检测中会被「零入边 ⇒ 真空满足」判据标为 `Skipped`（无害）。

---

## 5. 缺陷清单（按严重度）

| # | 缺陷 | 位置 | 影响 | 严重度 |
|---|---|---|---|---|
| **D1** | 容器体失败路径**不落库** | `mod.rs:6353-6391` | 失败不可见；审计误判；真因丢失 | **P0** |
| **D2** | 容器体子节点**不消费 retry/timeout** | `mod.rs:6273` | 模板声明 `maxRetries:1` 失效；一次抖动即终结 | **P0** |
| **D3** | fail-open 容器 × fail-closed 主图 **语义冲突** | `debate_executor.rs:126-145` × `mod.rs:2820-2870` | 单点子节点失败 ⇒ 下游全灭 | **P0** |
| **D4** | `continueOnFail=false` 与容器 fail-open **自相矛盾** | 模板 `bear-r3.continueOnFail=false` × 容器实现 | 声明与行为相反 | **P1** |
| **D5** | 落库未记录**被跳过的节点清单** | 决策落库链 | 「数据缺失」不自解释 | **P1** |
| **D6** | **超时声明值与实际生效值脱节 20 倍** | 模板 `streamChunkTimeoutSecs=300` × `openai.rs:998` 实际 15s | 用户以为有 300s 余量，实际 15s 判死 | **P0** |
| **D7** | 超时模型**线性** vs 上游 prefill **超线性** | `openai.rs:996-998` | 85.8KB 请求 15s 落在临界区 ⇒ 链尾必先失败 | **P1** |
| **D8** | 容器 `status` **硬编码 `"completed"`** | `debate_executor.rs:169-177` | 无论几个辩手失败都自称成功；下游质量闸门失去依据 | **P0** |
| **D9** | 多轮辩论实为**1 轮 + 复制品 + 假收敛** | `mod.rs:6193-6209` × `executors/mod.rs:210-215` | `max_rounds=3` 名不副实；多耗一次必败重跑 | **P1** |
| **D10** | `PartiallyCompleted` 混「分支跳过」与「故障跳过」 | `mod.rs:1726-1746` | **208/286 运行都是它**，用户无法分辨正常/异常 | **P0** |

> 本次同时暴露：**4 个 tool 节点 timeout 37s + 2 个 agent 节点 504**，说明上游 LLM/tool 稳定性本身也是风险源（非本次根因，但会反复触发 D1–D3）。

---

## 6. 修复方案（依 09-14 用户裁决重排）

> **用户裁决（2026-09-14）**：`bear-r3` 失败时**下游不该继续**，**但应该告知错误**。

⇒ 本文档初版的「方案 A：让下游继续（容器 fail-open 传染主图）」**作废**。
`fail-closed` 级联跳过是**期望行为，不修**。
⇒ 真正要修的是三件事：**① 把失败如实说清（告知）② 让失败能被重试救回（根因·放大器）③ 消除「结构上必然失败」的诱因（根因·诱因）**

### A 类 —— 告知（符合用户裁决，风险低，建议先做）

| 编号 | 动作 | 位置 | 风险 |
|---|---|---|---|
| **A1** | Err 分支补 `record_node_execution(status:"failed", error_code)` | `mod.rs:6353-6391` | **零**（纯埋点补全） |
| **A2** | 去掉容器 `status` 硬编码 `"completed"`（**D8**）：按失败辩手数输出 `partial`/`failed` + 失败清单 | `debate_executor.rs:169-177` | 低（下游若有 `status=="completed"` 判断需同步） |
| **A3** | `PartiallyCompleted` 拆语义（**D10**）：`skipped_nodes[].reason = upstream_failed \| branch_not_taken` | `mod.rs:1726-1746` + 落库链 | 低 |
| **A4** | 决策落库写 `skipped_nodes` + 失败原因；前端决策卡片显示「因 X 失败，N 个节点被跳过」 | 落库 + 前端 | 低 |
| **A5** | 统一 `continueOnFail` 语义（**D4**），消除「模板声明 vs 容器行为」自相矛盾 | 模板 + 容器实现 | 低 |

**A 类为什么最重要**：现状是「故障」与「正常分支跳过」共用同一个 `PartiallyCompleted`（**208/286 运行都是它**），
用户看到「数据缺失」无从判断是配置意图还是引擎故障。**这正是「应该告知错误」的落点。**

### B 类 —— 让失败可被救回（根因·放大器）

| 编号 | 动作 | 位置 | 风险 |
|---|---|---|---|
| **B1** | 容器体路径接上 `timeout + retry`（复用主循环封装），使模板的 `retry:{maxRetries:1}` 真正生效 | `mod.rs:6273` | 中 |
| **B2** | 上游 504 按 `category:"retryable"` 做**带退避**的 provider 级重试/降级 | provider 层 | 中 |

**B1 大概率有效，依据是本次运行内的对照**：
`a-policy` / `a-market-analyst` 吃了**同款 504，重试后都成功了**（`a-policy` 08:01:16 失败 → 08:02:00 成功）。
主图有重试所以活下来，容器体没有 —— **不对称就在这里，不在阈值。**

### C 类 —— 消除结构性诱因（**需裁决**）

| 编号 | 动作 | 位置 | 需裁决 |
|---|---|---|---|
| **C1** | 超时声明与生效对齐（**D6/D7**）：让 `streamChunkTimeoutSecs` 真正生效，或修正 `header_timeout_secs` 公式 | `openai.rs:996-998` | **阈值取值** → §7-① |
| **C2** | 辩论链 prompt 体积治理（85.8KB）：改「全文传递」为「摘要传递」 | 模板 | 是否接受信息损失 |
| **C3** | 多轮辩论二选一（**D9**）：让辩手消费 `__debate_history__`，或 `max_rounds=1` | 模板 / 引擎 | → §7-② |

---

## 7. 待裁决（2 项，均在根因层）

### ① 上游头超时阈值怎么定？

事实：`header_timeout_secs = 10 + ⌊body/64KB⌋ × 5`（封顶 60s）。
85.8KB 得 15s **失败**，70.4KB 得 15s **成功** ⇒ 临界区就在这一步之隔。

| 选项 | 含义 | 代价 |
|---|---|---|
| **①-a（推荐）** | **不动公式，只补 B1** | 零风险。而「小请求快速失败」是该公式的刻意设计（注释明写：避免大请求拖垮 `step_timeout(120s)`） |
| ①-b | 每 64KB 增量由 5s 提到 10s（85.8KB ⇒ 20s） | 大 prompt 节点等待翻倍，可能撞 `step_timeout(120s)` |
| ①-c | 让模板 `streamChunkTimeoutSecs` 决定头超时 | 语义变更大（它本意是 body 流分片超时，不是头超时） |

**推荐 ①-a**：`bear-r3` 的问题**不是阈值不够，而是没有重试机会**（D2）。
调大阈值只是掩盖症状，并把风险转嫁给 `step_timeout`。

### ② 多轮辩论要不要救活？

- **②-a**：把 `__debate_history__` 加进辩手 `contextSources` → 真多轮（但调用量与耗时 ×3）
- **②-b（我倾向）**：承认单轮，`max_rounds` 改 1。当前「3 轮」本来就是 1 轮 + 复制品，改成 1 只是**去掉假象**，还省一次必败重跑（45s）
- 这需要你判断：产品上的「多空辩论」是否本来就只需要一轮交锋？

---

## 8. 取证文件（全部在 `output/`）

| 文件 | 内容 |
|---|---|
| **`slog-lines.txt`** | **根因关键证据**：`~/.axagent/session_logs/agnes-*.jsonl` 实测每次 dispatch 的 prompt 字节数 —— 辩论链单调递增，`bear-r3` = 87.8 KB ⇒ 15s 档 |
| **`debate-cfg.txt`** | 容器与 6 个辩手完整配置（`contextSources` 累积链、retry/timeout/maxTokens/subGraph 边） |
| **`debater-cfg.txt`** | `debater_steps` / `max_rounds` / 各辩手 `modelRole` 与上下文源（证明 prompt 为何递增） |
| **`debate-rounds2.txt`** | 本次 debate 逐轮逐辩手输出（**round0 与 round1 字节级相同 ⇒ 假收敛**） |
| **`ne-timeline.txt`** | 39 节点起止时间 + input/output 体积（90.5s 缺口可复算） |
| **`pg-504-all.txt`** | 286 次运行的 504 分布（**仅 5 次含 504**）与 status 分布（**208 次 `partially_completed`**） |
| `ne-summary.txt` | 39 个节点逐项状态 + 6 个异常节点详情 |
| `ne-spec.txt` | 时间线 + `debate-bull-bear` 输出（含 `bear-r3` 的 504 错误） |
| `tpl-summary.txt` | 模板 62 节点 / 153 边 + `debate-bull-bear` 下游 BFS |
| `tpl-bear3.txt` | **`bear-r3` 下游 27 节点 + 未执行 29 节点对照** |
| `tpl-diff-snap-vs-now.txt` | v47 相对上一版的 diff（**仅新增 `sim-verify`**） |
| `pg-run-matrix.txt` | 15 次运行的 `bear-r3`/`sim-verify`/`portfolio-mgr` 交叉矩阵 |
| `pg-action-dist.txt` / `pg-missing-list.txt` | 全库 action 分布；`数据缺失` 仅此 1 条 |
| `mtime.txt` | 源文件最后修改时间（全天截止 01:26） |
| `timeline.txt` | 关键时间戳换算 |

复算脚本：`output/q504.mjs`（504 分布）、`output/analyze-ne.mjs`、`output/analyze-tpl.mjs`、`output/analyze-debate.mjs`、`output/diff-tpl.mjs`、`output/dump-spec.mjs`
