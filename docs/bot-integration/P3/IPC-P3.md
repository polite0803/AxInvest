# IPC-P3 — P3 Tauri IPC 契约（冻结稿）

> 状态：frozen-for-P3 ｜ 日期：2026-11-23 ｜ 基线：`2e8db65d6` ｜ 规则沿用 `IPC.md` / `IPC-P2.md` / `IPC-P2B.md`：两步注册（命令实现 + `register_commands.rs`）、camelCase DTO、`Result<T,String>` 包 ErrorResponse、错误码入 11 语言、`scripts/check-contracts.mjs` A–H 零红。
>
> 前端可据此契约与后端并行开工；任何字段增删须先改本文再改代码。

## 1. 命令总览（新增 8，修改 1，事件新增 1）

| 命令 | 类型 | 落点 |
|---|---|---|
| `create_bot_workflow` | 新增 | `src-tauri/src/commands/bot_workflow.rs`（新文件） |
| `update_bot_workflow` | 新增 | 同上 |
| `list_bot_workflows` | 新增 | 同上（可选 `departmentId` 过滤） |
| `delete_bot_workflow` | 新增 | 同上（非终态 run 存在 → `BOT_WORKFLOW_RUN_BUSY`） |
| `run_bot_workflow` | 新增 | 同上（手工触发，返回 `BotWorkflowRunDto`） |
| `list_bot_workflow_runs` | 新增 | 同上（可选 `workflowId` / `status`） |
| `cancel_bot_workflow_run` | 新增 | 同上 |
| `create_bot_workflow_schedule` | 新增 | `src-tauri/src/commands/scheduled_task.rs`（复用 `create_scheduled_task`，`task_type="bot_workflow"`） |
| `delete_bot_department` | **修改** | `src-tauri/src/commands/bot_department.rs`：先将该部门编排转实例归属（清 `department_id`、`owner_scope='instance'`）再删部门，定义保留（ADR-P3-3） |
| `delete_bot` / P2-b 委派命令 / G5 `delegate_task` | **不动** | 删 Bot 由运行前校验报错；委派一层语义不变 |

## 2. DTO 契约（camelCase）

### 2.1 `BotWorkflowStepDto`（也是 `create/update` 入参元素）

`{ stepKey: string, botId: string, promptTemplate: string, dependsOn: string[], timeoutSecs: number, onFailure: 'abort'|'continue' }`

### 2.2 `BotWorkflowDto`

`{ id, name, description, ownerScope: 'instance'|'department', departmentId: string|null, steps: BotWorkflowStepDto[], maxConcurrency: number, totalTimeoutSecs: number, createdAt: number, updatedAt: number }`

> DB 存 `steps_json`，命令层 parse → DTO 的 `steps` 数组；前端永不接触 JSON 字符串。

### 2.3 `BotWorkflowRunDto`

`{ id, workflowId, workflowName, status: 'queued'|'running'|'waiting'|'completed'|'failed'|'cancelled', origin: 'person'|'routine', requesterBotId: string|null, requesterConversationId: string, anchorMessageId: string|null, context: { inputs: Record<string,string>, outputs: Record<string,{assignmentId:string,resultText:string}>, skipped: string[] }, completedSteps, totalSteps, errorCode: string|null, errorDetail: string|null, startedAt: number|null, finishedAt: number|null, createdAt, updatedAt }`

### 2.4 入参

| 命令 | 入参 | 关键校验 |
|---|---|---|
| `create_bot_workflow` | `{ name, description?, ownerScope, departmentId?, steps, maxConcurrency?, totalTimeoutSecs? }` | 名称 1..100；ownerScope=department 时 departmentId 必填且部门存在；步骤 1..12、stepKey 唯一/格式、dependsOn 有定义、无环、Bot 存在且 enabled、部门成员、promptTemplate 引用合法；失败分别给 §4 错误码 |
| `update_bot_workflow` | `{ id, name?, description?, ownerScope?, departmentId?, steps?, maxConcurrency?, totalTimeoutSecs? }` | 同 create；在途 run 存在 → `BOT_WORKFLOW_RUN_BUSY`；`Option<Option<T>>` 清除语义沿用 P2-a（clear departmentId 传显式 null） |
| `list_bot_workflows` | `departmentId?: string|null` | 不传 = 全部可见；传 null = 仅 instance 归属 |
| `delete_bot_workflow` | `{ id }` | 非终态 run → `BOT_WORKFLOW_RUN_BUSY` |
| `run_bot_workflow` | `{ workflowId, requesterConversationId?: string|null, inputs?: Record<string,string> }` | 手工触发必须给线程 id（前端传当前线程）；未给则按 `ERR_VALIDATION` 拒绝（不做隐式建线程）；返回 queued run |
| `list_bot_workflow_runs` | `workflowId?: string, status?: string` | 空 = 全部；状态非法 → `ERR_VALIDATION` |
| `cancel_bot_workflow_run` | `{ runId }` | queued/running/waiting 可取消，终态 → `BOT_WORKFLOW_CANCEL_INVALID` |
| `create_bot_workflow_schedule` | `{ workflowId, cronExpression, timezone?: string|null, enabled?: boolean }` | 复用 `validate_cron_detail`（坏 cron → `BOT_SCHEDULE_CRON_INVALID`）/ `validate_timezone`（坏时区 → `BOT_SCHEDULE_TIMEZONE_INVALID`）；workflow 不存在 → `BOT_WORKFLOW_NOT_FOUND`；落 `task_type="bot_workflow"`、`params={"workflowId":id}`、`bot_id=None` |

### 2.5 `origin` 值域变更（仅 DTO 联合类型）

P2-b `BotAssignmentDto.origin` 为 `'person'|'bot'|'routine'`；P3 步骤行新增 `'workflow'`，前端类型与筛选器同步扩展（旧值不变，无数据迁移）。编排 run 的 origin 只有 `'person'|'routine'`。

## 3. 执行与事件契约

| 事件 | payload | 触发点 |
|---|---|---|
| `bot-workflow-run-updated` | `BotWorkflowRunDto` | run 落库 queued、每次 status 变迁（含 waiting↔running）、终态各发一次 |
| `bot-assignment-updated`（P2-b 既有） | `BotAssignmentDto` | 步骤 assignment 的状态三态照旧；**不新增步骤事件** |

- 步骤完成钩子（后端内部，不是 IPC）：run 更新 `context_json.outputs[stepKey]` → 追加回执消息 → 通知 → 解锁下游。
- 回执消息：role=Assistant，正文前缀 `[步骤 {stepKey} · {botName}] `，`parent_message_id = anchor_message_id`；anchor 是 run 开始时写入同一线程的 role=System 消息（正文含编排名与 runId）。
- 通知：`Notification.botId = 步骤 assigneeBotId`，复用 P2-a 管线；run 级不另发通知，避免一次编排刷 N+1 条。
- 前端监听 `bot-workflow-run-updated` 刷新运行列表/进度；步骤状态跟随既有 assignment 列表（可按 `workflowRunId` 过滤）。

## 4. 错误码（新增 10，`BOT_WORKFLOW_*` 段，入 11 语言）

落点：`src-tauri/src/commands/error_code.rs`（bot 段追加常量）+ `src/i18n/locales/*.json`（11 语言齐更）。

| 码 | zh-CN 源文案 | 触发点 |
|---|---|---|
| `BOT_WORKFLOW_NOT_FOUND` | 未找到 Bot 编排 | 改/删/触发不存在的定义 |
| `BOT_WORKFLOW_STEP_INVALID` | 编排步骤定义不合法 | stepKey 重复/格式、空步骤、prompt 引用未知 key |
| `BOT_WORKFLOW_CYCLE` | 编排步骤存在循环依赖 | 定义期拓扑校验成环 |
| `BOT_WORKFLOW_DEPARTMENT_REQUIRED` | 部门编排必须指定部门 | ownerScope=department 且 departmentId 缺失 |
| `BOT_WORKFLOW_MEMBER_CHANGED` | 编排步骤 Bot 已不属于该部门 | 运行前成员复查失败 |
| `BOT_WORKFLOW_RUN_NOT_FOUND` | 未找到编排运行记录 | 查询/取消不存在的 run |
| `BOT_WORKFLOW_RUN_BUSY` | 编排存在未结束的运行 | 改/删定义时被在途 run 挡住 |
| `BOT_WORKFLOW_STEP_BOT_MISSING` | 编排步骤引用的 Bot 不存在或已禁用 | 运行前 Bot 校验 |
| `BOT_WORKFLOW_TIMEOUT` | 编排运行超时 | 总超时 900s（或定义值）到点 |
| `BOT_WORKFLOW_CANCEL_INVALID` | 编排运行已结束，无法取消 | 终态 run 取消 |

> 错误码是**语义常量**，没有 1xx/2xx/3xx/4xx 数字段位；P3 按 `BOT_WORKFLOW_*` 语义前缀扩块，不要写数字段，`check-errorcode-alignment.mjs` 不认数字段。

> 复用不新增：步骤级 `BOT_DELEGATE_BUSY` / `BOT_DELEGATE_TIMEOUT` / `BOT_NOT_FOUND` / `BOT_DISABLED` 仍按 P2-b 语义继承到 run.error_code。门禁基线：P2-b 收口时后端 473 码 ↔ 11 语言对齐；P3 后应为 483 码（实测以 `check-errorcode-alignment.mjs` 输出为准）。

## 5. 契约审计扩展

`scripts/lib/bot-ipc-audit.mjs` 增补 P3 段：8 命令注册核对、`BotWorkflowDto`/`BotWorkflowRunDto` camelCase 字段核对、10 错误码 ×11 语言、`bot-workflow-run-updated` 进入 `IpcEventName` 枚举核对、`params.workflowId` 与 `task_type="bot_workflow"` 字符串常量核对。`check-contracts.mjs` A–H 节自动覆盖新增引用。

## 6. 门禁命令原文（WSL ext4，`ssh -o BatchMode=yes wsl 'bash -s'` 内执行）

```bash
# P2-b 基线回归（15 条台账，错误路径已替换为实测正确路径）
npm run typecheck
node scripts/vitest-run.mjs run
node scripts/check-contracts.mjs
node scripts/check-errorcode-alignment.mjs
node scripts/check-bot-i18n-keys.mjs
npm run check:layers
cargo test --manifest-path axagent-ai/Cargo.toml -p axagent-agent
cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao
cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --lib bot_
cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --lib reconcile
cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --lib counts::
cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --test schema_compliance
cargo test --manifest-path src-tauri/Cargo.toml --test bot_assignment --test bot_routine
cargo test --manifest-path axagent-runtime/Cargo.toml -p axagent-tools bot_delegate
npm run check:i18n
npm run check:i18n:key-parity
npm run check:routes
npm run check:serde
npx playwright test e2e/bot-delegation.spec.ts --project=chromium --grep-invert "@heavy" --timeout=120000 --reporter=list
# P3 新增门禁
cargo test --manifest-path src-tauri/Cargo.toml --test bot_workflow --test bot_assignment --test bot_routine
npx playwright test e2e/bot-workflow.spec.ts --project=chromium --grep-invert "@heavy" --timeout=120000 --reporter=list
```

> 台账第 9 条 `cargo test --manifest-path axagent-ai/Cargo.toml --test bot_assignment --test bot_routine` 是错误路径（exit 101），此处已替换为实测正确路径 `src-tauri/Cargo.toml`（台账 9a）。

> 已知存量红：`npm run check:serde`（`vector_store.rs` 三个结构缺 camelCase）自 P1 起未修，P3 不处理；新代码不得新增 serde 红。
