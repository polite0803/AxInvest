# MODEL-P2B — P2-b 数据模型变更（结论：新增 3 表 + 1 个 JSON 字段）

> 状态：active ｜ 日期：2026-11-23 ｜ 基线：`ee6690c3a` ｜ 前置：`MODEL.md`（P1）、`MODEL-P2.md`、`GAP-P2B.md`
>
> 铁律：`MIGRATIONS = &[]`，只走声明式 reconcile（`bootstrap_schema`：新表走 CreateTable，既有表加列走 AddColumn + DEFAULT）；新增实体文件在 `entities/src/lib.rs` 加 `pub mod` 即自动注册（`dao/build.rs` 扫描，零手工登记）；`schema_compliance.rs` 的 `SCHEMA` 清单**必须**同步登记新表（否则静默缺口）。

## 1. 为什么不复用 `fleets`/`fleet_members`

`fleets` 是「像素办公室」场景模型：`fleet_members.agent_id` 绑 **AgentSession**、`agent_slug` 是 Dispatcher 路由键、`room_id` 是 Phaser 渲染位置、执行器自带「不加载 MCP / PermissionMode::Prompt / 首个可用 provider」的办公室特化取舍（`fleet/executor.rs` 头注）。部门的语义是**对 Bot 的组织归属 + Chief**，双方生命周期与所有权都不同（办公室舰队可暂停/停止，部门是常设组织）。强行复用需把成员主键从 session 改到 Bot，牵动 executor、消息水位、办公室前端——违反「不重建已有能力」的镜像约束：**已有能力也不该被新场景改形**。故新表，与 fleets 并存，ADR-P2B-1 记录裁决与反转成本。

## 2. 新增实体（3 表）

### 2.1 `bot_departments`（部门）

| 列 | 类型 | 约束 | 语义 |
|---|---|---|---|
| `id` | TEXT PK | UUID | |
| `name` | TEXT NOT NULL | | 显示名（1..100） |
| `description` | TEXT | DEFAULT '' | |
| `chief_bot_id` | TEXT NULL | | Chief Bot；FK 语义在命令层校验（bots 表存在且 enabled），**不建 SQL 外键**（与 `conversations.bot_id` 同口径，P1 MODEL §3.2） |
| `metadata_json` | TEXT | DEFAULT '{}' | 预留（共享模板白名单等 P2-c 扩展位） |
| `created_at` / `updated_at` | BIGINT | NOT NULL | Unix 毫秒 |

落点：`axagent-core/crates/entities/src/bot_departments.rs`（新文件）+ `entities/src/lib.rs` 注册。

### 2.2 `bot_department_members`（归属 join 表）

| 列 | 类型 | 约束 | 语义 |
|---|---|---|---|
| `id` | TEXT PK | UUID | |
| `department_id` | TEXT NOT NULL, INDEXED | | |
| `bot_id` | TEXT NOT NULL, INDEXED | | UNIQUE(department_id, bot_id) —— 一个 Bot 只属一个部门（Q2 建议口径；改多对多只需去 UNIQUE 不动表） |
| `joined_at` | BIGINT | NOT NULL | |

落点：`axagent-core/crates/entities/src/bot_department_members.rs`（新文件）。删除级联：命令层（删部门→删成员行；Bot 被删→命令层 `delete_bot` 追加清成员行，GAP G-10 同款扫描），不依赖 SQL 外键。

### 2.3 `bot_assignments`（委派合同，GAP G-B6）

| 列 | 类型 | 约束 | 语义 |
|---|---|---|---|
| `id` | TEXT PK | UUID | assignment id |
| `root_id` | TEXT NOT NULL | | 树根（自链第一单的 id；一层委派时 = 自身 id） |
| `parent_id` | TEXT NULL | | 上游 assignment（P2-b 恒 NULL，字段先落，Q1） |
| `depth` | INTEGER | DEFAULT 1 | P2-b 恒 1 |
| `origin` | TEXT NOT NULL | DEFAULT 'person' | `person` / `bot` / `routine`（例程发起） |
| `requester_bot_id` | TEXT NULL | | person 发起时 NULL |
| `requester_conversation_id` | TEXT NULL | | 回执落回的发起线程 |
| `assignee_bot_id` | TEXT NOT NULL | | 被委派 Bot |
| `mode` | TEXT NOT NULL | DEFAULT 'dispatch' | `consult` / `dispatch` |
| `task_text` | TEXT NOT NULL | | 委派正文 |
| `status` | TEXT NOT NULL, INDEXED | DEFAULT 'queued' | `queued/running/completed/failed/cancelled`（五态，GAP G-B7） |
| `result_text` | TEXT NULL | | completed 时的回执正文 |
| `error_code` | TEXT NULL | | failed 时的错误码（复用错误码族，IPC-P2B §4） |
| `error_detail` | TEXT NULL | | 自由文案 |
| `started_at` / `finished_at` | BIGINT NULL | | 运行窗口（超时判定与展示） |
| `created_at` / `updated_at` | BIGINT | NOT NULL | |

落点：`axagent-core/crates/entities/src/bot_assignments.rs`（新文件）。**不做** `executions`/`evidence` 列（OMB 合同里有，但 P2-b 单次执行语义下 executions 恒 1、evidence 无承载，留 P3 加列即可——声明式加列无迁移成本）。

## 3. 既有表变更

| 表 | 变更 | 说明 |
|---|---|---|
| `bots` | **零变更** | 部门归属在 join 表；`section` 保留原义（GAP G-B1，ADR-P2B-1） |
| `conversations` | **零变更** | 回执 append 进既有 `requester_conversation_id` 线程 |
| `cron_jobs` | **零变更** | 例程发起委派时 `origin='routine'`，prompt 内含委派意图由 requester 轮的工具调用产生，不加字段 |

## 4. schema_compliance 登记（必做，防静默缺口）

`axagent-infra/crates/dao/tests/schema_compliance.rs` 的 `SCHEMA` 清单追加三表全列（对齐 §2.1–2.3 列名/类型：TEXT/BIGINT/INTEGER）。T1'' 的 DoD 含此项。

## 5. reconcile 安全核对

- 3 个新实体文件 → `entities/src/lib.rs` 注册 → `dao/build.rs` 自动提取 → `bootstrap_schema` 生成 CreateTable。
- 无既有表加列（`bots`/`conversations`/`cron_jobs` 全零变更）→ 无 AddColumn NOT NULL DEFAULT 陷阱。
- 旧库升级路径：首次启动即建 3 张空表，零数据迁移；降级无害（多出的表不读）。
- 索引：`bot_assignments.status`、`bot_department_members.department_id/bot_id` 声明 `#[sea_orm(indexed)]`，与 `fleet_members` 同风格。
