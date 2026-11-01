# PROPOSAL — 执行可观测性与超时治理（T-1 立项书）

> 立项时间：2026-09-12
> 状态：**已立项（待排期）**
> 前置纠正：原 T-1 定性「002353 链 5.5 小时超长」**已推翻**（数据误读），真实问题重述见 §1
> 关联报告：`output/legacy-items-c2-c1-t1-20260911.md` §3

---

## 1. 背景：原定性被推翻后的真实问题

### 1.1 原定性（错误）

> `e1e7dcff` 跑 19893 秒（5.5 小时）才被清理。

### 1.2 实证纠正

| 事实                                     | 数据                                                                                                                     |
| ---------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `002353` 本身不慢                        | `stock_analyses` 中 `748f818e` = **completed，743.4 秒（12.4 分钟）**                                                    |
| `e1e7dcff` 状态                          | `cancelled`（不是超时，是被清理）                                                                                        |
| `dur = updated − created` 对 `cancelled` | **不可信** —— `updated=09-11T04:19:54` 与同一时刻其他执行批量置为 `cancelled` 同时发生，反映的是**清理时刻**而非运行时长 |
| 健康基线（completed 19 条）              | **324s ~ 1636s（5.4 ~ 27.3 分钟）**，无一条超长                                                                          |

### 1.3 真实问题（本立项要解决的）

**不是「分析慢」，而是「长跑不可观测 + 卡死不自愈」**：

| # | 问题                                   | 证据                                                                                                                                                                       |
| - | -------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A | **节点级进度只有「完成」没有「开始」** | `stockAnalysisStore.ts:2014-2015` 已用 `completedNodes/totalNodes` 算百分比，但事件仅在节点**完成**时发                                                                    |
| B | **DB 无 running 记录**                 | `node_executions` 的**唯一写入点**是 `complete_execution()`（`engine/mod.rs:5596-5625`），从内存 `state.node_records` **一次性序列化**；执行中途 DB 查不到任何进度         |
| C | **无执行级超时**                       | 引擎仅有 `tool_timeout`（默认 30s，工具节点）+ `step_timeout`（默认 300s，LLM/agent 节点）；**无整个 workflow execution 的超时**，卡死执行只能等人工取消或下次启动批量清理 |
| D | **前端无法区分「正在跑」与「已卡死」** | 进度条长时间不动既可能是 LLM 长调用（正常 1-5 分钟），也可能是卡死                                                                                                         |

**业务影响**：用户发起分析后，在 19 个节点的 12~27 分钟里，除 `streaming` 增量的少数节点外
**无任何"正在进行哪个节点"的可见性**；一旦执行卡死，用户既拿不到结果也不知道要等多久。

---

## 2. 关键技术底座（已存在，本立项复用而非新建）

**这是本次立项最重要的前提核实结论** —— T-1 的改造点**大部分是「接线」而非「造轮子」**：

| 已有设施                                                                       | 位置                                                   | 现状                                                                      |
| ------------------------------------------------------------------------------ | ------------------------------------------------------ | ------------------------------------------------------------------------- |
| 节点级钩子事件 `WorkflowNodePreExecute` / `PostExecute` / `PostExecuteFailure` | `harness/src/runtime_types/hooks.rs:56-58`             | ✅ **已定义**；`dispatcher.rs:163` PreExecute 处**已触发**（且可 veto）   |
| Hook sink 实现                                                                 | `crates/harness/src/workflow_hook_sink.rs`             | ✅ 已存在                                                                 |
| 应用层进度事件通道（`app.emit`）                                               | `src/commands/stock_workflow/core.rs:520-575`          | ✅ 已存在，**含** `workflow-step-complete` / `-error` / `-delta` 三个分支 |
| 前端进度百分比                                                                 | `stockAnalysisStore.ts:2009-2015`                      | ✅ 已消费 `completedNodes/totalNodes`                                     |
| 节点级超时                                                                     | `engine/mod.rs:3199-3202`（工具）/ `:2559`（LLM 子步） | ✅ 已有，但**粒度是节点不是执行**                                         |

**缺口（本立项要补的三个洞）**：

1. 事件分支**缺 `started`** —— `core.rs:547` 的 `_ => return` 会**静默吞掉**任何未知状态，
   因此即使引擎发起始事件，前端也收不到
2. `node_executions` **只在结束时写一次** —— 缺增量写入
3. **没有执行级超时** —— 无 watchdog

---

## 3. 目标（三项，均为「补缺口」）

| #      | 目标               | 用户可见效果                                                                       |
| ------ | ------------------ | ---------------------------------------------------------------------------------- |
| **P1** | 节点**开始时**可见 | 前端实时显示「当前正在执行：t-valuation（第 12/19 节点）」，长节点期间进度不再静止 |
| **P2** | 执行级超时         | 单次执行超 40 分钟自动置 `cancelled` 并给出明确原因，不再依赖人工清理              |
| **P3** | 已运行时长         | 前端显示「已运行 8 分 32 秒」，可用它与健康基线（5.4~27.3 分钟）对照判断是否异常   |

## 3.1 非目标（明确排除，防止范围蔓延）

- ❌ 不优化分析速度（健康基线 5.4~27.3 分钟属可接受范围，无证据表明慢）
- ❌ 不改节点并发度 / 重试策略 / 供应商降级逻辑
- ❌ 不做「执行续跑」（断点恢复）—— 需要状态持久化重设计，另立项目
- ❌ 不改 `cancelled` 语义（该语义正确，只是 `dur` 字段对 `cancelled` 无意义）

---

## 4. 技术方案

### P1 — 节点开始可见（两段改动）

**段 1：引擎侧发起始事件**（`rt-workflow`）

`dispatcher.rs:163-172` 的 PreExecute 钩子点已带 `execution_id / workflow_id / node_id / node_type`，
是最自然的接入点。但钩子是「可 veto 的用户脚本钩子」，**不应混入进度上报语义**。

**推荐方案**：在引擎的 `StepProgressEvent` 产出链路中新增 `status: "started"` 事件（节点 dispatch 前），
与既有 `completed / failed / timeout / streaming` 并列。

**段 2：应用层补分支 + 写 DB**（`src/commands/stock_workflow/core.rs`）

```rust
// core.rs 事件映射 match 中新增（当前 _ => return 会吞掉）
"started" => (
    "workflow-step-start",
    serde_json::json!({
        "conversationId": format!("wf-{}", wf_id),
        "stepId": event.node_id,
        "nodeTitle": event.node_title,     // 需确认 payload 是否携带，缺则补
        "index": event.completed_nodes + 1,
        "totalNodes": event.total_nodes,
        "startedAt": now_ms(),
    }),
),
```

**DB 增量写入**：`node_records` 目前是内存态。最小改动是**复用 `workflow_executions.node_executions`
这一 JSON 列做「进行中快照」**（追加一条 `{node_id, status:"running", started_at}`），
在节点开始与结束时各更新一次 —— 与 `complete_execution()` 的最终全量覆盖不冲突。

> ⚠️ 分层约束：`rt-workflow` 是**纯引擎 crate，无 DB 依赖**。所有 DB 写入**必须在应用层**
> （`src/commands/`）完成，不得为省事给引擎 crate 引入 dao 依赖。

### P2 — 执行级超时

**方案**：`RunOptions` 新增 `execution_timeout: Duration`（默认 40 分钟，可配），
在执行主循环外包一层 `tokio::time::timeout`，超时后：

1. 将 execution 置 `cancelled` 并写入 `error = "execution timeout after 40m"`
2. 发 `workflow-error` 事件（复用既有通道）
3. 释放并发槽与运行中节点的资源

**替代方案（更轻，但不推荐）**：应用层 watchdog 定时扫描 `workflow_executions` 中
`status=running AND now - created_at > 40min` 的行 → 置 `cancelled`。
**不推荐原因**：进程重启后无法区分「真卡死」与「正在跑」，容易误杀（这正是当前批量清理的行为模式）。

**必须同时修**：清理逻辑写入 `cancelled` 时，`updated_at` 语义被污染（§1.2）。
建议**新增 `cancelled_at` 或 `duration_ms` 独立列**，让 `updated_at − created_at` 不再被当作时长。

### P3 — 前端已运行时长

`stockAnalysisStore.ts` 新增 `currentNode: { id, title, index, total }` 与 `elapsedMs`，
由 `workflow-step-start` 驱动计时器（`setInterval` 1s，`workflow-completed/error` 时清除）。

**必须先审计事件订阅范式**：面板事件订阅是模块级单例、**永不解绑**（Tabs `destroyOnHidden` 场景），
回调用 `useStore.getState()` —— 新增监听必须沿用该范式，否则会因 Tabs 切换丢失订阅。

---

## 5. 验收判据

| #  | 判据                                                                                       | 验证方式                     |
| -- | ------------------------------------------------------------------------------------------ | ---------------------------- |
| V1 | 发起分析后 ≤ 2s 内前端显示「正在执行：<节点名>（i/N）」                                    | 实跑观察 + DevTools 事件日志 |
| V2 | 长节点（LLM 5 分钟）期间 UI 显示该节点 + 递增时长，不静止                                  | 实跑观察                     |
| V3 | 执行中途查 `workflow_executions.node_executions` 含 `running` 条目                         | DB 直查                      |
| V4 | 构造超时场景（临时把阈值调到 60s）→ 执行被自动 `cancelled` 且 error 含 timeout 原因        | 实跑 + DB 复核               |
| V5 | 门禁全绿：clippy --workspace --all-targets -D warnings / fmt / tsc / dprint / i18n 11 语言 | 命令                         |
| V6 | 零回归：正常执行（12~27 分钟）的最终 `node_executions` 内容与改动前结构一致                | 改动前快照对比               |

---

## 6. 风险与取舍

| 风险                                    | 等级   | 缓解                                                                                                    |
| --------------------------------------- | ------ | ------------------------------------------------------------------------------------------------------- |
| 每节点增发事件 + DB 写入 → 执行开销上升 | 低     | 19 节点 × 2 次写 ≈ 38 次轻量 UPDATE；相比 12~27 分钟执行时长可忽略                                      |
| 超时阈值过短误杀长跑（辩论/研究报告）   | **中** | 默认 40 分钟 = 健康基线上限 27.3 分钟的 1.47 倍；且必须可配置（`seed_variables.rs` 加变量，禁止硬编码） |
| 前端事件新增导致 store 复杂度上升       | 中     | 复用既有单例订阅范式；新增字段与既有 `progress` 字段并存，不重构                                        |
| 超时中断留下半成品状态                  | 中     | 复用既有 `workflow-error` 通道 + 尾链 `continue_on_fail` 语义，让已产出结果仍可落库                     |

---

## 7. 工作量与优先级

| 子项                         | 改动面                                                                  | 预估 | 优先级             | 依赖                  |
| ---------------------------- | ----------------------------------------------------------------------- | ---- | ------------------ | --------------------- |
| P1 引擎发起始事件            | `rt-workflow`（engine）+ 可能 `harness` payload 结构                    | 中   | **P0**（收益最大） | 需重编译              |
| P1 应用层分支 + DB 增量写    | `src/commands/stock_workflow/core.rs`                                   | 小   | **P0**             | 同上，同批编译        |
| P3 前端当前节点 + 时长       | `stockAnalysisStore.ts` + 面板组件 + 11 locales                         | 小   | **P1**             | P1                    |
| P2 执行级超时                | `engine/mod.rs` RunOptions + 清理逻辑 + `cancelled_at` 列（+migration） | 中   | **P1**             | 需 migration + 重编译 |
| 附：`cancelled` 时长语义修复 | migration + 写入点                                                      | 小   | P2                 | P2                    |

**建议排期**：P1 整条链（引擎+应用层+前端）作为一个批次做完，可独立交付价值；
P2 涉及 migration 与超时策略，独立批次。

---

## 8. 与本次三件事的关系

| 项 | 动作           | 状态                                          |
| -- | -------------- | --------------------------------------------- |
| 1  | C2 路径 Y 执行 | ✅ 已完成（2026-09-12）                       |
| 2  | C2 路径 Z 排期 | ✅ 已排期 → `PLAN-VALUATION-CONFIG-WIRING.md` |
| 3  | T-1 立项       | ✅ 本文件                                     |

---

## 9. 实施记录与立项结论修正（2026-09-12）

### 9.1 立项判断有重大偏差 —— 逐层核实后的真相

实施前按「先查已有实现再动手」逐一核实，结果：**原以为要新建的三项里，两项早已实现**。

| 原立项判断                                          | 实施前核实结果                                                                                                                                                                                                                                                                                 | 处置                       |
| --------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------- |
| P1-a「引擎需新增 started 事件」                     | ❌ **已存在** —— `engine/mod.rs:3144-3156` 在节点 dispatch 前就发 `status:"running"` 的 `StepProgressEvent`（且已填 total_nodes / completed_nodes / execution_id）                                                                                                                             | **零改动**                 |
| P1-b「应用层 match 缺 started，`_ => return` 吞掉」 | ❌ **分支已有** —— `core.rs:518` 有 `"running" => ("workflow-step-start", …)`                                                                                                                                                                                                                  | 仅补 3 个字段（见下）      |
| P3「前端无法区分在跑/卡死」                         | ✅ **真缺口** —— `executionStore.ts:857` 早已订阅 `workflow-step-start`，但 **`stockAnalysisStore` 从未订阅** → 股票分析面板全程无当前节点信息                                                                                                                                                 | ✅ 已实施                  |
| P2「无执行级超时，需新增」                          | ❌ **已存在** —— `stock_workflow/core.rs:833-836` 已有 `tokio::time::timeout(total_timeout, engine.run_workflow)` + `tokio::select!` 监听 shutdown，超时分支执行 `cancel_workflow` + emit `workflow-error` + DB 置 `timeout`（注释标 `P0-T5`）；且 `total_timeout` 由模板变量暴露，可在设置面板调整（`stock_workflow/core.rs:640-641`） | **零改动**                 |
| 缺口 B「`node_executions` 执行中无快照」            | ✅ **真缺口** —— `node_records` 仅内存累积（`execution_state.rs:437`），只在 `complete_execution()`（`engine/mod.rs:5608`）一次性序列化写出                                                                                                                                                    | ⏸ **本次不做**，理由见 9.3 |

### 9.2 本次实际改动清单

| 文件                                                 | 改动                                                                                                                                                                                                                                |
| ---------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src-tauri/src/commands/stock_workflow/core.rs`      | `"running"` 分支 payload 补 `totalNodes` / `completedNodes` / `executionId`（这些值本就在 `event` 里，原代码白白丢弃 → 前端无法显示「第 i/N 个节点」）                                                                              |
| `src/stores/feature/stockAnalysisStore.ts`           | 新增 state `currentNodeId` / `currentNodeIndex` / `totalNodeCount` / `nodeStartedAt`；新增 `workflow-step-start` 订阅；终态时按「仅清当前展示节点」规则清理（并发 max_concurrent=8 下其他节点完成不得清掉当前展示）；3 处重置点同步 |
| `src/components/stock-analysis/AnalysisProgress.tsx` | 展示「正在执行：{{name}}（{{index}}/{{total}}）· 已运行 mm:ss」；新增本地 1s 计时器（不依赖后端心跳）；新增 `formatElapsed()`                                                                                                       |
| `src/stores/feature/stockWorkflowChatBridge.ts`      | `TOOL_NODE_TO_LABEL` 拆为 `TOOL_NODE_I18N_KEY`（存 key）+ 派生映射 —— 原实现用 `i18next.t()` 在**模块加载时**求值，切语言后不更新；同时消除双份映射                                                                                 |
| `src/i18n/locales/*.json`（11 个）                   | 新增 `stockAnalysis.progress.currentNode` / `.elapsed`                                                                                                                                                                              |

### 9.3 为什么不实施 DB 增量写

1. **收益边际**：进度已通过 Tauri 事件实时可见（P3 已落地），DB 快照主要用于**崩溃后诊断**；
2. **覆盖竞争**：`complete_execution()` 会**整体覆盖** `node_executions` JSON 列，中途写入的部分快照必须与其协调顺序，否则可能被半途覆盖或反过来污染最终态；
3. **状态维护成本**：需在应用层 `progress_cb` 里维护跨节点的累加快照（19 节点 × 2 次 UPDATE），并解决「同一 workflow 多次执行」的关联问题；
4. **链路不一致**：只在 `stock_workflow` 有该回调，serenity / demand-discovery 链路无，会造成能力参差。

**结论**：降级为 P2 候选 —— 待出现明确诊断需求（如实际发生「崩溃后无法定位跑到哪」）再启动。

### 9.4 教训

**「立项前必须 grep 基础设施」**：本次两个错误判断（引擎无 started 事件、无执行级超时）都源于**没有先查已有实现**。核实成本约 10 分钟；若按原方案实施，不仅重复造轮子，还会与既有 `total_timeout` / `workflow-step-start` 机制冲突。

**可复用判据**（本项目）：工作流可观测性/超时类需求，先查这 4 个点 ——
① `engine/mod.rs` 的 `StepProgressEvent` 构造点（是否已发 running）；② 应用层 `core.rs` 的事件 `match` 分支；③ 前端各 store 的 `listen("workflow-step-*")` 清单；④ `core.rs` 的 `total_timeout` / `RunOptions` 超时项。
