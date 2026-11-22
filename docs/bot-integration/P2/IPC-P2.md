# IPC-P2 — P2-a Tauri IPC 契约（冻结稿）

> 状态：frozen-for-P2-a ｜ 日期：2026-11-23 ｜ 基线：`0f73ec54b` ｜ 前置：`IPC.md`（P1，规则沿用：两步注册、camelCase、`Result<T,String>` 包 ErrorResponse、错误码入 11 语言、check-contracts.mjs A–H 节）
>
> **事件零新增**（GAP G-25）：复用 `AgentStarted/AgentDone/AgentError/AgentStatus/AgentPermissionRequest/AgentPermissionTimeout`，`IpcEventName` 枚举与生成 TS 类型零 diff。

## 1. 命令总览（新增 3，修改 3，不动 5）

| 命令 | 类型 | 落点 |
|---|---|---|
| `create_bot_scheduled_task` | **新增** | `src-tauri/src/commands/bot.rs`（Bot 语义入口，内部复用 scheduled_task 校验/落库） |
| `list_bot_scheduled_tasks` | **新增** | 同上（按 botId 过滤，含 paused/disabled 全量） |
| `list_bot_notifications` | **新增** | 纯前端派生，无后端命令——**实际不存在**，见 §4（此行仅占位说明，不注册） |
| `create_scheduled_task` | 修改 | `src-tauri/src/commands/scheduled_task.rs` |
| `list_scheduled_task` | 修改 | 同上 |
| `update_scheduled_task` | 修改 | 同上 |
| `delete_scheduled_task` / `pause_` / `resume_` / `execute_` / `get_task_execution_history` | **不动** | botId/timezone 随 DTO 自然透出 |

> §4 说明：通知读取沿用前端 `getNotifications()`（localStorage），不新增 IPC 命令——与 P1 「不新增事件」同源的克制原则。表里保留该行是为防止误以为缺命令。

## 2. 新增命令契约

### 2.1 `create_bot_scheduled_task`

- **入参**（camelCase）：
  - `botId: string` — 必须存在（不存在 → `BOT_NOT_FOUND`）
  - `name: string`（1..200）
  - `prompt: string`（非空；Bot 例程跑 agent_query 的 prompt）
  - `cronExpression: string` — 五段，复用 `validate_cron_expression`；**每分钟仍拒绝**
  - `timezone?: string \| null` — IANA 名；给出时用 `chrono_tz::Tz::from_str` 校验（失败 → `BOT_SCHEDULE_TIMEZONE_INVALID`）；`null/undefined` = 宿主 Local（与现状一致）
  - `enabled?: boolean`（缺省 true）
- **语义**：构造 `CronJob { bot_id: Some(botId), timezone, task_type: "bot_routine", prompt, schedule, status: Active, recurring: true, … }`；`next_run_at` 按 timezone 计算。
- **返回**：`ScheduledTaskDto`（含新字段）。
- **错误码**：`BOT_NOT_FOUND`（P1 已有）、`BOT_SCHEDULE_TIMEZONE_INVALID`（**新**）、`BOT_SCHEDULE_CRON_INVALID`（**新**，包 `validate_cron_expression` 的失败文案，避免前端解析后端自由字符串）、`ERR_VALIDATION`（名称/空 prompt 等既有通用码）。

### 2.2 `list_bot_scheduled_tasks`

- **入参**：`botId: string`（必填；不存在 → `BOT_NOT_FOUND`）
- **语义**：`store.list()` 反序列化后按 `data.botId == botId` 过滤（JSON 不可 SQL 索引，GAP G-10/MODEL §1），**不设分页**（单机量级），返回全部状态含 `Disabled`。
- **返回**：`ScheduledTaskDto[]`。
- **错误码**：`BOT_NOT_FOUND`。

### 2.3 `create_scheduled_task`（修改）

- 入参追加可选项：`botId?: string \| null`、`timezone?: string \| null`（校验同 §2.1）。
- `botId` 给出时同样校验 Bot 存在（错误码同上）——旧调用不传，行为逐字节不变。
- 语义：与 §2.1 同一套 CronJob 构造；`task_type` 仍由调用方传（Bot 语义入口只是预设 `"bot_routine"` 的便捷层）。

### 2.4 `list_scheduled_task`（修改）

- 入参追加：`botId?: string \| null` —— 给出时按 `data.botId` 过滤；不传返回全量（现状不变）。
- 返回 DTO 追加字段：`botId?: string \| null`、`timezone?: string \| null`。

### 2.5 `update_scheduled_task`（修改）

- 入参追加可选项并采用**双层 Option 清除语义**：
  - `timezone?: Option<Option<string>>` —— 外层 None = 不改；`Some(None)` = **清除**（回落宿主 Local）；`Some(Some(tz))` = 设置并校验。
  - `botId?: Option<Option<string>>` —— 同上；`Some(Some(botId))` 时校验 Bot 存在。
- 其余字段（schedule/prompt/enabled）语义沿用 P1；schedule 变更后按当前 timezone 重算 `next_run_at`。

## 3. 错误码（新增 2，入 11 语言错误区）

落点：`src-tauri/src/error_code.rs` + `src/i18n/locales/*`（11 语言齐更）+ 生成 TS 类型核对。

| 码 | en（示例源文案） | zh-CN |
|---|---|---|
| `BOT_SCHEDULE_CRON_INVALID` | Invalid cron expression (must be 5 fields, not every-minute) | 无效的 cron 表达式（必须为 5 段，且不能为每分钟） |
| `BOT_SCHEDULE_TIMEZONE_INVALID` | Unknown IANA time zone | 未知的 IANA 时区 |

其余 10 语言（en-GB/es/fr/de/ja/ko/pt-BR/ru/… 以 P1 `error_code` 现有清单为准）由 T5' 翻译落盘并过 i18n 门禁。

## 4. 通知读取（无新命令，契约即「无」)

- 读：`getNotifications()` / `NotificationBell` 现状（localStorage）。
- 写：`pushNotification` 增加可选 `botId` 字段（agentStore 监听 `agent-done` 时从事件 payload 的会话→Bot 映射带出；映射查不到则不写该字段，通知仍落）。
- Bot 例程轮结束的 agent-done payload 已含 `conversationId`，前端经 `Conversation.botId`（P1 DTO 已有）解析归属。

## 5. 冻结说明与待裁决项

契约以上为冻结稿；**未冻结**的三处见 PLAN-P2 §7 开放问题（Q1 auto→PermissionMode 映射、Q2 离线窗口、Q3 复用 DTO vs 独立 Bot 命令——本稿已按建议答案落笔：Q1=WorkspaceWrite、Q3=「复用既有命令 + Bot 便捷层」；若裁决相反，仅 §2.1/§2.2 位置互换，参数不变）。
