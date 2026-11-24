# MODEL-P3 — P3 数据模型变更（结论：新增 2 表 + 既有表加 2 列）

> 状态：frozen-for-P3 ｜ 日期：2026-11-23 ｜ 基线：`2e8db65d6` ｜ 前置：`MODEL.md`（P1）、`MODEL-P2.md`、`MODEL-P2B.md`、`GAP-P3.md`
>
> 铁律沿用 P2：`MIGRATIONS = &[]`；只走声明式 reconcile（CreateTable / AddColumn + DEFAULT）；实体文件加入 `entities/src/lib.rs` 后 `dao/build.rs` 自动注册；`dao/tests/schema_compliance.rs` 的 `SCHEMA` 清单必须同步登记，防静默缺口。

## 1. 总览

| 变更 | 对象 | 目的 |
|---|---|---|
| 新增表 | `bot_workflows` | 编排定义（步骤 DAG + 预算 + 归属） |
| 新增表 | `bot_workflow_runs` | 每次触发的运行实例（状态 + 上下文 + 线程锚点） |
| 加 2 列 | `bot_assignments`：`workflow_run_id` / `step_key` | 把 P2-b 已有 assignment 复用为编排步骤执行记录 |
| 零变更 | `bots` / `conversations` / `cron_jobs` / `bot_departments` / `bot_department_members` |  cron 侧只加 `task_type` 分支与 `params.workflowId`，CronJob 结构不动（P2-a JSON 字段已够） |

## 2. 新增实体

### 2.1 `bot_workflows`（编排定义）

| 列 | 类型 | 约束 | 语义 |
|---|---|---|---|
| `id` | TEXT PK | UUID | |
| `name` | TEXT NOT NULL | 1..100 | 显示名 |
| `description` | TEXT | DEFAULT '' | |
| `owner_scope` | TEXT NOT NULL | DEFAULT 'instance' | `instance` / `department`（ADR-P3-3） |
| `department_id` | TEXT NULL | | 仅 owner_scope=department 必填；FK 语义在命令层校验，不建 SQL 外键 |
| `steps_json` | TEXT NOT NULL | DEFAULT '[]' | 步骤数组，格式见 §2.1.1 |
| `max_concurrency` | INTEGER NOT NULL | DEFAULT 2 | 单 run 就绪步骤并发上限（1..4） |
| `total_timeout_secs` | INTEGER NOT NULL | DEFAULT 900 | run 总超时（60..3600） |
| `created_at` / `updated_at` | BIGINT | NOT NULL | Unix 毫秒 |

落点：`axagent-core/crates/entities/src/bot_workflows.rs`（新文件）+ `entities/src/lib.rs` 注册。

#### 2.1.1 `steps_json` 元素契约（前端与后端共用同一份 TS 类型）

```json
{
  "stepKey": "research",
  "botId": "1111…",
  "promptTemplate": "汇总今日盘面，重点 {{steps.other.output}}",
  "dependsOn": [],
  "timeoutSecs": 300,
  "onFailure": "abort"
}
```

| 字段 | 类型/范围 | 校验 |
|---|---|---|
| `stepKey` | 1..32，`[A-Za-z0-9_-]+` | 定义内唯一，重复 → `BOT_WORKFLOW_STEP_INVALID` |
| `botId` | 存在的 enabled Bot | 定义期 + 运行前各校验一次；部门编排须为当前成员 |
| `promptTemplate` | 1..20000 字符 | 引用只解析 `{{steps.<key>.output}}`；未知 key → `BOT_WORKFLOW_STEP_INVALID` |
| `dependsOn` | stepKey 数组，≤11 | 必须引用已定义 key；自引用/成环 → `BOT_WORKFLOW_CYCLE` |
| `timeoutSecs` | 1..300，缺省 300 | 与 P2-b `DISPATCH_TIMEOUT_SECS` 同上限 |
| `onFailure` | `abort` / `continue`，缺省 `abort` | continue 的失败步骤不计入 completed，其下游被跳过 |

定义期校验是纯函数（无 DB）：`bot_workflow_steps.rs`（新文件）实现 `parse_steps`/`validate_dag`；命令层再查 Bot 存在性与部门成员。步骤数上限 12（含）；并发上限 4（含）。JSON 演进方式沿用 `cron_jobs.data`（新增可选字段不需要 migration）。

### 2.2 `bot_workflow_runs`（运行实例）

| 列 | 类型 | 约束 | 语义 |
|---|---|---|---|
| `id` | TEXT PK | UUID | run id |
| `workflow_id` | TEXT NOT NULL, INDEXED | | 定义 id；删定义时命令层先拒绝在途 run |
| `status` | TEXT NOT NULL, INDEXED | DEFAULT 'queued' | `queued/running/waiting/completed/failed/cancelled` |
| `origin` | TEXT NOT NULL | DEFAULT 'person' | `person` / `routine`（编排 cron 触发） |
| `requester_bot_id` | TEXT NULL | | 预留：未来由 Bot 触发编排；P3 恒 NULL |
| `requester_conversation_id` | TEXT NOT NULL | | 单线程归属：所有步骤回执落这里 |
| `anchor_message_id` | TEXT NULL | | run 起始 System 锚点；步骤回执的 parent_message_id |
| `context_json` | TEXT | DEFAULT '{}' | `{ inputs: {}, outputs: { <stepKey>: { assignmentId, resultText } }, skipped: [] }` |
| `completed_steps` / `total_steps` | INTEGER | DEFAULT 0 | 进度展示，事件 payload 直接带 |
| `error_code` / `error_detail` | TEXT NULL | | 终态失败原因（继承步骤错误码或 `BOT_WORKFLOW_*`） |
| `started_at` / `finished_at` | BIGINT NULL | | 运行窗口 |
| `created_at` / `updated_at` | BIGINT | NOT NULL | |

落点：`axagent-core/crates/entities/src/bot_workflow_runs.rs`（新文件）+ `entities/src/lib.rs` 注册。

**状态语义**：`waiting` = 仍有步骤因依赖未就绪而没派发（不是交互式等待）；`needs_input` 不引入——ask 类操作在 assignee 轮按 P2-a `prompter=None` 自动 Deny。

### 2.3 `bot_assignments` 增量（2 列 + 链字段启用）

| 新列 | 类型 | 语义 |
|---|---|---|
| `workflow_run_id` | TEXT NULL, INDEXED | 编排步骤归属；**子孙继承父行值**，纯委派链恒 NULL |
| `step_key` | TEXT NULL | 对应 `steps_json[].stepKey`；**子孙继承父行值**，纯委派链恒 NULL |

P2-b 既有三列升级为动态树边（ADR-P3-2，用户翻 Q1 案）：

| 既有列 | P2-b 旧语义 | P3 新语义 |
|---|---|---|
| `root_id` | 恒等于自身 id | 恒指向链根 assignment id；子行复制父行 `root_id`，链根仍 `root_id=id` |
| `parent_id` | 恒 NULL | 根派发恒 NULL；子行 = 发起该子轮的 assignee assignment id |
| `depth` | 恒 1 | 根 depth=1；子 depth=parent.depth+1；取值域 **1..3**，`MAX_DEPTH=3` 硬门 |

落点：`axagent-core/crates/entities/src/bot_assignments.rs` 加两列并把头注改为「P3 放开多层 depth≤3，`root_id/parent_id/depth` 是运行期树边；`workflow_run_id/step_key` 沿树继承」。DAO `create_assignment` 接受内部字段 `parent_assignment_id`/`root_assignment_id`（不进 Tauri 入参 DTO），`root_id`/`parent_id`/`depth` 由 DAO 按上表计算，不由调用方自由填写。`root_id` 与 `parent_id` 声明索引支撑链查询（P2-b `status` 索引保留）。

#### 2.3.1 链状态机与防环

- **根派发**（person / routine / 编排步骤 / 非 assignee 轮工具）：`depth=1 / parent_id=NULL / root_id=id`；编排步骤同时写 `workflow_run_id`/`step_key`。
- **子派发**（assignee 轮 `delegate_to_bot`，`depth<3`）：`depth=parent.depth+1`、`parent_id=当前 assignment.id`、`root_id=当前 assignment.root_id`；`workflow_run_id`/`step_key` 继承；`origin='bot'`。`mode=consult` 直接落终态审计行；`mode=dispatch` 走 P2-b 状态机入 queued。
- **防环**：派发前按 `root_id` 查全链，取祖先 `assignee_bot_id` 集合；命中同 Bot → `BOT_DELEGATE_CYCLE`（新增码，直接自派仍用 `BOT_DELEGATE_SELF`）；`depth+1>3` → `BOT_DELEGATE_MAX_DEPTH`（新增码）。静态 `steps_json` 的 `validate_dag` 仍独立拦截定义期环（`BOT_WORKFLOW_CYCLE`）。
- **深度上下文传递**：`run_bot_non_interactive_turn` 增可选 `AssignmentContext { assignment_id, root_id, parent_id, depth, workflow_run_id, step_key }`，用 `tokio::task_local!` 限定在单轮内；`BotDelegateTool` 从 task-local 读取内部字段，LLM 的 JSON 入参只含 `botId`/`question`/`mode`/`requesterBotId`/`origin`，无 root/depth/parent 字段（防伪造）。例程轮传 None，行为不变。
- **子树终态**：`run` 的完成条件为 `workflow_run_id=run.id` 的所有 assignment 均终态（completed/failed/cancelled）；根步骤 completed 但子链在途时 run 仍 `running`。`abort`/`cancel`/总超时按 `root_id` 取消整棵非终态子树。

## 3. 既有表排除理由

| 表 | 为何零变更 |
|---|---|
| `cron_jobs` | 整行 JSON + `task_type`/`params` 已是扩展位；编排触发只需新 task_type 常量与新 params 键 |
| `conversations` | 单线程归属用 run 的 `requester_conversation_id` 字段表达；`bot_id` 保持 P1 单 Bot 语义，多 Bot 不塞进该列 |
| `bots` / 部门双表 | 编排是独立自动化资产；部门只做归属筛选与成员校验，不反向加列 |
| `messages` | 前缀 + `parent_message_id` 足够表达「哪一步、哪个 Bot」，不加新段类型 |

## 4. DAO 与 schema_compliance

- 新增 `axagent-infra/crates/dao/src/repo/bot_workflow.rs`、`bot_workflow_run.rs`（仿 `repo/bot_assignment.rs`：实体↔领域类型 + list/get/create/transition/context patch）。
- `bot_assignment.rs` repo 增 `list_by_workflow_run(run_id)` 与 `create_step_assignment`（复用 `create_inner` 校验，不自动调度）。
- `axagent-infra/crates/dao/tests/schema_compliance.rs`（唯一索引先例见 `axagent-infra/crates/dao/src/reconcile/extras.rs:844-856`）：SCHEMA 清单追加两表全列 + `bot_assignments` 条目追加 `workflow_run_id`/`step_key`；`counts::` 相关断言按实测数调整。
- 旧库升级：首次启动建两张空表 + 给 assignment 加两可空列；历史行两列为 NULL，语义 = 普通委派；降级无害。

## 5. 删除与级联口径（命令层，不建 SQL 外键）

| 触发 | 动作 |
|---|---|
| 删编排定义 | 存在非终态 run → `BOT_WORKFLOW_RUN_BUSY`；否则删定义 + 保留终态 run 作审计 |
| 删部门 | 该部门下编排转 `owner_scope='instance', department_id=NULL`，定义保留（ADR-P3-3） |
| 删 Bot | 编排定义保留；运行前校验发现步骤 Bot 不存在 → run failed + `BOT_WORKFLOW_STEP_BOT_MISSING`；in-flight 步骤及其整棵子树（同 `root_id`）按 P2-b 既有路径 cancelled |

## 6. 并发与超时的落库口径

- run 级并发来自 `max_concurrency`（默认 2），与 P2-b 命令层门（单 requester 4 / 全局 8）**叠加**：编排最多占 2 个名额，超 P2-b 门仍返回 `BOT_DELEGATE_BUSY`（步骤 assignment 的 error_code 继承）。
- 总超时 `total_timeout_secs` 到点：取消所有在途步骤 assignment → run `failed` + `BOT_WORKFLOW_TIMEOUT`。
- 单步超时沿用 P2-b 300s / `BOT_DELEGATE_TIMEOUT`；`timeoutSecs` 只许更小（1..300）。

## 7. 多层委派的链预算与回执口径（P3 翻 Q1 后）

| 预算/口径 | 规则 | 与既有门的关系 |
|---|---|---|
| 每深度并发 | `per(root_id, depth)` 在途 ≤2（新增链门，常量集中 runner 头部） | 与编排 `maxConcurrency`（仅 runner 放行就绪步骤）是**两套独立预算**，叠加生效；步骤根同时占链门 depth=1 与编排门 |
| 单 requester / 全局 | P2-b 4 / 8 保持，超限 `BOT_DELEGATE_BUSY` | 子 assignment 仍经 `create_inner` 校验，不绕过 |
| 链总超时 | 根到子孙共享 300s 预算（不逐层重置），到点整棵链 failed + `BOT_DELEGATE_TIMEOUT` | 编排总超时 900s 是更外层预算，先到者生效 |
| 回执 | 所有子孙写同一 `requester_conversation_id` 线程；`parent_message_id=根 anchor`；正文前缀 `[L{depth} · {stepKey} · {botName}]` | 不新增线程/消息类型/`receipt_message_id` 列；树形关系由 root/parent/depth 字段 + 前端按 `rootId` 过滤呈现 |
| 事件/通知 | 子孙同样发 `bot-assignment-updated`、同样按 assignee 写 `Notification.botId` | 复用 P2-b/P2-a 管线，不新增事件族 |
