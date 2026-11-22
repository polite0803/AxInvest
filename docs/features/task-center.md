# 任务中心与调度

> 状态：active ｜ 日期：2026-11-04 ｜ 对应版本：v3.0.0 ｜ 功能点：task-center

## 定位

覆盖「定时/手动把任务跑起来」这一层：任务中心统一运行视图、cron 表达式解析与档期计算、本地时区逐分钟扫描、无头调度（无 GUI 进程也能派发）、`recommendation_cron` 的 CRUD 与池扫描、手动触发按任务类型分流（关联工作流 / 内置 handler / 无 handler 的错误码）、非交易日的「跳过」语义与 `next_run_at` 口径、任务结果交付与告警。

不含：具体业务链的实现（决策、反思、行情取数等归各自功能点）；本功能点只负责「按时触发 + 记录运行状态」。

## 当前行为

### 任务中心统一运行视图

- 后台任务 API：`src-tauri/src/commands/background_tasks.rs:409` 的 `spawn_background_task`、同文件 `:704` 的 `list_background_tasks`、`:722` 的 `get_background_task_output`、`:744` 的 `stop_background_task`。
- 事件流：`:795` 的 `list_task_events`，环形缓冲上限 `MAX_TASK_EVENTS: u64 = 500`（`:785`）。
- 前端入口：`src/components/tasks/JobCard.tsx`（任务卡片；任务中心页按其挂载）。
- 视图收口：v3.0.0「任务中心 v2 统一运行视图（P2-1）」与「五源状态收口（触发来源 + 完成时刻 + 在跑登记）」（CHANGELOG v3.0.0 节；提交 `9bf92bada`）。
- 真完成 vs 写失败：CHANGELOG v3.0.0「任务中心区分『真完成』与『写出失败标记』」（提交 `966e709f9`）。

（对应版本：v3.0.0，当前实现）

### cron 分段构建器与本地时区逐分钟扫描

- 解析在 `axagent-runtime/crates/runtime-core/src/cron_job.rs:323` 的 `calculate_next_run`：
  - 必须 **5 段**，否则返回 `None`（`:324-327`）；
  - 词法门槛：不含 `* / , -` 的表达式无档期语义，直接判为算不出（`:328-331`）；
  - 时区固定 `chrono::Local`（`:333`），秒/纳秒归零后推进到下一分钟（`:334-338`）；
  - 在 `366 * 24 * 60` 分钟窗口内逐分钟匹配五段（`:340-350`），匹配上才返回毫秒时间戳（`:347-348`）。
- 任务结构 `CronJob` 在 `cron_job.rs:55`，持久化 `CronJobStore` 在 `:431`。
- 档期口径统一：v3.0.0「cron 档期口径归一 runtime-core，调度器改为到期即派发」（CHANGELOG v3.0.0 节）。

（对应版本：v3.0.0，当前实现）

### 无头调度与 leader 租约

- 入口：`src-tauri/src/init/services.rs:2133` 的 `start_cron_scheduler`（带 AppHandle）与 `:2148` 的 `start_cron_scheduler_headless`（无 GUI）；公共实现 `:2187` 的 `start_cron_scheduler_opt`。
- 无头化 + 跨进程 leader 租约属 v3.0.0：提交 `d787ebf37`（「调度器无头化 + 跨进程 leader 租约 + 决策否决门运行期可配 + 回放/验证双门禁」）。
- 每个任务执行前显式钉 live 模式：`services.rs:2318`（`cron_live` 用 `task_local` 的显式 `None`，避免定时任务被并发回放的 as-of 栈污染）。
- 交付通道：`services.rs:2262` 构造 `CronDeliverySink`；`:2257` 注释「任务可通过 `CronDeliveryConfig` 把执行结果推送到 webhook / 文件 / 通知渠道」。

（对应版本：v3.0.0，当前实现）

### `recommendation_cron` 的 CRUD 与池扫描

- CRUD：`src-tauri/src/commands/recommendation_cron.rs:111`（create）、`:146`（list）、`:160`（toggle）、`:182`（delete）。
- 工作流常量：`TREND_SCREENING_WORKFLOW_ID = "serenity-screening"`（`:211`）、快速链 `"serenity-screening-fast"`（`:213`）、任务类型 `"trend-screening"`（`:215`）。
- 趋势智选专用 CRUD：`:247`（create）、`:290`（list）、`:304`（toggle）、`:326`（delete）。
- 历史遗留分支（智能荐股）：同上文件 111-182 一组；两套 CRUD 并存，互不替代。

（对应版本：v3.0.0，当前实现）

### 手动触发按任务类型分流

- 入口：`src-tauri/src/commands/scheduled_task.rs:417` 的 `execute_scheduled_task`。
- 内置 handler 白名单：`BUILTIN_TASK_TYPES`（`scheduled_task.rs:134` 起、`:134-157` 逐个登记），对外只读 `builtin_task_types()` 在 `:170`。
- 分流规则（按代码与注释）：
  1. **有关联工作流** → 走工作流执行；
  2. **无工作流但有内置 handler** → 直接调内置 handler（如 `POOL_SCAN_TASK_TYPE`、`MOVER_RECALL_TASK_TYPE`、`STOCK_ANALYSIS_RERUN_TASK_TYPE`、`TIME_TRAVEL_BATCH_TASK_TYPE` 等）；
  3. **两者都没有** → 返回错误码（空壳任务不得静默成功）。
- 白名单必须与生产侧对表：`scheduled_task.rs:165-170` 注释指出「手动执行门直接引用 `BUILTIN_TASK_TYPES`，缺陷只有拿真实 task_type 对表才看得见」。

（对应版本：v3.0.0，当前实现）

### 非交易日「跳过」与 `next_run_at` 口径

- 闸门在 `src-tauri/src/init/services.rs:2341`：只拦「本轮产物依赖当日行情」的任务；`batch-reflection` 等不受影响（`:2360`）。
- 命中非交易日必须 **记 run 并标跳过**：`:2352` 注释「skip 也 record_run：任务列表里如实显示『非交易日，跳过』，静默 return 的…」；日志在 `:2369`、`:2376`。
- `next_run_at` 由 `calculate_next_run` 给出；算不出（非法表达式 / 一年内无匹配）返回 `None`，调度器对 `None` **不派发**（`cron_job.rs:321-323`）。

（对应版本：v3.0.0，当前实现）

### 告警与结果交付

- 交付契约：`axagent-core/crates/harness/src/cron_delivery.rs:40` 的 `CronDeliveryConfig`、`:54` 的 `CronDeliveryChannel`。
- 渠道装配：`src-tauri/src/init/services.rs:5017` 的 `create_cron_delivery_sink`（webhook / 文件 / 通知渠道）。
- 运行结果结构：`TaskRunResult`（`success` / `output` / `error` / `duration_ms` / `executed_at` / `skipped` / `partial`），在 `services.rs:2301-2314` 有构造样例。

（对应版本：v3.0.0，当前实现）

## 关键代码入口

- 后端命令：`src-tauri/src/commands/scheduled_task.rs:417`（手动执行）、`src-tauri/src/commands/recommendation_cron.rs:111`（荐股 cron CRUD）、`src-tauri/src/commands/background_tasks.rs:409`（后台任务）。
- 引擎/调度：`axagent-runtime/crates/runtime-core/src/cron_job.rs:323`（档期计算）、`src-tauri/src/init/services.rs:2089` / `:2104` / `:2143`（调度器启动）。
- 前端：`src/components/tasks/JobCard.tsx`。
- 数据表/迁移：`cron_jobs` / `cron_job_runs`（视图 DTO 在 `axagent-core/crates/harness/src/jobs_store.rs:32`、`:44`）；迁移在 `axagent-infra/crates/migration/`。

## 配置与开关

- cron 语法：5 段 + 必须含 `* / , -`（`cron_job.rs:324-331`）；匹配窗口 366 天（`:340`）。
- 事件上限：`MAX_TASK_EVENTS = 500`（`src-tauri/src/commands/background_tasks.rs:785`）。
- 种子档期：`src-tauri/src/init/state.rs:3049`（00:00 智能荐股 / 01:00 趋势智选 / 02:00 候选池分析 / 18:00 到期反思）。
- 任务类型常量：`src-tauri/src/commands/constants.rs`（`MOVER_RECALL_TASK_TYPE` 等）与 `recommendation_cron.rs:211-220`。
- i18n 前缀：`tasksCenter.*`（`src/i18n/locales/zh-CN.json:7376`）。
- 未发现独立 feature flag；无头模式由启动路径决定（`start_cron_scheduler_headless`），不是配置项。

## 已知限制与坑

1. **`next_run_at = None` 时永不派发** —— 非法表达式或一年内无匹配会让任务静默停摆（`cron_job.rs:321-323`）；排查先看表达式是否含 `* / , -`。
2. **非交易日静默 return 会丢审计** —— 必须 `record_run` 并标 `skipped`（`services.rs:2352`）。
3. **手动执行白名单易漂移** —— 新增 task_type 只加内置 handler 而忘登记 `BUILTIN_TASK_TYPES`，手动执行会落到「无 handler」错误码（`scheduled_task.rs:165-170`）。
4. **定时任务必须钉 live** —— 不包 `cron_live` 时，并发回放会把定时任务取数按别人的 as-of 截断（`services.rs:2264-2283`，2026-09-23 事故的镜像形态）。
5. **cron 表达式兼容性有历史债** —— 见 CHANGELOG v2.9.9「修复 Windows manifest 冲突和 cron 表达式兼容性问题」。
6. **错误码翻译** —— 任务中心运行记录必须按错误码翻译，不能显示后端 JSON 字面量（CHANGELOG v3.0.0「任务中心运行记录按错误码翻译后端串」）。

## 版本变更记录

| 版本 | 日期 | 变更 | 证据 |
|---|---|---|---|
| v2.9.4 | 2026-08-14 | Phase 3-4 需求发现：种子化工作流 / 定时任务 / 前端集成 | `CHANGELOG.md` v2.9.4 节「Phase 3-4 需求发现 - 种子化工作流/定时任务/前端集成」 |
| v2.9.5 | 2026-08-16 | 合并上游 AxAgent master：夜间长时自主任务、任务形态分类、调度器 | `CHANGELOG.md` v2.9.5 节「合并上游 AxAgent master 14 个 commit（夜间长时自主任务/任务形态分类/调度器/滚动条修复等）」 |
| v2.9.6 | 2026-08-20 | 修复调度器命令缺前端消费 | `CHANGELOG.md` v2.9.6 节「修复双层决策LLM兜底未装配与调度器命令缺前端消费」 |
| v2.9.9 | 2026-09-23 | cron 表达式兼容性修复；`start_cron_scheduler` 参数修正 | `CHANGELOG.md` v2.9.9 节「修复 Windows manifest 冲突和 cron 表达式兼容性问题」「修复 start_cron_scheduler 缺少 app 参数导致的编译错误」 |
| v3.0.0 | 2026-10-31 | Cron 构建器收编；任务中心 v2 统一运行视图；五源状态收口；调度器无头化 + leader 租约；档期口径归一 runtime-core、到期即派发；内置任务补跑 | `CHANGELOG.md` v3.0.0 节「时间旅行批次复盘上线，Cron 构建器收编」「任务中心 v2 统一运行视图（P2-1）」「任务中心五源状态收口」「调度器无头化 + 跨进程 leader 租约」「cron 档期口径归一 runtime-core，调度器改为到期即派发」「中断分析回收 + 自动补跑（内置任务 stock-analysis-rerun）」 |

## 相关文档

| 文档 | 状态 | 版本 | 关联 |
|---|---|---|---|
| `AUDIT-gate-and-daemon-impact-2026-10-26.md` | active | v2.9.9 | 门禁与常驻进程影响 |
| `AUDIT-gates-and-headless-scheduler-2026-10-26.md` | done | v2.9.9 | 门禁与无头调度 |
| `AUDIT-tasks-center-status-2026-10-22.md` | active | v2.9.9 | 任务中心状态 |
| `PLAN-b1-daily-pipeline.md` | done | v2.9.9 | 日更管道 |
| `PLAN-async-tracked-slow-commands.md` | active | v2.9.9 | 慢命令异步化 |
| `PLAN-monitor-chain-closure.md` | active | v2.9.9 | 监控链路收口 |
| `PLAN-daily-automation-pipeline.md` | active | v2.9.9 | 每日自动化管道 |
| `AUDIT-task-ledger-monotonic-2026-09-17.md` | active | v2.9.7 | 任务台账单调性 |
| `AUDIT-scheduled-tasks-2026-09-13.md` | active | v2.9.7 | 定时任务审计 |
| `PROPOSAL-nightly-autonomous-task-design.md` | 待评审 | v2.9.5 | 夜间自主任务设计 |
