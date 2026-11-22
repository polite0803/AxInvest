# GAP-P2B — P2-b 差距分析（团队/部门 + 多 Bot 委派运行时）

> 状态：active ｜ 日期：2026-11-23 ｜ 基线：`ee6690c3a`（P2-a 已收口）｜ 前置：`GAP.md`（P1 §4 G-14/G-15）、`GAP-P2.md`
>
> 方法沿用 P1：对 OpenMausBot（OMB）语义逐条判定 A（已有可复用）/ B（需改造）/ C（缺失），并给出 P2-b 范围内外的边界。

## 1. OMB 语义抽取（对照 `runtime-contract.md` / `team-sharing.md` / 本 app 实测）

| OMB 语义 | 要点 | 来源 |
|---|---|---|
| 团队/部门 | Bot 可分组（departments）；部门可设 Chief（Bot，原义 Chief of Staff：一人对外收口、统筹派工）；Bot 归属部门 | `team-sharing.md`、本 app 团队图谱 |
| 委派两种语义 | **consult**（问意见，从已有知识回答，不动手）与**派工**（执行并回报）；回执落回**发起 Bot 的线程**，收口人验证后合并/驳回 | `runtime-contract.md`（coordinate 语义） |
| Assignment 合同 | 统一记录：root/parent/depth、origin、requester、assignee、mode、reply、level、executions、result/evidence；状态机 queued→running→waiting/needs_input→completed/failed/cancelled | `runtime-contract.md` |
| 并行/预算 | 委派可并行但受预算与去重约束；被委派方以**自己的权限档**运行（外部不升权） | `runtime-contract.md`、`approval-levels.md` |
| 事故上报 | 被委派方 run 失败时上报给收口 Bot 的「待处理问题」线程，重试 ≤2 次 | `team-sharing.md`、`team-incidents.md` |
| 共享模板 | 部门共享白名单：不含聊天、记忆、密钥、模型、审批级别、目录 | `team-sharing.md` |

## 2. AxAgent 现状盘点（P2-b 视角，逐条判定）

| # | 能力 | OMB 对应 | AxAgent 现状 | 判定 | 证据 | P2-b 动作 |
|---|---|---|---|---|---|---|
| G-B1 | Bot 分组 | departments | `bots.section`（自由 TEXT，侧栏简单分组，P1 已建）；无组织语义（无 Chief/归属关系） | **B** | `bots.rs:28`、`bot.rs` | 部门是组织语义，与 section 并存（ADR-P2B-1）；不迁移 section |
| G-B2 | Chief 角色 | Chief of Staff | 无。最接近的是 `agent_roles.reports_to`（G5 岗位树，绑 profile 不绑 Bot） | **C** | `agent_roles.rs:36` | `bot_departments.chief_bot_id` 新增 |
| G-B3 | 成员归属 | 部门成员 | `fleet_members` 绑 **AgentSession + agent_slug**（办公室场景），不绑 Bot | **A'（模型不同形）** | `fleet_members.rs` | 不复用 fleets（ADR-P2B-1）；新 join 表 |
| G-B4 | 委派-固定角色 | — | `delegate_task`：仅 G5 三角色（analyst/implementer/reviewer），一次性 LLM 补全，无记录无状态机 | B（但**不改造**：它是 G5 工具，Bot 委派另起） | `multi_agent.rs:183-215` | 保留不动；Bot 委派走新命令族 |
| G-B5 | 委派-意图路由 | 派工 | `fleet_dispatch`：LLM 意图路由到 fleet 成员并真实执行（executor.rs），但成员是 session 不是 Bot | B（执行器**结构**可借鉴） | `fleet/executor.rs` | Bot 委派执行器与 P2-a 例程分支同构（ADR-P2B-2），不引入 fleet executor |
| G-B6 | Assignment 记录 | runtime.db | **无任何委派留痕**（delegate_task 结果即焚） | **C** | GAP-15 | `bot_assignments` 新表（MODEL-P2B §2） |
| G-B7 | 状态机 | queued→…→cancelled | 无 | **C** | 同上 | P2-b 落 queued/running/completed/failed/cancelled 五态（waiting/needs_input 留 P3，P2-b 无交互阻塞场景） |
| G-B8 | 非 Bot 执行的 headless 轮 | — | **P2-a 已建**：例程分支（临时会话 + prompter=None + Bot 档） | **A** | `init/services.rs` bot_id 分支、`bot_context.rs`、`bot_approval.rs` | 委派执行器直接复用三件积木，不新写 |
| G-B9 | 外部不升权 | level 不传递 | `resolve_effective_permission` 只降不升护栏已在本；`resolve_bot_approval_level` 非交互只取 Bot 档 | **A** | ADR-P2-1/2-4 | 委派沿非交互路径（第 2 条），天然不升权；补一条 ADR-P2B-3 把语义写死 |
| G-B10 | 回执落发起线程 | reply 落回 | 消息 append 到 conversation 的 DAO 已有（P1 线程归属）；无「委派结果」段类型 | **B** | `dao/repo/conversation.rs` | consult 同步返回 + 落 thread（Q3）；dispatch 落 thread + Bot 通知 |
| G-B11 | 并行限制/超时 | 预算与去重 | fleet 成员有 `max_concurrent/timeout_seconds`（agent_roles 层）；委派层无 | **C** | `agent_roles.rs:26-27` | 常量起步：requester 并发 4 / 全局 8 / 超时 300s（Q5，可后调） |
| G-B12 | 事故上报 | incidents | 无 | **C** | — | 留 P3（P2-b 失败即 failed + error_code，前端可见即够） |
| G-B13 | 共享模板白名单 | preset | 无 preset（GAP-17 判 C，P2-c） | **C** | GAP-17 | 留 P2-c；部门表设计不设障碍（chief/成员已可支撑白名单校验） |
| G-B14 | rooms/群聊 | 团队房间 | `fleet_messages` 群聊/DM + 水位协调门（办公室场景） | **A'（不涉）** | `fleet_messages.rs` | P2-b 不做 Bot 群聊；委派是点对点 |
| G-B15 | 委派深度/环 | depth | 无 | **C** | — | 表留 root/parent/depth 字段；P2-b 恒 depth=1（Q1），环随一层限制天然消除 |

## 3. 判定汇总

- **A（直接复用）**：G-B8 headless 轮三积木、G-B9 只降不升与 Bot 档解析、G-B14 fleet 消息层（不涉）。
- **B（改造/接线）**：G-B1/G-B3（组织语义新表）、G-B4（不改造只并列）、G-B5（借鉴结构）、G-B10（回执落线程）。
- **C（P2-b 新建）**：G-B2/G-B6/G-B7/G-B11/G-B15；**留 P3/P2-c**：G-B12 事故、G-B13 共享模板、Bot 群聊。

## 4. 主线流程图（P2-b 落地后）

```text
person/Bot ─→ delegate_to_bot(mode)
                │ consult：requester turn 内同步执行（工具语义，等结果）
                │ dispatch：后台 task，立即返回 AssignmentDto(queued)
                ▼
        bot_assignments 落库(queued) → 并行门（requester ≤4 / 全局 ≤8）→ running
                ▼
        ADR-P2-2 同构执行器：assignee Bot 临时会话 + SOUL/MEMORY + Bot 档（非交互）
                ▼
        completed(result_text) / failed(error_code) / cancelled / timeout→failed
                ▼
        回执：append 发起线程 +（dispatch）Bot 维度通知； consult 结果同步返回给 requester 轮
```

## 5. 与 T3'（agent_query）接线的关系

- consult/dispatch 的 assignee 轮 = `agent_query` 等价路径的非交互调用（与例程分支同一封装），**不新增执行引擎**；
- requester 轮内 consult 需要把「委派给 Bot」暴露为工具（tools 层新工具，runner 注入模式照抄 `init_delegate_task_runner`），T3' 的 `resolve_bot_approval_level` 在 assignee 侧生效；
- requester 的档位不进 assignee 上下文（ADR-P2B-3），故与 T3' 无耦合改动——接线点只有工具注册。
