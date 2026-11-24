# GAP-P3 — P3 差距分析（多 Bot 协作 / 例程编排）

> 状态：active ｜ 日期：2026-11-23 ｜ 基线：`2e8db65d6`（P2-b 收口）｜ 前置：`GAP.md`、`GAP-P2.md`、`GAP-P2B.md`
>
> 方法沿用 P1/P2：A（已有可复用）/ B（需改造或接线）/ C（缺失需新建），逐条给「P3 做 / 不做 + 理由」。
> OMB 语义源：P1 `GAP.md` §2.4 与 P2 `GAP-P2B.md` §1 已从 OpenMausBot 的 `runtime-contract.md` / `team-sharing.md` / `routine-schedules.md` / `approval-levels.md` 抽取（取回日 2026-11-22）。本轮环境离线，未重新联网复核原文；以下引用均为已落盘规格中的逐字转述，不新增未经核实的 OMB 细节。

## 1. P2 尚未覆盖的 OMB 语义

| OMB 语义 | 要点（已抽取） | 来源（经 P1/P2 规格） |
|---|---|---|
| Assignment 合同 | root/parent/depth、origin、requester、assignee、mode、reply、level、executions、result/evidence；状态 queued→running→waiting/needs_input→completed/failed/cancelled | `runtime-contract.md`（GAP.md §2.4） |
| 多 Bot 顺序/依赖协作 | 委派链与部门 Chief 收口；产物回执发起线程，收口人验证后合并/驳回 | `runtime-contract.md`、`team-sharing.md`（GAP-P2B §1） |
| 事故上报 | 被委派方 run 失败上报收口 Bot 的「待处理问题」线程，重试 ≤2 次 | `team-sharing.md`、`team-incidents.md`（GAP-P2B §1，P2-b G-B12 留 P3） |
| 例程唤醒 | 五字段 cron + IANA 时区，调度器到点唤醒；删 Bot 连带删其 schedules/runs | `routine-schedules.md`（GAP-P2 §2 G-1/G-10） |
| 权限边界 | 被委派方以自己权限档运行；「only copies its bot's level」 | `approval-levels.md`、`routine-schedules.md`（ADR-P2B-3） |
| 共享白名单 | 部门共享包不含聊天、记忆、密钥、模型、审批级别、目录 | `team-sharing.md`（P2-b G-B13 留 P2-c） |

## 2. AxAgent 现状逐条判定

| # | 能力 | AxAgent 现状（实测） | 判定 | P3 做/不做 | 理由 |
|---|---|---|---|---|---|
| G-P1 | 多 Bot 编排实体 | 无。`cron_jobs` 是单 Bot 单 prompt（`job.bot_id`），`bot_assignments` 是单对单委派；无「步骤列表 + 依赖边」 | **C** | **做**：新增 `bot_workflows`（定义）+ `bot_workflow_runs`（运行），步骤存 `steps_json` | 依赖边是编排的本质，不能塞进 CronJob 单个 prompt；JSON 步骤表沿用 CronJob 整行演进先例（GAP-P2 G-1） |
| G-P2 | 依赖顺序调度 | 无 DAG/拓扑排序；`cron` 调度器只做「到点派一个」 | **C** | **做**：runner 内 Kahn 拓扑 + 就绪集并发门 | 不能改 cron 调度器（30s tick/leader 租约/在途去重已验证），只在 task_type 分支内编排 |
| G-P3 | 委派执行器 | P2-b 已有 `bot_assignment_runner::run_assignment_with_timeout`（状态机/300s/事件/回执）与共享 `run_bot_non_interactive_turn` | **A** | **复用，不重建** | 编排的每一步就是一张 assignment；只加完成钩子驱动下一层，不写第二个模型调用路径 |
| G-P4 | 执行上下文积木 | `bot_context.rs`（SOUL/MEMORY）+ `bot_approval.rs`（只取 Bot 档）+ 占位会话清理，例程与委派共用 | **A** | **复用** | P2-a/P2-b 已锁行为，重造会产生审批与记忆注入的双真相源 |
| G-P5 | 委派深度 | `bot_assignments.root_id/parent_id/depth` 已落但恒 1；assignee 轮工具表禁 `delegate_to_bot`（ADR-P2B-5） | **B（接线改造，用户翻 Q1 案）** | **放开多层 depth≤3**：根 depth=1、子 depth+1、parent/root 写链；depth<3 出示委派工具，depth=3 禁用 | 用户已拍板翻掉 P2-b 一层案（ADR-P3-2）；字段是 P2-b 特意预留，运行时树与静态编排 DAG 各管一层语义 |
| G-P6 | waiting / needs_input | assignment 五态无 waiting；例程 ask 自动 Deny（prompter=None） | **B** | **只做 run 级 waiting**（依赖阻塞）；**不做 needs_input** | 编排是多 Bot 无人值守，needs_input 无回答者；等待依赖用 run.status=waiting 表达即可 |
| G-P7 | 部门归属 | `bot_departments` + `bot_department_members` 已有，一个 Bot 一部门；Chief 只是标注 | **A（需接线）** | **编排归属实例**，`department_id` 仅作可见性/成员校验（ADR-P3-3） | 编排是自动化资产，常跨部门；绑部门会让跨部门协作无处安放 |
| G-P8 | 部门成员校验 | `bot_department_members` DAO 有查询；编排定义无校验 | **B** | **做**：部门编排的每步 Bot 必须是当前成员，运行前复查 | 成员变更后运行要失败而不是静默越权；错误码 `BOT_WORKFLOW_MEMBER_CHANGED` |
| G-P9 | 单线程多 Bot 产物 | `message::create_message` 支持 `parent_message_id`；委派回执 append 到 `requester_conversation_id` | **B** | **做**：一个 run 一条线程，每步回执带 `[步骤 key · Bot]` 前缀并挂同一 anchor 消息 | 不新增消息表/段类型；线程唯一 owner 用 run 的 `requester_conversation_id` 表达（ADR-P3-4） |
| G-P10 | 通知 | P2-a `Notification.botId` + `agent-done` 推送链；P2-b 委派完成写 Bot 维度通知 | **A** | **复用** | 编排不建新通知通道；每步完成按 assigneeBotId 写既有通知 |
| G-P11 | 编排事件 | 仅 `bot-assignment-updated`；无 run 级事件 | **C** | **加 1 个**：`bot-workflow-run-updated` | 步骤事件已够细，run 级只需让前端刷新总状态与进度 |
| G-P12 | 例程触发编排 | `create_bot_scheduled_task` 固定 `task_type="bot_routine"`，`job.bot_id` 分支只跑单 Bot 轮 | **B** | **做**：新增 `bot_workflow` task_type 分支 + 一个 create 命令 | 复用 30s tick/时区/history/暂停，零新调度器；与 bot_id 分支互斥 |
| G-P13 | 失败/事故上报 | P2-b 失败仅落 `bot_assignments.error_code`；无「待处理问题」线程、无重试上限 | **C** | **部分做**：失败写 run 线程 + 通知 + `onFailure` 策略；**不做**自动事故线程与 ≤2 重试 | 自动重试会让成本与副作用翻倍；先做可见性，重试交人工/后续计划 |
| G-P14 | 输出传递 | 无 step 间变量；assignment 有 `result_text` | **C** | **做最小版**：`context_json.outputs[stepKey]` 可被下游 prompt 用 `{{steps.<key>.output}}` 引用 | 不做通用模板引擎；一次字符串替换 + 长度上限，足够表达「A 汇总→B 审稿」 |
| G-P15 | 预设/技能/Bot API | P2-c 清单（技能预设导入、Bot API key、记忆写入） | **C** | **不做**（保持 P2-c） | 与编排正交，本轮范围不吞 |
| G-P16 | Bot 群聊 / rooms | `fleet_messages` 是办公室场景，P2-b 已判不涉 | **A'（不涉）** | **不做** | 编排是任务图，不是聊天室 |
| G-P17 | 预算/并发 | P2-b 有单 requester 4 / 全局 8 / 300s；编排层无 | **B** | **加 run 级**：并发 ≤2（可配 1..4）、总超时 900s、步骤数 ≤12 | 与 P2-b 门叠加而非替代；防止一个编排吞掉全局 8 个委派名额 |
| G-P18 | 删除级联 | `delete_bot` 已清部门成员/chief/在途 assignment；编排定义引用 Bot 未纳入 | **B** | **做**：删 Bot 不删编排，运行时校验失败并给专用错误码 | 删部门同理：编排转实例归属保留（ADR-P3-3），自动化资产不随组织变更销毁 |
| G-P19 | 动态委派树 vs 静态编排 DAG | 静态 `steps_json` 依赖已冻结；运行期树（root/parent/depth）尚未启用 | **C** | **两套并存**：静态 DAG 驱动 run 步骤，动态树允许步骤内再委派；`workflow_run_id`/`step_key` 沿树继承，run 等整棵子树终态 | 两者输入源不同（定义期 vs 运行期），合并会让定义期校验失效（ADR-P3-2） |
| G-P20 | 多层防环/预算/回执 | 一层时防环退化为「无工具」，无逐层预算与树形汇总 | **C** | **做**：root 链 visited 集 + depth cap=3 + 新增 2 码；per(root,depth)≤2 与 P2-b 全局 8 叠加；回执仍单线程扁平、按 root 字段汇总 | OMB `runtime-contract.md` 的委派链语义落地；不新增线程/事件/消息类型 |

## 3. 判定汇总与 P3 边界

- **A（直接复用）**：G-P3 委派执行器、G-P4 三积木、G-P7 部门表、G-P10 通知管线、G-P16 不涉。
- **B（接线/改造）**：G-P6 run 级 waiting、G-P8 成员校验、G-P9 单线程回执、G-P12 cron 新分支、G-P17 预算、G-P18 删除语义。
- **C（新建）**：G-P1 两实体、G-P2 拓扑 runner、G-P11 run 事件、G-P14 最小变量替换。
- **P3 明确不做**：needs_input 交互阻塞、自动事故线程/自动重试、Bot 群聊、P2-c 三件事、新调度器/新会话池/新模型调用路径、图状任意回边（只放行无环 depth≤3 树）。

## 4. 落地后主线流程

```text
cron(task_type=bot_workflow, params.workflowId) ─┐
person 点「运行编排」(requesterConversationId) ───┼→ bot_workflow_runs(queued)
                                                  │      ↓ 定义期已校验 DAG / 部门成员
                                                  │  拓扑就绪集（run.status=waiting 表示有依赖未满足）
                                                  │      ↓ 每步：create_step_assignment（P2-b create_inner 复用，不自调度）
                                                  │      ↓ run_assignment（P2-b 状态机 + 300s + 事件 + 回执线程）
                                                  │      ↓ 完成钩子：写 context_json.outputs → 单线程 append 带前缀回执 → 通知 → 解锁下游
                                                  └→ run.status=completed / failed(onFailure=abort|continue) / cancelled / timeout
```

## 5. 复用边界（避免重造清单）

| 已有能力 | 复用方式 | 禁止重造 |
|---|---|---|
| `run_bot_non_interactive_turn`（`src-tauri/src/bot_routine_runner.rs:184`） | 每步 assignee 轮原样调用 | 第二个 headless 轮/第二套 SOUL 注入 |
| `run_assignment_with_timeout`（`bot_assignment_runner.rs`） | 抽 `run_assignment_inner` + 完成钩子，旧签名行为不变 | 新状态机、新 300s 计时、新事件发射点 |
| `commands::bot_assignment::create_inner` | 暴露 `create_step_assignment`（落库+并行门，**不调用 scheduler**） | 直接 INSERT assignment 绕过并行门与校验 |
| cron 30s tick / leader 租约 / history（`init/services.rs`） | 加 `task_type="bot_workflow"` 分支 | 新调度线程、新 tick、新时间解析 |
| `message::create_message`（`dao/repo/message.rs:143`） | 回执 role=Assistant、parent=anchor；起始锚点 role=System | 新消息表或新消息段类型 |
| `Notification.botId`（`src/lib/notification.ts`） | 每步完成按 assignee 写通知 | 新通知中心 |

> 各条落点文件、字段与命令契约见 `MODEL-P3.md` / `IPC-P3.md`；决策理由见 `ADR-P3.md`。
