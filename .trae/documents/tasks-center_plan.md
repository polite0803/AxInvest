# 系统级任务中心（统一调度收口）实施计划

## 背景与目标

用户结论：时间旅行批次复盘、股票分析管道、当日版每日任务（00:00 荐股 / 01:00 趋势智选 / 池扫描 / 反思等）本质都是「批量任务」，应由系统层面统筹，不应散落在投资中心标题栏 / 业务 tab / 设置页多个入口。

目标：建立**系统级「任务中心」**，一个页面统一呈现并管理：

1. 所有**周期计划任务**（跨投资 / OPC 工作流 / 系统维护 / 自定义）；
2. 投资域**手动批量作业**（股票分析管道、时间旅行批次复盘、多日回测扫描）。

## 仓库调研结论（决定性事实）

### 后端只有一套调度，无需迁移 / 无需合并存储

- 所有定时任务都是 runtime-core 的 `CronJob`，存于唯一 `state.cron_job_store`，经 DAO `CronJobPersistence` 落库（`crates/runtime-core/src/cron_job.rs:47`、`src-tauri/src/init/state.rs:562`）。
- 「系统 ScheduledTask」与「股票 CronJob」不是两套后端：`src-tauri/src/commands/scheduled_task.rs:211` 的 `list_scheduled_tasks` 直接 `cron_job_store.list()` **全量返回**；`pause/resume/delete_scheduled_task`（同文件 :321-341/:310）按 id 通用，不区分 task_type；`execute_scheduled_task`（:343）对 workflow_id 任务跑工作流、对内置 task_type 跑 handler；`get_task_execution_history`（:541）按 id 通用取执行记录。
- DTO `ScheduledTaskDto` 已含：name/description/taskType/cronExpression/status/nextRunAt/lastRunAt/lastResult(success,error,durationMs,skipped)/workflowId/params/config（`scheduled_task.rs:72`）。
- 股票族专用命令（`list_stock_crons` / `toggle_stock_cron` / `create_*_cron` 等）只是**创建参数特化**（period / tPlusNList / workflow 归链 / 个股 stockCode）；启停删除与通用命令等价（`stock_analysis.rs:4036` 起）。
- task_type 全集：
  - 投资：`stock-analysis`（个股）、`stock-recommendation`、`watchlist-scan`、`pool-scan`、`batch-reflection`、`validate-decisions`、`trend-screening`（后两条 serenity 链靠 workflowId=`serenity-screening[-fast]` 归链）；
  - 系统：`backup`、`cleanup`、`daily_summary`、`knowledge_source_fetch_all`、`custom`/`general`；
  - OPC：`opc_demand_scan` 等（workflow_id 非空为主）。
- 内置种子固定 id：`stock-reco-daily`、`trend-screening-daily`、`pool-scan-daily`、`reflection-due-daily`（`stock_automation_cron_seed.rs:29`），种子语义「存在即跳过」——删除后重启不自动重建（UI 需警示）。
- 手动作业后端：管道 `run_stock_pipeline` / `get_pipeline_history` / `get_pipeline_run_detail`（落 `stock_pipeline_runs` 表）；批次复盘 `run_time_travel_batch` / `get_time_travel_batch`（**BatchRunState 仅进程内存，无跨重启历史**）。
- 前端 `src/lib/browserMock.ts` **没有** scheduled_task 系列 mock，浏览器 dev 模式需补。

### 现有前端入口（收口对象）

| 入口 | 位置 | 处置 |
|---|---|---|
| 设置→调度 | `components/settings/SchedulerSettings.tsx`（自定义任务 + backup/cleanup/daily_summary 模板创建 + cron 可视化编辑器） | **保留**（深度编辑/模板创建），顶部加「任务中心」链接 |
| 设置→股票→定时分析 | `components/settings/ScheduledAnalysisTab.tsx`（6 类投资任务的参数化创建/开关） | **保留**（参数化创建），顶部加「任务中心」链接 |
| 设置→股票→定时荐股 | `components/settings/ScheduledRecommendationTab.tsx` | 不动 |
| 独立页 /scheduled-analysis | `pages/ScheduledAnalysisPage.tsx`（ScheduledAnalysisPanel，仅个股 stock-analysis） | 路由重定向到 `/tasks` |
| 投资中心第 7 tab「管道」 | `pages/PipelinePage/PipelinePage.tsx` | 内容迁入任务中心；InvestHub 删除该 tab |
| 标题栏批次 Drawer | `components/invest/InvestHub.tsx`（上一轮新增） | 删除，批次卡片移入任务中心 |
| OPC 页内 cron 管理 | `pages/opc/...`（demand subscription） | 不动（就地参数配置），任务中心只读监控+启停 |

## 最终信息架构

```
侧栏（general 域，紧邻「工作台」）：任务中心 /tasks
├── Tab 1 计划任务（周期）
│    统一表格 list_scheduled_tasks，按组分段/Tag + 分组筛选：
│    [投资自动化] [工作流] [系统维护] [自定义]
│    列：名称/描述 · 分组 · cron · 状态 Switch(pause/resume) · 下次运行
│        · 上次运行（成功/失败/跳过/耗时）
│    操作：立即执行(execute_scheduled_task) · 执行历史 Drawer(get_task_execution_history)
│        · 删除(delete_scheduled_task + Popconfirm 警示内置任务重启不重建)
│    顶部：刷新 / 15s 自动刷新开关；「新建自定义任务」跳设置→调度；
│          投资专用任务（反思档/回测窗口等）给「设置→股票→定时分析」入口
└── Tab 2 手动作业（投资）
     ├── 股票分析管道（复用 PipelinePage 主体：统计/立即执行/暂停继续/进度/历史）
     ├── 时间旅行批次复盘（TimeTravelBatchCard，原样复用）
     └── 多日回测扫描（ReplaySweep，原样复用）

投资中心 InvestHub：标题栏只留 live/as-of（PageTimeAnchor）；业务 tab 回归 6 个
（选股/工作区/跨市场/截图诊断/模拟观察/量化）——「管道」与「时间旅行」都不再是业务 tab
```

世界观（live/as-of）仍归投资中心标题栏；任务中心是系统运维层，两者正交。

## 变更文件清单

### 新增
- `src/pages/TasksCenterPage/TasksCenterPage.tsx`：页面壳（PageHeader + 两个 Tab）。
- `src/pages/TasksCenterPage/ScheduledJobsTab.tsx`：计划任务统一表格（分组、筛选、启停、立即执行、历史 Drawer、删除）。
- `src/pages/TasksCenterPage/ManualJobsTab.tsx`：手动作业 Tab（组合管道主体 + 批次卡片 + Sweep）。
- `src/pages/TasksCenterPage/ExecutionHistoryDrawer.tsx`：执行历史 Drawer（可选并入 ScheduledJobsTab 文件，优先单文件减少碎片）。
- 计划任务分组/状态展示工具（task_type→组 映射表）放 ScheduledJobsTab 同文件或 `src/pages/TasksCenterPage/utils.ts`。
- 对应测试：`src/pages/TasksCenterPage/__tests__/TasksCenterPage.test.tsx`。

### 修改
- `src/lib/pageRegistry.ts`：新增 `"tasks-center": "/tasks"`。
- `src/lib/navRegistry.tsx`：general 区新增任务中心导航项（图标 `ScheduleOutlined`，labelKey `nav.tasksCenter`）。
- `src/types/index.ts`：`PageKey` 联合类型加 `"tasks-center"`（约 :933 区块）。
- `src/components/layout/ContentArea.tsx`：注册 `/tasks` 懒加载路由 + `PageContextProvider`；`/scheduled-analysis` 改为 `<Navigate to="/tasks" replace />`。
- `src/components/layout/AppHeader.tsx`：路径白名单（:70 区域）加 `/tasks`（投资系页面隐藏全局头，按实际渲染定）。
- `src/components/invest/InvestHub.tsx`：
  - 删 `TimeTravelBatchCard`/`Button`/`Drawer`/`FlaskConical`/`useState` 等上轮引入；删批次 Drawer 与按钮，标题栏只留 `<PageTimeAnchor />`；
  - 删 `LazyPipeline` 与 pipeline tab；`InvestTabKey`/`VALID_TABS` 去掉 `"pipeline"`（回归 6 tab）；更新文件头注释。
- `src/pages/PipelinePage/PipelinePage.tsx`：将 `PipelinePageInner` 导出为可复用组件（或新建 `components/tasks/StockPipelinePanel.tsx` 搬运主体），ManualJobsTab 复用；删除 InvestHub 懒加载链后该文件只被任务中心引用。
- `src/components/time-travel/TimeTravelBatchCard.tsx`：挂载位置注释更新（任务中心手动作业 + /replay-workbench 两处）。
- `src/components/time-travel/ReplayWorkbench.tsx`：注释中批次卡入口描述同步。
- `src/components/settings/SchedulerSettings.tsx`、`ScheduledAnalysisTab.tsx`：顶部各加一个「在任务中心统一查看」跳转链接（轻量，不拆功能）。
- `src/lib/browserMock.ts`：补 `list_scheduled_tasks`（[]）、`pause/resume/delete/execute_scheduled_task`、`get_task_execution_history`（[]）mock，保证浏览器模式任务中心可用。
- i18n：`src/i18n/locales/{11}.json`
  - 新增 `nav.tasksCenter`、`tasksCenter.*`（标题、两个 tab、分组名、列名、操作、提示、历史 Drawer 等约 25 key）；
  - zh-CN 为源，其余 10 语言同步；`invest.tab.pipeline` 键确认无其它引用后删除（11 语言）。
  - Python `json.dumps(ensure_ascii=False, indent=2)` + LF 落盘（locale 历史 CRLF，强制 LF）。
- 测试：
  - `ScreenerToWorkspace.integration.test.tsx`：上轮「标题栏批次 Drawer」用例改为——6 个业务 tab、无「管道」、无「时间旅行」、标题栏只有世界观控件；任务中心入口在侧栏/路由。
  - 新增任务中心测试：分组正确（mock list 返回各组任务）、Switch 调 pause/resume、立即执行调 execute、历史 Drawer 调 history、删除带确认；手动作业 Tab 渲染管道/批次卡。
  - SchedulerSettings / ScheduledAnalysisTab 若有快照测试，加链接后同步。

### 不改动（明确边界）
- 后端 Rust：零改动（通用命令已齐备）。
- CronJob 表结构 / migration：不动。
- 时间旅行批次复盘持久化：**本轮不补**（内存态，任务中心只呈现当前轮实时状态；历史诉求后续单开）。
- 各专用「创建/编辑」表单（cron 可视化、反思档/回测参数、OPC 订阅配置）保留在原设置/OPC 页，任务中心不复制第二套编辑实现。
- /replay-workbench 独立页与 BacktestView 时钟入口保留。

## 实施步骤（依赖序）

1. 路由与导航骨架：pageRegistry / types PageKey / navRegistry / ContentArea（含旧路由重定向）/ AppHeader。
2. 页面壳 TasksCenterPage + 两个 Tab，先通路由可达。
3. ScheduledJobsTab：类型复用（从 SchedulerSettings 抽 ScheduledTask 镜像到 `@/types` 单一来源并 re-export，避免重复定义）、分组映射、表格、启停/立即执行/历史/删除、自动刷新。
4. ManualJobsTab：抽出管道主体组件并复用，挂载 TimeTravelBatchCard + ReplaySweep。
5. InvestHub 瘦身：删 Drawer/按钮与 pipeline tab，回归 6 tab + 纯世界观标题栏。
6. 旧入口收尾：/scheduled-analysis 重定向；两个设置面板加跳转链接；注释同步。
7. browserMock 补 mock。
8. i18n 11 语言（新增 + 删 pipeline tab key）。
9. 测试编写/改造。
10. 验证（见下）+ 浏览器实测。

## 关键注意事项

- DTO 字段消费一律 camelCase（`cronExpression`/`nextRunAt`/`lastRunAt`/`lastResult`/`workflowId`）。
- `execute_scheduled_task` 对 custom/general 无执行器任务会报错：这类行「立即执行」置灰或 catch 后按错误码提示，不弹原始串。
- 内置种子任务删除后重启不恢复：删除 Popconfirm 文案必须明示；暂停/恢复为推荐操作。
- antd v6：浮层自定义组件 forwardRef；Drawer 宽度用 `styles.wrapper.width`（`width` prop 已废弃）。
- dprint 必须 `--incremental=false`；locale 强制 LF。
- PowerShell 无 heredoc：Python 脚本写临时文件执行后删除。
- 类型定义禁重复：ScheduledTask/TaskConfig 以 `@/types` 为权威来源，SchedulerSettings 改 import。
- 组件命名导出、i18n 无硬编码、IPC 走 `@/lib/invoke`。

## 验证

- `npm run typecheck`
- `npm run lint:eslint`（oxlint）
- `npx dprint check --incremental=false <改动文件>`
- `npx vitest run` 任务中心新测试 + InvestHub 集成测试 + 相关设置面板测试
- 浏览器实测（1420）：
  1. 侧栏「任务中心」→ /tasks；计划任务 Tab 看到跨域任务（投资 4 个内置 + 其余）分组正确；
  2. 暂停/恢复某任务 → 状态翻转且重刷仍正确；立即执行可跑；历史 Drawer 有记录；
  3. 手动作业 Tab：管道立即执行/历史、批次复盘发起、Sweep 均可用；
  4. 投资中心：6 个 tab、无管道/时间旅行 tab、标题栏仅世界观控件；
  5. /scheduled-analysis 自动跳 /tasks；设置两个面板有跳转链接；
  6. 控制台无红色报错。

## 风险与应对

- **风险 1：分组误判**（workflowId 与 task_type 交叉，如 serenity 链）。应对：先按「投资 task_type 白名单 + serenity workflowId」判投资组，再按 workflowId 非空判工作流组，再次系统组，兜底自定义；单测覆盖边界。
- **风险 2：浏览器 mock 缺失导致白屏**。应对：步骤 7 前置补齐，并在浏览器模式实测。
- **风险 3：删除内置任务**。应对：Popconfirm 强警示；不做硬编码 id 黑名单（避免与 OPC 种子漂移）。
- **风险 4：i18n 量大易漏译**。应对：脚本保序写入，zh-CN 源 + 10 语言一次补齐，typecheck/键扫描兜底。
- **风险 5：管道组件抽出引发引用断裂**。应对：优先「原地导出 + 新页面 import」，不搬文件路径；grep 确认消费者仅任务中心后再清理懒加载。
