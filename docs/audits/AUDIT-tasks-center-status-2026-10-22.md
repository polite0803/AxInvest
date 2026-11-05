# AUDIT：任务中心状态模型（2026-10-22）

> 状态：active ｜ 日期：2026-10-22 ｜ 对应版本：v2.9.9


> 范围：任务中心（`/tasks`）五个任务源的「列表取数 → 状态映射 → 终态写入 → 时间口径 → 触发标注 → 刷新」全链路。
> 方法：代码逐文件读 + 现网 PG 读数（2026-10-22 00:10 前后）。只读审计，未改代码。
> ⚠ 文中的 `文件:行号` 以**审计时点**为准；§6 的改动落地后行号会整体漂移，按函数名/符号名检索。
> 背景：连续三轮实机反馈（管道 running 僵尸、单股分析不在列表、手动候选池分析不可见）指向同一类问题：**状态不是从执行实例来的，而是从各种「最近一次快照」拼出来的**。

## 0. 结论摘要

| 级别 | 问题 | 状态 |
|---|---|---|
| P0 | 定时任务的内置长任务在运行期完全不可见（列表只有 `lastResult`） | 已定位，修法待执行（等 pool-scan 跑完再动 Rust） |
| P1 | 单股分析的「结束时间/耗时」用 `updated_at` 反推，会被后续写库污染 | 待修 |
| P1 | 定时任务的历史与「最近运行」没有触发来源，手动执行被标成「定时」 | 待修 |
| P1 | 漏检补分析批次同样没有触发来源，cron 发起的批次被标成「手动」 | 待修 |
| P2 | `interrupted` 的时间旅行批次用回收时刻当完成时刻，伪造耗时 | 待修 |
| P2 | 同一份 cron 历史在「全部运行」抽屉与「计划任务」抽屉里对 partial 判据不同 | 待修 |
| P2 | 分析状态不是封闭词表（存在 `failed: {e}` 这种复合值） | 待修 |
| P2 | 管道的 `run_date` 用 UTC 日期 | 待修 |
| P3 | `attempts` 恒 0、失败分析无原因、同日管道同名、`stale` 徽章无人设置等 | 记录 |

## 1. 五个源的现状矩阵

| 源 | 列表取数 | 运行中可见 | 终态写入 | 时间口径 | 触发标注 | 启动收敛 |
|---|---|---|---|---|---|---|
| scheduled-cron | 每个任务定义的 `lastResult`（`cron.adapter.ts:32`） | **否** | 任务结束时 `record_run` | `lastRunAt - duration` | 恒 `schedule`（`:82`） | 不涉及（无运行行） |
| stock-pipeline | `stock_pipeline_runs` 行 | 是（DB 行） | wrapper 统一收口（本轮已修） | `completedAt - startedAt`；interrupted 不写完成时刻 | 恒 `manual`（`pipeline.adapter.ts:39`） | 有（本轮新增） |
| stock-analysis | `stock_analyses` 根记录 | 是（DB 行） | 工作流完成/失败/取消/超时/回收 | **`updatedAt - createdAt`（不可信，见 P1-1）** | 恒 `manual`（宿主股票源 adapter；该文件已随 #149 第 3 步删除，行号不再可定位） | 有（`scheduler/restore.rs`） |
| time-travel-batch | 批次表行 | 是（DB 行 + 内存） | 批次结束/回收 | `finishedAtMs - createdAtMs`；interrupted 的结束时刻=回收时刻 | 恒 `manual` | 有 |
| mover-recall-analysis | 批次表行 | 是（DB 行 + 内存） | 批次结束/回收 | 同上 | **恒 `manual`（cron 批次也标手动）** | 有 |

## 2. 发现

### P0-1 定时任务的内置长任务在运行期完全不可见

- 证据：`cron.adapter.ts:32-40` 只把每个任务的 `lastResult` 合成一条运行；`execute_scheduled_task` 对内置任务走 `scheduler.trigger_now()`，派发后立即返回（`awaited: false`），handler 在后台跑（`init/services.rs:2724` 的 pool-scan 分支），结束时才 `record_run` 写 `last_result` 与 `cron_job_history`。
- 实机读数：00:05 前后手动触发「候选池分析（内置）」，00:12 前 `cron_job_history` 里 `pool-scan-daily` 最新仍是当天 08:18 那条；列表无行、计划任务页的「最近运行」仍是旧结果。
- 影响：长任务（pool-scan 约 40 分钟、荐股/反思数分钟）在执行期间无任何读数；「活跃任务 2 秒轮询」因为没有 running 行也不会启动。
- 修法（已排期）：`CronJobStore` 增纯内存 in-flight 表（id → 开始时刻 + `manual`/`schedule`），调度器定时派发与 `trigger_now` 两处标记、`record_run` 清除；`ScheduledTaskDto` 增 `runningSince`/`runningTrigger`；适配器据此输出 running 行；手动执行后刷新一次 runs store。

### P1-1 单股分析的结束时间/耗时不可信

- 证据：`stock_analyses` 没有完成时刻列，适配器用 `updatedAt - createdAt` 当耗时（宿主股票源 adapter，该文件已随 #149 第 3 步删除，行号不再可定位），而 `updated_at` 会被后续写库（交易意图、结果回写等）顶掉。
- 现网读数：167 条 completed 根记录中，**109 条（65%）`updated_at - created_at ≥ 24 小时`**，最大 1527 小时；这些行的 `trade_intent_status` 均非空。适配器实际取的窗口（最近 50 行 → 27 条根）里 15 条 completed、**0 条漂移** ⇒ 当前屏幕上看不到，但行一旦被回写就会显示「跑了好几天」。
- 影响：耗时列会在无提示的情况下变错；`finishedAt` 同时决定 `sinceMs` 时间窗过滤与排序。
- 修法（二选一）：
  1. 正解：给 `stock_analyses` 增 `completed_at`，在所有终态写入点（`stock_workflow/core.rs:1082/1113/1183/1624/1932` + 启动回收）落值；历史行 NULL ⇒ 显示「-」。
  2. 过渡：本适配器先不显示结束时间/耗时（`finishedAt`/`durationMs` 留空），避免把「后续编辑时刻」当结束时刻。

### P1-2 定时任务手动执行被标成「定时」

- 证据：`cron_job_history` 与 `TaskRunResult` 都没有触发来源字段。Runs 列表那一行来自 `lastResult`，`cron.adapter.ts:82` 硬编码 `trigger: "schedule"`；手动执行同样走 `record_run` 覆盖 `lastResult`，因此手动跑完后那一行仍显示「定时」。
- 补充：`mapExecutionRecord`（`:113`）也硬编码 `schedule`，但它当前只被测试引用；生产的历史抽屉（`RunDetailDrawer.tsx` 的 `CronHistoryDrawer`）根本没有触发列——历史层面是「缺失」，不是「标错」。
- 影响：`触发方式` 列与筛选器失真；用户无法从列表区分「到点跑的」与「我点的」。
- 修法：`TaskRunResult` / `CronHistoryRecord` 增 `trigger`（`manual`/`schedule`），调度器在 `trigger_now` 与 tick 两处赋值；适配器消费。与 P0-1 的 in-flight 登记共用同一份触发来源。

### P1-3 漏检补分析批次的触发来源同样缺失

- 证据：`moverRecallAnalysis.adapter.ts:43` 硬编码 `trigger: "manual"`；而 `mover-recall-analysis-daily`（每天 06:00）创建的批次在同一张表里没有触发来源列（`request_json` 只有配额/阈值/轮次）。
- 现网读数：2026-10-22 06:00 的批次 `b6384168…` 会被显示为「手动」。
- 修法：给批次记录增 `trigger`（或把 `manual`/`schedule` 写进 request），适配器与详情抽屉同步消费。

### P2-1 interrupted 的时间旅行批次伪造耗时

- 证据：`mark_stale_batches` 在启动回收时把 `finished_at_ms` 写成回收时刻；`timeTravel.adapter.ts:50` 直接拿它减 `created_at_ms` 当耗时。现网 4 条 interrupted 批次的「耗时」为 0.4–1.8 小时，实际是进程存活时长，不是任务时长。
- 口径冲突：管道本轮已改成「interrupted 不写完成时刻」，单股分析也不显示耗时；只有 time-travel（以及同族的 mover-recall，当前无 interrupted 行）还在用回收时刻。
- 修法：interrupted 行不显示耗时/结束时间（或把回收时刻放到单独字段，说明是「回收时刻」）。

### P2-2 同一份 cron 历史，两个抽屉判据不同

- 证据：`RunDetailDrawer.tsx:151` 的 `CronHistoryDrawer` 用 `success ? 成功 : 失败`，忽略 `partial`；`ScheduledJobsTab.tsx:564` 则是 `success ? 成功 : partial ? 部分完成 : 失败`。
- 影响：同一轮「部分完成」的 pool-scan，从计划任务页看是「部分完成」，从全部运行点进去看是「失败」。
- 修法：抽一个共用的结果渲染（或让前者复用后者的判据）。

### P2-3 分析状态不是封闭词表

- 证据：`stock_workflow/core.rs:1932` 把状态写成 `failed: {e}`（错误原文拼进状态列），另有 `timeout` / `cancelled` / `completed` / `interrupted` 等。
- 影响：前端适配器的 `default → failed` 兜住了显示；但任何按 `status === "failed"` 精确匹配的消费方（统计、过滤、回收）都会漏掉复合值。状态列同时承担「状态 + 错误文案」两种职责。
- 修法：状态列只放封闭枚举，错误写 `decision_json`/单独列；或在写入点归一化。

### P2-4 管道的 `run_date` 用 UTC

- 证据：`stock_pipeline/core.rs:228` 用 `chrono::Utc::now().format("%Y-%m-%d")`；它同时作为「执行日期」显示与模板变量 `run_date`。
- 影响：北京时间 00:00–08:00 发起的管道会落到**前一天**；对 A 股「当日」语义是错的。
- 修法：改用本地日期（或明确按交易日口径取）。

### P3 小项

- `attempts` 所有源恒 0（`*.adapter.ts` 的 `attempts: 0`），但表格有「尝试次数」列——看起来像读数，实际是占位。
- 单股分析的列表 DTO 没有失败原因：失败行只能看到「失败」，原因要去工作台。
- 管道同一日多条运行时名称完全相同（`pipeline.adapter.ts:38` = 源名 + `runDate`，08-26 有两条）⇒ 列表无法区分。
- `attentionRuns` 认 `stale` 徽章（`model.ts:182`），但没有任何适配器会设置它——死能力。
- `moverRecallAnalysis.adapter.ts:70` 与 `timeTravel.adapter.ts:70` 的状态映射没有 `default`：服务端出现未登记状态时 `state` 为 `undefined`，标签会渲染成空白/原始键。
- cron 源在「全部运行」里只代表每个任务的最近一次运行；一轮里的多次历史只有点进抽屉才看得到。这是设计边界（`cron.adapter.ts` 文件头有说明），不是缺陷，但要知道它与「运行表」的语义不同。

## 3. 已验证正常的面

- 管道僵尸收敛已在现网生效：`stock_pipeline_runs` 的 4 条 running 已变 `interrupted`、带原因文本、`completed_at` 仍为 NULL（未伪造耗时）。
- 五个状态表当前 `running` 行数均为 0（pipeline / stock_analyses / workflow_executions / time_travel / mover_recall），没有额外的僵尸。
- 单股分析、时间旅行、漏检补分析都有启动回收（`scheduler/restore.rs`），进程退出不会永久 running。
- 管道的未知状态、单股分析的未知状态都默认映射为 `failed`（不再伪装 running）。
- 管道详情抽屉在 `summary` 为空时四个计数显示「-」（不再显示 0）。
- 各源失败用 `Promise.allSettled` 隔离，单源挂掉只产生一条警告，不拖垮整页。

## 4. 建议顺序

1. **P0-1 定时任务 in-flight 可见性**（已排期；同时把 P1-2 的触发来源一起做掉，二者共用同一份登记）。
2. **P1-1 单股分析完成时刻**：先按过渡方案去掉不可信的耗时，再补 `completed_at`。
3. **P1-3 批次触发来源**。
4. **P2-1 / P2-2 / P2-3 / P2-4** 四项口径统一。
5. P3 小项按需清理。

## 5. 待拍板（2026-10-22 已拍）

- P1-1：**补 `completed_at` 列**。历史行 NULL ⇒ 显示「-」；不再用 `updated_at` 反推。
- P1-2/P1-3：**允许改表**。`CronJob` 增 `last_trigger`（随 `cron_jobs.data` 持久化，不扩散到 `TaskRunResult` 的构造点）、`cron_job_history` 增 `trigger`；`mover_recall_analysis_batches` 增 `trigger`。历史行 NULL ⇒ 前端显示「未知」，不再标成定时/手动。

执行批次（等当前 pool-scan 跑完再动 Rust）：P0-1 + P1-1 + P1-2 + P1-3。P2/P3 仍按 §4 的顺序另行处理。

## 6. 执行状态（2026-10-22 凌晨）

已落地（未提交）：

- **P0-1**：`CronJobStore` 增纯内存在跑登记（`RunningJob { started_at_ms, trigger }`，6h 兜底寿命）；
  `CronScheduler` 在定时 tick 与 `trigger_now` 两处标记，`record_run` 摘除；
  `ScheduledTaskDto` 增 `runningSince`/`runningTrigger`；cron 适配器在有登记时输出 running 行
  （不再只读 `lastResult`）；手动执行后前端立即刷新一次 runs store。
- **P1-1**：`stock_analyses` 增 `completed_at`（实体列，声明式对账建列）；timeout/cancelled/
  completed（两条）/failed 五个终态写入点落值；运行中占位行与 interrupted 保持 NULL；
  列表 DTO 增 `completedAt`；适配器只用 `completedAt` 算耗时，NULL 显示「-」。
- **P1-2**：`CronJob` 增 `last_trigger`（随 `cron_jobs.data` 持久化）；`cron_job_history` 增
  `trigger` 列（`CronHistoryRecord` / 实体 / DAO / `ExecutionRecord` 同步）；两个 cron 历史抽屉
  都显示触发列；旧行 NULL → 「未知」。
- **P1-3**：`mover_recall_analysis_batches` 增 `trigger`；手动命令传 `manual`、定时调度传
  `schedule`；适配器按值映射，旧批次 NULL → 「未知」。
- 顺带修了 **P2-2**：`RunDetailDrawer` 的历史抽屉对 `partial` 与计划任务页同一判据（不再显示失败）。

仍未处理：P2-1（time-travel interrupted 的假耗时）、P2-3（状态词表）、P2-4（UTC run_date）、
P3 小项。

**残留缺口（本轮实测）**：进程在任务执行中被杀时，cron 任务**不留任何历史行** ——
`cron_job_history` 仍只在 `record_run`（结束）时写。2026-10-22 00:05 手动触发的候选池分析，
在约 01:02 的 dev 重建里随进程被杀，库里既没有它开始、也没有它结束的记录，任务中心只会
继续显示上一次（08:18）的结果。要让「被杀」也有痕，需要「开始时写一行 `completed_at=NULL` +
启动收敛」，这会重新引入需要收敛的 DB 行 —— 与本轮「纯内存登记、重启自然清空」是两套
取舍，另行拍板。

⚠ 实体新增列走声明式对账：**需要应用重启**才会在库里建出 `completed_at` / `trigger`；
旧数据一律 NULL（显示未知），不回填。
