# GAP-P2 — P2-a（审批执行 · Bot 例程 · 通知未读）差距分析

> 状态：active ｜ 日期：2026-11-23 ｜ 对应基线：`0f73ec54b`（P1 四提交后）｜ 来源：实地读码（2026-11-23）｜ 范围：`docs/bot-integration/P2/`
>
> 与 P1 `GAP.md` §6 的关系：那里只列了 8 个名字；本文对 P2-a 三项逐条**实地核码**（行号用于定位，落点以文件+函数名为准）。判定：**A=已有可复用（零改或仅透传）、B=需改造、C=缺失（需新增）**。

## 0. 一句话结论

三条链路的引擎全部现成：调度（`cron_jobs` JSON + `CronJobStore` + 30s tick 调度器）、审批（`PermissionPolicy` + `ChannelPermissionPrompter` + `agent_sessions.permission_mode`）、通知（`agent-done → pushNotification` 管线 + localStorage 通知中心）。P2-a 是**三处接线**：给 `CronJob` JSON 加 `botId`/`timezone` 两个字段、给审批加一张「Bot 五档 → AxAgent 运行档」映射与解析函数、给通知面加 `botId` 维度。**数据模型零 schema 变更**（详见 MODEL-P2）。

## 1. 主线：Bot 在非交互场景自己运行

三项共享同一条主线，单独做任何一项都跑不起来：

```text
cron 到点（scheduler 30s tick, leader 租约通过）
  → executor handler 识别 job.botId            ← 现状：无此分支（G-5）
  → 构造 Bot 会话上下文（复用 bot_context，P1 已建）
  → 审批级别解析：bot.approval_mode → PermissionMode   ← 现状：只有会话级三档（G-12/13）
  → agent_query 跑一轮（非交互 prompter：ask 级自动拒绝）
  → record_run 落 cron_job_history + last_result
  → agent-done 事件 → 前端通知中心 → Bot 未读角标      ← 现状：管线在，缺 botId 维度（G-22/23）
```

## 2. 例程与自动化（PLAN §6 第 1 项）

| # | 能力 | 现状（实测） | 判定 | 证据 |
|---|---|---|---|---|
| G-1 | cron 持久化 | `cron_jobs` 表只有 `id + data`（整行 JSON）。实体文档明说设计意图：`CronJob` 模型演进频繁，整行 JSON 使新增字段**无需配 migration**。加 `botId` = 纯 JSON 字段，**不建表、不动 schema_compliance**（该表本就不在 SCHEMA 清单里，实测 grep 为 0 条） | **A** | `axagent-core/crates/entities/src/cron_job.rs`（文件头注释即建表契约）；`axagent-infra/crates/dao/tests/schema_compliance.rs` |
| G-2 | `CronJob` 结构 | 现有 23 字段（schedule/prompt/workflow_id/task_type/status/recurring/next_run_at/config/delivery/priority/params…）。**无 `botId`、无 `timezone`**。两者都加 `#[serde(default)]` 即向后兼容旧库记录 | **B**（加两个 serde default 字段） | `axagent-runtime/crates/runtime-core/src/cron_job.rs:44-105` |
| G-3 | cron 档期计算 | `calculate_next_run`：五段 AND、逐分钟扫描上限 366 天、**硬用 `chrono::Local`**（无 IANA 时区参数）；day-of-month 与 day-of-week 取 AND（非 Vixie OR）。这是调度器唯一档期口径，改它要同步改单测 | **B**（加时区参数，None=Local 保持旧行为） | `runtime-core/src/cron_job.rs:323-354`（doc 注释明说「本函数的 Local 时区…就是调度器的全部语义」） |
| G-4 | 调度循环 | 30s tick、`list_due` 到期即派发、每轮最多派一个（防雪崩）、在途去重（`dispatched` 表）、跨进程 leader 租约（每轮实时判定）、错过档期补发一次 | **A**（零改） | `axagent-runtime/crates/runtime/src/cron/scheduler.rs:60-192` |
| G-5 | 到点执行分派 | executor handler 是 `init/services.rs` 里的巨型分支链：按 `task_type` 分派（荐股/反思/漏检补跑等）→ `workflow_id` 兜底跑工作流 → 都没有则只打日志。**没有 Bot 分支**。AppHandle 在闭包内可用（`app_handle`） | **B**（在 workflow 兜底前插 `job.bot_id` 分支） | `src-tauri/src/init/services.rs:2332`（`set_handler`）、`:4433`（workflow_id 兜底） |
| G-6 | 运行历史 | `cron_job_history`：`task_id/started_at/completed_at/success/output/error/duration_ms/partial/trigger`。`record_run` 同步推进 `next_run_at` 并落历史。Bot 例程的「输出落哪」已解决 | **A**（零改） | `axagent-core/crates/entities/src/cron_job_history.rs`；`runtime-core/src/cron_job.rs:723-759` |
| G-7 | 暂停/完成态 | 三态 `CronJobStatus::{Active,Paused,Disabled}`；一次性任务（`recurring=false`）跑完 `set_status(Disabled)`（各分支统一处理，含失败警告）；`pause/resume_scheduled_task` 命令现成。「Done」语义 = 一次性 + Disabled | **A**（零改） | `runtime-core/src/cron_job.rs:175-181`；`services.rs` 各分支 `set_status`；`commands/scheduled_task.rs:395-414` |
| G-8 | 命令层 | 8 个命令齐备（list/get/create/update/delete/pause/resume/execute + `get_task_execution_history`）。DTO `ScheduledTaskDto` camelCase。**无 botId/timezone 透传**；`create_scheduled_task` 不校验 Bot 存在性 | **B**（DTO + 校验 + 可选过滤参数） | `src-tauri/src/commands/scheduled_task.rs:252-405` |
| G-9 | cron 表达式校验 | `validate_cron_expression`：只查 5 字段 + 拒绝每分钟。**无 IANA 时区校验**（根本没有时区概念）。`chrono-tz` 已是 workspace 依赖（`rt-workflow` 在用），引入零成本 | **B**（加时区校验函数） | `commands/scheduled_task.rs:42-70`；`axagent-runtime/crates/rt-workflow/Cargo.toml:16` |
| G-10 | 例程与 Bot 的绑定关系 | OpenMausBot：删 Bot 连带删其 schedules 与 runs；例程运行「只复制它的 Bot 的级别」。AxAgent：`delete_bot` 只解绑 `conversations.bot_id`（ADR-009），**不知道 cron_jobs 的存在**（JSON 里的 botId 无法 SQL 级联） | **B**（`delete_bot` 命令层追加：删/停该 Bot 的例程） | `axagent-infra/crates/dao/src/repo/bot.rs:157`（delete_bot）；OMB `routine-schedules.md`「Deleting a bot deletes its schedules and webhooks too」 |

**应用事件自动化**（P1 GAP #13 的 app-triggers）：P2-a **不做**。理由：AxAgent 无应用事件总线入口（proactive 是定时建议非事件触发），且本批主线是「例程→审批→通知」，事件触发源是 P2-b/P2-c 体量。

## 3. Bot 级与线程级审批执行（PLAN §6 第 2 项）

### 3.1 现有执行链（全部可复用）

| # | 能力 | 现状（实测） | 判定 | 证据 |
|---|---|---|---|---|
| G-11 | Bot 级级别存储 | `bots.approval_mode TEXT DEFAULT 'full'`，P1 已建表已校验枚举（`ask|edits|auto|full|custom`），**只存不执行**（ADR-007） | **A**（P1 已就位） | `axagent-core/crates/entities/src/bots.rs:34`；`src-tauri/src/commands/bot.rs:162`（`validate_approval_mode`） |
| G-12 | 会话级档位存储 | `agent_sessions.permission_mode TEXT`，**实际写入的值只有三种**：`default` / `accept_edits` / `full_access`（创建时 `unwrap_or("default")`）；前端设置页（AdvancedSettings）与 `agent_update_session` 命令都在这三档内 | **A**（列现成，存新值无需迁移） | `axagent-core/crates/entities/src/agent_sessions.rs`；`axagent-infra/crates/dao/src/repo/agent_session.rs:95`；`src/components/settings/AdvancedSettings.tsx:92` |
| G-13 | 解析点（交互轮） | `agent_query` 开头：读 `db_session.permission_mode` → match 三档映射到运行时 `PermissionMode`（未知值**静默回落 Prompt**）→ `task_shape` 只降不升覆盖 → `PermissionPolicy::new(mode).with_permission_rules_from_lists(空, 空, STOCK_WRITE_TOOLS)`。**Bot 级别在此无参与** | **B**（插入 Bot 级解析，见 ADR-P2-1） | `src-tauri/src/commands/agent/mod.rs:2109-2137`、`:2333-2341` |
| G-14 | 工具调用前裁决 | `PermissionPolicy::authorize_with_context`：deny 规则 → ask 规则 → allow 规则 → `mode >= required` → 否则 `prompt_or_deny`（prompter 为 None ⇒ 直接 Deny）。单点、纯函数、可单测——「询问还是自动」的唯一裁决处 | **A**（零改） | `axagent-core/crates/harness/src/runtime_types/permissions.rs:266-364` |
| G-15 | 人审通道 | `ChannelPermissionPrompter`：emit `agent-permission-request` → 阻塞等 `agent_approve` → **300s 超时自动拒绝** + `agent-permission-timeout` 事件。会话级 always-allowed、工作区写边界 enforcer 都在这层 | **A**（交互路径零改） | `axagent-ai/crates/agent/src/session_manager.rs:1209-1456` |
| G-16 | 非交互先例 | `init/agent_turn_adapter.rs`（工作流节点路径）：**无 prompter ⇒ run_turn(None) ⇒ ask 类操作直接 Deny**；带委托工具时用 WorkspaceWrite。这就是 AxAgent 现成的「无人值守」先例，Bot 例程直接沿用该模式 | **A**（模式可复制） | `src-tauri/src/init/agent_turn_adapter.rs:183-193`；`session_manager.rs:968-976`（prompter 取不到传 None） |
| G-17 | approval_rules | 「批准沉淀」是 Bash 工具的 `(program, args_prefix) → allow/prompt/forbidden` 全局规则表，与 Bot 无关 | **A**（本批不动，也不按 Bot 拆） | `src-tauri/src/init/approval_rule_store.rs`；`commands/approval_rules.rs` |
| G-18 | 引擎能力上限 | OpenMausBot 按 `driverKind` 判档位支持（`supportsApprovalMode`，custom 仅 codex）。AxAgent 的引擎 = `ProviderType`（openai/anthropic/gemini/openclaw/hermes/ollama…），**全部是 API 引擎**：工具闸门每轮读级别（`PermissionPolicy` 每轮新建），无「引擎不支持某档」的既有表达 | **B**（建 `level_supported(provider, mode)` 表：custom 对所有 provider 返回 false，其余全支持） | `axagent-core/crates/harness/src/types/provider_model.rs:69-93`；OMB server `APPROVAL_MODES`/`supportsApprovalMode`（resources/server/index.js） |
| G-19 | 只降不升护栏 | `resolve_effective_permission`（task_shape）：严格级排序 `ReadOnly > Prompt > WorkspaceWrite > DangerFullAccess > Allow`，只降不升。**Bot 解析结果必须继续经过它** | **A**（保留不动） | `axagent-core/crates/harness/src/task_shape.rs:211-241` |

### 3.2 级别语义（OpenMausBot 实测口径，修正 P1 文档的转述）

OpenMausBot `docs/approval-levels.md`（GitHub API 取回全文，2026-11-23）+ 本机 `resources/server/index.js`（`APPROVAL_RANK` / `threadApproval`）实测：

- 档位序：`ask(0) < edits(1) < auto(2) < full(3)`，`custom` 独立（仅 codex）。
- **线程自设档位直接生效（不是简单 max）**：聊天窗口设 Full 可在 Ask Bot 上开 Full 线程（升），Ask 聊天在 Full Bot 上开 Ask 线程（降）——「specific 覆盖 general」。
- **例程运行的会话「只复制它的 Bot 的级别」**（原文：a routine run conversation opens, it only copies its bot's level）——非交互源不参与线程级合并。
- **外部源（webhook/他人 Bot）不升**：无设置权者只能降不能升。
- **custom 不传递**：custom 聊天开的线程落 Ask（同 Bot 自身除外）。
- **Bot 读自己的文件任何档位都不问**（SOUL/技能/记忆/会话目录；纯读动词命令；管道/重定向/多命令除外）。
- **无人在场的轮照常按 Bot 级别跑**，决策日志记录「无人在键盘」。

P1 `GAP.md` §2.3 的「level = max(own, inherited)」是转述失真，P2-a 按 OMB 原义实现（ADR-P2-1 有取舍记录，开放问题 Q1 请裁决）。

## 4. 通知与未读（PLAN §6 第 8 项）

| # | 能力 | 现状（实测） | 判定 | 证据 |
|---|---|---|---|---|
| G-20 | 通知中心后端 | **无表、无命令、无 IPC 事件**。通知是纯前端 localStorage（上限 50 条，`axagent-notifications` key），写入受偏好闸门（总闸/分类开关） | **A**（模型沿用前端存储，P2-a 不建后端表） | `src/lib/notification.ts`（全文） |
| G-21 | NotificationBell | 读 `getNotifications()` + `axagent:notification` CustomEvent 实时刷新；未读数 = `filter(!read)`；全局角标 | **A**（加 botId 过滤即可） | `src/components/layout/NotificationBell.tsx:53` |
| G-22 | 完成通知管线 | **agentStore 已监听 `agent-done`/`agent-error` 并 `pushNotification`**（成功/失败文案走 i18n）。Bot 例程复用 `agent_query` 后，通知**免费获得**，无需新增 IpcEventName | **A**（关键复用点） | `src/stores/feature/agentStore.ts:475,555` |
| G-23 | Bot 维度缺失 | `Notification` 接口无 `botId`；未读无法按 Bot 分组；BotsPage 无角标。会话→Bot 映射前端可查（`Conversation.botId` 已在 DTO） | **B**（`Notification.botId?` + Bell/BotsPage 过滤） | `src/lib/notification.ts:11-29`；`axagent-core/crates/harness/src/types/conversation.rs:39` |
| G-24 | nudge | NudgeService（trajectory）注入 system prompt + `nudge_*` 5 命令，是「建议」不是通知，语义不同 | **A**（本批不动） | `src-tauri/src/commands/agent_nudge.rs`；`commands/agent/mod.rs:1787-1823` |
| G-25 | 事件名复用 | `IpcEventName` 现有可复用成员：`AgentStarted/AgentDone/AgentError/AgentStatus`（例程轮全走）、`AgentPermissionRequest/AgentPermissionTimeout`（交互 Bot 线程）。**P2-a 零新增事件** | **A** | `axagent-core/crates/harness/src/event_names.rs:40-64` |
| G-26 | 离线窗口 | 例程在 Rust 侧跑，若前端当时未打开，`agent-done` 事件丢失、通知不落。OpenMausBot 的对应面是云端唤醒；AxAgent 桌面应用常驻是主场景 | **C→接受为 P2-a 已知限制**（文档化，不留假接口） | 本表即证据 |

## 5. 任务中心（顺带核对）

`TasksCenterPage` 的 `cronAdapter`（source `scheduled-cron`）已把 CronJob 定义/最近运行/历史/重试接进统一 Runs 视图。Bot 例程落同一 adapter 即可出现在任务中心，仅需 `raw` 里带出 `botId` 供筛选徽章——**A**（`src/pages/TasksCenterPage/adapters/cron.adapter.ts`，DTO 加字段自动透传）。

## 6. P2-a 不做清单

1. 应用事件自动化（app-triggers）——无事件总线，P2-b+ 评估。
2. 多 Bot 委派运行时与 Assignment 合同——P2-b。
3. 团队/部门、Chief、共享模板——P2-b。
4. approval_rules 按 Bot 拆分——全局语义保留。
5. 记忆写入/维护、每 Bot 技能、Bot API key——P2-c。
6. 通知后端持久化——前端 localStorage 语义保留（见 G-26）。

## 7. 未验证项

- `provider_type` 与「引擎档位支持」的对应未经实机全量验证（G-18 的表按 API 引擎统一全支持处理；OpenClaw 等 CLI 型 provider 若有例外，T1' 实测补表）。
- G-26 离线窗口的实际影响面未量化（取决于例程时段与应用常驻率），P2-b 评估是否补「错过运行的补发通知」。
