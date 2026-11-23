# ADR-P3 — P3 架构决策记录

> 状态：accepted（P3 范围内，Q 列表按建议答案）｜ 日期：2026-11-23 ｜ 基线：`2e8db65d6` ｜ 前置：`ADR.md`（P1 001..009）、`ADR-P2.md`、`ADR-P2B.md`（编号顺延）
>
> 每条决策附「理由 / 后果 / 反转成本」，供用户走 Q 列表替代路径时定位影响面。

## ADR-P3-1 — 编排 = 定义 + 运行两实体，步骤复用 `bot_assignments`

**决策**：新增 `bot_workflows`（DAG 定义）与 `bot_workflow_runs`（运行实例）；每一步是一张既有 `bot_assignments` 行，用新列 `workflow_run_id`/`step_key` 关联（MODEL-P3 §2）。不新建步骤表、不新建执行器、不新建会话池。

**理由**：P2-b 的 assignment 已携带 root/parent/depth、origin、状态机、300s 超时、回执与事件，是现成的「一次 Bot 工作」合同。新建步骤执行表会复制这套状态机并造成两套错误码/事件/通知口径；而 DAG 与运行进度是 assignment 不具备的新概念，必须另有实体承载（GAP-P3 G-P1/G-P3）。

**后果**：步骤列表存 `steps_json`（沿用 `cron_jobs.data` 整行 JSON 演进先例）；步骤级状态只有 assignment 五态，run 级另有 waiting；`origin='workflow'` 扩展 DTO 联合类型但不改列类型。

**反转成本**：若改为步骤表，需搬迁 `steps_json`→行、双状态机对齐，约 3 人日；本冻结稿不建议。

## ADR-P3-2 — 维持委派恒一层，防环放在定义期 DAG 校验（P2-b Q1 裁决不翻案）

**决策**：P3 **不放开**多层自由委派 / 图编排。`bot_assignments` 仍 `depth==1 / parent_id=NULL / root_id=id`；编排步骤目标由定义在编写时固定，assignee 轮工具表仍禁 `delegate_to_bot`（ADR-P2B-5 不动）。环由定义期 `validate_dag`（拓扑排序 + stepKey 唯一 + 已知键引用）消除，运行时再加「每 run 每个 stepKey 最多派发一次」的 visited 集做双保险。

**理由**：P2-b 的一层限制让防环零成本；放开多层同时需要树形状态机、跨层预算、循环检测与回执链合并，是新的运行时课题，不是编排升级的必要前置。OMB 的委派链语义已被 P2-b 的「恒一层 + 字段先落」有意识地收窄，本轮无新证据推翻它（GAP-P3 G-P5）。

**后果**：用户想表达「A→B→C」时，必须写成三个显式步骤（B 的产物通过 `{{steps.<key>.output}}` 传给 C），而不是让 B 自行再委派。步骤数上限 12 使链长有界；跨层递归不可能发生。

**反转成本（Q1 替代路径）**：放开多层需 ① 工具注册条件改为按 `depth < MAX_DEPTH` 放行；② assignment 写 `parent_id`/递增 `depth`；③ 每次派发前查 root 链 visited 集与深度门（建议 cap=3）；④ 逐层预算（每层并发 ≤2、总在途仍服从 P2-b 全局 8）；⑤ 回执链改为按 root 汇总。估 4–6 人日 + 新 e2e，影响 T3/T4/T6 三卡。

## ADR-P3-3 — 编排归属实例，部门只做可见性筛选与成员校验

**决策**：`bot_workflows.owner_scope` 可为 `instance` 或 `department`；department 表示「此编排仅可使用该部门成员且在该部门的视图里可见」，**不是**资产所有权。删部门时编排转 `instance`（清 `department_id`）保留；运行时复查成员资格，失效 → `BOT_WORKFLOW_MEMBER_CHANGED`。Chief Bot 在 P3 仍只是标注，编排不赋它运行时特权。

**理由**：编排是自动化资产，典型场景「盘前跨部门汇总」本身就跨部门；绑死部门会让跨部门编排无处安放，也让删部门误删自动化。P1 `conversations.bot_id` 的单owner语义和 P2-b 部门表的组织语义都不承载「多 Bot 资产的归属」，故新字段表达更准确（GAP-P3 G-P7/G-P8）。

**后果**：前端编排列表按部门筛选时，`instance` 编排始终可见（可加「实例级」徽章）；部门成员变更后已有编排定义保留但运行会失败，需人工改步骤 Bot 或改归属。

**反转成本（Q2 替代路径）**：改为部门所有（唯一 owner）只需 ① `owner_scope` 固定 `department`；② 列表过滤强制按当前部门；③ 删部门级联删定义与 run；改动集中在命令层与前端筛选，约 1 人日。

## ADR-P3-4 — 单一运行线程 + 锚点分组 + 按步骤 Bot 通知

**决策**：一次 run 对应一个 `requester_conversation_id`：手工触发用前端当前线程；cron 触发用新建的持久线程（`create_conversation_with_defaults(..., bot_id = 部门 Chief 或 None)`）。run 开始写一条 role=System 锚点消息，每步回执 role=Assistant、正文前缀 `[步骤 {key} · {botName}]`、`parent_message_id=anchor`。通知按步骤 assignee 的 `Notification.botId` 写既有管线，run 级不再发通知。

**理由**：多 Bot 产物分散到各 Bot 线程会让用户失去「这一次编排说了什么」的整体视图；新增消息段类型/新通知中心都要动 P1 已锁的线程语义。锚点 + parent 用现成 `message::create_message` 参数即可表达分组（GAP-P3 G-P9/G-P10）。

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

**后果**：`onFailure=continue` 下 run.failed 且部分步骤 completed，前端需展示 skipped 列表；用户可手工重跑整个 run（新 run id，不复用旧 outputs）。

**反转成本**：补自动事故线程需要「收口 Bot」概念与线程创建/重试计数（≈2 人日），列 P4 候选。

## ADR-P3-7 — 最小变量替换，不做通用模板引擎

**决策**：promptTemplate 只识别 `{{steps.<stepKey>.output}}`（以及 inputs 的 `{{inputs.<key>}}`），在派发前一次性字符串替换；未知 key 在定义期报错，输出超过 20k 字符截断。不引入模板引擎、不解析表达式、不写文件。

**理由**：编排的 80% 需求是「把上一步结果交给下一步」；通用模板带来注入面与测试面膨胀，而 assignment 的 result_text 已是唯一可信输出源（GAP-P3 G-P14）。

**后果**：无法做条件分支/循环；需要条件逻辑时拆成多个编排由外部决定触发。反转成本：接入既有模板库 + 沙箱 ≈3 人日，不建议 P3 做。
