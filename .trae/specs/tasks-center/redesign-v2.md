# 任务中心 v2：统一「运行」观测与操作系统（重做方案，待审批）

> 日期：2026-10-12　状态：**待用户审批，批准前不动代码**
> 背景：v1 被否决——仅把设置页定时任务列表搬到 `/tasks`，全应用 26 个任务源只纳管 4 个，且删掉了旧入口。
> 本方案 = P0 已恢复旧面板（B 方案，已完成）后的 P1：参照业界最先进任务系统重做。
> 事实基础：同目录 `task-sources-inventory.md`（26 源 × 8 能力矩阵，全部带代码证据）。

---

## 1. 业界参照（为什么这样设计）

| 系统 | 吸收的范式 |
|---|---|
| **Temporal Web UI** | ① 一个统一的 Workflow Executions 表，按 status / type / 时间过滤，不按来源分页面；② 详情页 = 事件历史 + Pending Activities（卡住的步骤）+ 输入/结果 + Stack trace；③ 操作：Cancel / Terminate / Signal；④ 失败与卡住的执行在一处可见 |
| **Windmill Runs** | ① Runs 页四视图：Live（实时进度）/ Queued（等 worker）/ 24h / All；② 过滤维度 = status + path + 时间；③ 每个 run 有 Inputs & logs、Live Timeline（步骤级实时）、Traces；④ 聚合视图：时间序列点（成功绿/失败红）；⑤ 队列 = `scheduled_for` 排序的 future jobs，独立的 Workers/Schedules 页；⑥ 失败即告警 |
| **GitHub Actions** | ① 跨 workflow 的运行聚合在一个列表，失败优先；② re-run failed jobs / cancel；③ job→step 两级日志 |
| **Celery Flower / BullMQ** | 统一任务状态机（pending/active/success/failed/retry/revoked）+ worker 健康 + 队列深度 + 速率统计 |

**结论（也是 v1 的根本错误）**：先进系统的任务中心不是「某一类任务的 CRUD 页」，而是**跨所有任务源的统一运行（Run）观测面 + 操作面**：
一个列表装下全系统所有 run；每个源用**适配器**接入；中心只理解统一模型；源页面保留业务操作，中心负责「看见一切 + 处理异常 + 跨源操作」。

---

## 2. 统一模型

### 2.1 统一 Run 与状态机（中心唯一理解的语言）

```
UnifiedRun {
  id: string                 // `${source}:${nativeId}`，全局唯一
  source: SourceId           // cron/background/workflow/index-job/...
  name: string               // 适配器给的展示名
  trigger: "schedule" | "manual" | "system" | "external"
  state: RunState
  stage?: string            // 运行中子阶段（如 indexing/embedding、training/validating、analysis/reflection）
  progress?: number          // 0–100，无源给进度则留空
  attempts: number
  queuedAt/startedAt/finishedAt?: number
  durationMs?: number
  error?: { code?: string; message: string }
  badges?: Array<"partial" | "retrying" | "orphan" | "interrupted" | "stale">
  parentRunId?: string       // DAG/父子（workflow node、batch 子链）
  deepLink?: string          // 跳源页面（含参数，如 /demand-discovery?id=）
  capabilities: RunCapabilities   // 该 run 此刻可执行的操作（随状态变化）
}

RunState = "queued" | "running" | "waiting" | "paused" | "succeeded" | "failed" | "canceled"
RunCapabilities = { cancel?; retry?; pause?; resume?; viewLogs?; viewSteps?; viewIO?; viewHistory?; openSource? }
```

各源原生状态 → 统一状态的映射表（适配器内实现，落库不改后端）：

| 源原生状态 | 统一状态 |
|---|---|
| pending / queued / processing 前 / retrying（badge:retrying, attempts+1）/ open | queued（delivery 的 open 映射 waiting） |
| running / processing / training / preparing / validating / in_progress | running（细分进 stage） |
| paused | paused |
| waiting_for_children / blocked / review / 审批挂起 / delivery open | waiting |
| success / completed / completed-failed 混合的 completed | succeeded（partially_completed / partial → succeeded + badge:partial） |
| failed | failed |
| canceled / cancelled / stopped / interrupted（badge:interrupted） | canceled |
| unknown / 孤儿超时 | running + badge:stale（中心标记，不改源数据） |

### 2.2 适配器协议（每源一个文件，中心零源特定逻辑）

```ts
interface TaskSourceAdapter {
  metadata: { id: SourceId; nameKey: string; icon; color; docsKey?: string };
  supports: { list; detail; actions; realtime?: "event" | "poll" };
  list(q: RunQuery): Promise<RunPage<UnifiedRun>>;   // 源命令 → UnifiedRun 映射
  get?(id: string): Promise<UnifiedRun>;
  cancel?(id): Promise<void>; retry?(id): Promise<void>;
  pause?(id): Promise<void>; resume?(id): Promise<void>;
  // 详情四块，按需实现（能力位决定 UI 是否出现）
  getSteps?(id): Promise<RunStep[]>      // DAG 节点 / task_events / pipeline step
  getLogs?(id, tail?): AsyncIterable<RunLogLine> | Promise<RunLogLine[]>
  getIO?(id): Promise<{ input?; output? }>
  getHistory?(id): Promise<UnifiedRun[]>   // 该定义的历次执行（cron 的 50 条等）
  eventTopics?: string[]              // 订阅的 Tauri 事件名
  deepLink(run): string | undefined
}
```

- 中心只通过 `adapterRegistry` 拿适配器；新增任务源 = 新增一个 adapter 文件 + 注册一行，**不改中心代码**。
- 列表加载策略：中心对各适配器 `Promise.allSettled` 并行拉取（默认每源 limit 50），**单源失败只显示该源错误条，不拖垮整页**；合并后按时间排序；源多时分页/虚拟列表。
- 不改后端：盘点证明 A 档源的列表/详情/操作命令已齐备；后端零改动即可完成本次重做。

### 2.3 实时层

- 适配器声明 `eventTopics`，中心订阅 Tauri 事件（已存在：`background-task:created/updated/failed`、`index-job-updated`、`fine-tune-training-progress`、`knowledge-import-progress/completed`、`agent-session-cancelled`、workflow 执行事件），收到事件 → 增量 patch 对应 run。
- 兜底：当前视图存在 running/queued 条目时，每 2 s 轮询相关适配器；无可视运行时停轮询。
- Live Timeline/日志：抽屉打开时按适配器 `getLogs` 流式/短轮询 tail。

---

## 3. 页面信息架构（`/tasks` 重做）

顶部全局工具条：全文搜索（名称/id）· 源 chips（多选）· 状态筛选 · 触发方式筛选 · 时间窗（1h/24h/7d/全部）·「新建任务」下拉（按可创建源列出，含投资管道/时间旅行/回放等手动入口）· 自动刷新开关。

### Tab 1 ｜概览（默认页，新增，v1 完全没有）

- **健康卡四数**：运行中 / 排队中 / 失败（红，跨源失败计数，点击即筛到失败列表）/ 等待处理（审批、blocked）。数据源：各适配器 + `index_jobs_stats`、scheduler `get_task_report`（TaskReport/BudgetState 现成）。
- **需关注清单**：全部源的 failed + waiting + 运行超阈值（stale）条目，一行一条，行内操作（重试/取消/继续）——这是"系统级收口"的核心：以前散落在 6 个页面的异常在此一处处理。
- **进行中 Live 区**：running run 卡片，进度条 + stage + 已运行时长 + 取消按钮（Windmill Live）。
- **近 24h 时间序列**：成功/失败点聚合（Windmill 聚合视图），点击点下钻到该时段 run。
- **队列与 worker**：index 队列深度/worker 并发、后台任务启动恢复状态（纯观测，不做编辑）。

### Tab 2 ｜全部运行（核心，取代 v1 的「定时任务列表 + 两张手动卡」）

- 统一表格列：状态 · 名称 · 源（彩色 chip）· 触发方式 · 进度/阶段 · 开始时间 · 耗时 · 尝试次数 · 行内操作（按 capabilities）。
- 排序、过滤、搜索均跨源；「投资管道/批次复盘」不再有专属分区，只是 `source=stock-pipeline / time-travel-batch` 的行（满足用户"按中心标准重做"）。
- 点击行 → **统一运行详情抽屉**（RunDetailDrawer）：
  - 头部：名称、源 chip、状态、时间线、**操作按钮组按 capabilities 动态出现**（取消/重试/暂停/恢复/在源页面打开）；
  - Tabs（按能力出现）：**步骤 Steps**（DAG/事件时间线）· **日志 Logs**（tail 流式）· **输入/输出**（结果 JSON/产物链接）· **历史**（同定义历次执行）。
- 无持久化的源（replay-sweep、evaluator 同步任务）不进运行表，只留在「新建任务」菜单。

### Tab 3 ｜计划任务（现有 ScheduledJobsTab 平移）

- 内容 = 现 v1 的「定时任务」全套（四组新建、cron builder、执行历史），数据源不变（CronJobStore）。
- 它在模型里是「schedule 触发器的 run 定义」，与 Tab2 中 `trigger=schedule` 的 run 互链（run 历史 ↔ 定义）。
- **旧设置面板 SchedulerSettings 原样保留**（P0-B 已恢复），两处入口操作同一后端，互不拆改；后续视用户反馈再决定是否在旧面板顶部加一条「任务中心」链接（本次默认不加，保持"原样"）。

---

## 4. 纳管范围（分批，对齐盘点报告 A/B/C 档）

**本期（P2）做 A 档——全部满足「有列表+详情+持久化、后端零改动」：**

| 批次 | 源 | 备注 |
|---|---|---|
| P2-1 | scheduled-cron（迁为适配器，Tab3 保持）、stock-pipeline、time-travel-batch | 现有 3 源迁移；手动卡片区删除，replay 降为新建菜单项 |
| P2-2 | **background-tasks**、**workflow-dag** | 能力最全的两源：取消/暂停/恢复/recover/步骤/日志全有 |
| P2-3 | **index-jobs** | 7 命令后端已建好、前端零调用：列表/stats/取消/重试/清理一键接上，性价比最高 |
| P2-4 | fine-tune-jobs、opc-demand-delivery、device-sync、fleet、dream/evolution | 列表+各自已有操作 |

**接线顺带修复（不新增后端能力，只改消费方）**：知识库目录导入从同步版切到已存在的 async/cancel/status/retry 四命令（盘点 3.1），然后以适配器纳管（纯内存历史，重启即失，可接受）。

**本期不做（另立 spec）**：
- B 档（需补后端命令/列表）：quant-backtest、wiki-sync-queue、tool-executions（只读审计流，可作会话下钻）、opc-work-items、backup；
- C 档（纯内存、需先补持久化）：parallel-execution、rl-training、sub-agent；
- 明确不纳管：skill 统计（聚合非实例）、evaluator/replay（同步返回）、reminders（领域对象）、market-sweeps（无运行记录）、scheduler-coordination（编排层，仅向概览供数）、agent-bus（**盘点已证伪：仓内不存在**）。

---

## 5. 代码结构

```
src/pages/tasks-center/
  model.ts                     # UnifiedRun/RunState/RunQuery/RunStep/RunLogLine 类型 + 状态映射工具
  adapters/
    registry.ts                # TaskSourceAdapter 注册 + allSettled 聚合 list/get
    cron.adapter.ts
    pipeline.adapter.ts
    timeTravel.adapter.ts
    backgroundTask.adapter.ts
    workflow.adapter.ts
    indexJob.adapter.ts
    fineTune.adapter.ts
    opcDelivery.adapter.ts
    deviceSync.adapter.ts
    fleet.adapter.ts
    evolution.adapter.ts
    knowledgeImport.adapter.ts
  store/taskCenterStore.ts      # zustand：filters、聚合 runs、健康计数、事件订阅+轮询
  components/
    OverviewTab.tsx
    RunsTab.tsx
    SchedulesTab.tsx           # 现 ScheduledJobsTab 平移
    RunDetailDrawer.tsx
    RunStatusTag.tsx/SourceChip.tsx/ProgressCell.tsx/HealthStatCards.tsx/TimelineSeries.tsx
  TaskCenterPage.tsx
```

- 现 `src/pages/TasksCenterPage/` 目录改名为小写连字符 `tasks-center/`（与其它目录风格一致）；`TaskCreateModal`、`CronScheduleBuilder`、forms/* 原样保留继续服务 Tab3。
- 类型从 `@/types` 进出（adapter 内部类型可本地）；IPC 一律走 `@/lib/invoke`，camelCase；`ExecutionRecord` snake_case 例外只在 cron adapter 内消化。
- 现有 `pipelineStore` 等业务 store 不拆不重写，adapter 直接调 invoke；后续可让 store 复用 adapter 的 normalize 函数（禁区 12：normalize/映射表在 model.ts 单点定义）。
- 浏览器 mock（browserMock）为每个新接入命令补 case。
- i18n：所有新文案进 11 语言；源名称/状态名/健康卡/空状态/错误条全走 t()。
- 旧组件删除零引用前 grep 确认（ManualJobsTab、PipelineJobCard、BatchRunDetailDrawer 等在新抽屉落地后再删，功能在新 RunDetailDrawer 中等价覆盖）。

## 6. 与既有页面的关系

- **不拆任何业务页面**：投资中心、OPC、知识库、微调、多智能体等页面保留全部业务操作；任务中心给的是跨源观测面 + `deepLink` 深链回源。
- 聊天侧面板 TaskPanel（background-tasks 现有消费方）保留；二期可让它直接复用 backgroundTask.adapter 的渲染件（统一 RunStatusTag 等），不反向依赖中心页面。
- 设置旧 SchedulerSettings 面板：保留不动。

## 7. 验收标准（演示剧本）

1. 打开 `/tasks` 落在概览：健康卡数字与后端一致；手动造一个 index job 失败 + 起一个后台 shell 长任务 + 起一个 workflow，三者都实时出现在「需关注/Live」，无需进各自源页面。
2. 全部运行表：源 chips 只选「知识库索引」只见 index_jobs；筛选 failed 只见失败；点 workflow run 行打开抽屉，Steps 节点树、Logs、IO、历史四 tab 齐，操作出现 暂停/恢复/取消/recover；点「在源页面打开」跳 `/workflow`。
3. 投资管道/批次复盘在表中作为普通行出现，详情抽屉内步骤/结果可见；页面上再无专属卡片分区。
4. 杀掉一个适配器模拟源（mock reject）：列表显示该源错误条，其余源与另两 tab 正常。
5. 计划任务 tab 功能不回归；设置→数据→定时任务旧面板完好（P0 已验）。
6. 11 语言无缺键；typecheck / dprint / oxlint / vitest / build / 全量 clippy（本期预期零 Rust 改动，无需全量 test）全绿。

## 8. 分期任务（批准后拆 tasks.md）

- **P2-1 框架+迁移**：model/registry/store + 三 tab 骨架 + cron/pipeline/timeTravel 三适配器 + 统一详情抽屉（覆盖现有手动卡全部功能），删旧分区。
- **P2-2 两强源**：background-tasks、workflow-dag 适配器（步骤/日志/操作全量）+ 事件订阅实时刷新。
- **P2-3 索引队列**：index-jobs 适配器 + 概览健康卡/队列区接线。
- **P2-4 余源 + 收尾**：fine-tune/opc-delivery/device-sync/fleet/evolution/knowledge-import（含切异步版）；i18n 11 语、全门禁、浏览器逐源验收。

## 9. 明确不做的事

- 不在本期末尾做 B/C 档（需要新增后端命令/持久化，避免范围爆炸）；
- 不做跨源任务依赖编排/DAG 编辑器（那是 workflow 编辑器的职责）；
- 不做告警外发（Slack/邮件），仅中心内可视化与徽标；
- 不做运行记录自动清理（既定 Q2：只 limit 展示，不清理数据）；
- 不动旧设置面板、不动各业务源页面。
