# PLAN — EvoFlow 借鉴清单与落地方案

- **日期**：2026-09-12
- **上游审计**：`AUDIT-evoflow-vs-axinvest-2026-09-12.md`
- **状态**：**待用户确认，未执行**。本文件只出方案，不动代码。
- **参考边界**：EvoFlow 为 PolyForm Noncommercial（非开源，商用须授权）且为 Python/LangGraph 栈。
  **本清单中所有条目均为「用自己的 Rust 栈重新实现」，不搬运代码。**

---

## 0. 优先级总表

| #        | 动作                                              | 性质                                                           | 依据                 | 预估量级 | 依赖         |
| -------- | ------------------------------------------------- | -------------------------------------------------------------- | -------------------- | -------- | ------------ |
| **P0-A** | 闭合 Plan 授权闸门                                | 🔴 **AxInvest 自有缺陷**（借鉴 EvoFlow 的设计意图）            | AUDIT §3.1 五条断链  | 中       | 无           |
| **P0-B** | 停止闸的入边供给（`max_steps` / `ReActConfig`）   | 🔴 **AxInvest 自有缺陷**（不借鉴 EvoFlow，它更差）             | AUDIT §3.2           | 小–中    | 无           |
| **P1-A** | 修 `GoalEvaluator` 的 fail-open 恒定输出          | 🔴 **AxInvest 自有缺陷**（铁律 #5 同向恒定）                   | AUDIT §3.3           | 小       | 无           |
| **P1-B** | 给 `check-contracts.mjs` 增加「入边供给」检查维度 | 🟢 **元机制**（借鉴 AxInvest 自身已有路径，非 EvoFlow）        | AUDIT §7.4           | 中       | 可与 P0 并行 |
| **P1-C** | 统一任务账本 + 「待闭环」态                       | 🟡 **借鉴 EvoFlow 概念，修正其两处缺陷**                       | AUDIT §4「统一账本」 | **大**   | 建议最后做   |
| **P2-A** | 记忆写入加防抖/队列；修 FTS fail-open             | 🟡 借鉴 EvoFlow 的防抖意图，但**不借鉴其队列实现**（会丢数据） | AUDIT §4「记忆」     | 中       | 无           |
| **P2-B** | 把 EvoFlow 的病历转成 code review checklist       | 🟢 方法论                                                      | AUDIT §4             | 小       | 无           |

---

## P0-A 🔴 闭合 Plan 授权闸门

### 问题（五条独立断链，同时存在）

| #  | 断链                                                                                                                                             | 证据                                                                                                                    |
| -- | ------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------- |
| 1  | 状态集合不相交，执行**必被拒**                                                                                                                   | `commands/plan.rs:822` 写 `"reviewing"`；`:872` 要求 `status ∈ {"draft","approved"}`                                    |
| 2  | 无任何代码写 `"approved"`                                                                                                                        | `grep 'Set("approved"' commands/plan.rs` → 0 命中                                                                       |
| 3  | 规则反而**允许未批准执行**（`draft` 放行）；显式传 `step_ids` 绕过 Approved 过滤（`:893-894`），而前端 `planStore.ts:116-161` 总是显式传全部 ids | 同上                                                                                                                    |
| 4  | 审批通道死胡同：`agent_plan_approvals` 全仓 4 处引用，**只有 `remove` 没有 `insert`**                                                            | `app_state.rs:264` / `init/state.rs:378,1420` / `agent/mod.rs:2949`                                                     |
| 5  | 事件有监听无发射：`PlanReadyForApproval` 无 emitter                                                                                              | `agentStore.ts:921` 监听；`event_bus.rs:48` / `frontend_adapter.rs:121,149` / `event_emitter.rs:104` 仅有枚举与计数测试 |
| 附 | 前端 `requirePlanApproval` 被 serde **静默丢弃**                                                                                                 | `conversationStoreSend.ts:784` 传参；后端 `agent/payloads.rs:227-239 AgentOptions` 无此字段                             |

### 借鉴来源（EvoFlow 真做对的那一条）

EvoFlow 把「执行授权」拆成了**独立于计划生命周期**的状态位：

- 授权位 `execution_authorized`（`persistence/schema.py:517`），与计划自身的状态分开存
- **写入白名单不含模型**：`authorize_execution.py:16 = {user, system, api, page, ui, automation}`
- 授权位**是工具可见性的输入**——未授权时在 `awrap_model_call` 里剥掉工具 schema（`plan_guard_middleware.py:1872`），模型**根本看不见**被禁工具，而不是等它调了再拒
- **改计划 → 撤权**（`authorize_execution.py:261`）

### 目标状态

1. `plans.status` 的取值集合**单一真源化**：一个枚举 + 一张静态迁移表，所有写路径收敛到唯一 `set_status()` 校验（现在 `plan.rs` 里散着写）。
2. **授权与计划生命周期解耦**：新增独立授权位（如 `plans.execution_authorized` / `authorized_at` / `authorized_by`），`plan_execute` 校验**授权位**而不是 `status`。
3. **授权位的写入者受限**：写入白名单为 `{user, system, api, ui, automation}`，**显式排除模型路径**；`authorized_by` 非白名单值直接拒绝。
4. **执行前收窄工具面**：未授权时不放行高副作用工具（对齐 EvoFlow 的三层拦截思路，但**修掉它的 fail-open**——异常必须 fail-closed）。
5. **修掉「显式传 `step_ids` 绕过 step 审批」**：过滤逻辑改为「无论是否传 `step_ids`，都只执行 Approved 的 step」。
6. **补上缺失的入边**：`agent_plan_approvals` 增加 `insert` 点；`PlanReadyForApproval` 增加发射点；`AgentOptions` 增加 `require_plan_approval` 字段（`#[serde(rename_all="camelCase")]`）。

### 验证方式

- 单测：状态迁移表全量覆盖；`plan_execute` 在未授权态**必须返回 Err**（当前是「必拒」但原因是状态集合不相交，属**错误的正确结果**，修完要确保拒绝原因正确）。
- 反事实：构造「生成 → 用户批准 → 执行」全链路，断言 `authorized_by == "user"`。
- 负向断言：`authorized_by = "model"` / `"lead"` 必须被拒。
- 回归：确保修复后**不再出现「生成后必然执行失败」**。

### 风险

- ⚠️ 状态集合不相交当前表现为「执行必拒」→ 修复后**执行会真的跑起来**，这是**行为变更**，不是纯 bug fix。需先确认 `plan_execute` 的调用方对「成功」的处理是否就绪。
- ⚠️ 不要把「授权位」和「审批 UI」耦合回一起（那是当前的病根：授权只有 UI 态）。

### 判定修正：第 4/5 条不是「入边缺失」，是「被替代的遗留链」

实施前的进一步核查推翻了对第 4/5 条的定性：

- `task_shape_approval_senders`（`app_state.rs:438`）+ `task-shape-approval-request` 事件（`cognitive.rs:1876`）+ `TaskShapeApprovalModal.tsx` + `respond_task_shape_approval`（`cognitive.rs:2731`，已注册 `register_commands.rs:203`）—— **完整闭环**：insert / emit / listener / UI / 回传齐全。
- 而 `agent_plan_approvals` 那条链：State 槽 + `agent_approve_plan` + TS 监听 + `PlanApprovalModal` 全在，**唯独 Rust 侧零 `insert`、零 `emit`**（真实模式下永不触发，仅浏览器 mock 会发事件）。

⇒ 定性应为 **「P0-2 旧实现被 P3 task_shape 实现取代」**，按铁律 #3（被覆盖 ⇒ 删）处置，而**不是**「补 insert/emit 让旧链复活」。本轮**不删**（`planApprovalEnabled` 是 `ChatView` 用户可见开关，删除属产品行为变更），改为在 `app_state.rs` / `agent/mod.rs` 处加警示注释标注为遗留链路，并把删除列入待决策。

### 实施记录（2026-09-12）

**状态：已实施并与 P0-B 合并提交验证（编译 / clippy / tsc / 单测 / i18n 全绿）。**

| 层         | 文件                                                                     | 改动                                                                                                                                                                                                                                                                                             |
| ---------- | ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Entity     | `crates/entities/src/plans.rs`                                           | +3 列 `execution_authorized` / `authorized_at` / `authorized_by`                                                                                                                                                                                                                                 |
| Migration  | `crates/dao/src/migrations/v225_plan_execution_authorization.rs`（新增） | 加 3 列（PG `IF NOT EXISTS` / SQLite duplicate-column 容错）+ 存量回填                                                                                                                                                                                                                           |
| 迁移注册   | `crates/dao/src/migrations/mod.rs`                                       | 三处：`pub mod` + `CURRENT_VERSION` 224→225 + `MIGRATIONS` 追加                                                                                                                                                                                                                                  |
| 错误码     | `commands/error_code.rs` + `crates/harness/src/error_codes.rs`           | 新增 `WORKFLOW_PLAN_NOT_AUTHORIZED`（分类 `PermissionDenied`）                                                                                                                                                                                                                                   |
| 命令层     | `src/commands/plan.rs`                                                   | ① 状态机单一真源（`from_db_str`/`as_db_str`/`allowed_next` + `Unknown` 变体）② 新增 `plan_authorize`（**刻意不带 `#[agent_command]`**）③ `plan_execute` 改校验授权位 + 迁移校验 ④ step 过滤修复（先 Approved 再 step_ids）⑤ `plan_modify_step` 内容变更撤权 ⑥ `plan_cancel`/`plan_activate` 撤权 |
| 前端类型   | `src/types/index.ts`                                                     | `Plan` +3 字段；`PlanStatus` +`"unknown"`；+`PlanAuthorizeRequest` / `PlanAuthorizationChangedEvent`；修 `PlanExecutionCompleteEvent.status` 漏 `partial`                                                                                                                                        |
| 前端 store | `src/stores/feature/planStore.ts`                                        | `approvePlan` 插入授权步骤（**先于** `plan_execute`）；监听 `plan-authorization-changed`                                                                                                                                                                                                         |
| Mock       | `src/lib/browserMock.ts`                                                 | +`plan_authorize` case                                                                                                                                                                                                                                                                           |
| i18n       | `src/i18n/locales/*.json`（11）                                          | +`error.WORKFLOW_PLAN_NOT_AUTHORIZED`（保序 + LF）                                                                                                                                                                                                                                               |
| 测试       | `planStore.test.ts` + `plan.rs` `mod tests`                              | 授权先于执行的顺序断言；状态回环不变量 / reviewing 不可直接执行 / 终态不可执行 / 白名单不含模型 / 未知值不报成已取消                                                                                                                                                                             |

**两条防线的落地形态**（对齐 EvoFlow 的设计意图，但修掉它的 fail-open）：

1. `plan_authorize` **不注册 `#[agent_command]` 元数据** ⇒ 不进 agent 工具表（`command_bridge.rs` 只暴露带元数据的命令）⇒ 模型无从调用。比字符串白名单更硬。
2. `authorized_by` 必须落在 `{user, system, api, ui, automation}`（`AUTHORIZED_BY_WHITELIST`），与 EvoFlow `authorize_execution.py:16` 一致 —— **不含 model**。

**撤权语义**（EvoFlow「改计划 → 撤权」的落地）：计划**内容**被修改、被取消、被恢复（`plan_activate`）时均清零授权位；仅变更「批准标志」不算内容变更。

**目标状态第 4 条（「执行前收窄工具面」）经复核判定为不需要做**：EvoFlow 需要它，是因为它的三层拦截里有 fail-open（`plan_guard_middleware.py:1659` `except Exception: return request`），必须补「工具可见性」这层兜底。而 AxInvest 的闸门已在**唯一执行入口**闭合，且模型**无法自行授权**：

- 模型调 `plan_execute` ⇒ 撞授权位检查（`execution_authorized == 0` ⇒ `WORKFLOW_PLAN_NOT_AUTHORIZED`）
- 模型想先授权 ⇒ `plan_authorize` 不在 agent 工具表（刻意无 `#[agent_command]`）⇒ 调不到

⇒ 无绕过路径可防，加这层只是扩大改动面。**判定不做**，理由记录于此备查。

**行为变更确认**：修复前执行必拒（原因是状态集合不相交），修复后执行会真正跑起来 —— 前端 `approvePlan` 已同步改造（授权先于执行），调用方就绪。

**已知未完项**：`executionAuthorized` 的前端消费点目前只有 store 状态同步，**尚无 UI 展示**（如「已由 user 于 X 批准」标记）。若要做需新增 11 语言 key，本轮为控制范围未做。

### P0 整体验证结果（2026-09-12）

| 检查                                                                                           | 结果                                                     |
| ---------------------------------------------------------------------------------------------- | -------------------------------------------------------- |
| `cargo check -p axagent --lib`                                                                 | ✅ PASS（**0 warning** —— `dead_code` 已随归属校验消除） |
| `cargo clippy -p axagent --lib -- -D warnings`                                                 | ✅ **PASS**（CI 门禁）                                   |
| `__TAURI_WORKSPACE__=true cargo test -p axagent --lib commands::plan`                          | ✅ **5/5 PASS**                                          |
| `cargo test -p axagent-dao --lib migrations::v225`                                             | ✅ **2/2 PASS**（幂等 + 加列/回填语义）                  |
| `cargo test -p axagent-agent --lib harness_adapter`                                            | ✅ **3/3 PASS**（P0-B）                                  |
| 前端：`tsc --noEmit` / `planStore.test.ts` / `i18n-scan.mjs` / `dprint fmt` / 11 语言 key 校验 | ✅ 全 PASS                                               |

**过程中修掉的三个自身缺陷**（值得记录，因为它们都是「只有跑对应命令才暴露」）：

1. `PlanAuthorizeRequest.conversation_id` 死参数 —— **`cargo check` 的 warning 暴露**（`field is never read`），补归属校验后消除。
2. 迁移测试漏 `use sea_orm::ConnectionTrait` —— **`cargo check` 不编译 `#[cfg(test)]`**，只有 `cargo test` 才报。
3. 迁移测试 seed 违反 `plans.conversation_id → conversations(id)` 外键 —— **只有真正跑测试才暴露**；修法是先造合法父行（优于关外键检查，保持约束真实性）。

---

## P0-B 🔴 停止闸的入边供给

### 问题

| 项                           | 现状                                                                                                                                                                           | 证据                                                         |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------ |
| `ReActConfig` 有完整闸门     | `max_iterations:50 / max_depth:10 / timeout_secs:300 / token_budget_limit:180k / max_retry_attempts:3 / cycle_detection / max_repeated_calls:3 / max_no_progress_iterations:5` | `crates/agent/src/reasoning_state.rs:131-158`                |
| **但全部硬编码、用户不可调** | `ReActConfig` 在 `src-tauri/src` **零引用**，无 settings / 配置注入路径                                                                                                        | 全仓 grep                                                    |
| **声明的参数无消费点**       | `AgentExecuteRequest.max_steps: Option<u32>` **被忽略**（execute 只用 `req.goal`）                                                                                             | `harness/src/agent.rs:22-23` vs `harness_adapter.rs:107-165` |
| **无 goal 持久化**           | `entities/src` 无 `goal*.rs`；`grep 'GoalStatus' / table_name="goals"` 无命中                                                                                                  | —                                                            |

### 目标状态（二选一，不许悬空）

按铁律 #6「四条独立链缺一即静默失效」——**要么接线，要么删声明**：

- **方案 1（推荐）**：把 `ReActConfig` 接进配置链：后端变量 → 前端 `resolve` 白名单 → 消费点 → **控件量程**（对齐既有的「参数四道门」，权威源 `PORTFOLIO_MGR_TUNABLE_PARAMS` 那套模式），并把 `max_steps` 从 `AgentExecuteRequest` 里真正消费掉。
- **方案 2**：确认这些参数不需要可配 ⇒ **删掉 `max_steps` 声明与 `ReActConfig` 的死分支**，只保留硬编码常量并注释说明「有意不可配」。

> ⚠️ **不借鉴 EvoFlow**：它的 `max_steps` 默认值三处不一致（1000/50/8）、`auto_stop` 默认 0 关闭、且 `tool_calls` 轮直接跳过计数导致步数闸永不触发。**它的实现比 AxInvest 现在更差，没有可抄之处。**

### 判定修正（2026-09-12 实施前复核）：问题比 §3.2 描述的深一层

对 P0-B 逐条取证后，**「参数不可配」只是表层**。真相比 AUDIT §3.2 严重：

| # | 事实                                                                                                                                                                                                       | 证据                                                                                                                                                                                                                                                                                                                                                         |
| - | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 1 | `ReActEngine` 生产装配面**全空** —— 9 个能力注入 builder 中，**除 `with_cancel_flag` 外零生产调用**                                                                                                        | `with_config` / `with_reasoning_provider` / `with_cycle_detection` / `with_token_budget` / `with_planner` / `with_checkpoint` / `with_reflector` / `with_self_improvement` 的调用点**全部落在 `crates/agent/tests/` 与 `react_engine.rs` 自身测试**                                                                                                          |
| 2 | 唯一生产构造点是 `ReActEngine::new()` ⇒ `config = ReActConfig::default()`，`cycle_detector = None`、`token_budget = NoopTokenBudgetTracker`（**注意：前者会被 `run()` 补救，后者不会 —— 见下方影响矩阵**） | `harness_adapter.rs:62`；`react_engine.rs:602-624`                                                                                                                                                                                                                                                                                                           |
| 3 | **`with_reasoning_provider` 也未接线 ⇒ 引擎无 LLM 能力**：`DefaultReasoningProvider` 的每个 trait 方法直接返回 `Err("not configured: inject a real LlmReasoningProvider…")`                                | `react_engine.rs:135-184`                                                                                                                                                                                                                                                                                                                                    |
| 4 | **专为该场景造的工厂零调用、却打印「注入完成」**（铁律 #12 说谎）                                                                                                                                          | `llm_providers.rs:94-101` `build_reasoning_provider_from_db` 全仓**零调用**；`llm_providers.rs:107-114` `build_llm_judge_from_db` 同样零调用 + 同样打印「注入完成」。同文件另两个工厂（`build_llm_decomposer_from_db` / `build_llm_tool_provider_from_db`）**有**调用（`orchestrator.rs:152` / `workflow_tool.rs:184`）—— 说明不是整套废弃，是**这两条漏接** |
| 5 | `max_steps` 不是「无入边」，是**末端丢弃**：MCP 请求确实把值传进来了，被 `execute` 静默丢掉                                                                                                                | 入边 `mcp/server.rs:118` `max_steps: req.max_steps` → `harness_adapter.rs:107-165` 函数体只读 `req.goal`（`:155` `engine.run(&req.goal)`）                                                                                                                                                                                                                   |
| 6 | 注释与代码相反                                                                                                                                                                                             | `agent_round_adapter.rs:64` 注释称 `ReActEngine::new().with_config(config)…`，但 `:75` 实际调的是 `with_engine_config`，而 `with_engine_config`（`:71`）**自身零调用**                                                                                                                                                                                       |

**⇒ 问题定义修正**：P0-B 不是「闸门参数接不接面板」，而是「**`ReActEngine` 的唯一生产消费者（MCP `agent_run` → `HarnessAgentAdapter`）是一台装了壳没装发动机的引擎**」。

#### 逐项影响矩阵（取证到 `run()` 内部，避免「零调用 ⇒ 全失效」的过度归因）

`run()`（`react_engine.rs:709-1010`）内部有**自动补救**逻辑，故「builder 零调用」≠「能力全失效」：

| 能力                                                             | `ReActConfig::default()`                         | 实际是否生效                                                                                                                            | 依据                                             |
| ---------------------------------------------------------------- | ------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| `max_iterations`/`max_depth`/`max_retry_attempts`/`timeout_secs` | 50 / 10 / 3 / 300                                | ✅ **生效**（值固定，不可配）                                                                                                           | `:754` / `:764` / `:787` / `:1307` 均真读 config |
| 循环检测                                                         | `cycle_detection_enabled: true`                  | ✅ **生效**（`run()` 自动构造 detector 补上了 `with_cycle_detection` 的缺位）                                                           | `:724-729`                                       |
| 反思 / 分析                                                      | `enable_reflection` / `enable_analyzing`: true   | ✅ 生效                                                                                                                                 | `:713` / `:801`                                  |
| token 预算                                                       | `token_budget_enabled: **true**` + `limit: 180k` | ❌ **静默 no-op** —— tracker 是 `NoopTokenBudgetTracker`，`with_token_budget` 零调用，`:816-818` 的 `check()` 永不触发 `Compact`/`Stop` | `:814-872`                                       |
| **LLM 推理**                                                     | —（非 config 项）                                | ❌ **每个 trait 方法直接 `Err`** ⇒ 引擎无推理能力                                                                                       | `react_engine.rs:135-184`                        |

**⇒ 真正的两个断口是**：① **`reasoning_provider` 未注入（致命）** ② `token_budget` tracker 为 no-op（静默）。其余闸门是「生效但不可配」。

#### 失败形态（按铁律 #12 查调用点 catch 语义，不做「必然崩溃」的过度归因）

provider 的 `Err` 经 `?` 上抛（`:1231`/`:1268`/`:1284`）→ `run()` 的 Err 分支（`:947`）`consecutive_failures += 1`，达 `max_retry_attempts`(=3) 后 `return ReActResult::failure(e.to_string(), …)`。

⇒ MCP `agent_run` 的实际表现是**重试 3 次后返回 `[failed] steps_taken=…`，输出为 "DefaultReasoningProvider not configured: inject a real LlmReasoningProvider…"** —— 即 **fail-visible（显式失败）**，不是静默降级。

该结论对方案选择有直接影响：**失败是可见的**，故若该链曾被使用，用户应早已见过此报错 ⇒ 大概率当前无人调用。**这也是为什么方案选择必须由用户确认，而非 AI 代为推断。**

**⇒ 方案随之修正（二选一，判据不再是「有没有调参需求」，而是「MCP agent_run 还要不要活」）**：

- **方案 1（接线，推荐若该链仍需可用）**：`init/state.rs:756` 构造 `HarnessAgentAdapter` 时，用 `build_reasoning_provider_from_db()` 的产物 + 一份 `ReActConfig` 组装 engine（改走 `from_engine()` 或加 `with_engine()`）。理由：`llm_providers.rs` 的整套设计（`LlmDrivenReasoningProvider` 的 5 个 prompt、`BridgeLlmExecutionService`）都是为这条链造的，**设计意图是活的，只差最后一跳**；删掉等于废弃一整块已完工基建。
- **方案 2（明确废弃并删声明）**：确认该链无人使用 ⇒ 删 `build_reasoning_provider_from_db` / `build_llm_judge_from_db` 两个死工厂 + 删 `AgentExecuteRequest.max_steps`（连带 `dto_locks.rs:70` 的 `[u8; 56]` size 断言、`serialization_contracts.rs` 4 处构造、`schema-gen` 导出、`mcp/server.rs:118` 构造点）+ 把 `ReActConfig` 注释为「有意不可配」。

**两方案共通、可立即修（本轮已做/待做）**：

- 修 `llm_providers.rs` 两条说谎日志：`build_*` 只负责**构造**，不负责**注入** ⇒ 改为 `debug!` 且措辞改为「已构建，注入由调用方完成」（铁律 #12）。
- `HarnessAgentAdapter` 未注入 provider 时应 **fail-loud**（启动即 `warn!`），而不是等第一次调用才暴露。

### 验证方式

- 若走方案 1：改了面板值 → 断言运行期 `ReActConfig` 实际取值变化（**取运行时值，不信默认常量**——铁律 #11）。
- 若走方案 2：grep 确认删除后零引用。

### 实施记录（2026-09-12）：走**方案 1（接线）**

| 层        | 文件                                  | 改动                                                                                                                                                                                                                                                                                                             |
| --------- | ------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 引擎      | `crates/agent/src/react_engine.rs`    | ① 新增 `max_iterations()` / `set_max_iterations()`（调用级预算存取器）② `run()` 开头对「`token_budget_enabled=true` 且 `limit=None`」显式 `warn!` ③ token 检查点改为 `enabled && let Some(limit)`                                                                                                                |
| 适配器    | `crates/agent/src/harness_adapter.rs` | ① 新增 `with_engine()`，**替代零调用的 `from_engine()`**（后者把 `caps` 置空、对外声明零能力）② `execute()` 消费 `max_steps` 并 save/restore；`Some(0)` 视为未指定                                                                                                                                               |
| 装配      | `src/init/state.rs`                   | `HarnessAgentAdapter::new("default")` → `.with_engine(engine)`；engine 由 `ReActEngine::new().with_config(..).with_token_budget(TokenBudgetTracker::new())` + `build_reasoning_provider_from_db(&master_key)` 组装 —— **`with_config` / `with_reasoning_provider` / `with_token_budget` 三者此前都是零生产调用** |
| 文档/归因 | `src/init/llm_providers.rs`           | 两个零调用工厂加「⚠️ 全仓零调用」doc 标注；日志「注入完成」→「已构建，注入由调用方完成」（铁律 #12：`build_*` 只构建不注入）                                                                                                                                                                                     |
| 测试      | `harness_adapter.rs` 新增 `mod tests` | ① `with_engine` 保留默认 caps（回归护栏）② `max_steps` 覆盖后被还原（跨调用不污染）③ `Some(0)` 视为未指定                                                                                                                                                                                                        |

**接线前提经实测而非推断**：`build_llm_components_from_db` 依赖 `harness::repositories` **全局注册表**，故必须确认它早于装配点就绪 —— 实测 `register_repositories` 在 `init/database.rs:289`（DB 初始化阶段）执行，早于 `create_app_state` 的装配点（同类时序已有先例注释 `init/database.rs:293`）⇒ 运行时能真正读到 provider 配置，不会静默落进 warn 分支。**若省略这步，接线会表现为「代码看起来对了但工厂恒返 None」——即铁律 #6 的另一种形态。**

**语义冲突收口**（接线**引入**的新脆弱点，必须同步修）：真实 `TokenBudgetTracker::check(None, _)` 返回 **Stop**，而 `ReActConfig::token_budget_limit: None` 的语义是「使用模型上下文窗口大小」。直接透传该 `Option` 会让**首次迭代即停** —— 比原来的 no-op 更糟。处置：① 装配点把 `limit` 收敛为确定值 `Some(180_000)`；② 引擎侧对「`enabled=true` 且 `limit=None`」显式 `warn!` 并按「不启用」处理，不再把 `None` 交给 tracker。

**行为变更清单**（本轮引入，非纯 bug fix，需知悉）：

1. MCP `agent_run` 从「重试 3 次后必失败」变为**可真正执行**
2. token 预算从 no-op **变为真实生效**
3. `AgentExecuteRequest.max_steps` 从被末端丢弃**变为生效**

**已知未完项**：

- `ReActConfig` 的 **UI 配置链（四道门）未做**。判断：`max_steps` 已可通过 MCP 请求参数按调用控制（更直接的入口），且内部对话路径不走 `ReActEngine` ⇒ 面板配置的边际价值低，**建议后置**。
- `build_llm_judge_from_db` + `RLEngine::with_llm_judge` / `set_llm_judge` 仍**零调用**（同形态的另一条断链，见 `llm_providers.rs` 的标注）。本轮未动 —— 应与本条一并决策，避免只修一半。

---

## P1-A 🔴 修 `GoalEvaluator` 的 fail-open 恒定输出

**现状**：连续未达成 ≥ 3（`max_not_achieved` 默认 3）时**强制 `achieved=true`**「强制进入综合阶段」——`crates/agent/src/goal_evaluator.rs:85-95`。

**为什么是缺陷**：这是铁律 #5 的「**同向恒定 = 偏置非证据**」形态。无论实际是否达成，连续 3 轮后恒定输出 `achieved=true` ⇒ 该信号在长任务后段**完全失去判别力**，而下游会把它当真实信号消费（`goal_evaluation_enabled: false` 说明它默认还没启用，但一旦启用即是陷阱）。

**目标状态**：区分三个语义不同的量：

1. `achieved`（真实的达成判定）
2. `attempts`（尝试次数）
3. **`forced_synthesis`**（因超限而放弃继续判定）

超限时应输出 `forced_synthesis=true` 且 `achieved` 保持原判（或显式 `None`），**下游按 `forced_synthesis` 走降级路径**，而不是把「放弃」伪装成「达成」。

**验证方式**：单测断言「连续 N 次未达成」时 `achieved` 不再被写成 `true`；检查全部消费点是否能区分两个字段。

---

## P1-B 🟢 扩展 `check-contracts.mjs`：从「架构边界」到「入边供给」

### 为什么这是最高 ROI

AxInvest 已经证明「约束可以机械化强制」——`scripts/check-contracts.mjs:177-198`（段 D）在 CI 实跑（`.github/workflows/ci.yml:98`），限定 `[agent, gateway, orchestrator, runtime-core]` 只能依赖 `axagent-harness`。**同一套机制可以扩展去执行铁律 #6**，把「人肉检法」变成 CI 门禁。

### 建议新增的检查段

| 段 | 检查内容                                                                                                                    | 针对本次发现的哪条                      |
| -- | --------------------------------------------------------------------------------------------------------------------------- | --------------------------------------- |
| E  | **声明的参数必有消费点**：扫 harness / harness_adapter 的请求 DTO 字段，若某字段无任何读取点 ⇒ fail                         | `AgentExecuteRequest.max_steps` 被忽略  |
| F  | **状态位的写读对称**：某个 `*Status` / `*State` 枚举的每个变体，若**无任何写入点** ⇒ fail                                   | `plans.status = "approved"` 从未被写    |
| G  | **事件发射/监听对称**：Tauri 事件常量若只有 TS 侧监听、Rust 侧无发射点 ⇒ fail                                               | `agent-plan-ready-for-approval`         |
| H  | **跨层字段名一致性**：前端 invoke 传参与后端 DTO 字段并集比对（已有 `.workbuddy/tmp/scan_ipc_args.py` 的先例，可升级为 CI） | `requirePlanApproval` 被 serde 静默丢弃 |
| I  | **状态集合交集检查**：若 A 处写取值 X、B 处校验取值集合 S，且 `X ∉ S` ⇒ fail                                                | `reviewing ∉ {draft, approved}`         |

> 段 F/I 建议用「**编译期或测试期断言**」而非脚本 —— 例如把状态迁移做成 Rust 的 `match` 穷尽检查 + 一张 `static` 迁移表，让类型系统承担一部分。脚本适合跨语言（TS↔Rust）的检查（段 G/H）。

### 验证方式

每个新段都要有**故意违反的样例**证明它会 fail（否则又是一个「声明了不生效」的机制——这本身就是本轮的主题）。

---

## P1-C 🟡 统一任务账本 + 「待闭环」态（量大，建议最后做）

### 借鉴什么

| EvoFlow 做对的                                                                                                                                               | EvoFlow 做错的（不要抄）                                                                                            |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------- |
| 三源（`chat`/`workflow`/`role`）写同一张表、**同一状态机真源**（`collab/task_source.py:16-18`）                                                              | `source` / `parent_task_id` / `handlers` 塞进 `extra_json` → **无列约束、无索引**                                   |
| **单一状态迁移表**（`supervisor_tool.py:156-188`，12 态）所有写路径收敛                                                                                      | 同域仍有 6+ 张并行表（`a2a_tasks`/`app_runs`/`mission_state`/`person_commitments`/`task_details`/内存 cron 状态机） |
| **「待闭环」态语义**：结案必须带 handler → 建子 Task → 软回执唤醒上级（`admin/tasks.py:1156 dispatch_confirmed_handlers`、`collab/upstream_receipt.py:365`） | `awaiting_close` **无自动推进**，靠人工或 3 天僵尸回收兜底（`collab/task_reclaim.py:75-136`）                       |
| **协作式取消**（标志位轮询，`cancellation/task_cancellation.py:14-48`）                                                                                      | **取消不级联**到 `parent_task_id` 关联的独立子任务（`routers/tasks.py:1905-1957`）—— 明确缺陷                       |

### AxInvest 现状

7+ 张带 `status` 的表、**≥10 套独立状态枚举**、**无统一 task 实体**、Rust 侧 `parent_task_id` / `awaiting_close` **0 命中**。

### 目标状态（文字描述，不写代码）

1. **单张 tasks 表**：`id / main_task_id / parent_task_id（真列 + 索引）/ name / status（枚举）/ progress / assignee / source（枚举 chat\|workflow\|role）/ created_at / started_at / completed_at / updated_at / summary / extra(json)`
2. **`task_events` 表**做审计与回执（替代 EvoFlow 的 `extra_json` 回执）
3. **状态枚举 + 静态迁移表**，全部写路径收敛到唯一 `set_state()` 校验
4. **`awaiting_close` 只能由「验收动作」或超时回收器推进终态**（不要留人工 `set_state` 这个后门）
5. **父子闭环**：子任务进终态时**写事件 + 软唤醒父（不改父状态）**，父再显式 completed；用 `parent_task_id` 索引做 rollup
6. **取消语义显式声明是否级联** —— EvoFlow 这里缺级联，是缺陷点，要修正
7. **队列**：单 tick 循环 + 计数并发上限 + FIFO + **持久化 `attempts` / `next_retry_at`**（失败=重新入队，非原地重试）

### 为什么放最后

量大且有迁移风险（涉及 7 张已有表）。**建议先在 P0/P1 上验证「入边供给检查」这套机制有效，再动账本。**

---

## P1 实施记录（2026-09-12）

### P1-A 已修：`GoalEvaluator` 的 fail-open 恒定输出

**改动**：`crates/agent/src/goal_evaluator.rs` + 唯一消费点 `crates/agent/src/react_engine.rs`。

`GoalEvaluation` 新增 `forced_synthesis: bool`，把原先混在一起的语义拆开：

| 字段               | 语义                           | 超限时取值                   |
| ------------------ | ------------------------------ | ---------------------------- |
| `achieved`         | 目标到底达成没有               | **`false`**（不再是 `true`） |
| `forced_synthesis` | 判定是否已被放弃（保护性放行） | `true`                       |

**消费点必须同步改 —— 这是本次最容易漏的一步**：原代码 `if !evaluation.achieved { 回 Thinking }`
依赖「超限时 `achieved=true`」来跳出循环，若只改定义端不改消费端 ⇒ **无限循环**。
现改为 `if forced_synthesis { warn + 放行 } else if !achieved { 回 Thinking }`。

**验证**：

- 5 个 `GoalEvaluation` 构造点全部覆盖 —— 靠编译器 `E0063` 兜住。**我手工只数到 4 个**，
  漏了 `goal_coverage < 0.5` 分支，是 `cargo check` 报出来的。
- 单测 `test_consecutive_not_achieved_force_through` **断言被反转**：原断言 `assert!(r2.achieved)`
  等于**把缺陷固化进测试**（"放弃判定"="已达成"），现改为 `assert!(!r2.achieved)` + `assert!(r2.forced_synthesis)`。
- 新增 `forced_synthesis_never_reports_achieved`：跑 10 轮断言放行期间 `achieved` 从不为 `true`，
  且放行轮数恰为 7 —— 后者用于证明**分支真的被走到**，避免「分支永不触发所以断言恒真」的假绿。

### P1-B 已做：`check-contracts.mjs` 扩展两个检查段

新增**段 F（登记字段必有消费点）**与**段 G（事件发射/监听对称）**。

> ⚠️ **上线时踩到的坑（已修）**：`ci.yml` 用的是 `npm run contracts -- --only=a,c,d`
> —— **白名单限定段**。新段不加进白名单就等于**没跑**，而脚本本身全绿、本地全跑也全绿，
> 唯一表现是 CI 少执行一段。**这是「声明了不生效的机制」的又一个实例，且正好发生在本轮
> 修这个主题的过程里。** 已改为 `--only=a,c,d,f,g`，并在 ci.yml 就地写了注释说明，
> 同时把「新增段必须同步 grep CI 调用点」写进 `rust-consistency-audit` skill 的方法十二。

**段 F 为什么是登记制而非全量扫描**：全量扫描「结构体字段是否在别处出现」会被
`timeout`/`id`/`name` 这类同名标识符大量误报，而**会误报的门禁最终会被加白名单或直接禁用** ——
这正是本项目反复出现的「声明了不生效的机制」。登记制零误报，代价是新增关键字段时手写一行。

**段 G 的三级判据**（第一版直接 fail 出 18 条，其中 7 条是假阳性）：

| 级别       | 判据                                                        | 处理          |
| ---------- | ----------------------------------------------------------- | ------------- |
| 严格命中   | Rust 侧有 `emit("evt")` 字面量，或指向字面量的 `const &str` | PASS          |
| 已知断链   | 在 `KNOWN_DEAD_EVENTS` 基线内                               | 打印，不 fail |
| 仅被提及   | Rust 源码（去注释后）含该字符串 ⇒ 可能经局部变量发射        | warning       |
| 完全不存在 | Rust 侧零命中 ⇒ 真断链                                      | **fail**      |

第一版假阳性的根因：`workflow-step-start` 等事件名**先赋给局部变量再发射**
（`stock_workflow/core.rs` 的 `let (event_name, payload) = match st {...}` → `emit(event_name, payload)`），
静态追踪不到 ⇒ 需要「提及」这一级。同时排除了前端测试文件（`__tests__` 里 `listen("my_event")`
是用假名验证 invoke 封装的桩代码，不是契约）。

**基线机制而非豁免**（`KNOWN_DEAD_EVENTS`）：豁免会让段 G 对该事件**永久失明**；基线则要求
数量与 PLAN 同步 —— 出现基线外的新断链 ⇒ fail，基线内已修好却未移除 ⇒ **也 fail**。双向防腐烂。

**覆盖范围自陈**：段 G 会打印「94 处 emit 的事件名为动态取值，本段未覆盖」，
避免「没报错」被误读成「已全覆盖」。

**故意违反验证**（PLAN 原要求「每个新段都要有故意违反的样例证明它会 fail」）：

| 场景             | 构造方式                                                                                 | 结果                                   |
| ---------------- | ---------------------------------------------------------------------------------------- | -------------------------------------- |
| 新增无发射监听   | 临时写 `src/lib/__tmp-contract-violation.ts` 含 `listen("zzz-deliberate-violation-zzz")` | ✅ FAIL（退出码 1）；删文件后恢复 PASS |
| 登记不存在的字段 | 脚本副本把 `field: "max_steps"` 改成 `totally_bogus_field_xyz`                           | ✅ FAIL                                |
| 基线腐烂         | 脚本副本把**有发射**的 `workflow-step-start` 塞进基线                                    | ✅ FAIL（触发 staleBaseline 分支）     |

---

## P1-D 🔴 事件断链 11 条（段 G 首次运行即发现）

段 G 上线后立刻发现 **11 个「前端 `listen`、Rust 侧零 `emit`」** 的事件。逐个 grep 甄别后
**全部确认为真断链**（Rust 侧连字符串都没出现过；`agent-plan-ready-for-approval` 的唯一命中是行注释，去注释后同样为零）。

| 事件                                                                        | 监听方                                  | 疑似处置方向                                                                 |
| --------------------------------------------------------------------------- | --------------------------------------- | ---------------------------------------------------------------------------- |
| `agent-started`                                                             | `backendStatusStore`                    | 待判定                                                                       |
| `agent-rate-limit`                                                          | `agentStore`                            | 待判定                                                                       |
| `agent-plan-ready-for-approval`                                             | `agentStore`                            | **删监听端** —— 已由 `task_shape_approval` 取代（见本文档 P0-A「判定修正」） |
| `agent-subagent-card`                                                       | `executionStore`                        | 待判定                                                                       |
| `knowledge-base-updated`                                                    | `backendStatusStore`                    | 待判定                                                                       |
| `memory-item-indexed`                                                       | `MemorySettings`                        | 待判定                                                                       |
| `memory-rebuild-complete`                                                   | `backendStatusStore` + `MemorySettings` | 待判定                                                                       |
| `worker-created` / `worker-progress` / `worker-completed` / `worker-failed` | `executionStore`                        | 待判定，疑为旧并行执行设计残留                                               |

**处置原则**：按铁律 #3「独占且无人接线 ⇒ 激活非删」，逐项判定是**补发射端**（功能仍需要）
还是**删监听端**（功能已被替代）。判定完成前它们停留在基线中（不 fail，但每次 CI 都会打印提醒）。

> ⚠️ 这批断链**不是 EvoFlow 借鉴项**，是段 G 顺手发现的既有缺陷。是否修需单独排期。

---

## P1-E 🔴 ReActConfig 的 UI 配置链（本轮新发现，已修）

### 问题：控件能动、能存、后端从不读

```
用户改「最大迭代次数」→ appConfigStore.setMaxIterations（只改内存）
                      → 面板卸载时 saveConfig → invoke("save_app_config") → DB app_config.maxIterations
                      → ❌ 死路：后端零读取点
运行期实际生效的：
session_manager.run_turn → dynamic_max_iterations(estimate_complexity(user_input)) → 硬编码 20/50/100
```

这是铁律 #6 的**完整四门形态**：门① 后端变量有、门② 前端 resolve 有、门④ 控件量程有
（`min=1 max=100` + store 内二次 `clamp`），唯独**门③ 消费点接错了来源**。

### 处置（已修）

复用项目自己修过的同形态先例 `SelfImprovementFlags`（`app_config.rs` 里注释写着「缺陷1修复」），四步接线：

1. `SessionManager` 加 `max_iterations_override: AtomicUsize`（`0` = 未配置哨兵）+ 读/写方法
2. `session_manager.rs` 消费点改为 **override 优先，未配置才走复杂度推导**（短路写法，顺带省掉一次 `estimate_complexity` 轨迹查询）
3. `save_app_config` **内联注入**（而非新增独立命令）—— 该命令是全项目唯一保存路径，内联可**结构性排除**「新增保存路径却忘了同步注入」这类缺口
4. `init/state.rs` 启动时从 DB 注入（保证重启后生效）

**前端补漏**：`setMaxIterations` 只改内存，而 `saveConfig` 原先**仅在 `SettingsPanel` 卸载时**触发
⇒ 用户在 `GeneralTab` 改完不关面板就不落库。已加 `onBlur` 触发保存。

**验证**：

- `max_iterations_override_uses_zero_as_unset_sentinel` 覆盖 `0` 哨兵语义 —— 若把 `0` 当上限消费，
  每个 turn 都会立即 `Max iterations (0) reached` 停摆
- `extract_max_iterations_accepts_only_positive_integers_in_range` 覆盖 11 组取值
  （含 `0` / 缺字段 / 字符串 / 浮点 / 负数 / 超上界）
- `extract_max_iterations_rejects_snake_case_key` —— 键名写成 `max_iterations` 会静默落空，
  且与「未配置」不可区分

---

## 本轮验证结果（2026-09-12）

| 检查                     | 命令                                                                       | 结果                               |
| ------------------------ | -------------------------------------------------------------------------- | ---------------------------------- |
| Rust 类型检查            | `cargo check -p axagent --lib`                                             | ✅ Finished，**0 error 0 warning** |
| `GoalEvaluator` 单测     | `cargo test -p axagent-agent --lib goal_evaluator`                         | ✅ **5/5 PASS**                    |
| override 哨兵单测        | `cargo test -p axagent-agent --lib session_manager::tests::max_iterations` | ✅ **1/1 PASS**                    |
| `maxIterations` 解析单测 | `cargo test -p axagent --lib commands::app_config`                         | ✅ **2/2 PASS**                    |
| 前端类型检查             | `npx tsc --noEmit`                                                         | ✅ 退出码 0                        |
| 前端格式                 | `npx dprint check`                                                         | ✅ 退出码 0                        |
| 前端单测                 | `npx vitest run`（planStore + settings 共 6 文件）                         | ✅ **33/33 PASS**                  |
| 契约检查（含新段）       | `npm run contracts -- --only=a,c,d,f,g`                                    | ✅ **PASS**（errors=0 warnings=6） |
| 三个「故意违反」演练     | 见 P1-B 表格                                                               | ✅ 全部按预期 FAIL                 |
| CI 门禁 fmt              | `cargo fmt --check`（全 workspace，与 CI 同款）                            | ✅ 退出码 0                        |
| **CI 门禁 clippy**       | `cargo clippy --workspace --all-targets --all-features -- -D warnings`     | ✅ **PASS**（16m46s，0 warning）   |

**过程中修掉的自身缺陷**（都属「只有跑对应命令才暴露」）：

| 缺陷                                              | 暴露方式                    |
| ------------------------------------------------- | --------------------------- |
| `GoalEvaluation` 漏 1 个构造点（5 个只改了 4 个） | `cargo check` 的 `E0063`    |
| `dprint` 格式不符（`onBlur` 换行）                | `dprint check` 退出码 20    |
| 新检查段不在 CI 白名单里 ⇒ 写了但没跑             | 手动 grep `ci.yml` 的调用点 |

---

## 同形态零调用断链：判定修正（2026-09-12）

上轮把 `build_llm_judge_from_db` 与 `build_reasoning_provider_from_db` 并列为「同一形态的两个断链」，
**这个并列是错的** —— 两条链性质相反，处置方向也相反：

| 链                     | 真实状态 | 依据                                                                                                                                                       | 处置                                                                                                       |
| ---------------------- | -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `LlmJudge`             | **未断** | `init/services.rs` 真实调用 `RLEngine::set_llm_judge()`，用的是 `runtime::llm_bridge::build_llm_bridge_from_db()`（全仓 **17 个调用者**，bridge 权威来源） | 删 `init/llm_providers.rs::build_llm_judge_from_db` —— 它是**平行副本**且零调用，按 AGENTS.md 第 12 条删除 |
| `LlmReasoningProvider` | **确断** | `ProviderLlmBridge` 未实现该 trait（只实现 Evolution / Judge / TextGrad / Tool / Prm 五个），`LlmDrivenReasoningProvider` 是唯一实现                       | 按 P0-B 接线（已完成）                                                                                     |

**我上轮写的注释是说谎的**：`llm_providers.rs` 里声称「`set_llm_judge()` 同样零生产调用」——
实测 `services.rs` 有真实调用。根因是我的 grep 模式是 `with_reasoning_provider|LlmReasoningProvider`，
`set_llm_judge` 不在其中。**已修正。**

**方法论沉淀**：判「某函数零调用」时，必须同时 grep **它消费侧的全部入口**
（此处是 `with_llm_judge` **和** `set_llm_judge` 两个），只查其中一个会得出「断链」的错误结论 ——
而两类结论的处置方向正好相反（虚构的断链要激活，真实存在的重复实现要删除）。

---

## P2-A 🟡 记忆写入：加防抖，但不抄 EvoFlow 的队列

### AxInvest 现状

同步 `await` 写 DB（`trajectory/src/memory_providers/service.rs:358`），**无防抖、无队列**；并发安全靠进程内 `tokio::sync::RwLock<WorkingMemory>`（`:244`）+ DB 主键 `OnConflict`（`trajectory/src/storage.rs:1244-1245`）。

### 借鉴与反借鉴

- ✅ **借鉴意图**：写入合并/防抖能显著降低写入放大。
- ❌ **不要借鉴实现**：EvoFlow 的 `UpdateQueue` 是**纯内存 `threading.Timer` + list，`daemon=True`，无持久化 ⇒ 进程退出即丢**（`memory/queue.py:38-41,110`）；Markdown 镜像的 tmp 名固定为 `<file>.tmp`、**无文件锁**（`assets/memory_mirror.py:45-49`）。
- ✅ **AxInvest 反而更稳**：写 DB + `OnConflict` 天然比「写文件 + 无锁」安全。**不要为了抄一个「更花哨的」机制而把更稳的实现换掉。**

### 真正该修的是 fail-open

`service.rs:371/444/492` 的 FTS 索引失败仅 `tracing::warn` ⇒ 索引静默落后于主数据。按 fail-open 三问复核：① 归因字段是否说谎 ② 降级产物有无消费者 ③ 是否本可补救却放弃（本可重试/入队补偿）。

### 目标状态

- 防抖：**先确认收益**（写放大是否真的是瓶颈），再决定做不做。
- 优先级更高的是：**给 FTS 索引失败加补偿路径**（重试队列或重建入口），并在 UI/日志上把「索引落后」暴露出来，而不是只 `warn`。

---

## P2-B 🟢 把 EvoFlow 的病历转成 code review checklist

**这是本次调研最可复用的产物**：一份「出口不严」形态的缺陷清单，用于 review 自己的和别人的代码。

| #  | 反模式                     | EvoFlow 实例                                                              | 检查问法                                            |
| -- | -------------------------- | ------------------------------------------------------------------------- | --------------------------------------------------- |
| 1  | 护栏默认关闭               | `guardrails_config.py:24 enabled=False` 且 provider 为 None 时不注册      | 这个 gate **默认开吗**？不配置时会怎样？            |
| 2  | bare except 导致 fail-open | `plan_guard_middleware.py:1659 except Exception: return request`          | 异常分支是**放行还是拦截**？                        |
| 3  | 护栏被主动改写成 no-op     | `loop_detection_middleware.py:672-675,680-686,769-771` 全改 `return None` | 这个检查**还在生效吗**？还是被 `return None` 掉了？ |
| 4  | 计数条件可被绕过           | `auto_continue:357-372` 遇 `tool_calls` 直接 return ⇒ 计数永不推进        | 计数器的**推进前提**能否被被观测对象规避？          |
| 5  | 同名默认值多处不一致       | `max_steps` = 1000 / 50 / 8                                               | 这个默认值**只有一个定义点吗**？                    |
| 6  | 文档与代码相反             | `config.example.yaml:248` 称桌面默认开，代码是关                          | 注释里的默认值与实际一致吗？                        |
| 7  | 边界只写在文档             | `CONTRIBUTING.md:170` 一条规则 + 60 处违规                                | 这条约束**被什么机器执行**？                        |
| 8  | 非原子写入                 | `assets/promote.py:46` 直接 `write_text`                                  | 写一半崩了会怎样？                                  |
| 9  | 软上限伪装成硬上限         | `replicas=3` 超限只 `warning` 后继续超建                                  | 这个上限**真的拦住了吗**？                          |
| 10 | dead config                | `allow_host_bash` 已 `deprecated=True` 但仍在文档/配置里                  | 这个配置项**还有人读吗**？                          |

---

## P1-C 第一阶段实施记录（2026-09-12）

### 原计划 vs 实际

原计划是「新建 7 张表 + 迁移」。**侦察后发现真正的断链比缺表严重得多**，
因此第一阶段改为从「事件脊 + 唯一写路径 + 注册补齐」入手，**没有新建 tasks 表**。

### 第一个重大发现：同一列有三个互不知晓的写入者

`background_tasks.status` 此前有 **3 条裸字符串写路径**：

| 写入者     | 位置                                          | 时间戳单位                    |
| ---------- | --------------------------------------------- | ----------------------------- |
| 命令层     | `commands/background_tasks.rs::update_status` | **毫秒** `timestamp_millis()` |
| DAO 仓库层 | `dao/background_task_repository.rs`           | **秒** `timestamp()`          |
| 恢复器     | `scheduler/restore.rs`                        | **秒** `timestamp()`          |

同一列两套时间戳单位 ⇒ 「按时间排序」静默错序（`stock_analyses` 用毫秒 /
`workflow_executions` 用秒，这是本项目的历史包袱，此处又复现一次）。

### 第二个重大发现：28 个仓库 getter 里 9 个「被消费但零注册」

全量审计 `agent_repositories.rs`：**9 个仓库从未注册**，而 getter 内部是
`.expect("... not initialized.")` ⇒ **运行期必 panic**。受影响的是**活跃链路**：

- 6 个 `Task*` agent 工具全部不可用（`TaskCreate` 一调即 panic）
- MCP 工具审计记录 panic
- 平台消息桥、动态工具持久化、4 个知识图谱工具

形态与事件断链**同源**：契约 / 实现 / 消费点三样齐全，唯独少「注册」这一句，
而 `cargo check` 与单测全绿。已全部补注册，并新增回归测试
`all_registered_repositories_are_retrievable` —— **逐个调用 getter**（getter 内部即
`.expect`）而不是断言注册表字段非空，否则测不到点子上。

### 落地内容

| 件                        | 位置                                                                      | 说明                                            |
| ------------------------- | ------------------------------------------------------------------------- | ----------------------------------------------- |
| `TaskStatus` + 静态迁移表 | `harness/src/task_state.rs`                                               | 单一真源；9 测试                                |
| `TaskSource` + 来源约束   | 同上                                                                      | `→ Pending` 只允许 `Restore`                    |
| 事件脊实体                | `entities/src/task_events.rs`                                             | 真列 + 索引，不塞 JSON                          |
| 迁移 v226                 | `dao/src/migrations/v226_task_ledger.rs`                                  | 3 测试（幂等 / 三索引真实存在 / NOT NULL 契约） |
| 唯一写入口                | `dao/src/task_ledger.rs`                                                  | 状态变更与事件写入**同事务**；13 测试           |
| 三条写路径收敛            | command / repo / restore                                                  | 全部改走 `transition_task`                      |
| 读路径接线                | `commands/background_tasks.rs::list_task_events` + `TaskPanel.tsx` 时间线 | 只写不读 = 死表                                 |

**fail-closed 语义**：非法迁移返回 `Err` 且**不写任何东西**
（对比 EvoFlow `plan_guard_middleware.py:1659` 的 `except Exception: return request`）。

**刻意未落地 `awaiting_close` 态**：没有「验收动作」的写入者、也没有推进器 ——
先加即制造一个新的静默失效（铁律 #6）。宁可留空，不留假入口。

### 顺带修掉的字段契约漂移

`TaskPanel.tsx` 的 `BackgroundTask` 接口写的是 **snake_case**（`task_type` / `exit_code`），
而后端 `BackgroundTaskInfo` 带 `#[serde(rename_all = "camelCase")]`，且 `@/lib/invoke`
**不做任何键名转换** ⇒ 这两个字段在前端**恒为 `undefined`**：
类型标签渲染成空 Tag、退出码永不显示。`tsc` 与单测全绿 —— 典型的「两侧类型各自自洽」。

同时也修掉了同一处的**归因说谎**：原实现
`STATUS_CONFIG[task.status] || STATUS_CONFIG.pending` 会把任何不认识的
status 显示成「等待中」。已改为显式展示「未知状态 (原始值)」。

---

## P2-A 实施记录（2026-09-12）：原前提不成立，真因深一层

### PLAN 原文的前提

> `service.rs:371/444/492` 的 FTS 索引失败仅 `tracing::warn` ⇒ 索引静默落后于主数据。
> 目标：给 FTS 索引失败加补偿路径（重试队列或重建入口）。

### 实测后：这三条 `warn!` 是**不可达的死代码**

| 位置                                        | 实际情况                                                                                                             |
| ------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `service.rs:491/641/670/687`（4 处 delete） | `storage.rs::delete_memory_fts` 内部是 `let _ = fts.delete_from_fts(...)` 后 `Ok(())` ⇒ **恒返回 Ok，warn 永不触发** |
| `service.rs:371/444`（2 处 index）          | `index_memory_fts` 在 `fts_searcher == None` 时走 `else { Ok(()) }` ⇒ 在 PG 后端同样**永不触发**                     |

### 真因：生产后端下整个 FTS5 子系统是静默 no-op

`src/init/state.rs:151` 明确分支：

```rust
let storage = if sea_db.get_database_backend() == sea_db::DbBackend::Postgres {
    TrajectoryStorage::new(...)          // ← fts_searcher: None
} else {
    TrajectoryStorage::with_fts_path(...) // ← 仅 SQLite 挂载 FTS
};
```

PostgreSQL **是生产数据源**。于是：

1. `search_fts` 返回 `Ok(Vec::new())` —— **「搜索无命中」与「功能未挂载」在 API 层完全同形**。
   UI 上的「0 条结果」是一句**无法证伪的话**（铁律 #12：归因字段说谎）。
2. 所有 FTS 委托方法（约 10 处）都是 `if let Some(fts) = ... else { Ok(()) }`
   ⇒ 「功能静默关闭」与「操作成功」不可区分。
3. 上述 6 处 `warn!` 是伪装成失败处理的死代码 —— **P2-A 想给它们加补偿路径，前提不成立**。

### 附带发现：`rebuild_indexes()` 是「越修越坏」的破坏性空操作

```rust
// 修复前
DROP TABLE IF EXISTS trajectories_fts;        // 4 张全删
DROP TABLE IF EXISTS memory_items_fts;
DROP TABLE IF EXISTS trajectory_skills_fts;
DROP TABLE IF EXISTS trajectory_messages_fts;
// 再 CREATE 成空表 —— 然后就结束了，
// 没有任何回填步骤，却 info!("rebuilt successfully") + Ok(())
```

- **零调用方**（`health_check` 同样零调用方 ⇒ 整个 FTS 健康/重建子系统是死代码）
- 方向是**越修越坏**：重建前可能还有部分索引，重建后必然全空
- 而返回 `Ok(())`，调用方无从发现
- `needs_rebuild` 定义为「四张索引表都为空」⇒ **重建后恒为 true，永远无法满足**

### 本轮实际修复

| 件                             | 修法                                                                                                                                                       |
| ------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `delete_memory_fts` 吞错       | `let _ =` → `?` 传播。内层已自行处理「不在索引中」的正常情形，故任何 `Err` 都是真故障                                                                      |
| `rebuild_indexes` 破坏性空操作 | **硬规则：不 DROP 一张没有回填路径的表**。本版本只重建 `memory_items_fts`（唯一具备回填路径者），其余 3 张原样保留并列入 `skipped`                         |
| 重建无自校验                   | 回填后**必须** `源行数 == 索引行数`，不等即 `Err`。破坏性操作不能靠「没报错」证明成功                                                                      |
| `needs_rebuild` 恒真           | 改为判定「有源数据但索引为空」，且**可被满足**（重建后转 false）                                                                                           |
| 「索引落后」不可观测           | `FTS5Health` 增加 `available` / `unavailable_reason` / `memory_items_source_count`；新增读命令 `get_memory_fts_health` + MemorySettings 状态行             |
| 「重建索引」只重建向量索引     | `rebuild_memory_index` 现在**同时**尽力重建 FTS 并以 `MemoryIndexRebuildResult` 回报；FTS 不可用时不报错（设计内降级），只在 `ftsUnavailableReason` 里说明 |

### 回填的列映射依据（`storage.rs::save_memory` 是唯一写入路径）

| FTS 列        | 源列                      | 依据                                                                                       |
| ------------- | ------------------------- | ------------------------------------------------------------------------------------------ |
| `memory_type` | `memory_items.title`      | `save_memory` 把 `MemoryEntry.memory_type` 存进了 `title` 列                               |
| `entities`    | `memory_items.tags`       | `index_memory` 写 `entities.join(" ")`，而 `save_memory` 存 JSON 数组串 ⇒ 需剥掉 `[ ] " ,` |
| `created_at`  | `memory_items.updated_at` | `memory_items` **没有** created_at 列；该列 UNINDEXED 仅用于回显                           |

### 仍未处置（待决策）

1. ~~**PG 侧全文检索是否实现**~~ → ✅ **已实现**（2026-09-12 第二轮，见下节）。
   `ax_cjk_ngram(regconfig, text)` + 生成列 + GIN 索引全部落地，Rust 查询侧
   `text_ngram::cjk_ngram` 与之同构并有跨侧一致性校验脚本。
2. ~~**`storage.rs:1148` 插入时硬编码 `index_status = "ready"`**~~ → ✅ **已修**
   （2026-09-12 第二轮）：改为 `pending`，并把「推进器」补上（孤儿扫描器），
   于是「先声称就绪」与「永不推进」两个缺陷同时消除。决策依据见下节。

### 仍未处置（新发现，2026-09-12 第二轮）

3. **`enqueue_job_sync` 恒返回 `Ok`**（`index_queue.rs:967-1039`）：函数签名是
   `-> Result<String, String>`，但体内 `tauri::async_runtime::spawn` 后无条件
   `Ok(item_id)` ⇒ **所有调用方的 `Err` 分支都是死代码**（4+ 处），入队失败只
   通过 `index-job-failed` 事件 + `tracing::error!` 上报。属「归因字段说谎」：
   调用方以为自己检查了入队结果。处置需先决策语义 —— 是改成真同步（调用方
   可感知失败，但命令层要等 DB 写入）还是保留投递式并把返回类型降为 `String`。
4. **`update_memory_item` 的 `job_type` 与 `container_type` 不匹配**：
   用 `JOB_TYPE_REINDEX_DOCUMENT` 配 `container_type = "mem"`。worker 按
   `job_type` 分派时只有 `extract_entities` 特殊，其余全走 `run_indexing`
   并按 `container_type` 二次分派 ⇒ **功能正常，仅作业统计里的类型标签错**。
   P3 级（观测性），非功能缺陷。
5. **迁移高水位线缺陷**（已修根因，见下节）：存量库 `applied == latest` 但中间
   有缺口时，`run_migrations` 永久跳过该缺口。**本轮已把判据改为集合成员**，
   故 `PLAN` 中原「等用户点修复 Schema」的兜底不再是唯一出路。

---

## 全文检索 + 向量索引 实施记录（2026-09-12 第二轮）

用户指令：「1、实现全文检索 2、向量索引」。本轮把 P2-A 遗留的两条待决策项收口，
并顺带钉死一个跨 227 个版本的迁移框架缺陷。

### 一、全文检索（中文）

| 件             | 内容                                                                                                |
| -------------- | --------------------------------------------------------------------------------------------------- |
| 分词函数       | PG 侧 `ax_cjk_ngram(regconfig, text)` 迁移落地（`v227_cjk_fts.rs`），PG 生成列调用                  |
| 生成列         | `memory_items.content_tsv` / `notes.tsv` 实测生成表达式确含 `ax_cjk_ngram(...)`（探针验证，非推断） |
| 索引           | GIN：`idx_memory_items_tsv` / `idx_notes_tsv`，另有 `messages.content_tsv` 等既有列                 |
| Rust 查询侧    | `text_ngram::cjk_ngram` 必须与 PG 侧**逐 token 同构**，否则静默失配                                 |
| 跨侧一致性校验 | `scripts/check-ngram-consistency.mjs`：函数一致性 34/34 通过，覆盖 5 张表共 **39 项检查全通过**     |
| 接入方式       | 接进 `scripts/ci-check.mjs` 本地聚合器（**不**进 `ci.yml`：CI 无 PG 会永远 SKIP，属假绿灯）         |

> 关键区分：`memory_items`/`notes` 走 CJK ngram；而遗留表 `trajectory_memories.tsv` /
> `trajectory_messages.tsv` 等仍是旧的无分词生成列 —— 那是 v101 未执行的旁证（见第四节）。

### 二、向量索引：孤儿扫描器

**缺陷形态**：`index_status = 'pending'` 是一个**声称「等待向量化」但无人查看**的状态。
`index_status` 由 DAO 层在插入时写入，而「入队」是命令层的独立动作 ⇒ 任何绕过命令层的
写入路径都会留下永久 pending。

生产实证（PG 探针）：

| 证据                               | 值                                                          |
| ---------------------------------- | ----------------------------------------------------------- |
| `memory_items` 全表                | 2 行，均 `pending`，`source='reflector'`，自 09-06 停 6 天  |
| `index_jobs` 中 `mem` 类           | **0 行**（`index_memory` 亦为 0）⇒ 记忆向量化从未入队过一次 |
| `vec_collections` 中的 memory 集合 | 不存在（只有 `capabilities` / `kb_*` / `wiki_*`）           |

**修法**：`IndexJobService` 增加与队列轮询（500ms）解耦的扫描器
（`ORPHAN_SWEEP_INTERVAL_MS = 60_000`），单条处置收敛到 DAO 层
`repair_pending_memory_item`，四段判定链互斥且穷尽：

| # | 判定                       | 结局                                                                              |
| - | -------------------------- | --------------------------------------------------------------------------------- |
| 1 | 已有活跃作业               | `AlreadyQueued`（幂等；`mem` 与 `memory` 两种写法都查，否则会产出重复作业做两遍） |
| 2 | 命名空间未配 provider      | `MarkedSkipped` + 写明原因                                                        |
| 3 | **绑定的 provider 已悬空** | `MarkedFailed` 终态，**不入队**（见下）                                           |
| 4 | 真孤儿                     | `Enqueued`                                                                        |

### 三、悬空 provider：为什么必须在入队**前**判

用户贴的生产日志是扫描器生效的直接证据，同时暴露新缺陷：

```
WARN axagent_lib::index_queue: 任务将重试 job_id=e2afecd0-... retry=1
     error=Not found: Provider af052547-1b84-41e1-b72a-94e56eec7df9
```

| 环节               | 事实                                                                                                         |
| ------------------ | ------------------------------------------------------------------------------------------------------------ |
| 悬空 id 的前置校验 | `rag::resolve_embedding_provider` **只**检查 `embedding_provider.is_some()`，不校验被引用的 id 存活          |
| 错误串             | `Not found: Provider <id>` **不含** `ERR_NO_EMBEDDING_PROVIDER` ⇒ 接不住 R9「确定性配置错误不重试」通道      |
| 后果               | 空转 `max_retries` 次，每次结果必然相同                                                                      |
| 生产库现状         | `af052547-…` 在 `providers` 表**不存在**；现存 `llama.cpp = 6f67c842-…`；两个命名空间**都**绑着已删的那个 id |

> **原则**：「重试一个确定性无效的动作」是缺陷，不是韧性。确定性配置错误必须在入队前判掉。

### 四、迁移高水位线缺陷（跨 227 个版本）

**判据错位**：`run_migrations` 用 `MAX(version)` 做高水位线，
`skip if m.version <= applied_max` ⇒ 任何**后补进清单的中间版本号**在既有库上被永久
静默跳过（新库按序跑则正常）。这是「新装 / 升级不一致」类缺陷。

集合差探针（源码清单 vs 版本表）：

| 指标              | 值                                         |
| ----------------- | ------------------------------------------ |
| 源码 `MIGRATIONS` | n = 68，min = 100，max = 227，重复版本号 0 |
| 生产库版本表      | n = 66，min = 100，max = 227               |
| **差集**          | **`{101, 139}`**（68 − 66 = 2，算术自洽）  |

两条缺口的产物缺失**全部实测确认**，不是推断：

| 缺口 | 内容                | 产物验证                                                                                                                                                  |
| ---- | ------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| v101 | `trajectory_*` 合并 | sentinel KB `__sys_trajectory__` = 0 行；sentinel NS `__sys_trajectory_memory__` = 0 行；旧表 `trajectory_entities`/`memories`/`relationships` **仍存在** |
| v139 | 建 `session_events` | `to_regclass('public.session_events')` = **NULL**                                                                                                         |

**v139 是活的静默降级**：`DbSessionEventSink::emit` 每次 `INSERT INTO session_events`
都会失败，且只 `tracing::warn!` 吞掉（`init/state.rs:2661`）⇒ 事件流持久化 100% 失败；
消费方 `agent_resume_from_events` 永远读不到事件。功能"存在"（sink 已注入、命令已注册），
实际不可用 —— 典型 fail-open。

**v101 目前无数据损失**：其 DROP 清单只有 `trajectory_entities`(0 行) /
`trajectory_relationships`(0 行) / `trajectory_memories`(0 行)，且数据迁移段被
`table_has_rows` 守卫；有数据的大表（`trajectory_workflow_reflections` 27212、
`trajectory_patterns` 1502 等）不在其 DROP 清单内 ⇒ **补跑安全**。

**修法（两层，刻意不加第三层）**：

| 层                                | 改动                                                                                                                                                                                                                 |
| --------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `run_migrations`                  | 判据 `MAX(version)` → **版本表集合成员**（新增 `read_applied_versions`）⇒ 缺口被**精确补跑**（只跑缺的 v101 / v139）                                                                                                 |
| `get_schema_status.pending_count` | `CURRENT_VERSION - applied` → **集合差大小**。旧算法在 `applied == latest` 时给 0，而 UI（`DatabaseSettings.tsx:265`）用 `pending_count > 0` 决定显示「待应用 N 条」还是「Schema 已是最新（v227）」⇒ **UI 直接谎报** |

> **刻意不做**：曾把 `init/database.rs` 的启动自愈判据也从「版本超前」扩为「超前 **或** 集合差非空」，
> 后**回退**。理由：`run_migrations` 改集合成员判定后，缺口已由它自身精确补跑；而自愈走的是
> `repair_schema` —— **把全部 68 条迁移无条件重跑一遍**。该函数在本库**从未执行过**
> （原判据 `applied > latest` 在此为假），在一个 227 版本的存量生产库上首次无人值守触发
> 重型全量重跑，属于"为完整而引入的风险"。只保留了已失效注释的更正。

> 附带纠正：`repair_schema` 里「下次启动 `run_migrations` 将重试失败的迁移」这句注释
> 在高水位线下**是假的**（失败迁移没写版本号，但比它大的版本号可能已写入 ⇒ `MAX` 越过它）。
> 改集合成员判定后这句话才成立。

**回归测试**（`crates/dao/src/migrations/mod.rs`）：`registered_versions_are_unique`、
`missing_versions_detects_middle_gap_even_when_max_is_current`（含对旧算法给出 0 的反证）、
`missing_versions_is_empty_when_all_registered_applied`、
`run_migrations_backfills_middle_gap_on_existing_db`（端到端：造缺口 → 必须补跑）。

### 五、两个红测按生产实际修正

| 测试                 | 历史期望  | 实际   | 判定依据                                                                                                                                                    |
| -------------------- | --------- | ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `stockAnalysisStore` | 5 个监听  | **6**  | 第 6 个是 `workflow-step-start`，注释含 `T-1 P1(2026-09-12)` 与精确行号 ⇒ **有意新增** ⇒ 改断言，并逐事件名断言                                             |
| `agentStore`         | 25 个监听 | **24** | 差异为 `agent-plan-ready-for-approval`，本文件 L382 已判「删监听端（被 `task_shape_approval` 取代）」⇒ 改断言并加 `not.toHaveBeenCalledWith` **上锁**防回归 |

### 六、契约段 A 唯一 error：`rebuild_memory_index` 未注册 —— 真因是**生成器**，不是漏加一行

段 A 报「已定义但未注册到 `generate_handler!`（前端 invoke 会 404）」。前端确有 **3 处生产调用方**
（`MemorySettings.tsx:1352` / `:1430`、`sourceStore.ts:184`）+ browserMock 1 处。

**⚠ 第一次处置是错的**：我在 `src/register_commands.rs` 手写补了一行，段 A 随即 `errors=0`。
但该文件**是 `build.rs` 的生成物**：

```
$ head -2 src-tauri/src/register_commands.rs
// Auto-generated by build.rs — DO NOT EDIT
// 手动修改将被下一次构建覆盖
```

`build.rs:275-279` 把生成结果同时 `fs::write` 到 `OUT_DIR` 与 `src/`。所以手写那一行
**在下次构建时被静默覆盖** —— 「修好了」只是被撤销前的假象。（历史上 09-12 的一份工作记录
写过「已补注册」，本条即为该记录的更正。）

#### 真根因：生成器的反向搜索「遇到文档注释即终止」

`build.rs::scan_file_for_commands` 按行文本回溯找命令注解，`///` 文档注释既非空行、不以 `#`
开头、也不含 `( ) = ,`，于是**落进 `break` 分支**：

| `commands/memory.rs`                 | 行      |
| ------------------------------------ | ------- |
| `#[tauri::command]`                  | 562     |
| `/// …`（10 行文档注释）             | 563–572 |
| `pub async fn rebuild_memory_index(` | 573     |

⇒ 回溯第一行就终止 ⇒ **该命令永远不被收录**，且**每次构建都自动复现**。

#### 🔑 无 git 也能做时序取证：构建快照

`target/debug/build/axagent-*/out/register_commands.rs` 是**源码在编译时刻的逐字节副本**，
构成一条与 git 无关的行级历史：

| 快照时间                                      | 含 `rebuild_memory_index`                    |
| --------------------------------------------- | -------------------------------------------- |
| 09-02 21:53 / 22:47、09-06 03:32、09-11 14:38 | ✅                                           |
| 09-12 17:34 / 18:49 / 19:36 / **19:52**       | ✅                                           |
| **09-12 21:08**、09-12 22:09                  | ❌                                           |
| 修复后 09-12 **22:53**                        | ✅（89218 → 89270 = +52 字节，恰为该行长度） |

源码 mtime 22:09 恰为一次构建时刻 ⇒ **消失时点与构建时刻重合**，坐实「被生成覆盖」。

#### 修复与验证

| 件           | 内容                                                                                                               |
| ------------ | ------------------------------------------------------------------------------------------------------------------ |
| 回溯跳过注释 | 空行 / `//` / `/*` / `*` 一律 `continue` 而**不终止**；窗口 10 → **64** 行                                         |
| 自检警告     | 整行精确匹配的命令注解数 ≠ 收录数 ⇒ `eprintln!`（用精确相等而非 `contains`，避免文档注释提及注解时凭空多算成噪音） |
| **去重**     | 见下                                                                                                               |

**验证**：重跑构建后生成物被 `fs::write` 整体重写仍含该行 ⇒ 只可能来自生成器；
新快照（22:53:19 / 89270 字节）与 `src/register_commands.rs` **逐字节一致**。

#### 顺带发现：生成器重复收录 13 条命令

同一源文件被扫两遍（`mod xxx;` 子模块 + `pub use xxx::*` 重导出）⇒ 生成的注册表里
**1500 条中有 13 条重复**，例如 `commands::browser::browser_click` 连续出现两次
（两条都带 `#[cfg(not(mobile))]`）。重复注册不报错、不告警，属静默冗余。
修法：`handlers.dedup_by(|a, b| a.path == b.path && a.cfg == b.cfg)` ——
**必须 cfg 也相等才合并**，cfg 不同的同 path 条目是合法平台变体，删掉会丢注册。

> 自证不误报：复刻修复后的解析规则扫全部 257 个命令文件，声明数 = 收录数 = **1500**，
> **0 个文件触发新警告**。

### 七、本轮验证

| 检查项                                                                                   | 结果                                                            |
| ---------------------------------------------------------------------------------------- | --------------------------------------------------------------- |
| `cargo test -p axagent-dao --test orphan_memory_index`                                   | **7/7 通过**                                                    |
| `cargo test -p axagent-dao --lib migrations::`                                           | **107 passed / 0 failed**（含新增 4 个回归）                    |
| `cargo test -p axagent-search -p axagent-dao`                                            | EXIT=0                                                          |
| `npm run contracts -- --only=a,c,d,f,g,h`                                                | **PASS（errors=0）**                                            |
| `scripts/check-ngram-consistency.mjs`                                                    | PASS：39 项全通过                                               |
| `npx vitest run` 三个 store 测试                                                         | 72/72 通过                                                      |
| `npx tsc --noEmit` / `oxlint --max-warnings=0` / `dprint check`                          | 全部通过                                                        |
| `scripts/check-line-endings.sh` / `check-hardcoded-i18n.sh --strict`                     | 全部通过                                                        |
| 生成器自检复刻扫描（257 个命令文件）                                                     | 声明 = 收录 = 1500，**0 个文件误报**                            |
| 注册表核验（末次生成物）                                                                 | 含 `rebuild_memory_index`、重复条目 0、与最新构建快照逐字节一致 |
| `cargo fmt --check --all`                                                                | 通过                                                            |
| `cargo check -p axagent --lib` / `cargo clippy -p axagent-dao --all-targets -D warnings` | 见 `output/final-verify.log`（末次编辑后全量）                  |

新增的 4 个迁移回归测试：`registered_versions_are_unique`、
`missing_versions_detects_middle_gap_even_when_max_is_current`（含对旧算法的反证）、
`missing_versions_is_empty_when_all_registered_applied`、
`run_migrations_backfills_middle_gap_on_existing_db`（端到端造缺口 → 必须补跑）。

### 八、悬空 provider 的**覆盖面缺口**（用户贴日志后二次修正）

用户贴出生产日志后我先做了「扫描器入队前预检」，但那只堵住**一条**路径。复查
R9 通道本身时发现缺陷还在，属同源问题在**第二个出口**复发（呼应铁律「修消费端 ≠
修定义端」）：

| 环节                                       | 修前状态                                                              |
| ------------------------------------------ | --------------------------------------------------------------------- |
| 孤儿扫描器（`repair_pending_memory_item`） | 已判悬空 ⇒ `MarkedFailed`，不入队 ✅（上一轮已修）                    |
| **队列侧 R9 通道**（`index_queue`）        | **只匹配 `ERR_NO_EMBEDDING_PROVIDER`** ⇒ 悬空引用接不住 ⇒ 空转重试 ❌ |
| 用户按提示点「重建索引」                   | 绕过扫描器，错误串仍无标记 ⇒ 再次空转重试 ❌                          |

第二条最刺眼：**我上一轮写的恢复指引本身指向一条仍会空转的路径**。

**四层修法**（按依赖顺序）：

| # | 位置                                    | 动作                                                                                                                                                                                         |
| - | --------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1 | `harness/src/constants.rs` `mod embed`  | 两个标记（`ERR_NO_EMBEDDING_PROVIDER` / `ERR_EMBEDDING_PROVIDER_GONE`）**同处权威定义**；新增 `DETERMINISTIC_CONFIG_MARKERS` 唯一清单 + `is_deterministic_config_error()` 谓词（从清单派生） |
| 2 | `search/src/rag.rs`                     | 原常量定义改为 `pub use` re-export（不重复定义；`axagent_search::rag::ERR_NO_EMBEDDING_PROVIDER` 既有路径不变）                                                                              |
| 3 | `src/indexing.rs` `build_embed_context` | `get_provider` 失败后查 `provider_exists`，确系悬空 ⇒ 改写为带标记的错误（**唯一咽喉**，覆盖知识库/记忆/Wiki/手动重建索引全部路径）                                                          |
| 4 | `src/index_queue.rs` R9 分支            | 判定改用 harness 谓词；归因文案由「embedding 未配置」改为「未配置或已悬空」（原文案对悬空场景说谎）                                                                                          |

**为什么谓词必须从清单派生**：缺陷形态就是「有标记②，判断却只写标记①」。
`DETERMINISTIC_CONFIG_MARKERS` 让新增标记的**唯一动作是加进数组**，谓词自动覆盖 ⇒
结构上不可能再漏。新增 2 个单测（`harness`，含**反向断言**：`429` / 超时 / 空串
不得被判为确定性错误，否则会把可自愈失败变成永久失败）。

**两个刻意的设计选择**：

- `provider_exists` 只在 `get_provider` **失败之后**才查（happy path 零额外 SQL）；
  且失败时 `unwrap_or(true)` 保守放行 —— DB 抖动不能被误报成配置错误。

> **⚠ 更正（同日第三轮）**：下面这条上一轮的结论**已被推翻**，理由见第九节。
> 原文保留以便对照。
>
> - ~~`search/src/rag.rs:416` 的「降级为纯 FTS」**不动**：那是另一套判定（降级 vs 终态
>   失败），且悬空错误在 resolve 阶段根本不出现（`resolve_embedding_provider` 对含
>   `::` 的串直接返回），硬接会把「配置错误」静默降级成「检索无结果」—— 正是本仓库
>   反复出现的 fail-open 形态。~~
>
> 两处事实修正：① 「悬空错误在 resolve 阶段不出现」是**对的**，但由此推出的
> 「所以不动」是**错的** —— 正因为错误出现在 resolve **之后**（`generate` 调用处），
> 才必须在**那个位置**也加判定，而上一轮只看了 resolve 处的 match、没看紧跟其后的
> `?`；② 后果被说反了 —— 降级 FTS 的结果是**有关键词结果**，不降级（现状）才是
> `Err` → 上游 `tracing::warn!` → 跳过该源 ⇒ **真的无结果**。fail-open 形态恰恰在
> 「不降级」这一侧。

| 检查项                                            | 结果                                                                                         |
| ------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| 注册表核验（`build.rs` 三处修复的决定性验证）     | **总条目 1487 = 唯一 1487，重复 0**（修前 1500/1487/13 重复）、`rebuild_memory_index` 命中 1 |
| `cargo test -p axagent-harness --lib constants::` | 新增 2 个单测（正向 + 反向断言），见 `output/r9-harness.log`                                 |
| `cargo check -p axagent --lib`（前一轮状态）      | **EXIT=0，error 0**，11m52s（`output/verify-mig.log`）                                       |
| 本轮 `cargo check -p axagent --lib` / clippy      | 见 `output/r9-verify.log`                                                                    |

**遗留（已记录，未做）**：错误串里的机器标记（`EMBEDDING_PROVIDER_GONE:`）会随
`index_error` 落库并显示在 UI 上。这是**既存模式**（`EMBEDDING_PROVIDER_NOT_CONFIGURED`
同样如此），不是本轮引入的，但值得在 UI 边界做一次剥标记处理。

---

### 九、R9 覆盖面的**第三个出口** + clippy 门禁失败（2026-09-12 第三轮）

#### 9.1 触发点：`clippy -D warnings` 挂在上一轮我写的代码上

`cargo clippy -p axagent-dao --all-targets -- -D warnings` 给出 **EXIT=101**：

```
crates/dao/src/repo/index_jobs.rs:659:5: error: this `if` statement can be collapsed
```

上一轮写悬空预检时用了嵌套 `if let … { if … }`。**`cargo check` 不跑 clippy lint**
⇒ 上一轮 `check -p axagent --lib` 的「0 error / 0 warning」把它盖住了，而 CI 的
`-D warnings` 会直接挂。修法：折叠为 let-chain（Edition 2024，仓库已有先例
`cache_persister.rs:58`）。

#### 9.2 按「枚举全部触发路径」穷举，发现第三个出口

| # | 出口                                                 | 修前状态                    | 处置               |
| - | ---------------------------------------------------- | --------------------------- | ------------------ |
| 1 | 孤儿扫描器入队前预检（`repair_pending_memory_item`） | 已修 ✅                     | —                  |
| 2 | 队列侧 R9 通道（`index_queue`）                      | 只认标记① ⇒ 悬空仍重试 ❌   | 已修（清单化谓词） |
| 3 | **检索端**（`search_hybrid_with_filter`）            | **只认标记①** ❌            | **本轮修**         |
| 4 | pipeline 的 Memory 语义匹配增强（`rag.rs:2294`）     | 两个 `Err` 都降级 `None` ✅ | 本就正确，不动     |

第 3 个出口的形态与第 2 个**不同**（不是"多跑几次重试"，而是"用户拿不到结果"）：

| 触发条件                     | 检索端行为                               | 用户感受       |
| ---------------------------- | ---------------------------------------- | -------------- |
| 未配置 provider（标记①）     | 降级纯 FTS                               | 拿到关键词结果 |
| **provider 已悬空**（标记②） | `Err` → 上游 `tracing::warn!` → 跳过该源 | **静默无结果** |

而生产库 `memory_namespaces` 的**两个命名空间恰好都绑着已被删除的 `af052547-…`**
（实际存在的是 `llama.cpp` = `6f67c842-…`）⇒ 第 3 个出口对该用户是**立刻生效**的。
这同时修正了第八节已被推翻的那条结论（见其 ⚠ 更正块）。

#### 9.3 修法

| # | 位置                                                                   | 动作                                                                                                                                       |
| - | ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| 1 | `search/src/rag.rs`                                                    | 新增 `fts_only_fallback()` —— 降级逻辑的**唯一实现**，两处 `Err` 共用（避免两段 `tracing::warn!` + 建索引代码分叉）                        |
| 2 | 同上：resolve 处 match guard                                           | `contains(ERR_NO_EMBEDDING_PROVIDER)` → `is_deterministic_config_error()`（清单化，与出口 2 同一判据）                                     |
| 3 | 同上：query embed 处原 `?`                                             | 改为 `match`：确定性错误 ⇒ 走同一降级；其他错误 ⇒ 原样 `Err`（不得把 DB 抖动降级成"检索无结果"）                                           |
| 4 | 两处过时注释（`src-tauri/crates/dao/src/repo/index_jobs.rs:649`、测试 `orphan_memory_index.rs:232`） | 「该错误串不含标记 ⇒ 接不住 R9」在 GONE 标记落地后已不成立 ⇒ 改为「现在拦得住；入队前判的价值是**更早更省**（零队列占用、零 embed 尝试）」 |

第 3 项是关键：**悬空错误正是在 `generate` 处才暴露**（`resolve_embedding_provider`
只读容器上的 provider 字符串、**不校验**它是否还存在）⇒ 只处理 resolve 处的 match
等于漏掉主路径。

`options` 以 `clone()` 传给降级函数：条件性 move 会让后续正常路径（`options` 仍要
传给 `search_with_filter`）报 `E0382`。反例保护：非确定性错误（`429` / 超时 / DB 抖动）
仍原样上抛。

#### 9.4 本轮验证

| 检查项                                                        | 结果                                                                                           |
| ------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| `cargo fmt --all` / `--check --all`                           | 见 `output/r9-final-verify.log`                                                                |
| `cargo check -p axagent-search --all-targets`                 | 见同上（新 match 分支借用检查）                                                                |
| `cargo clippy -p axagent-dao --all-targets -- -D warnings`    | 见同上（`collapsible_if` 修复验证）                                                            |
| `cargo clippy -p axagent-search --all-targets -- -D warnings` | 见同上                                                                                         |
| `cargo test -p axagent-dao --lib`                             | 见同上（全量 lib 单测，替代只跑 `migrations::`）                                               |
| `cargo test -p axagent-dao --test orphan_memory_index`        | 见同上（含悬空 provider 场景 3b/3c）                                                           |
| `cargo clippy -p axagent --lib -- -D warnings`                | 见同上                                                                                         |
| 标记消费点穷举（`grep -rn is_deterministic_config_error`）    | 5 处：harness 单测 ×2、`rag.rs` ×2、`index_queue.rs` ×1 ⇒ **已无 `contains(标记)` 形式的判定** |
| `resolve_embedding_provider` 调用点穷举                       | 2 处，均已核对（1 处修，1 处本就降级）                                                         |

**缺口（已记录，未做）**：`fts_only_fallback` 的降级行为本身需要 DB 才能测（属集成
测试范畴），本轮只做了「判定逻辑」层面的单测覆盖（在 `harness`，含反向断言），
降级路径无回归测试。

---

## 启动阻塞修复：迁移的 PG 兼容性（2026-09-12 第四轮）

> **触发**：用户启动应用，弹窗
> `数据库初始化失败: database initialization failed: Database error: Execution Error:
> error returned from database: 字段 "enabled" 的类型为 integer, 但表达式的类型为 boolean at line 596`

### 一、定性：这是上一轮修复的**直接下游后果**

上一轮我把 `run_migrations` 的判据从 `MAX(version)` 高水位线改成**版本表集合成员**，
于是生产库上从未执行过的 **v101 / v139** 被首次补跑。判据本身是对的（有单测 + 集合差
探针），但我**没有验证「被补跑的那两条迁移本身能不能在 PG 上跑」** —— 它们随即炸在启动路径上。

> ⚠ **这是本轮最重要的方法论增量**：造缺口测的是「判据能发现缺口」，**不是**「补跑能成功」。
> 凡「让某段代码首次执行」的改动，必须列出**首次被执行的全部条目**，逐条在**真实环境**跑一遍。
> 用 SQLite 单测环境验证 PG 路径等于没验（见第四节）。

### 二、错误链（逐层取证，零推理）

```
lib.rs:321/324            「数据库初始化失败: {e}」→ 弹窗
  ↑ init/database.rs:209   `create_pool(...).map_err(|e| format!("database initialization failed: {e}"))`
    ↑ dao/src/db.rs:78     `crate::ddl::run_initialization(&conn).await?`
      ↑ dao/src/ddl.rs:26  `migrations::run_migrations(db)`（纯转发 shim）
        ↑ migrations/mod.rs  `(m.up)(db.clone()).await?`   ← fail-fast，Err 直接冒泡
          ↑ v101_consolidate_knowledge_memory.rs  PHASE 1
             `INSERT INTO knowledge_bases (..., enabled) VALUES (..., FALSE)`  ❌
```

**v101 PHASE 1 的 `FALSE` 就是真因**，零副作用 PREPARE 一字不差复现（见第三节）。

### 三、`at line 596` 的误导 —— 用掉了我 4 轮 grep

先说结论：**不存在任何 596 行的 SQL**。这个行号与 SQL 文本毫无关系。

`sqlx-postgres-0.9.0/src/error.rs:157-165`：

```rust
impl Display for PgDatabaseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())?;
        if let Some(line) = self.line() {
            write!(f, " at line {line}")?;
        }
        Ok(())
    }
}
```

`self.line()` 来自 PG 错误报文字段 `PgErrorField::Line` —— 它是 **PostgreSQL 服务端 C 源码
的行号**（`file` / `line` / `routine` 三元组中的 `line`），**不是**用户 SQL 的行号。

⇒ 教训：错误串里出现 `at line N` 且 N 明显大于你的 SQL 长度时，**直接去读驱动源码**，
不要去找那条 SQL。我为此连续排查了 v100 巨型 DDL / v139 / repair_schema / `include_str!`
等一切"可能很长"的语句，全是空转。

补充：同一条错误在 `node-pg` 里只输出 `message` 字段（我第一版探针就没看到行号），
**这一差异本身就该是判据** —— 同一错误两种文案 ⇒ 差异部分必然来自驱动而非 server。

### 四、真因取证：三类独立证据

| # | 证据               | 方法                                                    | 结果                                                                                     |
| - | ------------------ | ------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| 1 | 列的真实类型       | PG `information_schema.columns`                         | `knowledge_bases.enabled` = **integer**，`column_default=1`                              |
| 2 | **类型不匹配复现** | **零副作用 `PREPARE`**（只 parse+analyze+plan，不执行） | ❌ `字段 "enabled" 的类型为 integer, 但表达式的类型为 boolean` —— **与用户报错一字不差** |
| 3 | 修法有效性         | 同一 PREPARE 把 `FALSE` 换成 `0`                        | ✅ 通过                                                                                  |

`PREPARE` 是**对生产库安全的类型检查器**：它不做任何写入，却能拿到 analyze 阶段的类型错误。
本仓库只用 `pg`（node）驱动做探针，正是因为它能裸发 `PREPARE` / `SAVEPOINT`。

### 五、修复（两处，第二处若不修会立刻接着崩）

#### 5.1 `v101_consolidate_knowledge_memory.rs` —— 类型层根治，而非改字面量

`knowledge_bases.enabled` 在两方言下都是整数语义（PG `data_type=integer`；repo 侧
`repo/knowledge.rs` 也按整数写 `{ 1 } else { 0 }`）。初版把 **PG 分支写成 `FALSE`**
（SQLite 分支写 `0`）—— 单侧笔误。

修法**不是**把 `FALSE` 换成 `0`，而是抽出类型化常量并把 SQL 改成 `{}` 插值：

```rust
/// Sentinel KB 的 `enabled` 取值：`0` = 禁用（系统内部 KB，不参与用户可见列表）。
pub const SENTINEL_KB_ENABLED: i32 = 0;
```

⇒ 「写错成布尔字面量」在**类型层**即不可能。**改字面量只修当前这一处，抽常量消灭整类写法。**

#### 5.2 `v139_create_session_events.rs` —— 从"手写分支"退回权威范式

初版建表 DDL 写的是 SQLite 语法：

```sql
CREATE TABLE IF NOT EXISTS session_events (id INTEGER PRIMARY KEY AUTOINCREMENT, ...)
```

PG 上直接语法错：`语法错误 在 "AUTOINCREMENT" 或附近的`（事务预演逐条复现）。

**本仓库早有权威范式**（`pg_ddl.rs` 模块文档明说「DDL 直接写 PostgreSQL 语法…新代码不应
使用 `pg_ddl()`」），`v100_consolidated` / `v207_chat_run` 都在用：

```rust
// DDL 只写一份 PG 语法（BIGSERIAL PRIMARY KEY），SQLite 侧由 exec_ddl 自动转换
exec_ddl(&db, is_pg, SESSION_EVENTS_DDL).await?;
```

⇒ 改为 `exec_ddl` + `BIGSERIAL PRIMARY KEY`。**手写 `if is_pg {PG} else {SQLite}` 两份 DDL
等于把「两方言都必须写对」变成人工义务 —— 而这类义务必然失守**（本缺陷正是失守的结果）。

顺带修掉两处归因错误：日志标签 `[v133]` → `[v139]`（v133 是 `opc_demand_discovery`，
与本表无关）；测试注释里的 `v133` 同步更正。

#### 5.3 `migrations/mod.rs` —— fail-fast 必须带归因

原实现 `(m.up)(db.clone()).await?` 裸传 DbErr ⇒ 调用方只看到一句 SQL 错误，**不知道是
68 条里的哪一条**（本轮就是活例子：错误串里完全没有 v101 的踪迹）。改为包一层带版本号 +
专项提示：

```rust
(m.up)(db.clone()).await.map_err(|e| {
    tracing::error!(version = m.version, description = m.description, error = %e,
        "[run_migrations] 迁移执行失败，数据库初始化中止");
    DbErr::Custom(format!(
        "迁移 v{} 执行失败: {e} —— 该迁移此前未应用过。\
         若它是首次在 PostgreSQL 上执行，请优先核对 DDL 是否为 PG 兼容语法\
         （SQLite 专有写法如 AUTOINCREMENT / INSERT OR IGNORE / 布尔字面量插整数列，PG 均不接受）。",
        m.version
    ))
})?
```

**刻意保持 fail-fast 语义**（不改成 warn 吞掉）：迁移失败意味着 schema 与代码不匹配，
硬撑着启动只会把故障从"起不来"变成更难查的"跑得起来但到处出错"。
**要做的是让错误说得清，而不是让它沉默。**

### 六、为什么长期潜伏：两条路径**都**测不到

| 路径       | 为什么漏                                                           |
| ---------- | ------------------------------------------------------------------ |
| 既有 PG 库 | 被 `MAX(version)` 高水位线**静默跳过**（直到上轮改成集合成员判据） |
| 单元测试   | dao 的迁移测试**全跑 SQLite in-memory** ⇒ PG 路径永不执行          |

`AUTOINCREMENT` 与「布尔插整数列」**都不是 SQLite 会报的错** ⇒ 「测试全绿」对 PG 路径
零信息量。

> ⚠ 附带结论：**全新 PG 库按序安装同样会崩**，并非存量库补跑专属。也就是说这两个缺陷
> 一直让"PostgreSQL 新用户装不上"，只是没人从零在 PG 上装过。

### 七、防复发：三层防线

| 层     | 手段                                                    | 覆盖什么                                                                                                                    |
| ------ | ------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| ① 范式 | **DDL 只写 PG 语法 + `exec_ddl`**（拒绝手写双份）       | 从源头消灭"某一侧写错"                                                                                                      |
| ② 单测 | 把方言判定抽成**纯函数**，无 PG 连接也能断言两分支产物  | v139 的 `session_events_ddl_is_pg_syntax` / `sqlite_ddl_rewrites_bigserial` / v101 的 `sentinel_kb_enabled_is_integer_zero` |
| ③ 实测 | **事务 + SAVEPOINT 回滚预演**（可对生产库跑，零副作用） | 真实 PG 的逐条可执行性                                                                                                      |

**第 ③ 层的实现要点**（`pg-v101-rehearse.js`）：

- 整体 `BEGIN ... ROLLBACK`（PG 的 DDL 是事务性的，DROP/ALTER/INSERT 全可回滚）
- **每条语句包 `SAVEPOINT` / `ROLLBACK TO SAVEPOINT`** —— 否则一条失败会让事务进入
  aborted 态，后续语句**全部假失败**（第一版没做，会得到完全错误的报告）
- 末尾做**零副作用自证**：sentinel 不存在 / `trajectory_*` 仍在 / 探测表无残留 / 版本表未写入

### 八、验证结果

| 检查项                                                                                     | 方法                             | 结果                                    |
| ------------------------------------------------------------------------------------------ | -------------------------------- | --------------------------------------- |
| `knowledge_bases.enabled` 类型                                                             | PG `information_schema`          | integer（default 1）                    |
| v101 PHASE 1（修复前）                                                                     | 零副作用 PREPARE                 | ❌ 复现用户报错（一字不差）             |
| v101 PHASE 1（修复后）                                                                     | 零副作用 PREPARE                 | ✅ 通过                                 |
| **v101 全部 38 条 SQL**                                                                    | 生产库事务预演（逐条 SAVEPOINT） | ✅ **38/38**                            |
| v139 修复前                                                                                | 事务预演（对照）                 | ❌ 1/1（`AUTOINCREMENT` 语法错）        |
| **v139 修复后 3 条 DDL**                                                                   | 生产库事务预演                   | ✅ **3/3**                              |
| 预演零副作用                                                                               | ROLLBACK 后 6 项核对             | ✅ 全部回到原点                         |
| `cargo fmt --all` / `--check`                                                              | —                                | ✅ EXIT=0                               |
| `cargo test -p axagent-dao --lib` / `--test orphan_memory_index` / clippy / 主 crate check | `output/db-fix-verify.log`       | ✅ 全部通过（详见下方「最终验证结果」） |

**最终验证结果（`output/fix3-verify.log`，全绿）**：

| 检查项                                                 | 结果                                            |
| ------------------------------------------------------ | ----------------------------------------------- |
| `cargo fmt --check --all`                              | ✅ EXIT=0                                       |
| `clippy -p axagent-dao --all-targets -D warnings`      | ✅ EXIT=0（error 0）                            |
| `cargo test -p axagent-dao --lib`                      | ✅ **174 passed / 0 failed**（含本轮新增 6 个） |
| `cargo test -p axagent-dao --test orphan_memory_index` | ✅ **7 passed / 0 failed**                      |
| `clippy -p axagent-search --all-targets -D warnings`   | ✅ EXIT=0（error 0）                            |
| `clippy -p axagent --lib -D warnings`                  | ✅ EXIT=0（7m27s）—— 含新改的 `state.rs`        |

**新增的 6 个测试（实测全过）**：`sentinel_kb_enabled_is_integer_zero` /
`v101_is_self_idempotent_and_creates_sentinels` / `session_events_ddl_is_pg_syntax` /
`sqlite_ddl_rewrites_bigserial` / `v139_creates_table_and_indexes` / `v139_is_self_idempotent`。

**修复过程中被 clippy 抓到的两个连带问题（均已修）**：

1. `src-tauri/crates/dao/src/repo/index_jobs.rs:659` 的嵌套 `if let … { if … }`（上轮遗留）⇒ 折叠为 let-chain。
2. `v139` 删掉 `execute_unprepared` 后顶部的 `use sea_orm::ConnectionTrait;` 在 lib target 下
   变 unused，**但测试模块需要它**（`query_one_raw` 是 `ConnectionTrait` 的方法，删掉后报
   `E0599: no method named query_one_raw`）⇒ 正确做法是**把 import 移到测试模块**。

### 九、本轮一并修掉的连带缺陷：`DbSessionEventSink` 三处 SQL 未参数化

**发现方式**：修完 v139 后自问「表建好了，但写入真的能成功吗」——**不能**。

`src/init/state.rs` 的 `DbSessionEventSink`（P0-3 事件流 sink）三处 SQL 全部用 `format!` 插值：

| 位置       | 原写法                                                                                 | 问题                                                                                               |
| ---------- | -------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| `next_seq` | `Statement::from_string(DbBackend::Sqlite, ...)` + `WHERE session_id = '{session_id}'` | ① backend 硬编码 Sqlite（即便连 PG）② id 含 `'` 直接语法错                                         |
| `emit`     | `VALUES ('{session_id}', {seq}, '{evt_type}', {payload_str}, '{ts}')`                  | `payload_str` 是 `serde_json::to_string` 结果（`{"conversationId":"x"}`）**不带外层引号** ⇒ 裸 `{` |
| `clear`    | `DELETE ... WHERE session_id = '{session_id}'`                                         | 同 id 含 `'`                                                                                       |

**PG 事务预演逐条实证**：

```text
INSERT ... VALUES ('sess-1', 1, 'TurnStarted', {"conversationId":"sess-1"}, '2026-...')
  ❌ 语法错误 在 "{" 或附近的
INSERT ... VALUES (..., NULL, ...)            ✅（None 时恰好合法）
参数化 $1..$5                                  ✅
SELECT ... WHERE session_id = 'it''s-a-session'（插值）  ❌ 语法错误
同 SQL 参数化                                  ✅
```

**致命之处**：调用点 `publish_agent_event`（`crates/agent/src/session_manager.rs`）
**总是传 `Some(payload)`** ⇒ 即使 v139 把表建好，**`session_events` 也永远写不进一条数据**，
而错误被 `tracing::warn!` 吞掉 ⇒ **完全静默的 fail-open**。

**修法**：三处全部改为 `Statement::from_sql_and_values` 参数化 + `self.db.get_database_backend()`；
占位符差异（PG `$1..$n` / SQLite `?`）收敛到单一函数 `bind_placeholders(n, is_pg)`：

```rust
fn bind_placeholders(n: usize, is_pg: bool) -> String {
    if is_pg {
        (1..=n).map(|i| format!("${i}")).collect::<Vec<_>>().join(", ")
    } else {
        vec!["?"; n].join(", ")
    }
}
```

> 占位符差异是 **driver 协议层面**的，上层无法消除（`from_sql_and_values` 不做转换），
> 所以只能按 backend 产出，但必须**收敛到一处**。注意仓库里 `from_sql_and_values` 有 166 处
> 用法，其中若干只写 `$1`（如 `semantic_cache/mod.rs`）—— 那些在 SQLite 上会失败，
> **属另一批待查项**（不在本轮范围）。

### 十、本轮仍未处置（记录）

1. **`v139` 的 `BIGSERIAL` 无 `setval` 对齐**：将来若从 SQLite 迁数据过来需手动对齐序列值
   （当前无此场景，仅记录）。
2. **仓库内 `from_sql_and_values` 的占位符方言一致性未审计**（166 处，见上）。
3. **`agent_resume_from_events` 的读取端同样是插值 + 硬编码 backend**
   （`src/commands/agent/mod.rs:3223-3228`）：

   ```rust
   let sql = format!(
       "SELECT seq, event_type, payload, created_at FROM session_events \
        WHERE session_id = '{conversation_id}' ORDER BY seq ASC"
   );
   let rows = db
       .query_all_raw(sea_orm::Statement::from_string(sea_orm::DbBackend::Sqlite, sql))
   ```

   **与 `DbSessionEventSink` 完全同族**（铁律 #30 意义上的「同一根因的第四个出口」）。
   本轮**刻意未改**，理由两条：① `conversation_id` 来自前端的 UUID，含单引号的概率极低；
   ② 改它会让当时正在运行的编译验证作废（`DbSessionEventSink` 与它分处两个 crate）。
   **但注意风险已因本轮改动而上升** —— 修好写入端后，这个读取端会第一次真正被调用。
   修法：`Statement::from_sql_and_values` + `db.get_database_backend()`；
   占位符 helper 应与 `init::state::bind_placeholders` 复用而非重写（先确认其可见性）。

---

### 十一、🔴 本轮最重要的方法论产出：**「谁会执行」修好了，但「被执行者能不能执行」从未验证**

上一轮我把 `run_migrations` 的判据从 `MAX(version)` 高水位线改成**集合成员**。判据本身是对的
（本轮生产库实证：缺口 = `{v101, v139}`，而高水位线 `MAX(227)` 会**静默跳过**这两条）。
**但我只造了缺口、验证了「判据能发现缺口」，没有验证「被补跑的那两条迁移本身能不能在 PG 上跑」。**
结果它们一经执行就让应用**完全起不来**。

> **造缺口测的是「判据能发现缺口」，不是「补跑能成功」。从未执行过的代码 = 未验证的代码。**

这条已固化为铁律 #33。下面这条测试腐烂的发现，是它的直接推论。

### 十二、🔴 发现：**专门用来挡这类问题的 PG 测试，本身已经腐烂到跑不过 —— 而因为 env-gated，从没人知道**

`crates/dao/tests/pg_migrations.rs` 本来就存在，它做的事恰好是「建全新库 → 跑全量
`run_migrations` → 断言 schema」。**它本该在第一时刻就挡住 v101/v139。** 但它腐烂了：

| 陈旧断言                                 | 为什么错                                       | 后果               |
| ---------------------------------------- | ---------------------------------------------- | ------------------ |
| `assert_eq!(max_v, 100)`                 | 注册表早已扩到 v227，`MAX(version)` 必然 ≠ 100 | 必失败             |
| `trajectory_memories` 断言「应存在」     | **v101 PHASE 9 会 DROP 它**                    | 修好上一条也必失败 |
| `trajectory_memories.tsv` 断言「应存在」 | 同上（表没了，列自然没了）                     | 同上               |

⇒ **它跑不过，但从没跑过**（`AXAGENT_TEST_PG_URL` 未设置即静默 `return`），
所以既没人修、也没人发现，恰恰漏掉了它本该拦住的事故。

**修复策略（关键：不能只把魔数改掉，那下次还会腐烂）**：把断言方式从「比对魔数」
升级为**比对注册表自身** —— `assert!(missing_versions(&applied).is_empty())`。
这样注册表再扩版本，本测试**不会**因数字过期而腐烂。同时补上本次两处修复的直接断言：

- 三张旧表**必须已被 DROP**（`trajectory_entities` / `trajectory_relationships` / `trajectory_memories`）
- sentinel KB 存在，且 `enabled` 列 `data_type == "integer"`、值 `== 0`
- `session_events` 表 + 两个索引存在，且**唯一索引真的唯一**（重复 `(session_id, seq)` 必须被拒）
- `run_migrations` **幂等**：二次执行不新增版本行
- 移除了原来会失败的 `trajectory_memories` / `trajectory_memories.tsv` 断言

### 十三、🎯 真机端到端验证（本轮最强证据）

不再用 JS 手抄 SQL 预演（那是**转录**，可能与源码漂移），而是**让真实 Rust 代码在全新 PG 库上跑**：

```bash
AXAGENT_TEST_PG_URL='postgres://postgres:***@localhost:5432/axinvest' \
  __TAURI_WORKSPACE__=true cargo test -p axagent-dao --test pg_migrations -- --nocapture
```

```
running 1 test
test pg_migrations_apply_and_search_works ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.34s
```

测试在独立库 `axagent_pg_migtest`（**不碰 `axinvest`**）上 `DROP+CREATE` 后跑**全部 68 条**
迁移，**全部成功且缺口为空**。这同时证明了两件事：

1. **启动 brick 已修复**（存量生产库走 v101+v139）；
2. **「PostgreSQL 全新用户装不上」也一并修好了** —— 这是此前只靠推理得出的判断，
   现在有了真机证据。

**两条路径的覆盖情况（此前两条都失明）**：

| 场景                 | 验证手段                       | 结果             |
| -------------------- | ------------------------------ | ---------------- |
| 存量生产库（有数据） | JS 事务预演（逐条 SAVEPOINT）  | ✅ 38/38 + 3/3   |
| 全新 PG 库（空库）   | **真实 Rust `run_migrations`** | ✅ 68/68，0 缺口 |

### 十四、📋 本轮新增的未修项（记录）

| # | 缺陷                                                                                               | 证据                                                                                           | 处置                                                                                                                                                                                                  |
| - | -------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1 | **4 个 PG 测试全部 env-gated，CI 里一条都不跑**                                                    | `pg_migrations.rs` / `pg_cjk_fts.rs` / `pg_runtime_test.rs` / `search/tests/pg_integration.rs` | 需要决策：CI 加 PG service（治本），或至少让「跳过」显式失败                                                                                                                                          |
| 2 | **3 个 env 变量名各不相同**                                                                        | `AXAGENT_TEST_PG_URL` / `AXAGENT_PG_TEST_URL`（注意少个 `_TEST`）/ `AXINVEST_PG_TEST`          | 统一为一个，收进 `docs/` 或 `.env.example`                                                                                                                                                            |
| 3 | **`pg_runtime_test.rs` 明文口令写进源码**                                                          | `const PG_URL = "postgres://postgres:Hjd***@127.0.0.1:5432/axinvest_pg_test"`                  | 开源仓库（AGPL）里不该有；改用 env                                                                                                                                                                    |
| 4 | **`pg_runtime_test.rs` 声称「跑全量迁移 v001..v013」**                                             | 注册表从 **v100** 起，v001..v013 早已不存在                                                    | 陈旧；连同 #3 一起重写或删除                                                                                                                                                                          |
| 5 | **`__sys_trajectory__` 常量三处重复定义**                                                          | `migrations/v101:29` / `repo/knowledge_graph.rs:19` / `trajectory/storage.rs:797`              | 违反 AGENTS.md 禁区 12。三者目前**字面量一致**，但改一处不改另两处 ⇒ `get_all_entities()` **静默返空**（它按此 id 过滤）。应上提到 `entities` crate 单点定义后 re-export                              |
| 6 | **`run_migrations` 不包事务**（每条迁移裸执行）                                                    | `migrations/mod.rs:209` `(m.up)(db.clone()).await`                                             | 修 v101 时实测失败发生在**首条**语句 ⇒ 本次零副作用。但判据改集合成员后**重试成为常态** ⇒ 中途失败的 partial apply 会被反复叠加。已确认全仓**无** `CONCURRENTLY`/`VACUUM`/自开事务 ⇒ 包事务技术上可行 |
| 7 | `src/commands/learning_graph.rs:100` 注释仍写「trajectory_entities / trajectory_relationships 表」 | 实际调 `trajectory_storage.get_all_entities()`，读的是 `knowledge_entities`（已合并）          | 仅注释过期，无功能影响                                                                                                                                                                                |

**顺带实证的一件好事**：v101 DROP 这三张表**是安全的** —— 全仓无 sea-orm 实体、无裸 SQL
引用（除 v101 自身），唯一活跃调用链 `learning_graph → trajectory_storage::get_all_entities`
读的是合并后的 `knowledge_entities` 表。

---

## 明确不做（避免范围蔓延）

| 项                                              | 原因                                                                                                                                         |
| ----------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| 治理套件（多账号 / SSO / 费用中心）             | 个人单机项目无需求；EvoFlow 侧也未文档化                                                                                                     |
| 资源市场 / 组织包分发                           | AxInvest 已有 `marketplace.rs` + `workflow_marketplace` 表，无缺口                                                                           |
| 代码符号/引用图谱                               | EvoFlow 该模块实现细节未公开，**无法作为参考**                                                                                               |
| Plan/Goal 的产品形态（确认条 / 侧栏隐藏子任务） | 属 UI 层，且 EvoFlow 的实现依赖其 LangGraph 中断语义，Rust 图引擎不通用                                                                      |
| 中间件管道范式的整体搬迁                        | 与 `rt-workflow` 的节点/边范式冲突。**仅建议在 ReAct agent 运行时内部**用中间件链承载横切机制（护栏/记忆/压缩/循环检测），**不动工作流引擎** |

---

## 执行前置条件（需要用户决策）

> 状态更新（2026-09-12）：原 1、2 条已决策并实施完毕；原 4 条已过时。

1. ~~**P0-A 是行为变更**：确认调用方就绪~~ → ✅ **已实施并全绿**（见「P0 整体验证结果」）。
2. ~~**P0-B 二选一**~~ → ✅ **已决策走「接线」**（见「P0-B 实施记录」）。
3. **P1-C 是否要做**（未决）：涉及 7 张已有表的迁移，量大。**前置条件已满足** ——
   P1-A / P1-B 均已落地，且 P1-B 的段 G 上线即发现 11 条真断链，证明这套机制确实有效。
   是否启动待确认。
4. ~~**本轮只出文档**~~ → 已进入实现：**P0-A / P0-B / P1-A / P1-B 全部落地并验证**。

### 当前待决策项

| # | 事项                                    | 说明                                                                                                                                                                                                             |
| - | --------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1 | **P1-D 的事件断链是否修**               | 需逐项判定「补发射端 or 删监听端」。2026-09-12 第二轮：基线由 **11 条缩到 6 条**，且 `agent-plan-ready-for-approval` 已确认为「有意删除」并在测试里上锁；剩余 6 条全在 `executionStore`/`agentStore`，待逐项定性 |
| 2 | `agent_plan_approvals` 遗留链是否删     | 涉及 `planApprovalEnabled` 用户可见开关；上轮已定性为「被 task_shape 取代」                                                                                                                                      |
| 3 | **P1-C 统一任务账本是否启动**           | 唯一剩余大项，迁移风险高（7 张表）                                                                                                                                                                               |
| 4 | `executionAuthorized` 的前端 UI 展示    | 需 11 语言 key，本轮为控制范围未做                                                                                                                                                                               |
| 5 | `ReActConfig` 其余 20 个字段是否也上 UI | 本轮只接通了 `maxIterations`（用户实际已有的控件）。其余字段（`timeout_secs` / `max_repeated_calls` / `token_budget_limit` 等）目前仍只有代码默认值                                                              |
