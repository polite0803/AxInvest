# MODEL-P2 — P2-a 数据模型变更（结论：零 schema 变更）

> 状态：active ｜ 日期：2026-11-23 ｜ 基线：`0f73ec54b` ｜ 前置：`MODEL.md`（P1）、`GAP-P2.md`
>
> 铁律：`MIGRATIONS = &[]`，只走声明式 reconcile（实体 + `dao/build.rs` 注册 + `schema_compliance` SCHEMA 清单）。本文逐项核对后结论：**P2-a 不新增任何表、不新增任何 SQL 列、不触碰 schema_compliance**。

## 1. 结论：cron_jobs.botId 不需要新表

`cron_jobs` 是**整行 JSON 存储**（`id TEXT` + `data TEXT`，data = serde 序列化的完整 `CronJob`）。设计文档（`axagent-core/crates/entities/src/cron_job.rs` 文件头）明说这是为「CronJob 模型频繁演进、免 migration」服务的。因此：

- `bot_id` / `timezone` 是 `CronJob` 结构体上的 **JSON 字段**，加 `#[serde(default)]` 后旧库既有行反序列化自动得 `None`/缺省，声明式机制完全不感知。
- 不能 SQL 级联/索引 JSON 字段是接受的代价：按 Bot 过滤在**命令层**做（`store.list()` 反序列化后 filter），规模是单机桌面级（<10³ 行），可接受；这也是 `delete_bot` 级联必须在命令层扫而非 SQL 删的原因（GAP G-10）。
- 该表**本就不在** `schema_compliance` SCHEMA 清单中（实测 grep=0），无需注册。

## 2. 变更明细

### 2.1 `axagent-runtime/crates/runtime-core/src/cron_job.rs` — `CronJob` 结构体（唯一定义处）

| 字段 | 类型 | serde | 语义 | 缺省 |
|---|---|---|---|---|
| `bot_id` | `Option<String>` | `#[serde(default, skip_serializing_if = "Option::is_none")]` | 例程归属的 Bot id；`None` = 普通（非 Bot）定时任务，走既有分支，行为不变 | `None` |
| `timezone` | `Option<String>` | 同上 | IANA 时区名（`chrono-tz::Tz` 可解析，如 `Asia/Shanghai`）；`None` = 宿主 `Local`，与现状完全一致 | `None` |

**不新增** `CronJobStatus` 变体（三态 `Active/Paused/Disabled` 沿用，一次性 + Disabled = 「Done」，GAP G-7）；**不改** `cron_job_history` 实体（历史行的 `task_id` 已能回查 Bot 例程）。

### 2.2 `calculate_next_run` 签名（runtime-core 同文件 :323）

由 `fn calculate_next_run(expr, from)` 改为 `fn calculate_next_run(expr, from, tz: Option<Tz>)`：`None` → 现行为（`chrono::Local`），`Some(tz)` → 以该时区解释 cron 五段并产出 `next_run_at` 时间戳。所有既有调用点传 `None`（编译器兜底找全）；`record_run` 推进 `next_run_at` 时透传 `job.timezone`。

### 2.3 不变的表（逐个排除）

| 表/实体 | 结论 | 依据 |
|---|---|---|
| `bots` | **不动**。`approval_mode`（`ask\|edits\|auto\|full\|custom`，DEFAULT `'full'`）P1 已建已校验 | `axagent-core/crates/entities/src/bots.rs:34` |
| `agent_sessions` | **不动**。`permission_mode` 是自由 TEXT，P2-a 只在其解析点读 Bot 级别参与映射，不引入新存储值 | `agent_sessions.rs`；`dao/src/repo/agent_session.rs:95` |
| `cron_job_history` | **不动** | GAP G-6 |
| `conversations`（线程归属） | **不动**。P1 已有 `bot_id`；例程轮复用 Bot 归属会话，不新开归属维度 | `MODEL.md`（P1） |
| 通知 | **无后端表**。前端 `Notification` 接口加可选 `botId?: string`（localStorage 语义沿用，上限 50 条不变） | `src/lib/notification.ts:11-29` |

### 2.4 前端类型（非 schema，列此备查）

- `Notification`（`src/lib/notification.ts`）：`+ botId?: string`。
- `ScheduledTaskDto`（`src-tauri/src/commands/scheduled_task.rs` DTO，TS 侧类型由契约脚本生成核对）：`+ botId?: string \| null`、`+ timezone?: string \| null`。

## 3. 反向核对（reconcile 安全性）

- 无新实体文件 → `dao/build.rs` 注册表零变化 → 无孤儿表 DROP 风险。
- 无新列 → `schema_compliance` SCHEMA 清单零变化 → 门禁 `schema compliance` 用例零 diff。
- 旧数据兼容证明路径：既有 `cron_jobs` 行的 `data` JSON 无 `botId/timezone` 键 → serde default 补 `None` → 调度行为与升级前逐字节一致（T1' 单测覆盖「旧 JSON 行反序列化 + 计算档期不变」）。
