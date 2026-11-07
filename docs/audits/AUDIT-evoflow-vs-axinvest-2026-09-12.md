# AUDIT — EvoFlow vs AxInvest 源码级对比审计

> 状态：active ｜ 日期：2026-09-12 ｜ 对应版本：v2.9.7


- **日期**：2026-09-12
- **对象**：`https://github.com/EvovexAI/EvoFlow`
- **本地快照**：`output/tmp-evoflow-src/EvoFlow-main`（codeload tarball，73.05 MB）
- **证据口径**：**一切以代码为准**。文档/README/注释与代码冲突时，以代码为准并明确标注。
- **取证方式**：HTTP tarball 快照，**未使用任何 git 命令**（用户铁律）。

---

## 0. 边界声明

| 项           | 说明                                                                                                                                                       |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 许可         | PolyForm Noncommercial 1.0.0 —— **源码可见，非 OSI 开源，商用须书面授权**                                                                                  |
| 本审计产出   | **只提取机制设计意图**，产出「用自己的栈重新实现」的方案，**不搬运代码**                                                                                   |
| 栈差异       | EvoFlow = Python + LangGraph（编排范式=分层中间件管道）；AxInvest = Rust + Tauri + rt-workflow（编排范式=节点/边图）。**代码级移植不成立，机制级借鉴成立** |
| EvoFlow 规模 | 1848 个 Python 文件 / **448,153 行** / star 136 / Releases 为空 / Gitee「源码未公开」说法**不成立**（GitHub 公仓含完整 backend + evopanel）                |

---

## 1. 总判断

> **两家得的是同一类病——「声明的机制」与「运行的机制」之间存在断链——但断链形态相反。**

|      | AxInvest                                                                                                                | EvoFlow                                                                                                                            |
| ---- | ----------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| 病型 | **入边缺失型**                                                                                                          | **出口不严型**                                                                                                                     |
| 表现 | 零件齐全、**线没接**：参数被声明但无消费点、状态位有读无写、事件有监听无发射                                            | 线接上了，但**默认关闭 / 异常静默放行 / 边界不强制**                                                                               |
| 典型 | `max_steps` 参数被忽略、`ReActConfig` 零引用、`requirePlanApproval` 被 serde 静默丢弃、`agent_plan_approvals` 无 insert | `guardrails enabled=False`、`except Exception: return request`、`loop_detection` 被改成 `return None`、边界纯文档口号（60 处违规） |
| 检法 | **入边供给检查**（每个声明必须有消费点，每个状态位必须有写入点）                                                        | **出口检查**（每个 gate 的异常分支是否 fail-open、默认值是否开启）                                                                 |

**推论（本审计最有价值的一条）**：两类病的检法不同，**不能互相套用**。AxInvest 需要补的是「入边检查」，EvoFlow 需要补的是「出口检查」。把 EvoFlow 当范本去抄，等于用错误的药。

---

## 2. 逐维度对照

| 维度                                    | AxInvest                                                                                                                                                                                                                                                                                      | EvoFlow                                                                                                                                                                                                                                                                           | 谁强                             |
| --------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------- |
| **Harness 架构边界**                    | ✅ **CI 机械化强制**：`scripts/check-contracts.mjs:177-198`（段 D）限定 `CONSUMER_CRATES=[agent,gateway,orchestrator,runtime-core]` 只允许依赖 `axagent-harness`；CI 实跑 `.github/workflows/ci.yml:98`。且 `crates/harness/Cargo.toml:8-29` **零 axagent-\* 依赖**，边界在结构上不可能被违反 | ❌ **纯文档口号**：只写在 `CONTRIBUTING.md:170` + `docs/contribute/repo-map.md:20`；`ruff.toml:5` 只有 E/F/I/UP（**无 TID**）；`make check` 只查依赖装没装；CI 只有 3 个 release workflow，被引用的 lint workflow **在快照中不存在**；实际 **27 个文件 / 60 处 `from app.`** 违规 | **AxInvest 完胜**                |
| **Plan 授权**                           | ⚠️ 授权位**存在**（`entities/src/plans.rs:16 status:String`）但**闸门从未闭合**（见 §3.1）                                                                                                                                                                                                    | ⚠️ 设计正确（授权位 + 写入白名单不含 model + 三层拦截 + 改计划撤权），但 PlanGuard 全 fail-open + 有「懒授权」文本正则启发式                                                                                                                                                      | **各有半条**                     |
| **长任务停止闸**                        | ⚠️ ReAct 层有 ①②③ 但**全硬编码**且不可配；④ 缺失；`max_steps` 参数被忽略；`GoalEvaluator` fail-open（见 §3.2/§3.3）                                                                                                                                                                           | ❌ 默认无有效上限 + 结构性可绕过 + 循环护栏被主动关闭（见 §4）                                                                                                                                                                                                                    | **都不合格**，但病型不同         |
| **统一任务账本**                        | ❌ 7+ 张带 `status` 的表 + **≥10 套状态枚举**；无统一 task 实体；无 `parent_task_id`；无「待闭环」态（见 §3.4）                                                                                                                                                                               | ⚠️ 确实同表 `evoflow_collab_tasks`，但有单一真源的状态迁移表；缺陷是 `extra_json` 兜底 + 6 张并行表 + `awaiting_close` 靠人工/3 天回收 + **取消不级联**                                                                                                                           | **EvoFlow 略强**（有单一迁移表） |
| **记忆写入**                            | ❌ 同步 `await` 写 DB，**无防抖、无队列**；FTS 索引失败 fail-open 仅 `tracing::warn`（`service.rs:371/444/492`）                                                                                                                                                                              | ⚠️ 有防抖（**默认 120s**）但有明确丢数据窗口（队列=纯内存 daemon Timer、tmp 名固定无文件锁）                                                                                                                                                                                      | **都不行**                       |
| **Guardrail 护栏**                      | ⚠️ 有 `tools/src/approval.rs` + `harness/src/approval_policy.rs` + 节点级 `approval_executor.rs`                                                                                                                                                                                              | ❌ `enabled=False` 默认关闭、无会话隔离、文档称桌面默认开**与代码相反**                                                                                                                                                                                                           | **AxInvest 略强**                |
| **可观测 / 沙箱 / 多 Agent / 自我进化** | ✅ 零件更多（telemetry crate、三平台真隔离、swarm+debate+fleet、trajectory 全家桶）                                                                                                                                                                                                           | ⚠️ 桌面默认 `NoopSandboxProvider` **等于无隔离**；AIO 容器真加固但容器侧不解 symlink                                                                                                                                                                                              | **AxInvest 强**                  |

---

## 3. AxInvest 侧断链实证（本次新发现，优先级最高）

### 3.1 🔴 Plan 授权闸门从未闭合 —— **五条独立断链同时存在**

这是本次审计最严重的发现，是「铁律 #6：四条独立链缺一即静默失效」的教科书级案例，而且**比 EvoFlow 的任何问题都严重**。

| #  | 断链                            | 证据                                                                                                                                                                                                                 |
| -- | ------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1  | **状态集合不相交 ⇒ 执行必被拒** | `plan_generate` 写 `status="reviewing"`（`commands/plan.rs:822`）；而 `plan_execute` 要求 `status in {"draft","approved"}` 否则 `return Err`（`commands/plan.rs:872`）。**`reviewing` 永远进不了这个集合**           |
| 2  | **无任何代码写 `approved`**     | `grep 'Set("approved"' commands/plan.rs` → **0 命中**；`plan_modify_step` 只改 `steps_json`，从不改 `plans.status`                                                                                                   |
| 3  | **规则反而允许未批准执行**      | 校验放行 `draft`（未批准态）；且**显式传 `step_ids` 时绕过 Approved 过滤**（`plan.rs:893-894`）——而前端 `planStore.ts:116-161` 恰恰总是显式传 `allStepIds`                                                           |
| 4  | **审批通道是死胡同**            | `app_state.agent_plan_approvals` 全仓**仅 4 处引用**：定义 `app_state.rs:264` + 构造 `init/state.rs:378/1420` + **`remove`**（`agent/mod.rs:2949`）——**无任何 `insert`**                                             |
| 5  | **事件有监听无发射**            | `agent-plan-ready-for-approval` 仅 TS 侧监听（`agentStore.ts:921`）；Rust 侧 `PlanReadyForApproval` 只有枚举/映射/计数测试（`event_bus.rs:48`、`frontend_adapter.rs:121/149`、`event_emitter.rs:104`），**无发射点** |
| 附 | **前端参数被静默丢弃**          | `conversationStoreSend.ts:784` 传 `requirePlanApproval`，但后端 `AgentOptions`（`agent/payloads.rs:227-239`）**无此字段** → serde 默认忽略未知字段 → **静默丢弃，不报错**                                            |

> **📌 勘误（2026-09-12 实施前复核）**：第 4 / 5 条的定性需修正 —— **不是「入边缺失」，而是「被替代的遗留链」**。
>
> 复核发现存在**另一条完整闭环的审批链**：`task_shape_approval_senders`（`app_state.rs:438`）→ `task-shape-approval-request` 事件（`cognitive.rs:1876`）→ `TaskShapeApprovalModal.tsx` → `respond_task_shape_approval`（`cognitive.rs:2731`，已注册 `register_commands.rs:203`）。insert / emit / listener / UI / 回传**全部齐备**。
>
> 即：`agent_plan_approvals` 那条链（State 槽 + `agent_approve_plan` + TS 监听 + `PlanApprovalModal`）是 **P0-2 的旧实现，已被 P3 的 task_shape 实现取代**。按铁律 #3「被覆盖 ⇒ 删」应删除，而**不是**「补 `insert`/`emit` 让旧链复活」。本轮改为加警示注释 + 列入待决策（`planApprovalEnabled` 是 `ChatView` 用户可见开关，删除属产品行为变更）。
>
> 第 1 / 2 / 3 条（状态集合不相交、`approved` 无写入点、`draft` 放行 + `step_ids` 绕过）**定性不变，已修复**。

**附加缺口**：不存在「未授权时收窄工具面」的机制。`commands/plan.rs` 内无 `allowed_tools`/白名单，仅 `PermissionPolicy::new(PermissionMode::Prompt)`（`plan.rs:670`）。`allowed_tools` 只存在于 `agent_profiles`（agent 级白名单），与 plan 授权无关。

**模型/LLM 侧**：`plans.status` 与 `step.status` 只由上述命令写入，LLM 不直接写。但 `plan_execute` 是否可被 LLM 间接作为工具调用 —— **未验证**（见 §6 不确定项）。

### 3.2 🟠 长任务停止闸：有闸、但不可配 + 声明参数无消费点

**有闸（这部分 AxInvest 强于 EvoFlow 的表面印象）**：`crates/agent/src/reasoning_state.rs:131-158` `ReActConfig::default` 硬编码：

```
max_iterations: 50            timeout_secs: 300
max_depth: 10                 token_budget_limit: Some(180_000)
max_retry_attempts: 3         cycle_detection_enabled: true
max_repeated_calls: 3         max_no_progress_iterations: 5
checkpoint_interval: 10       goal_evaluation_enabled: false
for_simple_task: 20 / 60s；for_complex_task: 100 / 600s / 200k / 8
```

硬停止点在 `react_engine.rs:754`（`iteration >= max_iterations`）、token 预算 `:817`、超时仅包住 LLM 调用 `:1307-1318`；循环检测在 `cycle_detector.rs`。

**但是**：

- 🔴 **`ReActConfig` 在 `src-tauri/src` 零引用** —— 无任何 settings / 配置注入路径 → **数值全部硬编码，用户不可调**。这正是铁律 #6「可配置 ≠ 有 UI 入口 ≠ 有入边供给」。
- 🔴 **声明的参数无消费点**：`AgentExecuteRequest.max_steps: Option<u32>`（`harness/src/agent.rs:22-23`）**被忽略** —— `harness_adapter.rs:107-165` 的 execute 只用了 `req.goal`。
- 🔴 **无 goal 持久化**：`crates/entities/src` 无 `goal*.rs`；`grep 'GoalStatus' / table_name="goals"` 无命中（`goal_status` 只是 `agent/src/reflector.rs:410` 的局部变量）。

### 3.3 🔴 `GoalEvaluator` 是 fail-open：连续未达成时**强制判定为达成**

`crates/agent/src/goal_evaluator.rs:85-95`：连续未达成 ≥ `max_not_achieved`（默认 3，`:158-162`）时**强制 `achieved=true`**「强制进入综合阶段」。

这是**放行而非停闸**——语义与「停止闸」完全相反。且属于铁律 #5 的「同向恒定」形态：无论实际达成与否，连续 3 轮后恒定输出 `achieved=true` ⇒ **该信号在长任务后段失去判别力**。

### 3.4 🟠 统一账本缺失（与铁律 #6 同源）

- **7+ 张带 `status` 的表**：`plans`、`workflow_executions`、`background_tasks`、`workflow_approvals`、`tool_executions`、`index_jobs`、`opc_work_items`、`opc_follow_up_tasks`
- **≥10 套独立状态枚举**：`harness/src/plan_types.rs:97 PlanStatus / :107 PhaseStatus / :116 TaskStatus`、`harness/src/types/session_state.rs:131 TaskStatus / :208 StepStatus`、`agent/src/task.rs:40 AgentTaskStatus`、`rt-workflow/src/task_contract.rs:109 ContractStatus`、`rt-workflow/.../fsm_manager.rs:26 FsmInstanceStatus`、`runtime/src/task_registry.rs:16 RegistryTaskStatus`、`runtime/src/swarm/team_helpers.rs:134 SwarmTaskStatus`、`runtime/src/tasks/{remote_agent_task.rs:54, in_process_teammate_task.rs:16, dream_task.rs:70}`、`gateway/src/handlers/runs.rs:41 RunStatus`
- **无统一任务实体**：`entities/src` 无 `tasks.rs`；Rust 侧 `parent_task_id` / `unified_task` / `task_ledger` / `awaiting_close`（含「待闭环/待验收」）**0 命中**；`acceptance_status` 只出现在专家 md 文档里

---

## 4. EvoFlow 侧病历（声称 vs 代码事实）

| 机制           | 声称                               | 代码事实                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | 判定                                                                                                                         |
| -------------- | ---------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| **架构边界**   | `evoflow.*` 不得 import `app.*`    | 仅 2 处 md + 1 个 wiki 生成器；`ruff.toml:5` 无 TID；`make check`（`scripts/check.py`）只检测依赖是否安装；CI 只有 3 个 release workflow；实际 27 文件 60 处违规（多数是函数内延迟 import）                                                                                                                                                                                                                                                                                                                                                                                                                                                             | ❌ 口号                                                                                                                      |
| **Guardrail**  | 安全护栏，fail_closed              | `config/guardrails_config.py:24 enabled=False`；`provider` 为 None 时中间件不注册（`middlewares/tool_error_handling_middleware.py:633`）；策略全局单例、无会话隔离（`guardrails/middleware.py:30-38`）；`config.example.yaml:248-249` 注释称桌面默认开启，**与代码相反**                                                                                                                                                                                                                                                                                                                                                                                | ❌ 默认关闭 + 文档说谎                                                                                                       |
| **Plan 授权**  | 计划授权闭环                       | 三层拦截：输入侧剥工具 schema（`plan_guard_middleware.py:1872`）、输出侧注 error ToolMessage（`:1951`）、执行侧合成错误（`:832`）。授权位 `persistence/schema.py:517 execution_authorized INTEGER DEFAULT 0`；写入白名单 `authorize_execution.py:16 = {user,system,api,page,ui,automation}`（**不含 model**）；改计划撤权（`:261`）                                                                                                                                                                                                                                                                                                                     | ✅ 设计对 / ❌ 实现 fail-open（`:1659 except Exception: return request` 等多处 bare except）+ 懒授权文本正则（`:1573-1660`） |
| **Goal 双闸**  | `max_steps` + `auto_stop` 先到先停 | 默认值**三处不一致**：`goal/goal.py:41`=1000、`auto_continue:243`=50、`goal_service.py:1712`=8；`auto_stop_minutes` 默认 **0=关闭**（`:284-312` 仅 `>0` 生效）；**`auto_continue_middleware.py:357-372` 遇 `tool_calls` 直接 `return None`** → `turn_no` 不增（`_count_goal_turns:56-76` 只数判定器干预）→ **max_steps 永不触发**；`loop_detection_middleware.py` 的 explore/streak/error 三类拦截被**显式改成 `return None`**（`:672-675, :680-686, :769-771`）                                                                                                                                                                                        | ❌ 默认无有效上限 + 结构性可绕过 + 护栏被主动关闭                                                                            |
| **统一账本**   | 三源一表                           | 确实同表 `evoflow_collab_tasks`（`persistence/schema.py:553-574`）且 `collab/task_source.py:16-18` 定义 `chat\|workflow\|role`；但 `source`/`parent_task_id`/`handlers` 全塞 `extra_json`（`persistence/task_row_mappers.py:21-52 _TASK_KNOWN` 不含它们）→ **无列约束无索引**；同域仍有 `a2a_tasks`/`app_runs`/`mission_state`/`person_commitments`/`task_details`/内存 cron 状态机等并行表；`awaiting_close` **无自动推进**（`tools/builtins/supervisor_tool.py:141-188` 12 态迁移表），靠人工 `set_task_state` 或 3 天僵尸回收（`collab/task_reclaim.py:75-136`）；**取消不级联**到 `parent_task_id` 关联的独立子任务（`routers/tasks.py:1905-1957`） | ⚠️ 「中统一」，缺陷明确                                                                                                      |
| **记忆**       | 防抖 + 原子写                      | 防抖默认 **120s**（`config/memory_config.py:86-91`，非 30s）；队列=纯内存 `threading.Timer` + list、`daemon=True`、**无持久化 ⇒ 进程退出即丢**（`memory/queue.py:38-41,110`）；Markdown 镜像 tmp+replace 但 **tmp 名固定 `<file>.tmp`、无文件锁**（`assets/memory_mirror.py:45-49`）；各阶段独立 try/except 只打日志（`queue.py:168,185,218,248,267`）                                                                                                                                                                                                                                                                                                  | ⚠️ 有明确丢数据窗口                                                                                                          |
| **注入策略**   | ≤15 条 ≤2000 token                 | 实际 full=10 / chat_compact=**5** 条（`memory/prompt.py:273,350`），token 2000                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          | ❌ 话术不符                                                                                                                  |
| **craft→技能** | 经验自动晋升技能                   | **只手动**，唯一入口 `POST /craft/promote`（`routers/assets.py:442-459` → `assets/promote.py:49`），全仓仅此调用点；且该写入**非原子**（`promote.py:46` 直接 `write_text`）                                                                                                                                                                                                                                                                                                                                                                                                                                                                             | ❌ 话术夸大                                                                                                                  |
| **用户画像**   | 画像抽取                           | **无 LLM 抽取器**，靠提示词强制模型写入（`assets/user_profile_dims.py:322-465`）；**无版本号、无冲突解决**                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | ⚠️ 机制弱                                                                                                                    |
| **沙箱**       | 隔离执行                           | 桌面默认 `sandbox/noop.py:45 NoopSandboxProvider` **直接 raise = 无隔离**；AIO 容器真加固（`local_backend.py:236-243,264-265`：`--read-only`/`--cap-drop ALL`/`--security-opt no-new-privileges`/`--tmpfs`）；`replicas=3` 是**软上限**（只淘汰 warm pool，`_evict_oldest_warm:471-489`）；AIO 侧写白名单**不解 symlink**（`aio_sandbox.py:59-60` 注释自承）→ 理论逃逸；`allow_host_bash` 已 `deprecated=True` **dead config**（`config/sandbox_config.py:36-40`）                                                                                                                                                                                      | ⚠️ 容器路径真做，默认路径无隔离                                                                                              |

---

## 5. AxInvest 强于 EvoFlow 的地方（不要妄自菲薄）

1. **架构边界已机械化强制，且强于 EvoFlow**：`scripts/check-contracts.mjs:177-198`（段 D）+ `.github/workflows/ci.yml:98` 实跑；`crates/harness/Cargo.toml` **零 axagent-\* 依赖**（`use axagent_*` 仅 1 处，在文档注释 `provider.rs:308`）→ 边界在结构层面不可能被违反。**EvoFlow 在这一条上是反例（60 处违规），不能作为参考。**
2. **沙箱是真实三平台隔离**：Linux `unshare` 命名空间 / Windows JobObject / macOS `sandbox-exec`（`runtime-core/src/sandbox.rs:196`），而非「桌面默认无隔离」。
3. **可观测零件齐全**：`crates/telemetry`（tracer/span/metrics/otel）+ `session_token_ledger.rs` + Trace UI。
4. **自我进化零件远超**：`trajectory/` 下 dream_consolidation / rl / skill_evolution / learning_graph / coevolution / behavior_learner / user_profile，对比 EvoFlow 的 4 类资产实体。

---

## 6. 不确定项（未验证，不要当结论用）

1. `plan_execute` 是否可被 LLM 间接作为工具调用（`agent_command` → tool 暴露路径未追）。
2. 生产 chat 主循环是否真走 `ReActEngine`：`HarnessAgentAdapter` 走 `ReActEngine::new()`（默认 50），但 `agent_query` 的 turn 循环在 `session_manager`，**可能存在另一套未核对的限制**。→ §3.2 的「有闸」结论需在生产路径上复核。
3. EvoFlow 的 `.github/workflows/` 中被引用的 `lint-check.yml` / `backend-unit-tests.yml` / `docs.yml` 在公开快照中不存在（公仓未同步），**无法确认其 CI 是否真的跑边界检查**——但反向 grep 已证明 60 处违规长期共存，故「无强制」结论成立。
4. EvoFlow 代码索引底层实现（tree-sitter / LSP 未验证）；费用账本与多账号/SSO 表结构未文档化。

---

## 7. 结论

1. **EvoFlow 不是范本，是同类病友**。「它把零件焊成了闭环」这个前提**不成立**——它的旗舰机制（Plan 授权、Goal 长任务）落地是 fail-open / 默认关闭 / 结构性可绕过的。
2. **唯一值得借鉴的是它的设计意图，而不是它的实现**。具体只有一条它真做对了：**把「执行授权」做成后端一个模型不可写的状态位，且该状态位是工具可见性的输入**。
3. **AxInvest 的真问题不在「缺功能」，在「入边缺失」** —— 声明了却没接线（`max_steps` 无消费点、`ReActConfig` 零引用、`requirePlanApproval` 被 serde 丢弃、`agent_plan_approvals` 无 insert、`PlanReadyForApproval` 无发射点、plan 状态集合不相交导致执行必拒）。
4. **AxInvest 已有一条正确的解题路径**：`check-contracts.mjs` 证明「架构约束可以机械化强制」。**扩展它去做「入边供给检查」，是本轮最高 ROI 的动作**——把铁律 #6 从「人肉检法」变成「CI 门禁」。

→ 落地方案见 `PLAN-evoflow-borrowings.md`。
