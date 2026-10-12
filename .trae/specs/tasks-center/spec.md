# 系统级任务中心 — 需求规格（spec）

## Overview

- **Summary**：在应用侧栏新建系统级「任务中心」（`/tasks`），作为全应用**唯一的批量作业与计划任务控制面**：统一管理投资自动化、OPC 需求作业、系统维护（备份/清理/摘要）、自定义工作流四类计划任务，以及投资域三个手动批量作业（股票分析管道、时间旅行批次复盘、多日回测扫描）。同步完成：时间旅行批次复盘的持久化与运行历史、管道 UI 按任务中心标准重做、设置页股票业务项清退、投资中心业务 tab 瘦身。
- **Purpose**：用户判断「时间旅行批次复盘、管道、当日版每日任务本质都是批量任务，应在系统层统筹」。现状是同一套后端调度（`CronJobStore`）被 5+ 个前端入口割裂（设置→调度、股票设置弹窗两个定时 tab、OPC 需求页、投资中心管道 tab、投资中心标题栏 Drawer），且批次复盘无历史、重启即丢。
- **Target Users**：AxInvest 桌面端使用者（个人投资者 + 系统运维角色同一人）。

## Goals

- G1：任务中心成为计划任务的**唯一管理面**：一个列表看全跨域任务，统一分组、启停、立即执行、执行历史、删除、创建入口。
- G2：投资域手动批量作业在任务中心有统一的发起与历史体验；管道 UI 脱离「单工作流页面」形态，重做符合任务中心的作业卡 + 历史表标准。
- G3：时间旅行批次复盘状态落库，重启后可查看历史批次与结果。
- G4：设置页不再承载任何股票业务任务项；股票分析设置弹窗回归「纯分析配置」。
- G5：OPC 全部定时任务（需求发现日/周报、需求订阅扫描、任何挂 workflow 的 OPC cron）在任务中心可见可控。
- G6：投资中心回归纯业务，标题栏只保留 live/as-of 世界观。

## Non-Goals

- N1：不做任务运行中的**崩溃续跑**（as_of 栈是进程级的，无法跨进程恢复封闭世界；只做终态留痕与僵尸状态收敛）。
- N2：不合并/改动 CronJob 后端存储模型与调度执行器（后端本就是一套，无需迁移）；不改各 cron 的业务执行逻辑。
- N3：不把 OPC 任务的专有配置（delivery 投递、platform、enabled_toolsets、需求订阅参数）编辑搬进任务中心——这些仍在 OPC 页面就地配置。
- N4：不改造数据型仪表盘（用量/成本 Dashboard、插件 dashboard 面板、OPC/跨市场行情看板）。
- N5：不动 `/replay-workbench` 独立工作台与 BacktestView 时钟入口（单股深度复盘链路保留）。
- N6：不做多用户、权限、任务编排 DAG、失败告警通知（本轮只保留任务既有的 notification 配置不丢失）。

## Background & Context

### 后端事实（调研实证）

1. **调度只有一套**：所有定时任务都是 runtime-core `CronJob`（`crates/runtime-core/src/cron_job.rs:47`），存于唯一 `state.cron_job_store`，经 `CronJobPersistence` 落库（`src/init/state.rs:562`）。
2. **通用命令已覆盖大部分面**：`list_scheduled_tasks`（`commands/scheduled_task.rs:209`，全量 `store.list()`）、`get_scheduled_task`、`create/update/delete_scheduled_task`、`pause/resume_scheduled_task`、`execute_scheduled_task`（:343，workflow 任务走 work_engine、内置 task_type 走 handler）、`get_task_execution_history`（:541）。DTO `ScheduledTaskDto`（:72）含 taskType/cronExpression/status/nextRunAt/lastRunAt/lastResult{success,error,durationMs,skipped}/workflowId/params/config。
3. **股票族命令只是创建参数特化**：`list_stock_crons/list_pool_scan_crons/list_batch_reflection_crons/list_trend_screening_crons/list_decision_backtest_crons/list_watchlist_scan_crons/list_recommendation_crons` + 对应 create/toggle/delete（`commands/stock_analysis.rs`、`stock_recommendation.rs` 等）。4 个内置种子固定 id：`stock-reco-daily`/`trend-screening-daily`/`pool-scan-daily`/`reflection-due-daily`（`init/stock_automation_cron_seed.rs:29`），存在即跳过，删除后重启不重建。
4. **OPC 任务**：种子 `opc-demand-discovery-daily`、`opc-demand-discovery-weekly`（workflowId=`opc-demand-discovery`，taskType=`opc-demand-discovery`，`commands/opc_setup/seed_opc_cron.rs:15`）；需求订阅扫描 taskType=`opc_demand_scan`，默认 `0 */6 * * *`，带 delivery/platform/toolsets，由 `opc_ensure_demand_scan_job` 幂等创建（`commands/opc_demand_subscription.rs:36,248`）。全部在同一 CronJobStore，list 通用命令即可见。
5. **手动作业**：管道 `run_stock_pipeline`/`get_pipeline_history`/`get_pipeline_run_detail`，历史落 `stock_pipeline_runs` 表（`crates/entities/src/stock_pipeline_runs.rs`，标量列 + 4 个 JSON 列）；批次复盘 `run_time_travel_batch`/`get_time_travel_batch`，`BatchRunState`（`commands/time_travel_batch.rs:93-169`，字段：batchId/asOf/tradingDaysAgo/status/phase/message/total/ok/fail/recoPicks/serenityPicks/serenityError/analyses[]/horizons[]/reflectionSummary/errorCode/errorDetail/createdAtMs/updatedAtMs/finishedAtMs）存在进程内 `static BATCH_RUNS: HashMap`（:47），重启即丢。
6. **建表机制**：版本化 MIGRATIONS 已清空，DAO 启动跑声明式 reconcile——在 `crates/entities/src/lib.rs` 加 `pub mod xxx;` 即自动建表（`crates/dao/build.rs` 扫 lib.rs 生成实体清单，schema_diff 对比 + `CREATE TABLE IF NOT EXISTS` 收敛）。新增持久化表**不需要写 migration 文件**，照 `stock_pipeline_runs` 加 entity 即可。

### 前端入口现状（收口对象）

| 现状入口 | 位置 | 处置 |
|---|---|---|
| 设置侧栏 data→调度 | `components/settings/SchedulerSettings.tsx`（自定义任务 CRUD、cron 可视化 builder、backup/cleanup/daily_summary 模板、执行历史） | 能力整体迁入任务中心；设置侧栏该 section 移除 |
| 个股分析设置弹窗→定时分析 | `ScheduledAnalysisTab.tsx`（6 类投资任务参数化创建/启停/删除） | 迁入任务中心；弹窗删除该 tab |
| 个股分析设置弹窗→定时荐股 | `ScheduledRecommendationTab.tsx` | 迁入任务中心；弹窗删除该 tab |
| 设置侧栏 data→模拟观察 | `PaperOverviewPanel.tsx`（SettingsPage:331 挂载） | 移入投资中心「模拟观察」业务 tab |
| 投资中心第 7 tab「管道」 | `pages/PipelinePage/PipelinePage.tsx` | 重做后并入任务中心；InvestHub 删除该 tab |
| 投资中心标题栏批次 Drawer | `components/invest/InvestHub.tsx`（上上轮产物） | 删除 |
| OPC 需求发现页 cron 配置 | `pages/DemandDiscoveryPage.tsx:754`（ensure scan job + cron 输入） | 保留（专有配置就地）；任务中心提供监控与跳转 |
| 孤儿页 `/scheduled-analysis` | `pages/ScheduledAnalysisPage.tsx`（无任何路由引用，PageKey 死键） | 删除页面与死键 |

### 与现有仪表盘的系统梳理（边界结论）

| 面板 | 性质 | 本轮 |
|---|---|---|
| DashboardPage `/dashboard`（用量/成本/网关/office） | **数据看板**（指标可视化） | 不动 |
| 插件 dashboard panels（PluginPanelHost + dashboardPlugins 设置） | **插件数据面板** | 不动 |
| OPC dashboard、CrossMarketDashboard | **域数据看板** | 不动 |
| PaperOverviewPanel | 业务面板错放在设置 | 移到投资中心模拟观察 |
| **任务中心（新建）** | **作业控制面**（计划/发起/状态/历史），不展示业务指标 | — |

原则：任务中心是「控制面」，只呈现作业自身的状态（跑没跑、成败、耗时、产物摘要入口），不复制任何业务指标看板。

## Functional Requirements

### FR-1：导航与页面骨架

- FR-1.1：侧栏 general 区新增「任务中心」导航项（图标 Schedule，紧邻工作台/会话），路径 `/tasks`，11 语言 label。
- FR-1.2：`/tasks` 页面结构：PageHeader（标题+说明）+ 顶层两个 Tab：**计划任务**、**手动作业**。
- FR-1.3：投资中心标题栏移除批次复盘按钮与 Drawer，仅保留 `<PageTimeAnchor />`；业务 tab 由 7 个减为 6 个（删除 pipeline）。
- FR-1.4：旧 `?tab=replay`、`?tab=pipeline` 参数静默回退选股 tab，不报错。

### FR-2：计划任务 Tab（统一管理面）

- FR-2.1：进入即调 `list_scheduled_tasks` 拉全量，按 taskType/workflowId 归入四个分组并提供分组筛选：**投资自动化**（stock-recommendation/trend-screening/pool-scan/watchlist-scan/stock-analysis/batch-reflection/validate-decisions 及 serenity workflow）、**OPC 需求**（taskType 前缀 `opc` 或 workflowId 前缀 `opc-`）、**系统维护**（backup/cleanup/daily_summary/knowledge_source_fetch_all）、**自定义/其他**（custom/general/工作流兜底）。
- FR-2.2：表格列：名称+描述、分组 Tag、cron（人话描述+原始表达式）、状态 Switch（active/paused → resume/pause 通用命令）、下次运行、上次运行（成功/失败/跳过 Tag + 相对时间 + 耗时）。
- FR-2.3：行操作：立即执行（`execute_scheduled_task`；无执行器的 custom/general 置灰并 tooltip 说明）、执行历史（Drawer 调 `get_task_execution_history`，展示每次开始/耗时/成败/错误）、删除（Popconfirm，内置固定 id 任务额外警示「删除后重启不会自动重建，建议暂停」）。
- FR-2.4：顶部操作：刷新、15s 自动刷新开关（页面卸载时清理定时器）、「新建任务」下拉：自定义任务（Drawer/Modal 内嵌现有 `CronScheduleBuilder` + workflow 选择 + TaskConfig：runOnStartup/retry/timeout/notification）、投资任务（弹出迁入的 6 类参数化创建表单）、系统维护任务（backup/cleanup/daily_summary 模板快捷创建）。
- FR-2.5：OPC 任务行额外提供「在 OPC 中编辑投递配置」跳转（DemandDiscoveryPage）；任务中心不编辑 delivery/platform/toolsets，列表中以只读 Tag 展示投递目标（数据来自现有 DTO；若 DTO 无 delivery 字段则本次不展示该列，仅保留跳转）。
- FR-2.6：所有错误走错误码 i18n 翻译，不弹原始 e.message。

### FR-3：手动作业 Tab（投资）

- FR-3.1：三个统一风格的**作业卡**（同一种 Card 规格：图标+名称+一句话说明+「立即运行」主按钮+最近一次运行状态条）：
  - 股票分析管道（迁移 PipelinePage 的发起/统计/进度/暂停继续能力）；
  - 时间旅行批次复盘（复用 `TimeTravelBatchCard`，视觉对齐作业卡规格）；
  - 多日回测扫描（复用 `ReplaySweep`，外层包统一作业卡壳）。
- FR-3.2：作业卡下方统一「运行历史」分区（Tabs 或分段切换：管道历史 / 批次复盘历史）：
  - 管道历史：现有 `get_pipeline_history` 数据，表格列重做（开始时间、run_date、asOf、状态、候选数、新分析数、耗时、详情 Drawer 复用现有详情查看）；
  - 批次历史：新增 `list_time_travel_batches`（见 FR-4），表格列：asOf、状态、phase、total/ok/fail、两链候选数、发起时间、耗时、详情 Drawer（analyses 明细 + horizons 四档 + 错误）。
- FR-3.3：手动作业区不展示与作业无关的业务指标；进度与状态实时轮询，卸载清理。
- FR-3.4：浏览器 mock 模式（localStorage dev）下列表为空数组、操作命令有 mock，不白屏。

### FR-4：批次复盘持久化与历史（Rust）

- FR-4.1：新增 entity `time_travel_batch_runs`（`crates/entities/src/time_travel_batch_runs.rs` + `pub mod` 注册），列：`id`(=batch_id, PK String)、`as_date`(indexed)、`trading_days_ago`(nullable int)、`status`(indexed)、`phase`、`message`(nullable)、`total/ok/fail/reco_picks/serenity_picks`(int)、`serenity_error`(nullable)、`request_json`（发起参数快照）、`analyses_json`、`horizons_json`、`reflection_summary`(nullable)、`error_code`(nullable)、`error_detail`(nullable)、`created_at_ms`、`updated_at_ms`、`finished_at_ms`(nullable)。声明式建表自动生效。
- FR-4.2：`run_time_travel_batch` 创建批次时 insert 一行；现有每次 `save_state`（`time_travel_batch.rs:171`）在更新内存的同时 upsert 落库（尽力而为：落库失败 tracing 警告，不阻断业务）；终态写 finished_at_ms。
- FR-4.3：`get_time_travel_batch` 读取顺序：内存命中→返回；未命中查库→返回历史态；都无→NOT_FOUND 错误码。
- FR-4.4：新增 `list_time_travel_batches(limit?: number, status?: String, as_of?: String)` 命令，默认按 created_at_ms 倒序返回 50 条摘要（不含 analyses 明细，详情仍走 get）。
- FR-4.5：应用启动时（批次模块 init 或首次访问时）将库内遗留 `status='running''` 的行收敛为 `status='interrupted'`、写 message，避免重启后僵尸 running；前端状态渲染识别 interrupted。
- FR-4.6：`execute_scheduled_task` 若不支持 `opc_demand_scan`/`opc-demand-discovery` 的手动执行，则补齐分派（复用 services.rs 既有执行分支）；不可复用部分以错误码返回，前端置灰。
- FR-4.7：Rust 侧遵守 harness 分层（entity 在 entities crate，读写 SQL 放 implementor 侧命令/dao；不引入跨层依赖），`cargo fmt` + 定点 clippy + 相关测试通过。

### FR-5：设置页股票业务项清退

- FR-5.1：`StockAnalysisSettings` 删除 schedule、recommendation 两个 tab；保留 experts/roles/profiles/data/params（专家/角色提示词、Agent profile、数据供应商、分析参数——纯分析配置）；`StockAnalysisPage`/`StockAnalysisSettingsModal` 传入的 defaultTab 兼容处理（schedule/recommendation 回退 experts）。
- FR-5.2：两个被迁出的表单组件（ScheduledAnalysisTab/ScheduledRecommendationTab）从 settings 目录迁移到任务中心目录（或 tasks 子目录），复用全部 create 命令；迁移后设置弹窗中不残留定时任务入口。
- FR-5.3：设置侧栏 data 组移除 `scheduler` section；SchedulerSettings 的自定义任务/系统模板/cron builder 能力在 FR-2.4 任务中心落地后，删除该设置组件（设置侧栏点击旧锚点回退 data 默认项）。
- FR-5.4：设置侧栏 data 组移除 `paperOverview` section；PaperOverviewPanel 挂到投资中心「模拟观察」tab（PaperTradingPage 内合适位置，不与现有功能重复，挂载前先核对该页内容）。
- FR-5.5：`ScheduledAnalysisPage.tsx`、PageKey 中 `scheduled-analysis` 死键、AppHeader 白名单对应项删除。

### FR-6：i18n 与复用

- FR-6.1：所有新增 UI 文案 11 语言（zh-CN 源 + zh-TW/en-US/ja/ko/de/fr/es/ar/hi/ru），键统一在 `tasksCenter.*` 与 `nav.tasksCenter`；删除失效的 `invest.tab.pipeline` 等残留键。
- FR-6.2：不重复造类型：任务 DTO 镜像收敛到 `@/types` 单一来源；IPC 全走 `@/lib/invoke`；命名导出；locale 文件 LF + 保序。

## Non-Functional Requirements

- NFR-1：TS strict、oxlint、dprint（`--incremental=false`）、Rust fmt/clippy 全绿；新增逻辑配单测/集成测试。
- NFR-2：列表/轮询在 Tauri 与浏览器 mock 两种模式均可用；页面卸载停止所有轮询。
- NFR-3：批次 upsert 单轮额外 DB 写次数与既有 save_state 调用次数同阶，不新增每分析项一次的高频写（按阶段保存：创建、分析完成、反思完成、失败）。
- NFR-4：DTO/命令参数全程 camelCase；antd v6 浮层自定义组件 forwardRef。

## Constraints

- **Technical**：Tauri v2/React 19/antd 6；harness 分层铁律；禁区 1-13；声明式建表（不写 migration）；as_of 进程级约束决定 N1。
- **Business**：内置种子任务删除不重建（UI 强警示，默认引导暂停）；批次复盘不污染 live reco_picks 的既有铁律不变。
- **Dependencies**：FR-4 后端改动是 FR-3.2 批次历史的前置；FR-2.4 依赖 FR-5 表单迁移。

## Assumptions

- A1：用户「设置页不应保留股票业务项目」的范围 = 两个定时 tab + paperOverview + scheduler 通用调度收口；专家/角色提示词、Agent profile、数据供应商、分析参数视为「分析配置」保留在股票设置弹窗。审批时如有不同理解需指出。
- A2：OPC 专有配置编辑留 OPC 页，任务中心只做统一控制面 + 跳转（见 N3）。
- A3：任务中心放侧栏 general 区（系统级），不放投资中心内部。
- A4：批次历史仅存批次自身状态/摘要 JSON；逐只分析的完整产物仍从既有分析记录查看，历史详情 Drawer 展示批次内 analyses 摘要即可。
- A5：SchedulerSettings 里 daily_summary 等系统任务模板「创建后」仍由既有 services.rs handler 执行，任务中心只是换了创建/管理入口。

## Acceptance Criteria

### AC-1：任务中心可达且结构正确
- **Type**: `rule`
- **Given**：应用启动
- **When**：在侧栏点击「任务中心」
- **Then**：进入 `/tasks`，可见「计划任务」「手动作业」两个 Tab，默认计划任务
- **Pass Condition**：浏览器实测 + 路由/导航注册代码存在；11 语言 label 均有翻译
- **Evidence**：浏览器截图、navRegistry/pageRegistry/ContentArea diff、locale 键扫描

### AC-2：计划任务跨域全量可见可控
- **Type**: `rule`
- **Given**：库中存在投资内置 cron、OPC cron、系统任务（或 mock 等价数据）
- **When**：打开计划任务 Tab
- **Then**：四类分组正确归组（含 serenity 归投资、opc- 前缀归 OPC）；Switch 暂停/恢复调用通用命令并翻转状态；立即执行触发对应执行路径；历史 Drawer 展示执行记录；删除有确认且内置任务有重启不重建警示
- **Pass Condition**：分组单测覆盖全部 taskType 分支；组件测试验证 4 个操作的 invoke 载荷；浏览器实测至少投资+OPC 两类真实任务
- **Evidence**：vitest 输出、浏览器实测记录

### AC-3：分组判据正确性
- **Type**: `rule`
- **Given**：taskType/workflowId 组合矩阵（serenity-fast、opc_demand_scan、backup、custom、空 taskType+普通 workflowId）
- **When**：执行分组函数
- **Then**：每种组合落入且仅落入一个预期分组
- **Pass Condition**：单测表驱动 5+ 用例全过
- **Evidence**：vitest 用例与输出

### AC-4：手动作业三区与历史
- **Type**: `rule`
- **Given**：任务中心手动作业 Tab
- **When**：查看页面
- **Then**：管道/批次复盘/Sweep 三张同规格作业卡；管道历史与批次历史两个历史表可切换；批次历史详情 Drawer 展示 analyses/horizons
- **Pass Condition**：组件测试 + 浏览器实测发起一次批次复盘后历史表出现该批次
- **Evidence**：vitest、浏览器实测（含轮询与卸载清理的代码证据）

### AC-5：批次复盘持久化
- **Type**: `rule`
- **Given**：发起一次批次复盘并跑完（或中途失败）
- **When**：重启应用后打开批次历史 / 直接调 get_time_travel_batch
- **Then**：该批次从库中可查，字段与终态完整；重启前遗留 running 行被收敛为 interrupted；历史列表默认倒序 50 条
- **Pass Condition**：Rust 集成测试（内存 SQLite）覆盖 insert→重启模拟查询→running 收敛→list 倒序；`__TAURI_WORKSPACE__=true cargo test` 相关模块通过
- **Evidence**：测试输出、表结构 schema_diff 绿

### AC-6：设置页股票业务项清零
- **Type**: `rule`
- **Given**：全局搜索设置相关目录
- **When**：检查股票分析设置弹窗、设置侧栏
- **Then**：弹窗无定时分析/定时荐股 tab（纯 5 个配置 tab）；侧栏 data 组无 scheduler、paperOverview；不存在对 SchedulerSettings/ScheduledAnalysisPage 的有效引用；defaultTab 旧值安全回退
- **Pass Condition**：grep 零引用证据 + 组件测试 + 浏览器走查设置弹窗与侧栏
- **Evidence**：grep 输出、浏览器截图

### AC-7：投资中心瘦身
- **Type**: `rule`
- **Given**：打开投资中心
- **When**：检查标题栏与 tab 栏
- **Then**：恰好 6 个业务 tab 无「管道/时间旅行」；标题栏仅 live/as-of 控件；旧 URL 参数回退选股
- **Pass Condition**：集成测试 + 浏览器实测
- **Evidence**：vitest、浏览器截图

### AC-8：任务中心交互质量
- **Type**: `rubric`
- **Dimension**：任务中心信息架构与视觉一致性
- **Scale**: 1-5
- **Anchors**: 1 = 三个手动作业风格各异、列表拥挤难用；3 = 功能齐全但卡片/表格风格不统一；5 = 计划/手动两区一目了然，作业卡同规格、状态色彩与 Tag 体系一致、密度适中、空态/加载/错误态完整
- **Pass Threshold**: >= 4
- **Evidence**：浏览器实测走查 + 截图评审

### AC-9：代码卫生与门禁
- **Type**: `rule`
- **Given**：全部改动完成
- **When**：运行 typecheck/oxlint/dprint/Rust fmt+定点 clippy+相关 vitest
- **Then**：全部通过；无重复类型定义；locale 全部 LF
- **Pass Condition**:命令输出全绿
- **Evidence**:CI 口径命令输出

## Open Questions

- [ ] Q1（见 A1）：股票设置弹窗保留的 5 个「配置类」tab 是否认可保留？若也要移出，需另行定义它们的归宿。
- [ ] Q2：批次历史保留策略——按条数（最近 50）还是也加按天清理？本轮默认只做查询 limit，不做清理任务。
- [ ] Q3：OPC 任务行是否需要只读展示 delivery 投递目标（需给 ScheduledTaskDto 加字段），还是仅保留「去 OPC 编辑」跳转即可？默认后者。
