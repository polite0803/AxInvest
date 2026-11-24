# PLAN-P4 — P4 分期计划（frozen-for-P4）

> 状态：**frozen-for-P4（2026-11-25 用户按建议裁决 D1–D8）** ｜ 日期：2026-11-25 ｜ 基线：`416094880`（`feat/p3-bot-workflow-frontend`；P3 Q1 多层修订已提交）｜ 对应版本：v3.0.0+（未定版，待确认）｜ 来源：本地原创
>
> 体例沿用 `PLAN.md` / `PLAN-P2B.md` / `PLAN-P3.md`：所有命令在 WSL ext4 内执行（`ssh -o BatchMode=yes wsl 'bash -s'`），e2e 用 `--grep-invert "@heavy"`。
>
> 责任人代码：**后端** = Bot 运行时后端工程师；**前端** = Bot 前端工程师；**门禁** = Bot 门禁与验证工程师；**架构** = Bot 集成架构师。
>
> **生效状态**：§8 的 D1–D8 已于 2026-11-25 按建议值裁决并回写（D1=A、D2=A+C、D3=A、D4=A、D5=A、D6=A、D7=A、D8=A）；冻结包五份已落盘（§11），状态已改 `frozen-for-P4`。§6 实现卡自本版起可开工。

## 0. 来源与事实基线

### 0.1 P3 已收口（本计划的前置）

`P3/ACCEPTANCE-P3.md` §6「PLAN-P3 §10 收口条件逐条签字」六条全 PASS，收口结论原文：「§10 六条全部 PASS，P3 可以收口」。P3 的 8 命令、2 事件族、12 错误码、8+7 条 Rust 运行态测试与 6 条 e2e 均已实测。**P4 不重开 P3 已冻结语义**，只做两条遗留项的增量。

### 0.2 两条遗留项原文（逐字）

| # | 原文 | 出处 | 性质 |
|---|---|---|---|
| Q7 | `预算常量（编排并发 2 / 总超时 900s / 12 步 / 链每层 2 / 单步与链总 300s）？` → `**按左值起步**，编排常量在 bot_workflow_runner.rs、链常量在 bot_assignment_runner.rs 头部；两套独立叠加` → `改常量即可，IPC/MODEL 不变；若要按部门配额则需新表，另立 P4` | `docs/bot-integration/P3/PLAN-P3.md:136` | 用户尚未裁决「要不要按部门配额」；原文只给了“需新表、另立 P4”的方向，**没有给配额维度与归属** |
| Q8 | `OMB waiting/needs_input 全量落地？` → `**只做 run 级 waiting**（依赖未就绪）；不做 needs_input（无人值守）` → `补 needs_input 需要交互入口与超时答应口径（T3'''/T6'''，+3 人日，建议 P4）` | `docs/bot-integration/P3/PLAN-P3.md:137` | 用户尚未裁决 needs_input 的适用范围；原文只给了“交互入口 + 超时答应口径”两个必备件 |

### 0.3 P3 对 needs_input 的四条排除口径（P4 若要改，必须显式 supersede）

| 出处 | 原文 |
|---|---|
| `docs/bot-integration/P3/MODEL-P3.md:77` | 「`waiting` = 仍有步骤因依赖未就绪而没派发（不是交互式等待）；`needs_input` 不引入——ask 类操作在 assignee 轮按 P2-a `prompter=None` 自动 Deny。」 |
| `docs/bot-integration/P3/ADR-P3.md:84` | 「等待态只做依赖阻塞（run.status=waiting），不做 needs_input 交互阻塞（无人在场）。」 |
| `docs/bot-integration/P3/I18N-KEYS-P3.md:56` | 「`bot.workflowRun.statusWaiting` 译为「等待依赖」，不得译成「等待用户输入」（P3 无 needs_input，ADR-P3-6）。」 |
| `docs/bot-integration/P3/GAP-P3.md:28` | 「**只做 run 级 waiting**（依赖阻塞）；**不做 needs_input** ｜ 编排是多 Bot 无人值守，needs_input 无回答者」 |

### 0.4 本轮只读实测（决定 P4 不能用“从零造”的方式写）

| 事实 | 证据（本轮读取） | 对 P4 的意义 |
|---|---|---|
| 交互式提问工具已存在 | `axagent-runtime/crates/tools/src/tools/todo_write.rs:113` `AskUserQuestionTool`；工具表默认注册 | Q8-A 可复用，不必新造提问工具 |
| 已有提问桥：发 `agent-ask-user`、阻塞等答、300s 超时 | `src-tauri/src/commands/agent/mod.rs:71-171` `AppAskUserBridge::ask_user_blocking`；答复命令 `:3078` `agent_respond_ask`；桥接线 `:2238` | Q8 的真缺口不是“能不能问”，而是**持久化、可恢复、可列举、超时口径** |
| 待答请求只在内存；切会话即清理 | `src-tauri/src/commands/agent/mod.rs:136-171`（oneshot + 内存 map）；`src/stores/feature/agentStore.ts:46` `pendingAskUser`、`:755-760` 按会话清理 | 刷新/切换后卡片消失，无法从线程重新作答 |
| 非交互轮没有提问桥，prompter 也为空 | `src-tauri/src/bot_routine_runner.rs:269-281`（`prompters` 空表）；`todo_write.rs:189` 无桥时退化为纯文本；`session_manager.rs:1196` 为交互桥实现 | 例程/委派/编排当前**不会**进入 needs_input；Q8 若覆盖无人值守，必须新增桥与超时语义 |
| P3 预算全是常量 | `src-tauri/src/bot_workflow_runner.rs:31-35`（2 / 900 / 12）；`src-tauri/src/bot_assignment_runner.rs:33-35`（300 / 每层 2） | Q7 的部门配额必须在**不改这些默认值**的前提下做覆盖层 |
| 部门表已有，无任何配额列/表 | `axagent-core/crates/entities/src/bot_departments.rs`、`bot_department_members.rs`；`axagent-infra/crates/dao/src/repo/` 仅 `bot.rs` / `bot_assignment.rs` / `bot_department.rs` / `bot_workflow*.rs` | Q7 确需新表；不能塞进 `metadata_json` 当暗配置（无法校验、无法索引、无法审计） |
| 错误码与 i18n 现状 | `src-tauri/src/commands/error_code.rs:45-108`（Bot 段）；P3 台账实测 485 码、11 语言、必需 UI 键 151 | P4 新增码必须“先 locale 后代码”，并扩 `check-bot-i18n-keys` 必需集合 |

## 1. 目标与范围

### 1.1 一句话目标

把 P3 留下的两条“已知但未做”收成一个可独立上线的 P4：**部门级运行配额（Q7）** 与 **Bot 的 needs_input 交互答应入口（Q8）**。两条切片互不依赖，可分别上线；不建议合并成一个大改。

### 1.2 建议范围：两条独立纵向切片

| 切片 | 目标 | 用户可见结果 | 明确不改 |
|---|---|---|---|
| **P4-a 部门运行配额** | 每个部门可设一组“只收紧、不放松”的运行上限：单编排并发、总超时、步数、链每层在途、单步超时、部门同时在跑编排数 | 部门弹窗可查看/编辑/恢复默认；超限在**保存定义**或**触发运行**时被明确拒绝，并给出字段名与上限 | 不改 P3 的实例级默认行为；不改全局链深 3；不改调度器 |
| **P4-b needs_input 交互答应** | 把现有 `AskUserQuestion` 的待答请求落库，Bot 线程刷新/切换后仍能看到并作答；超时/重启有明确终态 | Bot 线程出现持久化提问卡（含选项、剩余时间、作答/取消）；`/bots` 侧栏可看到待答数；超时后卡片转“已过期” | 不改通用聊天既有 `AskUserCard` 行为；不默认开启无人值守 needs_input（见 D4） |

### 1.3 明确不做（P4 范围外）

- **费用/Token 配额口径**（除非 D1 选 B）：无 Bot 维度用量账本，需新增记账链路，估 8–12 人日，建议另立 P5。
- OMB 的“自动事故线程 + ≤2 次重试”（P3 ADR-P3-6 已明确不做）。
- P2-c 三件事：技能预设导入、Bot API key、记忆写入；与本草案正交。
- Bot 群聊/rooms、通用模板引擎、图状任意回边、新调度器/新会话池/新模型调用路径。
- 应用重启后的“断点续跑”：P4-b 只保证**待答请求可见、可答、可过期**，不保证跨进程恢复被中断的模型轮。

## 2. 与 P3 的边界

| 维度 | P3 冻结口径 | P4 增量 | 是否触碰冻结 |
|---|---|---|---|
| 编排预算 | 常量 2 / 900s / 12 步；链 300s / 每层 2 | 新增部门覆盖层；**取 min，只收紧**；无部门行时逐字节等于 P3 | 否（默认路径不变） |
| 委派深度 | 硬门 `depth≤3`，`MAX_DELEGATE_DEPTH=3` | **不动**；部门配额不含深度维度 | 否 |
| assignment 状态机 | 五态 `queued/running/completed/failed/cancelled` | **建议不动**（D6=A，独立请求表表达待答） | 否 |
| run 状态 | `waiting`=依赖未就绪，不是交互等待 | 建议保留原义；待答用请求表 + UI 徽章，不改 run 状态（D6） | 否（若 D6=B 才需显式 supersede） |
| 单线程归属 | 一个 run 一条线程，回执挂 anchor | 不动；提问卡也落在同一线程 | 否 |
| 回执前缀 | `[步骤 …]` / `[L{depth} …]` | 不动；提问卡不是消息回执，不加前缀 | 否 |
| 失败策略 | `abort`/`continue`，run 终态 failed | 不动；超时只影响请求行与当前轮返回值 | 否 |
| cron 触发 | `task_type="bot_workflow"` 分支并列 | 触发时增一次配额校验；超额记 history 失败，不新建分支 | 否 |
| 部门删除 | 编排转实例归属后删部门 | 追加：同事务删该部门配额行 | 否 |
| 契约/i18n | 485 码、11 语言、151 必需键 | 只增不改；先 locale 后代码 | 否 |

## 3. 冻结约束（拟，随状态一起待确认）

1. **零 migration**：`MIGRATIONS = &[]`；只走声明式 reconcile（CreateTable / AddColumn + DEFAULT）；实体登记 `entities/src/lib.rs`，`dao/build.rs` 自动注册；`dao/tests/schema_compliance.rs` 的 `SCHEMA` 清单同步登记。
2. **两步注册**：命令实现 + `src-tauri/src/register_commands.rs`；camelCase DTO；`Result<T, ErrorResponse>`；禁止裸 `e.to_string()` 回传用户可见错误。
3. **错误码语义化**：`BOT_*` 前缀，不写数字段；新增码先落 11 语言 locale，再落 `error_code.rs`；`check-errorcode-alignment.mjs` 零孤儿。
4. **i18n 11 语言**：新增 UI 键先落 11 locale，再扩 `scripts/check-bot-i18n-keys.mjs` 的必需集合，最后前端引键。
5. **契约审计同步**：`scripts/lib/bot-ipc-audit.mjs` 新增 P4 段；`check-contracts.mjs --selftest=bot` 必须含 P4 单点破坏自检。
6. **无新执行路径**：不新建调度器/会话池/模型调用路径；配额与待答都挂在 P3/P2 既有 runner 与 `run_bot_non_interactive_turn` 上。
7. **只收紧不放松**：部门配额是硬天花板；定义值与配额取 `min`；**禁止**自动放大 P3 默认值，**禁止**静默夹紧（超额必须可见地拒绝，D3）。
8. **无人值守默认不变**：除非 D4 选 B，例程/委派/编排仍按 `prompter=None` + 无提问桥执行；P4-b 只覆盖“有人在场的 Bot 线程”。
9. **不 push**：本轮及后续实现均在本地提交；不 undelete/不改他人在途文件；P1–P3 冻结稿只读，若必须改口径，先在本文件留修订记录。

## 4. 数据模型增量（已冻结 → `MODEL-P4.md`）

> 两张新表都是**纯增量**，不改任何既有表列。若 D1 选 B（费用配额）或 D2 选通用 scope 表，表结构按各节“替代”说明调整；未裁决前不写实体。

### 4.1 `bot_department_quotas`（Q7 建议口径）

| 列 | 类型 | 约束 | 语义 / 默认 |
|---|---|---|---|
| `id` | TEXT PK | UUID | 配额行 id |
| `department_id` | TEXT NOT NULL | UNIQUE、INDEXED | 部门 id；FK 语义在命令层校验（与 `chief_bot_id` 同口径，不建 SQL 外键） |
| `max_concurrency` | INTEGER NOT NULL | DEFAULT 2，CHECK 1..4 | 单编排步骤并发上限（对应 `bot_workflow_runner.rs:31`） |
| `total_timeout_secs` | INTEGER NOT NULL | DEFAULT 900，CHECK 60..3600 | 单编排总超时上限（对应 `bot_workflow_runner.rs:33`） |
| `max_steps` | INTEGER NOT NULL | DEFAULT 12，CHECK 1..12 | 单编排步数上限（对应 `bot_workflow_runner.rs:35`，硬顶 12 不放开） |
| `max_in_flight_per_depth` | INTEGER NOT NULL | DEFAULT 2，CHECK 1..4 | 链同层在途上限（对应 `bot_assignment_runner.rs:35`） |
| `step_timeout_secs` | INTEGER NOT NULL | DEFAULT 300，CHECK 30..900 | 单步/链总超时上限（对应 `bot_assignment_runner.rs:33`） |
| `max_concurrent_runs` | INTEGER NOT NULL | DEFAULT 2，CHECK 1..8 | 建议新增：该部门同时在跑的编排数上限（Q7 未提，若用户不要可删，见 D2） |
| `created_at` / `updated_at` | BIGINT NOT NULL | | 时间戳 |

- **唯一索引**：`UNIQUE(department_id)` 由 `axagent-infra/crates/dao/src/reconcile/extras.rs` 的 `MULTI_COL_INDEXES` 声明（实体语法表达不了；先例 `extras.rs:844-856`）。
- **落点**：新增 `axagent-core/crates/entities/src/bot_department_quotas.rs`；`entities/src/lib.rs` 注册；新增 `axagent-infra/crates/dao/src/repo/bot_department_quota.rs`；`dao/tests/schema_compliance.rs` 登记全列 + 唯一索引。
- **删除级联**：`delete_bot_department` 同事务删配额行；不依赖 SQL 外键。
- **替代结构**：若 D2=B（实例默认也入库），改为 `bot_execution_quotas(scope, scope_id, …)`，`UNIQUE(scope, scope_id)`，实例行 `scope='instance', scope_id=''`；迁移成本 +0.5 人日，但多一层查询与 UI。

### 4.2 `bot_input_requests`（Q8 建议口径）

| 列 | 类型 | 约束 | 语义 |
|---|---|---|---|
| `id` | TEXT PK | UUID | 请求行 id |
| `ask_id` | TEXT NOT NULL | UNIQUE、INDEXED | 与现有 `agent-ask-user` 的 `askId` 对齐，用于投递答复 |
| `conversation_id` | TEXT NOT NULL | INDEXED | 所在 Bot 线程 |
| `bot_id` | TEXT NULL | INDEXED | 线程归属 Bot（从 `conversations.bot_id` 取） |
| `workflow_run_id` | TEXT NULL | INDEXED | 预留：无人值守方案（Q8-B）使用；交互线程恒 NULL |
| `assignment_id` | TEXT NULL | INDEXED | 预留：委派轮使用；交互线程恒 NULL |
| `origin` | TEXT NOT NULL | DEFAULT 'person' | `person` / `routine` / `workflow`（为 Q8-B 预留，A 方案恒 `person`） |
| `question` | TEXT NOT NULL | | 主问题文本（现有桥只取第一个问题；P4 沿用） |
| `options_json` | TEXT NOT NULL | DEFAULT '[]' | 选项数组（字符串） |
| `status` | TEXT NOT NULL | DEFAULT 'pending'，INDEXED | `pending` / `answered` / `expired` / `cancelled` |
| `answer` | TEXT NULL | | 答复正文 |
| `answered_at` | BIGINT NULL | | 答复时间 |
| `expires_at` | BIGINT NOT NULL | | 超时点（当前交互桥 300s；D5 可改） |
| `created_at` / `updated_at` | BIGINT NOT NULL | | 时间戳 |

- **生命周期**：`pending → answered | expired | cancelled`；终态不可回退。`answer_bot_input_request` 只接受 `pending`。
- **重启**：启动时把上一进程遗留的 `pending` 全部置 `expired`（记固定原因文案），**不做断点续跑**。
- **落点**：新增 `axagent-core/crates/entities/src/bot_input_requests.rs`；`entities/src/lib.rs` 注册；新增 `axagent-infra/crates/dao/src/repo/bot_input_request.rs`；`schema_compliance.rs` 登记全列 + 唯一索引。
- **不动的表**：`bot_assignments` / `bot_workflow_runs` / `bots` / `conversations` / `messages` 零变更（D6=A 的前提）。

### 4.3 既有表排除理由

| 表 | 为何零变更 |
|---|---|
| `bot_departments` | 配额是独立生命周期与校验规则，塞 `metadata_json` 会失去类型校验、索引与审计；加列则让部门记录承担运行策略，职责混杂 |
| `bot_workflows` / `bot_workflow_runs` | 配额是部门级天花板，不是单个定义字段；run 记录只读生效值，不反向存配额 |
| `bot_assignments` | 待答请求不是 assignment 状态机的第五/第六态（D6=A）；`ask_id` 与 assignment 可选关联即可 |
| `messages` | 提问卡是 UI 投影，不是聊天消息；落库表支撑刷新恢复与审计，不污染消息流 |
| `cron_jobs` | 无需新字段；配额校验发生在触发后、run 创建前 |

## 5. IPC 契约增量（已冻结 → `IPC-P4.md`）

> 以下契约已随 `IPC-P4.md` 冻结；字段增删须先改 `IPC-P4.md`。

### 5.1 命令总览（新增 6，修改 1）

| 命令 | 类型 | 落点 | 说明 |
|---|---|---|---|
| `get_bot_department_quota` | 新增 | `src-tauri/src/commands/bot_quota.rs`（新文件） | 入参 `{ departmentId }`；无行时返回默认值 + `isDefault=true` |
| `set_bot_department_quota` | 新增 | 同上 | 全量 upsert；逐字段范围校验；失败 `BOT_QUOTA_INVALID` |
| `reset_bot_department_quota` | 新增 | 同上 | 删行，恢复 P3 默认；部门不存在 → `BOT_DEPARTMENT_NOT_FOUND` |
| `list_bot_input_requests` | 新增 | `src-tauri/src/commands/bot_input.rs`（新文件） | 过滤 `conversationId?` / `botId?` / `status?` / `workflowRunId?` |
| `answer_bot_input_request` | 新增 | 同上 | 入参 `{ requestId, answer }`；只接受 pending；投递到活着的 oneshot |
| `cancel_bot_input_request` | 新增 | 同上 | 入参 `{ requestId }`；pending → cancelled，唤醒等待方为“已取消” |
| `agent_respond_ask` | **修改（无签名变更）** | `src-tauri/src/commands/agent/mod.rs:3078` | 若 `askId` 命中 P4 行，额外把行置 answered；返回保持 `Result<(), String>`，通用聊天路径零变化 |

> 命令名一律 snake_case、前端 camelCase DTO；两步注册齐。`list_bot_input_requests` 的 `status` 非法 → `ERR_VALIDATION`（沿用 P3 口径，不新增数字码）。

### 5.2 DTO（camelCase）

```ts
interface BotDepartmentQuotaDto {
  departmentId: string;
  maxConcurrency: number;
  totalTimeoutSecs: number;
  maxSteps: number;
  maxInFlightPerDepth: number;
  stepTimeoutSecs: number;
  maxConcurrentRuns: number;
  isDefault: boolean;      // true = 无配额行，展示 P3 默认
  updatedAt: number | null;
}

interface BotInputRequestDto {
  id: string;
  askId: string;
  conversationId: string;
  botId: string | null;
  workflowRunId: string | null;
  assignmentId: string | null;
  origin: 'person' | 'routine' | 'workflow';
  question: string;
  options: string[];
  status: 'pending' | 'answered' | 'expired' | 'cancelled';
  answer: string | null;
  answeredAt: number | null;
  expiresAt: number;
  createdAt: number;
  updatedAt: number;
}
```

- Rust DTO 落 `axagent-core/crates/harness/src/types/bot.rs`；TS 镜像落 `src/types/bot.ts`；审计脚本按字段白名单核对（P3 同款）。
- `options_json` 在命令层 parse 成 `options: string[]`；前端永不接触 JSON 字符串。

### 5.3 事件

| 事件 | payload | 触发点 |
|---|---|---|
| `bot-input-request-updated`（新增） | `BotInputRequestDto` | 请求落库 `pending`、`answered`、`expired`、`cancelled` 各一次 |
| `agent-ask-user`（既有） | 原 payload | **保持原样**；交互桥继续发，保证通用聊天与旧卡片不回归 |
| 配额事件 | **不新增** | 配额保存后前端主动重取；配额不参与实时刷新 |

- `IpcEventName` 枚举落 `axagent-core/crates/harness/src/event_names.rs:136-139`，`as_str()` = `bot-input-request-updated`；枚举完整性测试的变体清单同步追加。

### 5.4 错误码（建议新增 5，沿用既有 3）

| 码 | zh-CN 源文案 | 触发点 |
|---|---|---|
| `BOT_QUOTA_INVALID` | Bot 部门配额取值不合法 | 任一字段越界/非整数；返回字段名 |
| `BOT_QUOTA_EXCEEDED` | 超出部门运行配额 | 保存/更新定义或触发 run 时超过 `min(定义值, 部门配额)`；detail 带字段与上限 |
| `BOT_INPUT_REQUEST_NOT_FOUND` | 未找到待答请求 | 答复/取消不存在的 id |
| `BOT_INPUT_REQUEST_NOT_PENDING` | 待答请求已回答或已失效 | 请求已是 answered/expired/cancelled |
| `BOT_INPUT_ANSWER_EMPTY` | 答复内容不能为空 | 去空白后为空 |

- **沿用不新增**：`AGENT_INPUT_WAIT_REPLY_TIMEOUT`（现有超时码，`src-tauri/src/commands/error_code.rs:780`）、`AGENT_INPUT_CHANNEL_CLOSED`、`BOT_DEPARTMENT_NOT_FOUND`、`BOT_NOT_FOUND`、`ERR_PERMISSION`。
- 门禁基线：P3 收口实测 485 码；P4 后预计 **490**（以 `check-errorcode-alignment.mjs` 实测为准，不写死）。

### 5.5 契约审计扩展

`scripts/lib/bot-ipc-audit.mjs` 新增 P4 段（仿 `:745-805` 的 `P3_COMMANDS` 结构）：

- 6 条新命令的 `pub async fn` 定义 + `register_commands.rs` 两步注册；
- `BotDepartmentQuota` / `BotInputRequest` 的 Rust/TS 字段一致（camelCase）；
- `bot-input-request-updated` 进入 `IpcEventName` 与完整性清单；
- 5 个新码 × 11 语言；
- `BotInputRequestDto.origin` 值域与 `status` 值域；
- `agent_respond_ask` 仍存在且签名未变（防误删通用路径）。

`check-contracts.mjs --selftest=bot` 至少加 4 条单点破坏自检：删命令、漏注册、DTO 缺字段、事件漏枚举。

### 5.6 i18n 键（建议，最终以冻结的 `I18N-KEYS-P4.md` 为准）

| 前缀 | 示例键 | 约数 |
|---|---|---|
| `bot.quota.*` | `title`、`subtitle`、`inheritHint`、`maxConcurrency`、`maxConcurrencyHint`、`totalTimeout`、`totalTimeoutHint`、`maxSteps`、`maxStepsHint`、`perDepth`、`perDepthHint`、`stepTimeout`、`stepTimeoutHint`、`maxRuns`、`maxRunsHint`、`save`、`reset`、`resetConfirm`、`invalidRange`、`saved` | 20 |
| `bot.input.*` | `pendingTitle`、`pendingBadge`、`question`、`placeholder`、`submit`、`cancel`、`cancelConfirm`、`expiresIn`、`expired`、`expiredHint`、`answered`、`cancelled`、`answerFailed`、`empty` | 14 |
| `error` 段 | 上表 5 码的 11 语言文案 | 5 × 11 |

- 文案约束：`bot.quota.*` 必须明示“只收紧，不会放宽 P3 默认值”；`bot.input.expiredHint` 必须说明“应用重启或超时后不可再作答”，不得暗示可恢复被中断的模型轮。

## 6. 分期拆卡（已冻结）

### 6.1 依赖图与并行度

```text
D0 冻结包（用户裁决 + 5 份规格）─┬→ A1 配额实体/DAO ─→ A2 解析器+runner ─→ A5 e2e/门禁
                                   │        └→ A3 IPC/审计/码 ─→ A4 前端配额面板 ─┘
                                   └→ B1 待答实体/DAO ─→ B2 桥持久化+超时 ─→ B5 e2e/门禁
                                            └→ B3 IPC/事件/码 ─→ B4 前端提问卡 ─┘
```

- 关键路径：**D0 → A1 → A2 → A5** 与 **D0 → B1 → B2 → B5**；两条互不阻塞。
- 并行组：A3 与 A2 可并行（DTO 先冻结）；B3 与 B2 可并行；A4/B4 在各自 DTO 冻结后开工，不等 runner 全绿。

### 6.2 拆卡总表

| ID | 卡 | 负责人 | 依赖 | 是否可立即开工 | 估时（人日） |
|---|---|---|---|---|---|
| **D0** | 冻结包：用户裁决回写 + GAP/ADR/MODEL/IPC/I18N-P4 成稿 + INDEX 登记 | 架构 | 用户裁决 | **已完成**（2026-11-25） | 0.5–1 |
| **A1** | 配额实体/DAO/SCHEMA | 后端 | D0、D1=A、D2=A | 条件可（按建议口径可开工；若 D1=B 则表不同） | 1.5 |
| **A2** | 配额解析器 + workflow/assignment runner 接入 + 校验 | 后端 | A1、D3 | 否（需 D3 定“拒绝/夹紧”） | 2.5 |
| **A3** | 配额 3 命令 + 错误码 + 契约审计 + TS DTO | 后端（门禁复核） | A1、§5 冻结 | 部分（DTO 可随 A1 并行） | 1.5 |
| **A4** | 部门配额面板 + store + i18n | 前端 | A3 DTO | 否（需 A3 DTO 冻结） | 2 |
| **A5** | 配额 Rust 测试 + e2e + 全量门禁 | 门禁 | A1–A4 | 否 | 1.5 |
| **B1** | 待答实体/DAO/SCHEMA | 后端 | D0、Q8≠C | 条件可（建议 A/B 都需要此表） | 1 |
| **B2** | 交互桥持久化 + 超时 + 重启清扫 | 后端 | B1、D5、D6、D7 | 否（超时/终态口径未定） | 2 |
| **B3** | 待答 3 命令 + 事件 + 错误码 + 审计 + TS DTO | 后端（门禁复核） | B1、D7 | 部分（DTO 可先冻结） | 1.5 |
| **B4** | Bot 线程持久化提问卡 + 待答数 + store + i18n | 前端 | B3 DTO、B2 事件 | 否 | 2 |
| **B5** | 待答 Rust 测试 + e2e + 全量门禁 | 门禁 | B1–B4 | 否 | 1.5 |

**合计**：D0 0.5–1 + P4-a 9 + P4-b 8 = **17.5–18 人日**，三人对齐约 **6–7 个工作日**。若 D4 选 B（无人值守 needs_input），B2/B4/B5 共 +3–4 人日。

### 6.3 P4-a 逐卡

#### A1 — 配额实体/DAO/SCHEMA

- **负责人**：后端
- **输入**：D1=A（运行配额）、D2=A（部门覆盖表）裁决结论；`MODEL-P4.md`。
- **输出（落点）**：新增 `axagent-core/crates/entities/src/bot_department_quotas.rs`；`entities/src/lib.rs` 注册；新增 `axagent-infra/crates/dao/src/repo/bot_department_quota.rs`（get / upsert / delete / list）；`dao/tests/schema_compliance.rs` 登记表与唯一索引；`reconcile/extras.rs` 加 `UNIQUE(department_id)`。
- **测试**：建表幂等（reconcile 重跑零 diff）；upsert 幂等；默认值 round-trip；删除部门后行被清（命令层测试）；旧库升级自带空表。
- **门禁**：`cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --lib bot_ && cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --test schema_compliance`
- **依赖**：D0、A1 前置裁决。
- **可立即开工**：**条件可**。若用户选 D1=B（费用配额），本卡作废重写。

#### A2 — 配额解析器 + runner 接入 + 校验

- **负责人**：后端
- **输入**：A1 实体/DAO；P3 常量与 runner 现状；D3 超额处理裁决。
- **输出（落点）**：新增 `src-tauri/src/bot_quota.rs`（纯函数：`resolve_workflow_limits`、`validate_workflow_against_quota`、`QuotaLimits`）；`src-tauri/src/bot_workflow_runner.rs` 改为从 `QuotaLimits` 读取（常量保留为默认）；`src-tauri/src/bot_assignment_runner.rs` 的 `DISPATCH_TIMEOUT_SECS`/`MAX_IN_FLIGHT_PER_DEPTH` 改为默认值，经 `AssignmentContext` 注入生效值；`src-tauri/src/commands/bot_workflow.rs` 在 create/update/run 前校验；`src-tauri/src/commands/bot_department.rs` 删部门时级联删配额；`src-tauri/src/init/services.rs` 的 `task_type=="bot_workflow"` 分支触发前校验。
- **测试**：① 无配额行 → 生效值逐项等于 P3 常量；② 部门配额更小 → 取 min；③ 定义值超配额 → `BOT_QUOTA_EXCEEDED` 且 detail 带字段；④ 部门并发 run 超限 → 拒绝；⑤ 两部门互不影响；⑥ cron 触发超限 → history 失败、不创建 run；⑦ 链每层在途按部门配额生效；⑧ 删部门清配额。
- **门禁**：`cargo test --manifest-path src-tauri/Cargo.toml --test bot_quota --test bot_workflow --test bot_assignment --test bot_delegation_chain`
- **依赖**：A1、D3。
- **可立即开工**：**否**；D3 未定则无法写“拒绝 vs 夹紧”的断言。

#### A3 — 配额命令 + 错误码 + 契约审计 + TS DTO

- **负责人**：后端（门禁复核）
- **输入**：A1；§5 契约冻结。
- **输出（落点）**：新增 `src-tauri/src/commands/bot_quota.rs`（get/set/reset + 范围校验）；`src-tauri/src/register_commands.rs` 三步注册；`src-tauri/src/commands/error_code.rs` 加 2 码；`axagent-core/crates/harness/src/types/bot.rs` 加 DTO；`src/types/bot.ts` 镜像；`scripts/lib/bot-ipc-audit.mjs` P4 段；`src/i18n/locales/*.json` 11 语言；`scripts/check-bot-i18n-keys.mjs` 必需集合。
- **测试**：越界字段逐项 → `BOT_QUOTA_INVALID`；部门不存在 → `BOT_DEPARTMENT_NOT_FOUND`；无行 get 返回 `isDefault=true`；set/reset 后 get 读数正确；审计脚本 P4 段 0 漏。
- **门禁**：`node scripts/check-contracts.mjs --selftest=bot && node scripts/check-errorcode-alignment.mjs && node scripts/check-bot-i18n-keys.mjs && npm run typecheck`
- **依赖**：A1、§5 DTO 冻结。
- **可立即开工**：**部分**（DTO 与 locale 可先落，命令实现待 A1）。

#### A4 — 部门配额面板 + store + i18n

- **负责人**：前端
- **输入**：A3 DTO；现有 `BotDepartmentModal` 体例。
- **输出（落点）**：新增 `src/components/bot/BotQuotaPanel.tsx`（6 个数字输入 + 默认值提示 + 恢复默认）；`src/components/bot/BotDepartmentModal.tsx` 编辑态挂载；`src/stores/feature/botStore.ts` 加 `departmentQuotas` slice（get/set/reset）；`src/types/bot.ts` DTO。
- **路由**：**无新增**；沿用 `/bots` 的部门弹窗（`src/pages/BotsPage.tsx`）。
- **i18n**：`bot.quota.*` 20 键 × 11 语言。
- **测试**：vitest：默认值展示 `isDefault`、越界前端预校验与后端码映射、保存后回显、恢复默认二次确认；不破坏现有部门 CRUD 测试。
- **门禁**：`npm run typecheck && node scripts/vitest-run.mjs run --grep-invert "@heavy"`
- **依赖**：A3 DTO。
- **可立即开工**：**否**（DTO 未冻结）。

#### A5 — 配额 Rust 测试 + e2e + 全量门禁

- **负责人**：门禁
- **输入**：A1–A4。
- **输出（落点）**：`src-tauri/tests/bot_quota.rs`；`e2e/bot-quota.spec.ts`（沿用 P3 shim：`e2e/helpers/p3-tauri-shim.ts` 扩展 6 命令）；`docs/bot-integration/P4/ACCEPTANCE-P4.md`（实现后补实测）。
- **场景**：部门设 1 并发/60s/3 步 → 建 4 步编排被拒；设 2 步 → 建 3 步被拒；恢复默认后通过；两部门配额互不影响；cron 触发超限 → history 失败；UI 默认值/越界/保存回显。
- **门禁**：`npx playwright test e2e/bot-quota.spec.ts --project=chromium --grep-invert "@heavy" --timeout=120000 --reporter=list` + §9.2 全量。
- **依赖**：A1–A4。
- **可立即开工**：**否**。

### 6.4 P4-b 逐卡

#### B1 — 待答实体/DAO/SCHEMA

- **负责人**：后端
- **输入**：D4（Q8 选 A 或 B，非 C）。
- **输出（落点）**：新增 `axagent-core/crates/entities/src/bot_input_requests.rs`；`entities/src/lib.rs` 注册；新增 `axagent-infra/crates/dao/src/repo/bot_input_request.rs`（create / list / transition / expire_stale）；`schema_compliance.rs` 登记全列 + `UNIQUE(ask_id)`；`reconcile/extras.rs` 加唯一索引。
- **测试**：五列索引断言；状态迁移合法/非法；`UNIQUE(ask_id)`；过期清扫批量正确；旧库升级空表。
- **门禁**：`cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --lib bot_ && cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --test schema_compliance`
- **依赖**：D0、D4≠C。
- **可立即开工**：**条件可**（表结构对 A/B 都是超集；建议先落）。

#### B2 — 交互桥持久化 + 超时 + 重启清扫

- **负责人**：后端
- **输入**：B1；D5（超时与动作）、D6（状态表达）、D7（答复投递）。
- **输出（落点）**：`src-tauri/src/commands/agent/mod.rs:71-171` `AppAskUserBridge` 增可选 `request_context`（conversation/bot/workflow_run/assignment），进入阻塞前落 `pending` 行、答复/超时后转终态并 emit；`:2238` 组装处从 `conversations.bot_id` 取上下文；新增 `src-tauri/src/bot_input_request.rs`（行生命周期 helper）；`src-tauri/src/init/services.rs` 启动时 `expire_stale`。
- **测试**：① 交互 Bot 轮提问 → 行 pending + 事件；② 答复 → answered + 事件 + 模型收到答案；③ 300s 超时 → expired + 现有 `AGENT_INPUT_WAIT_REPLY_TIMEOUT`；④ 重启清扫 → pending 全 expired；⑤ 通用聊天（无 Bot）不落行、行为零变化；⑥ 无活 sender 时答复 → `BOT_INPUT_REQUEST_NOT_PENDING`（新命令路径）。
- **门禁**：`cargo test --manifest-path src-tauri/Cargo.toml --test bot_input --test p2_bot_routine_tests`
- **依赖**：B1、D5–D7。
- **可立即开工**：**否**。

#### B3 — 待答命令 + 事件 + 错误码 + 审计 + TS DTO

- **负责人**：后端（门禁复核）
- **输入**：B1/B2；§5 契约冻结。
- **输出（落点）**：新增 `src-tauri/src/commands/bot_input.rs`（list/answer/cancel）；`register_commands.rs`；`axagent-core/crates/harness/src/event_names.rs` 加 `BotInputRequestUpdated`；`error_code.rs` 加 3 码；`harness/types/bot.rs` + `src/types/bot.ts` DTO；`scripts/lib/bot-ipc-audit.mjs` P4 段；11 语言 locale；`check-bot-i18n-keys.mjs`。
- **测试**：过滤组合；答复非 pending → 码；空答复 → 码；cancel 唤醒等待方；事件三态各发一次；审计 P4 段零漏。
- **门禁**：`cargo test --manifest-path src-tauri/Cargo.toml --test bot_input && node scripts/check-contracts.mjs --selftest=bot && node scripts/check-errorcode-alignment.mjs && node scripts/check-bot-i18n-keys.mjs`
- **依赖**：B1、§5 DTO 冻结。
- **可立即开工**：**部分**（DTO/locale 可先落）。

#### B4 — Bot 线程持久化提问卡 + 待答数 + store + i18n

- **负责人**：前端
- **输入**：B3 DTO/事件；现有 `src/components/chat/AskUserCard.tsx` 视觉与交互。
- **输出（落点）**：新增 `src/components/bot/BotInputRequestCard.tsx`（问题、选项、剩余时间、作答/取消、过期态）；`src/components/chat/ChatViewMessages.tsx` 在 Bot 线程渲染持久化卡并隐藏同 `askId` 的通用卡（避免双卡）；`src/stores/feature/botStore.ts` 加 `inputRequests` slice（按 `conversationId` 加载/upsert/终态移除）；`src/pages/BotsPage.tsx` 侧栏待答数徽章；`src/types/bot.ts`。
- **路由**：**无新增**；沿用 `/chat?bot=<botId>`（`src/pages/ChatPage.tsx:37`）与 `/bots`。
- **i18n**：`bot.input.*` 14 键 × 11 语言。
- **测试**：vitest：刷新后从 `list_bot_input_requests` 恢复 pending 卡；answered/expired/cancelled 三终态渲染；空答复禁用提交；同 askId 不双卡；侧栏计数随事件增减。
- **门禁**：`npm run typecheck && node scripts/vitest-run.mjs run --grep-invert "@heavy"`
- **依赖**：B3 DTO、B2 事件。
- **可立即开工**：**否**。

#### B5 — 待答 Rust 测试 + e2e + 全量门禁

- **负责人**：门禁
- **输入**：B1–B4。
- **输出（落点）**：`src-tauri/tests/bot_input.rs`；`e2e/bot-needs-input.spec.ts`（扩展 shim：list/answer/cancel + 事件）；`docs/bot-integration/P4/ACCEPTANCE-P4.md`。
- **场景**：Bot 线程提问 → 卡片出现 → 作答 → 模型继续；刷新页面 → pending 卡恢复；超时 → 过期态；cancel → 取消态；非 Bot 聊天 → 旧卡片与旧命令零变化；重启清扫（Rust 测试）→ pending 全过期。
- **门禁**：`npx playwright test e2e/bot-needs-input.spec.ts --project=chromium --grep-invert "@heavy" --timeout=120000 --reporter=list` + §9.2 全量。
- **依赖**：B1–B4。
- **可立即开工**：**否**。

## 7. 两个遗留项：实现口径与候选方案

### 7.1 Q7 — 按部门配额

#### 7.1.1 建议口径（Q7-A：运行配额，部门覆盖）

P3 的五个预算常量原样保留为**实例默认值**；新增 `bot_department_quotas` 作为**部门级天花板**。生效规则：

```text
effective_max_concurrency      = min(workflow.maxConcurrency, dept.max_concurrency, 4)
effective_total_timeout_secs   = min(workflow.totalTimeoutSecs, dept.total_timeout_secs, 3600)
effective_max_steps            = min(steps.length, dept.max_steps, 12)
effective_in_flight_per_depth  = min(dept.max_in_flight_per_depth, 4)   // 无部门行 = 2
effective_step_timeout_secs    = min(dept.step_timeout_secs, 900)       // 无部门行 = 300
effective_concurrent_runs      = dept.max_concurrent_runs               // 无部门行 = 2
```

- **适用对象**：`owner_scope='department'` 的编排 run；由该 run 派生的步骤与子链继承同一部门配额。
- **不适用**：`owner_scope='instance'` 的编排、P2-a 单 Bot 例程、纯 Bot 对 Bot 委派（保持 P3 常量）。理由：这些对象没有部门归属，强行套部门配额会产生“跨部门链取谁”的歧义（见 D2）。
- **保存/更新定义时**：若定义值超过部门配额，直接 `BOT_QUOTA_EXCEEDED`，不静默改写定义。
- **触发运行时**：再次校验（配额可能在下定义后被调低）；超额拒绝创建 run；cron 触发同样拒绝并记 history 失败。
- **恢复默认**：删配额行；下一次读取 `isDefault=true`，行为回到 P3。

#### 7.1.2 候选对比

| 候选 | 内容 | 成本 | 风险 | 结论 |
|---|---|---|---|---|
| **Q7-A（建议）** | 部门运行配额表 + `min` 生效 + 拒绝超额 | 9 人日 | 需要新增一张表与解析层；验证面集中在 workflow runner | **建议 P4 做** |
| Q7-B | 费用/Token 配额：按部门累计 token/费用并拦截 | 8–12 人日 | 现无 Bot 维度用量账本；`messages` 只有 token 列、无 bot_id（需 join），例程会话还会被清理；跨 provider 计费口径不统一 | **不进 P4**；另立 P5（D1=B 时才考虑） |
| Q7-C | 不做表，只保留常量 | 0 | 直接违背 Q7 原文“按部门配额需新表”；部门管理员无法约束成本/并发 | 不推荐；仅作为“暂不做”选项 |

#### 7.1.3 需要用户裁决

- **D1 配额口径**：运行配额（A）还是费用/Token 配额（B），或两者分两期。
- **D2 归属与作用域**：只做部门覆盖（A）还是实例默认也入库（B）；配额只约束部门编排（建议）还是也约束部门成员参与的纯委派链。
- **D3 超额处理**：拒绝（建议）/ 自动夹紧到上限 / 警告放行。

### 7.2 Q8 — needs_input 交互答应

#### 7.2.1 现状与真实缺口

AxAgent **已有**交互提问能力：`AskUserQuestionTool` → `AppAskUserBridge` → `agent-ask-user` → 前端 `AskUserCard` → `agent_respond_ask`，阻塞 300s。缺的是：

1. **不落库**：待答只在内存，刷新/切会话即丢失；无法从线程重新作答。
2. **不可列举**：没有“待我回答”的列表/计数，侧栏看不到。
3. **超时无终态**：超时只返回错误码，前端没有“已过期”行可渲染。
4. **无人值守路径没有桥**：例程/委派/编排的 `prompters` 为空且未装 `ask_user_bridge`，AskUser 退化为纯文本，不会等待。

因此 Q8 的正确切口不是“新造提问工具”，而是**把既有交互提问变成可持久、可列举、有终态的 Bot 能力**。

#### 7.2.2 候选对比

| 候选 | 内容 | 成本 | 风险 | 结论 |
|---|---|---|---|---|
| **Q8-A（建议）** | 只覆盖**交互 Bot 线程**：待答落库、刷新恢复、可列举、300s 超时、重启清扫；例程/委派/编排保持 P3 现状 | 8 人日 | 无人值守仍无 needs_input，OMB 全量语义未达 | **建议 P4 做**；先解决“有人在场的答应入口” |
| Q8-B | 全量：交互 + 例程/编排/委派 run 进入 needs_input，落库、UI 应答、超时策略、恢复执行 | 11–12 人日（+3–4） | 需要给非交互轮装桥、定义 run 等待态与超时动作；长轮占用并发名额；跨重启无法真恢复 | 若用户要 OMB 全量对齐，选 B；否则 A |
| Q8-C | 保持 P3：不做 needs_input，只记录为已知差距 | 0 | Q8 永久悬空；用户已明确“建议 P4” | 不推荐，除非用户主动降级 |

#### 7.2.3 建议口径（Q8-A）的生命周期

```text
模型调用 AskUserQuestion
  → 生成 askId，落 bot_input_requests(status=pending, expires_at=now+300s)
  → emit agent-ask-user（旧卡兼容）+ bot-input-request-updated
  → 阻塞等待（现有 oneshot，300s）
  → 答复：answer_bot_input_request → status=answered → 唤醒模型
  → 取消：cancel_bot_input_request → status=cancelled → 唤醒为“已取消”
  → 超时：status=expired → 返回 AGENT_INPUT_WAIT_REPLY_TIMEOUT
  → 应用重启：启动清扫把残留 pending 置 expired（不恢复模型轮）
```

- **可见性**：Bot 线程挂载时 `list_bot_input_requests({conversationId, status:'pending'})`；事件增量更新；`/bots` 侧栏显示待答数。
- **答复权限**：建议**不做额外权限门**（本地单用户），但命令层校验 `requestId` 存在且 pending；若未来多用户，再加 owner 校验。
- **消息落库**：提问与答复不写成聊天消息，只以卡片投影；避免污染消息流与 token 统计。

#### 7.2.4 超时与重启口径

- **超时时长**：建议沿用 300s（与现有桥一致），不在 P4 做成可配；若要可配，进 D5。
- **超时动作**：建议 `expired` + 现有错误码，模型收到“未在时限内回答”；**不**自动采用默认答案（避免替用户决策）。
- **重启**：pending → expired，**不续跑**；UI 明示“应用已重启，本次提问失效”。真正的断点续跑需要检查点/恢复机制，明确不在 P4。
- **若 D4=B（无人值守）**：超时动作必须改为“保守终止当前步骤/run”，不能默认继续；且超时时长必须小于 P3 单步 300s / 编排 900s，建议 120s；这会改变 B2/B5 的测试与估时。

#### 7.2.5 需要用户裁决

- **D4 适用面**：只做交互 Bot 线程（A，建议）还是也做例程/编排/委派（B）。
- **D5 超时**：300s 固定（建议）/ 可配 / 无超时；超时动作 = 过期返回错误（建议）/ 使用默认答案 / 终止 run。
- **D6 状态表达**：独立请求表 + run 状态不变（建议）/ 给 assignment/run 增加 `needs_input` 状态（需 supersede P3 口径）。
- **D7 答复投递**：只投递给活着的轮（建议）；无活轮时拒绝 `NOT_PENDING`；不接受“先存答案、下次续跑”。

## 8. 开放项清单（需用户裁决）

| ID | 问题 | 候选 | 建议 | 阻塞卡 | 状态 |
|---|---|---|---|---|---|
| D1 | 部门配额的口径是什么？ | A 运行配额（并发/超时/步数/在途）/ B 费用·Token 配额 / C 两者分期 | **A** | A1–A5 | **已裁决 A**（2026-11-25） |
| D2 | 配额归属与作用域？ | A 只做部门覆盖表 / B 实例+部门通用表 / C 仅部门编排（建议作用域） / D 也约束纯委派链 | **A + C** | A1、A2 | **已裁决 A+C**（2026-11-25） |
| D3 | 超额如何处理？ | A 拒绝并报字段 / B 自动夹紧 / C 警告放行 | **A** | A2 | **已裁决 A**（2026-11-25） |
| D4 | needs_input 覆盖到哪？ | A 只交互 Bot 线程 / B 也覆盖例程/编排/委派 / C 不做 | **A** | B2、B4、B5 | **已裁决 A**（2026-11-25） |
| D5 | 超时与超时动作？ | A 300s + 过期返回错误 / B 可配 / C 默认答案 / D 终止 run | **A** | B2、B5 | **已裁决 A**（2026-11-25） |
| D6 | 待答如何表达状态？ | A 独立请求表，run/assignment 状态不变 / B 增加 `needs_input` 状态 | **A** | B1、B2 | **已裁决 A**（2026-11-25） |
| D7 | 答复投递与重启语义？ | A 只投活轮 + 重启清扫过期 / B 存答案待续跑 | **A** | B2、B3 | **已裁决 A**（2026-11-25） |
| D8 | 发布顺序？ | A 两条都做 / B 先 P4-a 配额 / C 先 P4-b needs_input | **A**（两条独立，可同批） | 全部 | **已裁决 A**（2026-11-25） |

> 裁决记录：用户 2026-11-25 指示“继续集成直到任务结束”，即按建议执行；本表已回写，架构落点见 `ADR-P4.md` 决策对照表。

## 9. 验收条件（§10 式，已冻结）

### 9.1 收口七条

| # | 条件 | 判定证据 | 签字 |
|---|---|---|---|
| ① | D1–D8 全部有用户裁决并回写本文；状态已改 `frozen-for-P4`；冻结包五份文档落盘 | 本文 §8 状态列无“待用户裁决”；`GAP-P4`/`ADR-P4`/`MODEL-P4`/`IPC-P4`/`I18N-KEYS-P4` 存在且 INDEX 登记 | 架构 ✅（2026-11-25） |
| ② | D0 + A1–A5（或 B1–B5，按 D8）全绿且提交链完整；新表 reconcile 幂等；旧库升级零破坏 | `git log --oneline` 提交链；`cargo test ... schema_compliance`；reconcile 重跑零 diff | 待签 |
| ③ | 配额默认路径逐字节等于 P3：无部门行时 2/900/12/2/300 生效；部门配额只收紧 | `bot_quota` 单测逐项断言；P3 `bot_workflow`/`bot_assignment`/`bot_delegation_chain` 全绿 | 待签 |
| ④ | 配额超额可见：保存定义、手工触发、cron 触发三条路径都返回 `BOT_QUOTA_EXCEEDED` 且带字段；两部门互不影响；删部门清配额行 | `bot_quota` Rust 测试 + `e2e/bot-quota.spec.ts` + P3 cron 测试 | 待签 |
| ⑤ | needs_input：Bot 线程提问→落库→刷新恢复→作答/取消/超时三终态；非 Bot 聊天与旧 `agent_respond_ask` 零变化；重启 pending 全过期 | `bot_input` Rust 测试 + `e2e/bot-needs-input.spec.ts`；通用聊天回归 | 待签 |
| ⑥ | P1–P3 既有回归零新增红：`bot_assignment`、`bot_routine`、`bot_delegation_chain`、`bot_workflow`、i18n、契约、e2e 六组 | §9.2 命令逐条 exit 与摘要；`check:serde` 存量红不新增、不扩大 | 待签 |
| ⑦ | 未改 P1–P3 冻结口径；若必须改（如 D6=B 引入 needs_input 状态），已在本文件留修订记录并显式 supersede 对应条款；未 push | `git diff -- docs/bot-integration/P2 P3` 无未记录改动；`git status` 无待推送 | 待签 |

### 9.2 门禁命令原文（WSL ext4）

```bash
# 数据层
cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --lib bot_
cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --test schema_compliance

# P4 新增 Rust 测试
cargo test --manifest-path src-tauri/Cargo.toml --test bot_quota --test bot_input

# P1–P3 回归（不新增红）
cargo test --manifest-path src-tauri/Cargo.toml --test bot_workflow --test bot_assignment --test bot_routine --test bot_delegation_chain
cargo test --manifest-path axagent-runtime/Cargo.toml -p axagent-tools bot_delegate

# 契约与 i18n
npm run typecheck
node scripts/check-contracts.mjs --selftest=bot
node scripts/check-contracts.mjs
node scripts/check-errorcode-alignment.mjs
node scripts/check-bot-i18n-keys.mjs
npm run check:i18n
npm run check:i18n:key-parity
npm run check:routes
npm run check:layers
npm run check:serde   # 存量红（vector_store.rs）保持，不新增

# 前端单测与 e2e
node scripts/vitest-run.mjs run --grep-invert "@heavy"
npx playwright test e2e/bot-quota.spec.ts e2e/bot-needs-input.spec.ts --project=chromium --grep-invert "@heavy" --timeout=120000 --reporter=list
npx playwright test e2e/bot-workflow.spec.ts e2e/bot-delegation-chain.spec.ts --project=chromium --grep-invert "@heavy" --timeout=120000 --reporter=list
```

### 9.3 收口判定

P4 收口需同时满足 ①–⑦；任何一条未签，P4 保持“进行中”，不得写入 `ACCEPTANCE-P4.md` 的 PASS。若 D8 选“只做一条”，则另一条明确标 `deferred`，并把未做项连同原因写进 ACCEPTANCE-P4 的“范围外”节，不能假装全量完成。

## 10. 风险与反转成本

| # | 风险 | 影响 | 缓解 | 反转成本 |
|---|---|---|---|---|
| R1 | `AppAskUserBridge` 在 `block_in_place` 中阻塞；落库若也放进阻塞段，可能死锁/卡 UI | B2 | 进入阻塞前完成落库与事件；答复后单独更新行；禁止在阻塞段 await DB | 低（结构已隔离） |
| R2 | 配额查询进入每次派发热路径，增加 DB 读 | A2 | 在 run 创建时解析一次 `QuotaLimits`，经 `AssignmentContext` task-local 传入；子链不重复查 | 低 |
| R3 | 应用重启后 pending 行残留，用户看到“永远待答” | B2/B4 | 启动清扫 + 前端只显示未过期行 + 过期文案 | 低 |
| R4 | ~~P3 规格文件在工作树有他线在途修改~~（**已消除**：P3 Q1 修订已于 `416094880` 单独提交） | D0 | 实现前确认 HEAD 含 `416094880`；其他线脏文件仍不触碰 | 低 |
| R5 | 错误码/i18n 先写代码后补 locale → 孤儿码红 | A3/B3 | 固化“先 locale 后代码”顺序；`check-errorcode-alignment` 门禁 | 低 |
| R6 | 配额“只收紧”若被实现成“静默夹紧”，用户不知道定义被改 | A2 | D3 选拒绝；测试断言错误码与字段名；前端展示配额上限 | 低 |

## 11. 冻结时需要补齐的文档（D0 的产物）

1. `docs/bot-integration/P4/GAP-P4.md`：Q7/Q8 两条的 A/B/C 判定与 P4 做/不做清单。
2. `docs/bot-integration/P4/ADR-P4.md`：配额生效规则、待答生命周期、超时与重启口径、D6 状态选择。
3. `docs/bot-integration/P4/MODEL-P4.md`：两张新表全列、索引、SCHEMA 登记、删除级联、替代结构。
4. `docs/bot-integration/P4/IPC-P4.md`：6 新命令 + 1 修改、DTO、事件、5 码、审计扩展、门禁原文。
5. `docs/bot-integration/P4/I18N-KEYS-P4.md`：`bot.quota.*` / `bot.input.*` 全部键 + 5 码 × 11 语言译文 + 落盘顺序。
6. `docs/INDEX.md`：P4 六份文档已于 2026-11-25 登记为 `active`（本批提交）。

## 12. 变更记录

| 日期 | 作者 | 变更 |
|---|---|---|
| 2026-11-24 | Bot 集成架构师 | 首版草案：基于 P3 收口台账与 Q7/Q8 原文，给出两条切片的范围、数据模型、IPC、拆卡、候选方案与 D1–D8 待裁决项 |
| 2026-11-25 | Bot 集成架构师 | **冻结**：D1–D8 按建议裁决（A / A+C / A / A / A / A / A / A）；基线更新为 `416094880`；§4/§5/§6/§9 去草案标注；R4 标记已消除；冻结包五份落盘并登记 INDEX |

---

> **本文件已冻结（frozen-for-P4）。** D1–D8 已按建议裁决并回写；实现卡可据此开工。若需改判，先在 §8 与 `ADR-P4.md` 留修订记录，再改 `MODEL-P4`/`IPC-P4`/`I18N-KEYS-P4`。