# ADR-P3 — P3 架构决策记录

> 状态：accepted（P3 范围内；Q1 用户翻案，Q2..Q8 按建议答案）｜ 日期：2026-11-23 ｜ 基线：`2e8db65d6` ｜ 前置：`ADR.md`（P1 001..009）、`ADR-P2.md`、`ADR-P2B.md`（编号顺延）
>
> 每条决策附「理由 / 后果 / 反转成本」，供用户走 Q 列表替代路径时定位影响面。

## ADR-P3-1 — 编排 = 定义 + 运行两实体，步骤复用 `bot_assignments`

**决策**：新增 `bot_workflows`（DAG 定义）与 `bot_workflow_runs`（运行实例）；每一步是一张既有 `bot_assignments` 行，用新列 `workflow_run_id`/`step_key` 关联（MODEL-P3 §2）。不新建步骤表、不新建执行器、不新建会话池。

**理由**：P2-b 的 assignment 已携带 root/parent/depth、origin、状态机、300s 超时、回执与事件，是现成的「一次 Bot 工作」合同。新建步骤执行表会复制这套状态机并造成两套错误码/事件/通知口径；而 DAG 与运行进度是 assignment 不具备的新概念，必须另有实体承载（GAP-P3 G-P1/G-P3）。

**后果**：步骤列表存 `steps_json`（沿用 `cron_jobs.data` 整行 JSON 演进先例）；步骤级状态只有 assignment 五态，run 级另有 waiting；`origin='workflow'` 扩展 DTO 联合类型但不改列类型。

**反转成本**：若改为步骤表，需搬迁 `steps_json`→行、双状态机对齐，约 3 人日；本冻结稿不建议。

## ADR-P3-2 — P3 翻 P2-b Q1 案：放开多层委派 depth≤3（运行时动态树）

> **裁决变更**：用户已拍板选 Q1 替代路径。P2-b「委派恒一层」（ADR-P2B-5 / PLAN-P2B Q1）自本版起 **superseded**；历史 P2-b 规格保留为当时冻结记录，实现验收以本节为准。Q2..Q8 不变。

**决策**：P3 放开多层委派，`bot_assignments.root_id/parent_id/depth` 三列从「预留字段」升级为**运行期动态树边**；深度上限 `MAX_DEPTH=3`。两种图边界并存且互不替代：
- **静态编排 DAG**（`steps_json`）：定义期固定步骤 Bot 与 `dependsOn`，拓扑排序驱动 run；每个步骤是一棵委派树的根（`depth=1 / parent_id=NULL / root_id=自身 id`）。
- **动态委派树**（root/parent/depth）：assignee 轮内由模型通过 `delegate_to_bot` 运行时决策下游 Bot，形成树；允许编排步骤内再多层委派，但子树的根就是该步骤 assignment，`workflow_run_id`/`step_key` 由父行继承给所有子孙，run 能按 root/group 汇总。

**理由**：用户明确翻案，要求映射 OMB `runtime-contract.md` 的委派链语义（P1 GAP §2.4）；P2-b 的字段预留正是为此。静态 DAG 负责「可预期的顺序/依赖」，动态树负责「运行时发现并分派」，二者输入源不同，合并成一张图会让定义期校验失效。

**后果与五条反转清单落地口径**：

1. **工具注册与深度取值**：根派发（person / routine / 编排步骤 / G5 之外的 Bot 委派入口）恒 `depth=1`、`parent_id=NULL`、`root_id=id`；子派发从 `depth=d` 的 assignee 轮发起时为 `depth=d+1`，取值域 1..3。`bot_routine_runner.rs::build_turn_tool_registry` 从 `bool` 改为按轮次+深度判定：`depth < MAX_DEPTH(=3)` 的 assignee 轮**出示** `delegate_to_bot`（含 `mode=consult|dispatch`）；`depth==3` 的轮次**不注册/显式禁用**该工具；`Agent`/`RemoteTrigger` 仍恒禁用。内部深度上下文由 `run_bot_non_interactive_turn` 经 task-local 注入，不进工具 JSON schema，防模型伪造。
2. **parent/depth/root 写入**：`create_assignment` 增 `parent_assignment_id`（当前 assignee 的 assignment id）与 `root_assignment_id`（当前轮 root_id）；新行 `depth=parent.depth+1`、`parent_id=parent.id`、`root_id=parent.root_id`；根派发仍按旧逻辑自链。`workflow_run_id`/`step_key` 从 parent 继承。root_id 恒指链根，不随深度改变；`root_id`/`parent_id` 加索引支撑链查询。
3. **防环与深度门**：每次派发前按 `root_id` 查整链 assignment，构造 ancestor Bot 集合；`assignee_bot_id` 已在祖先集合 → `BOT_DELEGATE_CYCLE`（新增码）；`parent.depth+1 > 3` → `BOT_DELEGATE_MAX_DEPTH`（新增码）；两者在工具层转成 LLM 可读错误文本，同时在审计行落 error_code。原 `BOT_DELEGATE_SELF` 只管直接自派发，语义不变；P2-b 的 depth=1 工具禁用不再是环防护，改由本条承担。`steps_json` 定义期 `validate_dag` 仍管静态环（用 `BOT_WORKFLOW_CYCLE`），两套环检查独立且都必须存在。
4. **逐层预算**：`per(root_id, depth) 在途 ≤2`；P2-b 既有「单 requester≤4 / 全局≤8」继续生效，超限时按既有 `BOT_DELEGATE_BUSY` 返回。编排 `maxConcurrency` 是**另一套独立预算**，只限制 runner 同时放行的就绪步骤数（默认 2）；两套预算叠加而非共用：步骤作为 depth=1 根同时占用链预算与全局门，子 assignment 只占链预算与全局门。每棵根链总超时沿用 300s（根到子共享一个 `tokio::time::timeout` 预算，非每层重置）。
5. **回执与汇总**：**不建第二套线程**——ADR-P3-4 的「单线程 + parent=anchor 扁平分组」继续成立；所有子孙回执仍追加同一 `requester_conversation_id` 线程，`parent_message_id` 仍指 run/根 anchor，靠正文前缀 `[L{depth} · {stepKey} · {botName}]` 表达树中位置。树形关系由 `bot_assignments.root_id/parent_id/depth` 字段承载，`list_bot_assignments(rootId)` 过滤后按 root 汇总给前端；不新增 `receipt_message_id` 列、不新增消息段类型。`bot-assignment-updated` 与按 assignee 的通知对子孙**同样复用**（每条子孙各发一次，不新增事件族）；run 完成条件改为「所有 `workflow_run_id=run.id` 的 assignment 终态」，即等待整棵子树，而不是只等根步骤 completed。

**反转成本（回到 P2-b 一层案）**：删掉工具注册的深度放行与 `parent/depth` 写入两个改动即可恢复；新增两码可保留不用。估 ≤1 人日，但需重跑 T3/T7 的链测试。

## 继承修订清单（P2-b 既有 DoD / 台账，实现线必须同步）

| 位置 | 原口径 | 修订后口径 |
|---|---|---|
| `src-tauri/src/bot_routine_runner.rs:141` `build_turn_tool_registry(assignee_round: bool)` 及 `p2_bot_routine_tests::assignee_round_tool_table_excludes_delegate` | assignee 轮一律禁 `delegate_to_bot`，断言「深度=1 防环」 | 改按 depth 判定：depth=1/2 有工具、depth=3 无工具；`Agent`/`RemoteTrigger` 恒禁；测试改名 `delegate_tool_respects_depth_cap` |
| `PLAN-P2B.md` T3'' 测试⑤ / DoD | 「assignee 轮工具表无 `delegate_to_bot`（深度=1 防环）」 | 改为「depth=3 无工具；depth<3 有工具且携带 task-local 上下文」；历史文档保留原文，以本节为准 |
| `IPC-P2B.md` §4 六码 / `PLAN-P2B.md` T4'' | 只有 P2-b 六码 | 六码不动，P3 追加 `BOT_DELEGATE_CYCLE`/`BOT_DELEGATE_MAX_DEPTH` 两码 |
| `src-tauri/tests/bot_assignment.rs` 四个既有测试 | 单笔 dispatch 状态机 | 断言不动，另加 `src-tauri/tests/bot_delegation_chain.rs` 覆盖链 |
| `e2e/bot-delegation.spec.ts`（P2-b 2 例） | 一层委派 e2e | 原 2 例不动；新增 `e2e/bot-delegation-chain.spec.ts` 覆盖 depth/cycle/预算/子树等待 |
| `P2/ACCEPTANCE-P2B.md`（历史台账） | 记录当时一层实现 | 不改历史记录；`ACCEPTANCE-P3.md` 中登记「P2-b 一层断言自 P3 起 superseded」及新测试命令 |

## ADR-P3-3 — 编排归属实例，部门只做可见性筛选与成员校验

**决策**：`bot_workflows.owner_scope` 可为 `instance` 或 `department`；department 表示「此编排仅可使用该部门成员且在该部门的视图里可见」，**不是**资产所有权。删部门时编排转 `instance`（清 `department_id`）保留；运行时复查成员资格，失效 → `BOT_WORKFLOW_MEMBER_CHANGED`。Chief Bot 在 P3 仍只是标注，编排不赋它运行时特权。

**理由**：编排是自动化资产，典型场景「盘前跨部门汇总」本身就跨部门；绑死部门会让跨部门编排无处安放，也让删部门误删自动化。P1 `conversations.bot_id` 的单owner语义和 P2-b 部门表的组织语义都不承载「多 Bot 资产的归属」，故新字段表达更准确（GAP-P3 G-P7/G-P8）。

**后果**：前端编排列表按部门筛选时，`instance` 编排始终可见（可加「实例级」徽章）；部门成员变更后已有编排定义保留但运行会失败，需人工改步骤 Bot 或改归属。

**反转成本（Q2 替代路径）**：改为部门所有（唯一 owner）只需 ① `owner_scope` 固定 `department`；② 列表过滤强制按当前部门；③ 删部门级联删定义与 run；改动集中在命令层与前端筛选，约 1 人日。

## ADR-P3-4 — 单一运行线程 + 锚点分组 + 按步骤 Bot 通知

**决策**：一次 run 对应一个 `requester_conversation_id`：手工触发用前端当前线程；cron 触发用新建的持久线程（`create_conversation_with_defaults(..., bot_id = 部门 Chief 或 None)`）。run 开始写一条 role=System 锚点消息，每步回执 role=Assistant、正文前缀 `[步骤 {key} · {botName}]`、`parent_message_id=anchor`。通知按步骤 assignee 的 `Notification.botId` 写既有管线，run 级不再发通知。

**理由**：多 Bot 产物分散到各 Bot 线程会让用户失去「这一次编排说了什么」的整体视图；新增消息段类型/新通知中心都要动 P1 已锁的线程语义。锚点 + parent 用现成 `message::create_message` 参数即可表达分组（GAP-P3 G-P9/G-P10）。

**多层补充（P3 翻 Q1 后）**：动态委派树的子孙回执不另起线程，仍落同一 run 线程、parent 仍指根 anchor；树形位置用 `[L{depth} · {stepKey} · {botName}]` 前缀表达，关系数据留在 `bot_assignments.root_id/parent_id/depth`。前端按 `rootId` 过滤形成「委派链」视图，不再需要「一层不回归」类断言（ADR-P3-2 五条）。

**后果**：线程 owner 只能是单个 Bot（或 NULL）；`conversations.bot_id` 按 P1 语义填 Chief Bot / NULL，不表达多 Bot 归属——多 Bot 归属由 `runs` 表承担。`parent_message_id` 的 UI 折叠行为沿用现有消息渲染，前端只加前缀样式。

**反转成本（Q4 替代路径）**：每步独立线程只需 run 记录 `threadIds[]`（加列或 context_json）并改回执落点；前端需加线程间跳转，约 2 人日。

## ADR-P3-5 — cron 复用调度器，`bot_workflow` 分支与 `bot_routine` 分支并列

**决策**：编排定时触发新增 `task_type="bot_workflow"` + `params.workflowId`，在 `src-tauri/src/init/services.rs` 的 Bot 分支**之前**加一条新分支；30s tick、leader 租约、在途去重、时区计算、`cron_job_history`、暂停/恢复全部复用。`create_bot_workflow_schedule` 只是 `create_scheduled_task` 的薄封装。

**理由**：调度器是 AxAgent 已验证能力（GAP-P2 G-4/G-6/G-7）；新调度线程会重新引入在途去重与跨进程租约问题。编排与单 Bot 例程的差异在「到点后跑什么」，是分支级差异（GAP-P3 G-P12）。

**后果**：`task_type=bot_workflow` 与 `bot_id` 必须互斥，命令层创建时 `bot_id=None`；若历史脏数据两者并存，以 `task_type` 为准并记 warn（与 P2-a `workflow_id` 冲突处理同款）。手工触发不经过 cron，直接走 run 命令。

**反转成本（Q3 替代路径）**：「合并进 bot_routine」要扩展 `CronJob.bot_id` 为多值或新增 `workflow_id` 字段并改 P2-a 分支内部；约 2 人日且回归面更大，不建议。

## ADR-P3-6 — 失败可见、不自动事故线程与重试

**决策**：步骤失败按 `onFailure` 处理：`abort` 取消其余在途/排队步骤并把 run 置 failed（error_code 继承步骤码）；`continue` 跳过失败步骤的直接/间接下游（`context_json.skipped`），独立分支继续，最终 run 仍为 failed。失败结果写 run 线程 + 按 assignee 通知。OMB 的「待处理问题线程 + ≤2 重试」**不进 P3**。

**理由**：自动重试会重复副作用（写文件/交易类工具），而 P2-b 失败已能被前端看见；先把可见性与依赖传播做对，再谈自动事故处理。等待态只做依赖阻塞（run.status=waiting），不做 needs_input 交互阻塞（无人在场）。

**多层补充（P3 翻 Q1 后）**：`abort`/`cancel`/总超时必须覆盖整棵子树（`root_id` 相同且非终态的所有 assignment），不能只取消根步骤；子 assignment 的 error_code 继承规则与根一致。run 终态条件 = 该 run 下所有 assignment 终态（ADR-P3-2 ⑤）。

**后果**：`onFailure=continue` 下 run.failed 且部分步骤 completed，前端需展示 skipped 列表；用户可手工重跑整个 run（新 run id，不复用旧 outputs）。

**反转成本**：补自动事故线程需要「收口 Bot」概念与线程创建/重试计数（≈2 人日），列 P4 候选。

## ADR-P3-7 — 最小变量替换，不做通用模板引擎

**决策**：promptTemplate 只识别 `{{steps.<stepKey>.output}}`（以及 inputs 的 `{{inputs.<key>}}`），在派发前一次性字符串替换；未知 key 在定义期报错，输出超过 20k 字符截断。不引入模板引擎、不解析表达式、不写文件。

**理由**：编排的 80% 需求是「把上一步结果交给下一步」；通用模板带来注入面与测试面膨胀，而 assignment 的 result_text 已是唯一可信输出源（GAP-P3 G-P14）。

**后果**：无法做条件分支/循环；需要条件逻辑时拆成多个编排由外部决定触发。反转成本：接入既有模板库 + 沙箱 ≈3 人日，不建议 P3 做。
