# MODEL-P4 — P4 数据模型变更（结论：新增 2 表，既有表零变更）

> 状态：frozen-for-P4 ｜ 日期：2026-11-25 ｜ 基线：`416094880` ｜ 前置：`MODEL.md`（P1）、`MODEL-P2.md`、`MODEL-P2B.md`、`MODEL-P3.md`、`ADR-P4.md`
>
> 铁律沿用 P2/P3：`MIGRATIONS = &[]`；只走声明式 reconcile（CreateTable / AddColumn + DEFAULT）；实体加入 `entities/src/lib.rs` 后 `dao/build.rs` 自动注册；`dao/tests/schema_compliance.rs` 的 `SCHEMA` 清单必须同步登记；唯一/复合索引用 `reconcile/extras.rs` 的 `MULTI_COL_INDEXES` 声明（实体语法表达不了，先例 `extras.rs:844-856`）。

## 1. 总览

| 变更 | 对象 | 目的 |
|---|---|---|
| 新增表 | `bot_department_quotas` | 部门运行配额（6 个上限 + 时间戳），`UNIQUE(department_id)` |
| 新增表 | `bot_input_requests` | 持久化待答请求（四态生命周期），`UNIQUE(ask_id)` |
| 既有表 | 全部零变更 | `bot_departments`/`bot_workflows`/`bot_workflow_runs`/`bot_assignments`/`bots`/`conversations`/`messages`/`cron_jobs` |

## 2. `bot_department_quotas`（ADR-P4-1 / ADR-P4-2）

| 列 | 类型 | 约束 | 语义 / 默认 |
|---|---|---|---|
| `id` | TEXT | PK | UUID |
| `department_id` | TEXT | NOT NULL, UNIQUE, INDEXED | 部门 id；FK 语义在命令层校验（与 `chief_bot_id` 同口径，不建 SQL 外键） |
| `max_concurrency` | INTEGER | NOT NULL, DEFAULT 2, CHECK 1..4 | 单编排步骤并发上限（对应 `bot_workflow_runner.rs:31`，默认 2） |
| `total_timeout_secs` | INTEGER | NOT NULL, DEFAULT 900, CHECK 60..3600 | 单编排总超时上限（对应 `bot_workflow_runner.rs:33`，默认 900） |
| `max_steps` | INTEGER | NOT NULL, DEFAULT 12, CHECK 1..12 | 单编排步数上限（对应 `bot_workflow_runner.rs:35`，硬顶 12） |
| `max_in_flight_per_depth` | INTEGER | NOT NULL, DEFAULT 2, CHECK 1..4 | 链同层在途上限（对应 `bot_assignment_runner.rs:35`） |
| `step_timeout_secs` | INTEGER | NOT NULL, DEFAULT 300, CHECK 30..900 | 单步/链总超时上限（对应 `bot_assignment_runner.rs:33`） |
| `max_concurrent_runs` | INTEGER | NOT NULL, DEFAULT 2, CHECK 1..8 | 该部门同时在跑的编排数上限 |
| `created_at` | BIGINT | NOT NULL | epoch millis |
| `updated_at` | BIGINT | NOT NULL | epoch millis |

**CHECK 上限即“只收紧不放松”的硬边界**：任何列的 CHECK 上限都等于 P3 默认值，配额永远不能把行为放大到 P3 之上（ADR-P4-2）。`max_steps` 的 12 与 P3 runner 上限一致，不可放宽。

**索引**：`UNIQUE(department_id)` 由 `axagent-infra/crates/dao/src/reconcile/extras.rs` 的 `MULTI_COL_INDEXES` 声明；`department_id` 同时作为查询主键列，不另建单列索引。

**DAO**（`axagent-infra/crates/dao/src/repo/bot_department_quota.rs`）：
- `get_by_department(&str) -> Result<Option<BotDepartmentQuota>>`
- `upsert(&BotDepartmentQuota) -> Result<()>`（按 `department_id` 判存在）
- `delete_by_department(&str) -> Result<()>`
- `list_all() -> Result<Vec<BotDepartmentQuota>>`（供启动/审计；不在热路径）

**删除级联**：`delete_bot_department` 同事务删配额行，再按 ADR-P3-3 把该部门编排转 `instance`。不依赖 SQL 外键。

## 3. `bot_input_requests`（ADR-P4-3 / ADR-P4-4 / ADR-P4-5）

| 列 | 类型 | 约束 | 语义 |
|---|---|---|---|
| `id` | TEXT | PK | UUID |
| `ask_id` | TEXT | NOT NULL, UNIQUE, INDEXED | 与 `agent-ask-user` 的 `askId` 一致，用于答复投递与去重 |
| `conversation_id` | TEXT | NOT NULL, INDEXED | 所在 Bot 线程 |
| `bot_id` | TEXT | NULL, INDEXED | 线程归属 Bot（取 `conversations.bot_id`） |
| `workflow_run_id` | TEXT | NULL, INDEXED | 预留（D4=A 恒 NULL） |
| `assignment_id` | TEXT | NULL, INDEXED | 预留（D4=A 恒 NULL） |
| `origin` | TEXT | NOT NULL, DEFAULT 'person' | `person` / `routine` / `workflow`（D4=A 恒 `person`） |
| `question` | TEXT | NOT NULL | 主问题文本（沿用桥的“只取第一个问题”） |
| `options_json` | TEXT | NOT NULL, DEFAULT '[]' | 选项字符串数组的 JSON |
| `status` | TEXT | NOT NULL, DEFAULT 'pending', INDEXED | `pending` / `answered` / `expired` / `cancelled` |
| `answer` | TEXT | NULL | 答复正文 |
| `answered_at` | BIGINT | NULL | 答复时间 |
| `expires_at` | BIGINT | NOT NULL | `created_at + 300_000`（ADR-P4-4） |
| `created_at` | BIGINT | NOT NULL | epoch millis |
| `updated_at` | BIGINT | NOT NULL | epoch millis |

**生命周期**：`pending → answered | expired | cancelled`，终态不可回退。`answer` 只接受 `pending`（否则 `BOT_INPUT_REQUEST_NOT_PENDING`）；`expire` 只对 `pending` 生效；`cancel` 只对 `pending` 生效。

**重启清扫**：应用启动（`src-tauri/src/init/services.rs`）调用 `expire_stale(now)`，把所有遗留 `pending` 置 `expired`，`updated_at=now`，原因写 `answer=NULL`；不恢复被中断的模型轮（`PLAN-P4` §1.3）。

**索引**：`UNIQUE(ask_id)` 走 `MULTI_COL_INDEXES`；`conversation_id`、`bot_id`、`status`、`workflow_run_id`、`assignment_id` 为单列索引（`list_bot_input_requests` 的过滤组合按前两者最常用）。

**DAO**（`axagent-infra/crates/dao/src/repo/bot_input_request.rs`）：
- `create(&BotInputRequest) -> Result<()>`
- `get_by_id(&str)` / `get_by_ask_id(&str)`
- `list(filter: &BotInputRequestFilter) -> Result<Vec<_>>`（conversationId / botId / status / workflowRunId，全部可选）
- `transition(id, from_status, to_status, answer, now) -> Result<bool>`（行级条件更新，返回是否命中）
- `expire_stale(now) -> Result<usize>`
- `count_pending_by_bot(bot_id) -> Result<i64>`（侧栏徽章）

## 4. 既有表排除理由

| 表 | 为何零变更 |
|---|---|
| `bot_departments` | 配额独立生命周期与校验规则；塞 `metadata_json` 失去类型校验/索引/审计，加列让部门记录承担运行策略（职责混杂） |
| `bot_workflows` / `bot_workflow_runs` | 配额是部门级天花板，不是单个定义字段；run 只读生效值，不反向存配额 |
| `bot_assignments` | 待答不是 assignment 第五/第六态（ADR-P4-3）；`ask_id` 关联已够 |
| `messages` | 提问卡是 UI 投影，不是聊天消息；落库表支撑刷新恢复与审计，不污染消息流 |
| `cron_jobs` | 无需新字段；配额校验发生在触发后、run 创建前 |

## 5. SCHEMA 与 reconcile 登记清单（实现线必须全做）

1. `axagent-core/crates/entities/src/bot_department_quotas.rs`、`bot_input_requests.rs` 新增；两文件加入 `entities/src/lib.rs`。
2. `axagent-infra/crates/dao/src/reconcile/extras.rs` 的 `MULTI_COL_INDEXES` 加两条唯一索引声明。
3. `axagent-infra/crates/dao/tests/schema_compliance.rs` 的 `SCHEMA` 清单登记两表全列 + 两条唯一索引。
4. `axagent-infra/crates/dao/src/repo/mod.rs` 导出两个新 repo。
5. 旧库升级：reconcile 自动建空表；无数据迁移、无回填。

## 6. 回滚

两表均为纯增量：回滚 = 删实体注册 + 删 SCHEMA 行（表留在库中不影响旧代码，reconcile 不会 drop）。不触碰既有表，因此 P3 及以前版本可在同库继续运行。
