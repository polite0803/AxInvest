# IPC-P2B — P2-b Tauri IPC 契约（冻结稿）

> 状态：frozen-for-P2-b ｜ 日期：2026-11-23 ｜ 基线：`ee6690c3a` ｜ 规则沿用 `IPC.md`/`IPC-P2.md`：两步注册、camelCase、`Result<T,String>` 包 ErrorResponse、错误码入 11 语言、check-contracts.mjs A–H 节（含 BOT 节扩展为 BOT+P2B 审计）。

## 1. 命令总览（新增 8，修改 1，事件新增 1）

| 命令 | 类型 | 落点 |
|---|---|---|
| `list_bot_departments` | 新增 | `src-tauri/src/commands/bot_department.rs`（新文件） |
| `create_bot_department` | 新增 | 同上 |
| `update_bot_department` | 新增 | 同上（name/description/chief_bot_id） |
| `delete_bot_department` | 新增 | 同上（级联见 ADR-P2B-6） |
| `add_bot_to_department` / `remove_bot_from_department` | 新增 | 同上（一个 Bot 只属一个部门，重复 add 报错） |
| `create_bot_assignment` | 新增 | `src-tauri/src/commands/bot_assignment.rs`（新文件） |
| `list_bot_assignments` | 新增 | 同上（按 requester/assignee/botId/status 过滤，皆可选） |
| `cancel_bot_assignment` | 新增 | 同上（queued/running 可取消，completed/failed 报错） |
| `delete_bot` | **修改** | `src-tauri/src/commands/bot.rs`：追加「清部门成员行 + 清 chief 引用 + 取消该 Bot 在途 assignment」两处扫描（ADR-P2B-6） |
| `delegate_task`（G5）等既有委派命令 | **不动** | GAP G-B4：并列不改造 |

## 2. DTO 契约（camelCase，TS 类型由生成物核对）

### 2.1 `BotDepartmentDto`
`{ id, name, description, chiefBotId: string|null, botIds: string[], createdAt, updatedAt }`（botIds 由命令层 join 成员表得出）。

### 2.2 `BotAssignmentDto`
`{ id, rootId, parentId: string|null, depth, origin: 'person'|'bot'|'routine', requesterBotId: string|null, requesterConversationId: string|null, assigneeBotId, assigneeName, mode: 'consult'|'dispatch', taskText, status: 'queued'|'running'|'completed'|'failed'|'cancelled', resultText: string|null, errorCode: string|null, errorDetail: string|null, startedAt: number|null, finishedAt: number|null, createdAt, updatedAt }`

### 2.3 `create_bot_assignment` 入参
`{ assigneeBotId: string, taskText: string (1..20000), mode: 'consult'|'dispatch' = 'dispatch', requesterConversationId?: string|null, requesterBotId?: string|null }`
- 校验：assignee 存在且 enabled（否则 `BOT_NOT_FOUND`/`BOT_DISABLED`）；两者不得为同一 Bot（`BOT_DELEGATE_SELF`）；`requesterBotId` 给出时同样校验。
- **consult 由前端禁用**（consult 是 assignee 轮内工具语义，person 不直接发起 consult；入参保留 mode 字段是为工具 runner 与命令共用 DTO，传 `consult` → `ERR_VALIDATION`）。

## 3. 并行门与超时（命令层，常量起步）

| 门 | 值 | 超限错误码 |
|---|---|---|
| 单 requester 在途 dispatch | 4 | `BOT_DELEGATE_BUSY` |
| 全局在途 dispatch | 8 | `BOT_DELEGATE_BUSY` |
| 单轮内 consult 工具调用 | 2（工具层计数） | 工具返回错误文本（不落错误码，LLM 可读） |
| dispatch 超时 | 300s → status=failed，errorCode=`BOT_DELEGATE_TIMEOUT` | — |

> Q5 待确认：数值后调不影响契约（常量集中 `bot_assignment.rs` 头部，前端无感知）。

## 4. 错误码（新增 6，入 11 语言错误区）

落点：`src-tauri/src/commands/error_code.rs`（bot 段）+ `src/i18n/locales/*`（11 语言齐更）。

| 码 | en（源文案） | zh-CN |
|---|---|---|
| `BOT_DEPARTMENT_NOT_FOUND` | Bot department not found | 未找到 Bot 部门 |
| `BOT_DEPARTMENT_MEMBER_EXISTS` | Bot already belongs to a department | 该 Bot 已归属其他部门 |
| `BOT_ASSIGNMENT_NOT_FOUND` | Bot assignment not found | 未找到委派记录 |
| `BOT_DELEGATE_SELF` | A bot cannot delegate to itself | Bot 不能委派给自己 |
| `BOT_DELEGATE_BUSY` | Too many in-flight delegations | 委派并发已达上限 |
| `BOT_DELEGATE_TIMEOUT` | Delegation timed out | 委派执行超时 |

> `BOT_DELEGATE_PERMISSION_DENIED`（ADR-P2B-3 提及）在 P2-b 走通用 `ERR_PERMISSION` 语义（assignee ask 档写拒绝已是既有路径），不新增码；P3 若需要细分再加。`BOT_DISABLED` 沿用 P1 错误码族既有值（若 T5'' 实测发现缺失，按既有流程补码 + 11 语言）。

## 5. 事件（新增 1，入 `IpcEventName` 枚举）

| 事件 | payload | 触发点 |
|---|---|---|
| `bot-assignment-updated` | `BotAssignmentDto` | status 变迁（queued→running→终态）各发一次；前端列表实时刷新 |

> 前端监听该事件更新 Assignment 列表；回执正文落线程走既有会话刷新路径（无新事件）；Bot 通知复用 P2-a `Notification.botId`（assignee=Bot 时写 assigneeBotId）。

## 6. 契约审计扩展

`scripts/lib/bot-ipc-audit.mjs` 增补 P2-b 段：8 命令注册核对、`BotAssignmentDto` camelCase 核对、新错误码 ×11 语言、`bot-assignment-updated` 事件枚举一致性（G 节自动覆盖发射引用）。
