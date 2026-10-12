# 系统级任务中心 — 实施计划（tasks）

> AC 与 TR 定义见同目录 `spec.md`。任务按依赖序排列；实施时一次只取一个 ready 的最高优先级项。

## Task 1: Rust — 批次复盘历史表 entity
- **Status**: `pending`
- **Priority**: high
- **Depends On**: None
- **Description**:
  - 新建 `src-tauri/crates/entities/src/time_travel_batch_runs.rs`，字段按 spec FR-4.1（id PK String、as_date indexed、trading_days_ago nullable、status indexed、phase、message、total/ok/fail/reco_picks/serenity_picks、serenity_error、request_json/analyses_json/horizons_json、reflection_summary、error_code、error_detail、created_at_ms、updated_at_ms、finished_at_ms nullable），形态照抄 `stock_pipeline_runs.rs`（DeriveEntityModel + camelCase serde）。
  - 在 `crates/entities/src/lib.rs` 追加 `pub mod time_travel_batch_runs;`（build.rs 自动登记，声明式 reconcile 自动建表）。
- **Acceptance Criteria Addressed**: AC-5、AC-9
- **Test Requirements**:
  - `rule` TR-1.1: `cargo check -p axagent-entities` 通过；实体 `Entity::find()` 可编译被引用
  - `rule` TR-1.2: 应用启动后 SQLite 出现 `time_travel_batch_runs` 表且列集与 entity 一致（schema reconcile 无 diff；以 `cargo test -p axagent-dao` 实体普查测试为证据）
- **Notes**: 不写版本化 migration（MIGRATIONS 已空，引擎是唯一建表来源，见 spec 背景 6）。

## Task 2: Rust — 批次状态落库、历史查询、僵尸收敛
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 1
- **Description**:
  - 在 `commands/time_travel_batch.rs`（或新建 dao/repository 辅助，保持分层）实现：创建批次时 insert；将现有 `save_state` 扩展为内存 upsert + DB upsert（按阶段调用：创建、分析完成、反思完成、失败；落库失败 tracing::warn 不阻断）；终态写 finished_at_ms。
  - `get_time_travel_batch`：内存→DB→NOT_FOUND（错误码）。
  - 新增命令 `list_time_travel_batches(limit: Option<u32>, status: Option<String>, as_of: Option<String>)`：倒序默认 50，摘要不含 analyses 明细。
  - 启动/首次访问收敛遗留 `running` → `interrupted` + message（在 init 或模块 OnceInit 中执行，给出位置注释说明 as_of 进程级语义）。
  - 核对 `execute_scheduled_task`（scheduled_task.rs:343）对 `opc_demand_scan`/`opc-demand-discovery` 的分派：复用 services.rs 既有执行分支补齐；不能安全复用的返回错误码（FR-4.6）。
  - `commands/mod.rs` 无需额外动作（#[tauri::command] + pub mod 自动注册）；前端 DTO 字段 camelCase。
- **Acceptance Criteria Addressed**: AC-5、AC-2（OPC 立即执行）、AC-9
- **Test Requirements**:
  - `rule` TR-2.1: Rust 集成测试（内存 SQLite + 测试 pool）覆盖：insert 后重新打开查询（模拟重启）字段完整；list 倒序 + limit + status/as_of 过滤；遗留 running 被收敛为 interrupted
  - `rule` TR-2.2: `__TAURI_WORKSPACE__=true cargo test -p axagent --lib commands::time_travel_batch`（或对应包）通过
  - `rule` TR-2.3: 定点 `cargo clippy -p <crate> --all-targets -- -D warnings` 与 `cargo fmt --all --check` 对改动文件通过
- **Notes**: 落库写次数与现有 save_state 同阶（NFR-3），不新增逐项高频写。

## Task 3: 前端契约层 — 类型收敛 + browserMock
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 2
- **Description**:
  - 在 `src/types/` 新增/收敛：`ScheduledTask`/`TaskConfig`/`TaskRunResult` 从 SchedulerSettings 内联定义迁至 `@/types` 权威来源并 re-export；新增 `TimeTravelBatchSummary`（list 摘要）+ `TimeTravelBatchStatus` 增加 `interrupted`。
  - `src/lib/browserMock.ts` 补 mock：`list_scheduled_tasks`（含投资/OPC/系统示例或空数组）、pause/resume/delete/execute_scheduled_task、`get_task_execution_history`（[]）、`list_time_travel_batches`（[]）。
- **Acceptance Criteria Addressed**: AC-2、AC-5、AC-9
- **Test Requirements**:
  - `rule` TR-3.1: `npm run typecheck` 通过；grep 确认无第二份 ScheduledTask interface 定义
  - `rule` TR-3.2: 浏览器 mock 模式打开任务中心不白屏、命令有应答（后续任务实测时复核）

## Task 4: 前端路由与导航骨架
- **Status**: `pending`
- **Priority**: high
- **Depends On**: None
- **Description**:
  - `src/lib/pageRegistry.ts` 加 `"tasks-center": "/tasks"`；`src/types/index.ts` PageKey 加 `"tasks-center"`；`src/lib/navRegistry.tsx` general 区加导航项（ScheduleOutlined，labelKey `nav.tasksCenter`，位置紧邻工作台之后）。
  - `ContentArea.tsx` 注册 `/tasks` 懒加载 + PageContextProvider；`AppHeader.tsx` 路径白名单按投资系页面策略处理 `/tasks`（任务中心用自有 PageHeader，与投资系一致隐藏全局头）。
  - 新建空壳 `src/pages/TasksCenterPage/TasksCenterPage.tsx`（PageHeader + Tabs 占位）。
- **Acceptance Criteria Addressed**: AC-1
- **Test Requirements**:
  - `rule` TR-4.1: typecheck 通过；浏览器访问 `/tasks` 出现页面壳；侧栏出现入口且高亮正确
  - `rule` TR-4.2: navRegistry/PageKey/pageRegistry 三处键一致（grep 证据）

## Task 5: 任务中心页面壳与分区
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 4
- **Description**:
  - 完成 TasksCenterPage：PageHeader（标题/说明 i18n）+ antd Tabs 两项 `scheduled`/`manual`，URL query 同步 active tab；渲染 ScheduledJobsTab、ManualJobsTab（下一阶段填充）。
- **Acceptance Criteria Addressed**: AC-1、AC-8
- **Test Requirements**:
  - `rule` TR-5.1: 两个 Tab 可切换、刷新保持、默认计划任务（组件测试）

## Task 6: 计划任务 Tab — 列表/分组/操作/自动刷新
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 3、Task 5
- **Description**:
  - 新建 `src/pages/TasksCenterPage/ScheduledJobsTab.tsx` + 分组工具（纯函数 + 单测）：归组判据按 FR-2.1（serenity workflow 归投资；`opc` 前缀归 OPC；系统 taskType 白名单；兜底自定义）。
  - 表格列按 FR-2.2；Switch→pause/resume、立即执行→execute（custom/general 置灰 tooltip）、历史 Drawer（get_task_execution_history）、删除 Popconfirm（内置固定 id 警示）。
  - 顶部：刷新、15s 自动刷新 Switch（卸载 cleanup）、分组筛选、错误码 i18n 翻译。
  - OPC 行「去编辑投递配置」跳转 DemandDiscoveryPage。
- **Acceptance Criteria Addressed**: AC-2、AC-3、AC-8、AC-9
- **Test Requirements**:
  - `rule` TR-6.1: 分组函数表驱动单测 ≥8 组合（AC-3 矩阵 + serenity-fast + 空 taskType 普通 workflow）
  - `rule` TR-6.2: 组件测试：渲染分组 Tag、Switch 调用 pause/resume 且 camelCase 载荷、立即执行、历史 Drawer 拉取、删除确认调用
  - `rule` TR-6.3: 自动刷新定时器卸载清理（fake timers 断言）

## Task 7: 任务创建能力迁移（自定义/投资/系统模板）
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 6
- **Description**:
  - 将 `components/settings/CronScheduleBuilder.tsx` 与 SchedulerSettings 中自定义任务创建表单迁入任务中心「新建任务→自定义」Modal/Drawer（workflow 选择 + TaskConfig runOnStartup/retry/timeout/notification + 模板快捷创建 backup/cleanup/daily_summary）。
  - 将 `ScheduledAnalysisTab.tsx`、`ScheduledRecommendationTab.tsx` 的**创建表单部分**迁入 `src/pages/TasksCenterPage/forms/`，作为「新建投资任务」的分类表单（6 类：个股/候选池/趋势/反思/回测/自选股 + 定时荐股），继续调用各 create_*_cron 命令；列表/启停/删除不再使用这些表单（统一表格承担）。
- **Acceptance Criteria Addressed**: AC-2、AC-6
- **Test Requirements**:
  - `rule` TR-7.1: 三类新建路径均能提交并调用既有创建命令（组件测试 mock invoke 载荷断言）
  - `rule` TR-7.2: cron 校验（5 段、最小 2 分钟）行为与迁移前一致（复用既有校验逻辑，单测平移）

## Task 8: 手动作业 Tab — 三作业卡 + 双历史
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 3、Task 5
- **Description**:
  - 新建 `ManualJobsTab.tsx`：统一 JobCard 规格（图标/名称/说明/立即运行/最近状态条），挂：股票分析管道（PipelinePageInner 主体改造为面板组件复用，保留 run/pause/resume/进度/统计）、TimeTravelBatchCard（包 JobCard 壳对齐视觉）、ReplaySweep（同）。
  - 历史分区：管道历史表（get_pipeline_history，列按 FR-3.2 重做 + 详情 Drawer 复用现有详情查看）；批次历史表（list_time_travel_batches：asOf/状态/phase/total-ok-fail/两链数/发起时间/耗时 + 详情 Drawer 展示 analyses/horizons/serenityError）。
  - 轮询挂载/卸载清理；空态/加载/错误态完整。
- **Acceptance Criteria Addressed**: AC-4、AC-8
- **Test Requirements**:
  - `rule` TR-8.1: 组件测试：三卡渲染、批次历史 mock 渲染成行、点击详情打开 Drawer
  - `rubric` TR-8.2: 作业卡视觉一致性；1-5；anchors 按 AC-8；threshold >=4；证据为浏览器截图走查

## Task 9: InvestHub 瘦身
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 8
- **Description**:
  - InvestHub：删除批次按钮/Drawer/TimeTravelBatchCard/FlaskConical 等上轮引入；删除 LazyPipeline 与 pipeline tab；InvestTabKey/VALID_TABS 回归 6 项；`?tab=replay`/`?tab=pipeline` 回退 screener；标题栏仅 `<PageTimeAnchor />`；更新文件头注释。
- **Acceptance Criteria Addressed**: AC-7
- **Test Requirements**:
  - `rule` TR-9.1: 集成测试改写：恰 6 tab、无 pipeline/replay、标题栏仅有 page-time-anchor、旧参数回退（更新 ScreenerToWorkspace.integration.test.tsx）

## Task 10: 设置页股票业务项清退
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 7、Task 8
- **Description**:
  - StockAnalysisSettings 删 schedule/recommendation 两 tab（保留 5 配置 tab）；defaultTab 旧值（schedule/recommendation）回退 experts；同步 StockAnalysisPage/Modal 的 settingsDefaultTab 传参点。
  - 设置侧栏 data 组移除 `scheduler`；删除 SchedulerSettings 组件及其 SettingsPage 挂载点（能力已在 Task 7 落地）。
  - data 组移除 `paperOverview`；PaperOverviewPanel 挂入投资中心「模拟观察」tab（先读 PaperTradingPage 确认不重复，选合适分区）。
  - 删除 ScheduledAnalysisPage.tsx、PageKey `scheduled-analysis` 死键、AppHeader 白名单项。
- **Acceptance Criteria Addressed**: AC-6
- **Test Requirements**:
  - `rule` TR-10.1: grep 零引用：SchedulerSettings、ScheduledAnalysisPage、ScheduledAnalysisTab 旧路径（新位置除外）；typecheck 绿
  - `rule` TR-10.2: 股票设置弹窗测试更新为 5 tab；旧 defaultTab 回退测试
  - `rule` TR-10.3: 浏览器走查：设置侧栏无 scheduler/paperOverview；模拟观察 tab 可见 PaperOverview 功能

## Task 11: i18n 11 语言
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 6、Task 7、Task 8
- **Description**:
  - 11 语言新增 `nav.tasksCenter` 与 `tasksCenter.*`（两个 tab、四分组、列名、操作、历史 Drawer、新建任务各表单、空/错态、interrupted 状态等）；删除 `invest.tab.pipeline` 及失效键。
  - Python `json.dumps(ensure_ascii=False, indent=2)` + `newline=""` LF 保序落盘；zh-CN 为源。
- **Acceptance Criteria Addressed**: AC-1、AC-9
- **Test Requirements**:
  - `rule` TR-11.1: 11 文件均含新键前缀；locale 键扫描脚本/测试无缺译；文件全 LF

## Task 12: 测试补齐与门禁
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 9、Task 10、Task 11
- **Description**:
  - 新建 TasksCenterPage 测试；更新受影响既有测试（InvestHub 集成、SchedulerSettings 删除后的设置测试、DemandDiscoveryPage 不受影响回归）。
  - 全量前端门禁：typecheck、oxlint、dprint `--incremental=false`、相关 vitest 全跑。
  - Rust 全量门禁在后端任务完成后按 CI 口径跑一次：`cargo fmt --all --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`、`__TAURI_WORKSPACE__=true cargo test --workspace`。
- **Acceptance Criteria Addressed**: AC-9
- **Test Requirements**:
  - `rule` TR-12.1: 上述命令输出全绿（记录输出摘要）

## Task 13: 浏览器实测与收尾
- **Status**: `pending`
- **Priority**: high
- **Depends On**: Task 12
- **Description**:
  - 1420 实测：侧栏入口；计划任务四组真实归组 + 暂停/恢复/立即执行/历史；新建自定义与投资任务；手动作业三卡 + 发起一次批次复盘 + 历史出现 + 重启模拟（dev 难模拟重启则用命令直验库）；投资中心 6 tab 纯世界观标题栏；设置弹窗 5 tab、侧栏无 scheduler/paperOverview；模拟观察内 PaperOverview；控制台无红错。
- **Acceptance Criteria Addressed**: AC-1、AC-2、AC-4、AC-6、AC-7、AC-8
- **Test Requirements**:
  - `rule` TR-13.1: 实测清单逐项 PASS 记录（截图/控制台证据）
  - `rubric` TR-13.2: 任务中心整体 IA/视觉；1-5；anchors 按 AC-8；threshold >=4
