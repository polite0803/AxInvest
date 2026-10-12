# 异步 / 后台 / 批处理 / 定时任务源穷举清单

> 调研日期：2026-09-26　性质：只读调研（未改任何代码）
> 用途：为统一任务中心 UI（`/tasks`，见 `spec.md` / `tasks.md`）提供适配器清单。
> 所有命令行号基于当前工作区代码；实体表经 dao build.rs 声明式 reconcile 自动建表（无 migration）。

## 0. 判定口径

**计入「任务源」**：存在「发起后脱离单次点击生命周期」的执行单元——定时触发、后台队列、长时同步等待、可轮询状态、有历史/结果落盘之一即计入。

**能力列定义**（矩阵用）：

| 列 | 含义 |
|---|---|
| 列表 | 有专门的「列出多个任务/作业」命令（不含业务表翻页） |
| 详情 | 可按 id 单独拉取单任务状态/结果 |
| 取消 | 有 stop/cancel/abort 类命令（删除任务定义不算） |
| 重试 | 有 retry / 基于旧任务重新执行的命令 |
| 暂停 | 有 pause/resume（或等价 stop→restart 语义） |
| 日志 | 可拉取执行过程日志 / 事件流 / 步骤轨迹（仅有 error_message 不算） |
| 输出 | 结果实体持久化且可取回（同步返回、不落库的标 △） |
| 进度 | 有百分比 / stage / phase 级进度可取（事件或轮询） |

**触发方式**：`cron`（CronJobStore）/ `手动`（IPC 命令）/ `系统`（启动恢复或后台 tick）/ `外部`（webhook、上游投递）。

---

## 1. 已被任务中心纳管的 4 个源

### 1.1 scheduled-cron —— 统一定时任务（CronJobStore，核心源）　业务域：全局

所有定时任务（投资 7 类 + OPC 扫描 + 系统维护 + 自定义工作流）都是同一 `CronJob` 实体，由 runtime-core 调度、DAO 端口落库，是任务中心「计划任务」Tab 的唯一后端。

- **后端命令**（[scheduled_task.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/scheduled_task.rs)）：
  `list_scheduled_tasks` :211、`get_scheduled_task` :221、`create_scheduled_task` :232、`update_scheduled_task` :272、`delete_scheduled_task` :312、`pause_scheduled_task` :323、`resume_scheduled_task` :334、`execute_scheduled_task` :345（手动立即跑）、`get_scheduled_task_templates` :445、`create_daily_summary_task` :475、`create_backup_task` :495、`create_cleanup_task` :515、`get_task_execution_history` :543。
- **同族创建命令（参数化 CronJob，历史遗留入口）**：
  - [stock_analysis.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/stock_analysis.rs)：个股 `create/list/toggle/delete_stock_cron` :4013/:4036/:4048/:4070；自选股扫描 :5325/:5349/:5363/:5385；决策校验 :5409/:5442/:5456/:5478；批量反思 :5498/:5538/:5552/:5574。
  - [recommendation_cron.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/recommendation_cron.rs)：定时荐股 :111/:146/:160/:182；趋势筛选 :247/:284/:298/:320。
  - [pool_scan.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/pool_scan.rs)：候选池扫描 cron :474/:521/:535/:557。
  - [backtest_validation.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/backtest_validation.rs)：决策回测 cron :854/:900/:914/:936。
  - OPC：[opc_demand_subscription.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/opc_demand_subscription.rs) `opc_ensure_demand_scan_job` :259（默认 `0 */6 * * *`）；种子 [seed_opc_cron.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/opc_setup/seed_opc_cron.rs#L19)（demand-discovery daily/weekly）。
  - 固定种子：[stock_automation_cron_seed.rs](file:///d:/OneManager/AxInvest/src-tauri/src/init/stock_automation_cron_seed.rs)（4 个固定 id，存在即跳过）。
- **模型/状态**：[cron_job.rs](file:///d:/OneManager/AxInvest/src-tauri/crates/runtime-core/src/cron_job.rs) `CronJob` :50、`CronJobStore` :351；任务状态 `CronJobStatus` = `active / paused / disabled`；`TaskRunResult`（success/output/error/duration_ms/executed_at/skipped）。内置 taskType 白名单 :113（knowledge_source_fetch_all、stock-recommendation、watchlist-scan、pool_scan、batch-reflection、validate-decisions、opc_demand_scan、decision_backtest 等）。
- **持久化/保留**：表 `cron_jobs`（id + data JSON）、`cron_job_history`；历史固定取最近 **50 条/任务**（:648 `load_history(task_id, 50)`），未见全局 TTL/清理。
- **触发**：cron + 手动（execute）+ 启动恢复（load_scheduled_tasks_from_db :535）。
- **前端**：类型 [scheduler.ts](file:///d:/OneManager/AxInvest/src/types/scheduler.ts)（ScheduledTask/TaskExecutionRecord/TaskReport/BudgetState；注意 `ExecutionRecord` 是后端少数无 rename_all 的 snake_case DTO）；页面 [ScheduledJobsTab.tsx](file:///d:/OneManager/AxInvest/src/pages/TasksCenterPage/ScheduledJobsTab.tsx)、[taskGrouping.ts](file:///d:/OneManager/AxInvest/src/pages/TasksCenterPage/taskGrouping.ts)、forms/*、[investTaskDefs.ts](file:///d:/OneManager/AxInvest/src/pages/TasksCenterPage/forms/investTaskDefs.ts)；旧入口 [SchedulerSettings.tsx](file:///d:/OneManager/AxInvest/src/components/settings/SchedulerSettings.tsx)、[ScheduledAnalysisPanel.tsx](file:///d:/OneManager/AxInvest/src/components/stock-analysis/ScheduledAnalysisPanel.tsx)、[ReflectionPanel.tsx](file:///d:/OneManager/AxInvest/src/components/stock-analysis/ReflectionPanel.tsx)。
- **路由/深链**：`/tasks`（计划任务 Tab，URL `?tab=scheduled`）。OPC 行可链 `/demand-discovery`。

### 1.2 stock-pipeline —— 股票分析管道　业务域：投资

经 rt-workflow `WorkEngine` 跑 `stock-pipeline` 模板的按日管道。

- **后端命令**（[core.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/stock_pipeline/core.rs)）：`run_stock_pipeline` :479、`get_pipeline_history` :519（默认 limit 20）、`get_pipeline_run_detail` :555。**无取消/暂停/重试命令**。
- **状态**：`running / completed / failed`；节点级 `pipeline_step` 事件（nodeId + running/completed/failed/timeout）。
- **持久化/保留**：表 `stock_pipeline_runs`（run_date/as_of_date/status/candidates_json/new_analyses_json/reassessed_json/summary_json/error_message/started_at/completed_at），无清理命令（永久）。
- **触发**：手动；也可被 cron/工作流间接触发。
- **前端**：[pipelineStore.ts](file:///d:/OneManager/AxInvest/src/stores/feature/pipelineStore.ts)、[PipelineJobCard.tsx](file:///d:/OneManager/AxInvest/src/pages/TasksCenterPage/jobs/PipelineJobCard.tsx)、[PipelineRunDetailDrawer.tsx](file:///d:/OneManager/AxInvest/src/pages/TasksCenterPage/jobs/PipelineRunDetailDrawer.tsx)。
- **路由**：`/tasks?tab=manual`。

### 1.3 time-travel-batch —— 时间旅行批次复盘　业务域：投资

- **后端命令**（[time_travel_batch.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/time_travel_batch.rs)）：`run_time_travel_batch` :381、`get_time_travel_batch` :514、`list_time_travel_batches` :536（limit/status/as_of 过滤，默认 50）。**无取消/暂停/重试**；进程级全局互斥锁 + 时间窗拒绝。
- **状态**：批次 `running / completed / partial / failed / interrupted`（启动时把遗留 running 收敛为 interrupted）；`phase = analysis / reflection`；含 total/ok/fail/两链推荐数等计数。
- **持久化/保留**：内存 `static BATCH_RUNS`（:59 HashMap）**+ 表 `time_travel_batch_runs` 双写**，get 顺序 内存→DB；无清理（永久）。
- **触发**：手动。
- **前端**：[pipelineStore 同域 pipelineStore.ts](file:///d:/OneManager/AxInvest/src/stores/feature/pipelineStore.ts)、[TimeTravelBatchCard.tsx](file:///d:/OneManager/AxInvest/src/components/time-travel/TimeTravelBatchCard.tsx)、[BatchRunDetailDrawer.tsx](file:///d:/OneManager/AxInvest/src/pages/TasksCenterPage/jobs/BatchRunDetailDrawer.tsx)、[ManualJobsTab.tsx](file:///d:/OneManager/AxInvest/src/pages/TasksCenterPage/ManualJobsTab.tsx)、类型 [timeTravel.ts](file:///d:/OneManager/AxInvest/src/types/timeTravel.ts)。
- **路由**：`/tasks?tab=manual`；相关工作台 `/replay-workbench`。

### 1.4 replay-sweep —— 多日回放扫描　业务域：投资

- **后端命令**：[stock_analysis.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/stock_analysis.rs) `run_replay_backtest` :2159（同步返回 `ReplaySweepResult`，**无独立持久化、无历史命令**）。
- **前端**：[ReplaySweep.tsx](file:///d:/OneManager/AxInvest/src/components/time-travel/ReplaySweep.tsx)（挂在手动作业 Tab）。
- **路由**：`/tasks?tab=manual`。
- **说明**：严格说是「长时同步命令」，任务中心目前以作业卡形式展示入口；除「输出」（同步返回）外无任何生命周期能力。

---

## 2. 线索点名但尚未纳管的任务源

### 2.1 background-tasks —— 后台 shell / agent 任务　业务域：系统/终端

通用后台任务台账（bash 命令、agent 任务），是最接近「统一任务模型」的既有设施：harness 单一状态真源 + DB 台账 + 事件 + 启动恢复。

- **后端命令**（[background_tasks.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/background_tasks.rs)）：`spawn_background_task` :185、`list_background_tasks` :479、`get_background_task_output` :497、`stop_background_task` :519、`list_task_events` :565（limit 钳制 1..500，默认 100）。
- **状态机**：harness [task_state.rs](file:///d:/OneManager/AxInvest/src-tauri/crates/harness/src/task_state.rs#L48-L68) `TaskStatus` = `pending / running / completed / failed / stopped / unknown`（DB 字符串；stop 仅允许 Running|Pending→Stopped）。task_type = `bash / agent`；含 idempotency_key / attempt / resume_from 字段。bash 走 tokio::process，stdout/stderr 逐行 append；panic 有 DropGuard 兜底置 failed。
- **事件**：`background-task:created / updated / failed`。
- **持久化/保留/恢复**：表 `background_tasks` + 审计表 `task_events`，无清理命令（永久）；[restore.rs](file:///d:/OneManager/AxInvest/src-tauri/src/scheduler/restore.rs) 启动时把 pending/running 重新入队。
- **触发**：手动 spawn、agent 执行、系统恢复。
- **前端**：[TaskPanel.tsx](file:///d:/OneManager/AxInvest/src/components/chat/TaskPanel.tsx)（组件内内联定义 BackgroundTask/TaskEvent 类型，**无独立 store / 无路由**，作为聊天侧面板）；browserMock 有 mock。
- **能力**：列表√ 详情√ 取消√ 重试×（字段预留，无命令）暂停× 日志√ 输出√ 进度×。

### 2.2 workflow-dag —— DAG 工作流执行（能力最全）　业务域：工作流

存在**两套命令面、同一个 DAG 引擎与同一张执行表**（rt-workflow WorkEngine；旧 [workflow_engine.rs](file:///d:/OneManager/AxInvest/src-tauri/crates/rt-workflow/src/workflow_engine.rs) 只剩共享类型）：

- **新命令面** [work_engine.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/work_engine.rs)（21 个）：`start_workflow_execution` :191、pause :210、resume :226、cancel :242、get_status :258、list :285、`execute_workflow_node` :340、`list_node_executor_types` :397、`debug_run_workflow` :406、断点 :659/:675/:685/:703、循环检查点 :728、审批 :754/:830/:904、`list_paused_workflow_executions` :957、recover :980、recover_all :996、`cancel_all_paused` :1011。
- **旧命令面** [workflows/mod.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/workflows/mod.rs)：`workflow_execute` :95、get_status :775、cancel :800、pause :822、resume :865、list :908、`list_active_executions` :925、`get_workflow_steps` :935 等。
- **状态枚举**（harness workflow_types.rs）：NodeStatus :1787（pending/ready/running/completed/failed/skipped）、WorkflowStatus :1848（created/running/completed/partially_completed/failed/cancelled）、ExecutionStatus :1860（running/paused/completed/partially_completed/failed/cancelled）。
- **持久化/保留**：表 `workflow_executions`（execution_state_json 支持崩溃恢复、paused_at）、`loop_checkpoints`、`workflow_approvals`；无清理命令。
- **触发**：手动、cron（自定义 workflow 任务）、外部（rt-workflow `trigger/` 下 [scheduler.rs](file:///d:/OneManager/AxInvest/src-tauri/crates/rt-workflow/src/trigger/scheduler.rs)、[event_bus.rs](file:///d:/OneManager/AxInvest/src-tauri/crates/rt-workflow/src/trigger/event_bus.rs)、[webhook_server.rs](file:///d:/OneManager/AxInvest/src-tauri/crates/rt-workflow/src/trigger/webhook_server.rs)）。
- **前端**：[workEngineStore.ts](file:///d:/OneManager/AxInvest/src/stores/feature/workEngineStore.ts)、[workflowStore.ts](file:///d:/OneManager/AxInvest/src/stores/feature/workflowStore.ts)、[approvalStore.ts](file:///d:/OneManager/AxInvest/src/stores/feature/approvalStore.ts)；[ExecutionHistoryPanel.tsx](file:///d:/OneManager/AxInvest/src/components/workflow/ExecutionHistoryPanel.tsx)、[ExecutionReplayPanel.tsx](file:///d:/OneManager/AxInvest/src/components/stock-analysis/ExecutionReplayPanel.tsx)；页面 `/workflow`（WorkflowPage、WorkflowMarketplace）。
- **能力**：列表√ 详情√ 取消√ 重试√（recover/单步续跑）暂停√ 日志√（节点轨迹/检查点）输出√ 进度√（节点级状态）。

### 2.3 agent-session —— Agent 会话运行态　业务域：对话

分两层：会话台账（DB）与会话内 agent 主循环控制。

- **会话台账命令**（[agent_session.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/agent_session.rs)）：`agent_session_status` :21、`agent_session_cancel` :43（emit `agent-session-cancelled`）、`agent_session_list` :74。表 `agent_sessions`（runtime_status 字符串、conversation_id、workspace_locked、total_tokens/cost）；broker 在 harness `agent_session_broker.rs`，repo 在 dao。**前端无真实消费**（仅 invoke 元数据）。
- **主循环命令**（[agent/mod.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/agent/mod.rs)）：`agent_cancel` :3135、`agent_is_running` :3157、`agent_pause` :3169、`agent_resume` :3208、`agent_resume_from_events` :3253、`agent_is_paused` :3366、`agent_runtime_stats` :3404。
- **持久化**：会话行落库；运行态在进程内 broker。**内部 DAG 不算独立源**：[crates/agent/src/task.rs](file:///d:/OneManager/AxInvest/src-tauri/crates/agent/src/task.rs) 的 AgentTaskStatus（pending/running/completed/failed/skipped）随会话存活，无 IPC 列表。
- **能力**：列表√ 详情√ 取消√ 重试× 暂停√ 日志× 输出× 进度×（runtime_stats 仅计数）。

### 2.4 parallel-execution —— 并行执行批次　业务域：实验/编排

- **后端命令**（[parallel_execution.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/parallel_execution.rs)）：create :14、get :33、list :43、`get_next_pending_task` :52、`update_task_result` :62、`update_task_error` :77、cancel :92、`get_execution_result` :105、delete :115、start :125、verify :136、`check_parallel_timeouts` :154。
- **状态**：ExecutionStatus :251（pending/running/completed/failed/cancelled）；ParallelTaskStatus :111（多一个 `timeout`）；ParallelTask 含 `progress: f32`（由 worker 回写）。
- **持久化/保留**：**纯内存** [parallel_execution.rs](file:///d:/OneManager/AxInvest/src-tauri/crates/trajectory/src/parallel_execution.rs#L805)（`Arc<RwLock<HashMap>>` + max_executions 上限淘汰），重启全丢。
- **前端**：**零调用**（src 中 grep 无任何引用），无 UI/路由。
- **能力**：列表√ 详情√ 取消√ 重试× 暂停× 日志× 输出√ 进度△（字段存在，靠回写）。

### 2.5 index-jobs —— 索引作业队列（知识库/记忆/Wiki）　业务域：RAG

最完整的「队列型」源，但前端完全没有接。

- **后端命令**（[index_jobs.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/index_jobs.rs)）：`index_jobs_list` :25（默认 limit 50，可按 status/sourceType/sourceId/targetId 过滤）、`index_jobs_stats` :75、`index_jobs_retry` :110、`index_jobs_cancel` :136、`index_jobs_retry_all_failed` :162、`index_jobs_clear_completed` :178、`index_jobs_reindex_collection` :189（kb/memory/wiki）。
- **Worker**：[index_queue.rs](file:///d:/OneManager/AxInvest/src-tauri/src/index_queue.rs) `IndexJobService`：信号量并发 2、500ms 轮询、60s 孤儿扫描、指数退避；emit `index-job-updated`。
- **状态/类型**（[repo/index_jobs.rs](file:///d:/OneManager/AxInvest/src-tauri/crates/dao/src/repo/index_jobs.rs#L11-L15)）：`pending / processing / completed / failed / retrying / cancelled`；job_type = index_document/index_memory/index_wiki_note/rebuild_container/reindex_document/extract_entities。
- **持久化/保留**：表 `index_jobs`（含 progress:int、current_stage、retry_count/max_retries、priority）；仅手动 clear_completed，无自动 TTL。
- **前端**：**零调用、零 UI**。
- **能力**：列表√ 详情× 取消√ 重试√ 暂停× 日志×（仅 error_message）输出× 进度√（progress/current_stage）。

### 2.6 fine-tune-jobs —— LoRA 微调训练　业务域：DevTools

- **后端命令**（[fine_tune.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/fine_tune.rs)）：`list_training_jobs` :206、`get_training_job` :213、`create_training_job` :223、`start_training_job` :248、`cancel_training_job` :487、`delete_training_job` :499、`get_training_stats` :512（另有数据集/基座模型/适配器系列命令）。
- **状态**：[lora.rs](file:///d:/OneManager/AxInvest/src-tauri/crates/agent/src/fine_tune/lora.rs#L58-L66) `JobStatus` = `Pending / Preparing / Training / Validating / Completed / Failed / Cancelled`；TrainingJobInfo 含 progressPercent、currentLoss、outputLora；事件 `fine-tune-training-progress`；取消用 AtomicBool flag，轮询远程训练 API。
- **持久化/保留**：**job 进程内**（FINE_TUNE_STATE static HashMap，重启丢）；数据集落盘 `~/.axagent/fine_tune/datasets.json`、`samples.json`。
- **前端**：[fineTuneStore.ts](file:///d:/OneManager/AxInvest/src/stores/devtools/fineTuneStore.ts)（TrainingJobInfo 本地定义）、FineTunePage；browserMock 有 mock。
- **路由**：`/devtools/fine-tune`。
- **能力**：列表√ 详情√ 取消√ 重试× 暂停× 日志×（进度事件含 loss）输出√（outputLora）进度√。

### 2.7 tool-executions —— MCP/技能工具执行记录　业务域：MCP/会话

- **后端命令**（[mcp.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/mcp.rs)）：`list_tool_executions` :259（参数 conversation_id）；DAO `repo::tool_execution`。同文件 17 个 server CRUD/test/oauth 命令非任务源。
- **持久化/保留**：表 `tool_executions`（status 默认 pending、duration_ms、approval_status、skill_steps_json、depends_on），**随 conversation 级联删除**。
- **前端**：[mcpStore.ts](file:///d:/OneManager/AxInvest/src/stores/feature/mcpStore.ts#L141)、[agentStore.ts](file:///d:/OneManager/AxInvest/src/stores/domain/agentStore.ts#L659)；browserMock :3161。
- **能力**：列表√（按会话）详情× 取消× 重试× 暂停× 日志√（skill_steps）输出× 进度×。
- **定位**：只读审计流，适合作为任务中心「会话维度」的下钻，不适合做可控作业。

### 2.8 skill-execution-stats —— 技能执行统计（只读聚合，非任务实例）　业务域：技能

- **后端命令**（[skills/analysis.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/skills/analysis.rs)）：`get_skill_execution_stats` :444，读表 `trajectory_skills`（success_rate/usage_count/avg_execution_time_ms；quality_score/last_used_at 当前恒 null）。
- **前端**：[SkillStatsPanel.tsx](file:///d:/OneManager/AxInvest/src/components/skill/SkillStatsPanel.tsx)（SkillsPage 内）。
- **能力**：仅聚合列表，无实例生命周期。建议不纳入任务中心，或仅作统计卡片。

---

## 3. 线索之外新发现的任务源

### 3.1 knowledge-import —— 知识库目录异步导入　业务域：知识库

- **后端命令**（[knowledge.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/knowledge.rs)）：`import_knowledge_directory_async` :1303（返回 taskId）、`cancel_knowledge_import` :1404、`get_knowledge_import_status` :1417、`retry_knowledge_import_errors` :1535；另有同步版 `import_knowledge_directory` :1223、`scan_knowledge_directory` :1432。
- **状态**：phase = scan / done / cancelled / error；processed/total/imported/skipped/failed；事件 `knowledge-import-progress / completed`。
- **持久化/保留**：**纯内存**（AppState.knowledge_import_status HashMap + knowledge_import_cancels CancellationToken），重启丢。
- **前端（重要落差）**：UI 实际只调**同步版**——[DirectoryImportWizard.tsx](file:///d:/OneManager/AxInvest/src/components/settings/DirectoryImportWizard.tsx#L261) 与 [knowledgeStore.ts](file:///d:/OneManager/AxInvest/src/stores/feature/knowledgeStore.ts#L283) 调 `import_knowledge_directory`；async/cancel/status/retry 四个命令**前端零调用**。
- **能力（后端）**：列表× 详情√ 取消√ 重试√ 暂停× 日志× 输出× 进度√。

### 3.2 wiki-sync-queue —— Wiki 同步队列　业务域：Wiki

- **后端命令**（[llm_wiki.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/llm_wiki.rs)）：`wiki_sync_process_pending` :1266、`wiki_sync_enqueue` :1350、`wiki_sync_get_queue` :1387（limit 100）、`wiki_sync_process` :1414（按 id 处理单项，可用于失败重处理）。
- **状态**：[wiki_sync_queue.rs](file:///d:/OneManager/AxInvest/src-tauri/crates/entities/src/wiki_sync_queue.rs) `pending / processing / completed / failed`，retry_count ≥ 3 置 failed；SyncStatus 提供 pending/processing/failed 计数与 last_sync_at。
- **持久化/保留**：表 `wiki_sync_queue`（随 wiki 级联），无清理命令。
- **前端**：[llmWikiStore.ts](file:///d:/OneManager/AxInvest/src/stores/feature/llmWikiStore.ts)、[SyncStatus.tsx](file:///d:/OneManager/AxInvest/src/components/llm-wiki/SyncStatus.tsx)（挂 WikiGraphPage）。
- **能力**：列表√ 详情× 取消× 重试△（重新 process）暂停× 日志× 输出× 进度×（仅计数）。
- **同族**：`llm_wiki_import_folder` :1669（文件队列落盘在 `~/.axagent/wiki_{id}/import_queue`，但命令内同步逐条处理、无状态/取消命令）。

### 3.3 quant-backtest —— 量化回测　业务域：量化

- **后端命令**（[quant_backtest.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/quant_backtest.rs)）：`quant_backtest_run` :151（命令内 pending→跑→completed/failed，**IPC 同步阻塞**）、`quant_run_get` :241。**无列表/取消/重试/暂停/进度命令**。
- **持久化/保留**：表 [quant_runs](file:///d:/OneManager/AxInvest/src-tauri/crates/entities/src/quant_runs.rs)（pending/running/completed/failed、config_json/result_json/error_message + walk_forward 字段），无删除命令（永久）。
- **前端**：[backtestStore.ts](file:///d:/OneManager/AxInvest/src/stores/feature/quant/backtestStore.ts)（列表仅存 store 内存）、[types/quant.ts](file:///d:/OneManager/AxInvest/src/types/quant.ts)、QuantLabPage。
- **路由**：`/quantlab` → 重定向 `/invest?tab=quant`。
- **能力**：列表× 详情√ 取消× 重试× 暂停× 日志× 输出√ 进度×。

### 3.4 rl-training —— 强化学习训练　业务域：DevTools

- **后端命令**（[rl_training.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/rl_training.rs)）：`start_rl_training` :176、`stop_rl_training` :220（置 paused）、`get_training_metrics` :239、`save_checkpoint` :259、`run_rl_training_step` :425。
- **持久化**：training_runtime() 进程内单例（running/paused），checkpoint 落盘；重启状态丢。
- **前端**：[rlTrainingStore.ts](file:///d:/OneManager/AxInvest/src/stores/feature/rlTrainingStore.ts)；browserMock 有 mock。
- **路由**：`/devtools/rl-training`。
- **能力**：列表×（单例）详情√ 取消×（stop=paused）重试× 暂停√ 日志× 输出√（checkpoint）进度√（metrics）。

### 3.5 evaluator —— 评测 / 基准 / A-B　业务域：DevTools

- **后端命令**（[evaluator.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/evaluator.rs)）：`evaluator_run_benchmark` :42、`evaluator_run_ab_test` :230、`replay_workflow_execution` :384、`run_evaluation` :468（另有 evaluator_list_benchmarks/get_ab_results/generate_report 等）。均为**同步返回报告**，无任务表、无取消/进度。
- **前端**：[evaluatorStore.ts](file:///d:/OneManager/AxInvest/src/stores/devtools/evaluatorStore.ts)、BenchmarkConfig/BenchmarkReportView（browserMock :5388 起）。
- **路由**：`/devtools/benchmark`。
- **能力**：列表× 详情√（报告返回/结果查询）取消× 重试× 暂停× 日志× 输出△ 进度×。

### 3.6 device-sync —— 设备同步　业务域：同步

- **后端命令**（[device_sync.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/device_sync.rs)）：`full_sync` :287、`incremental_sync` :305（均 IPC 内同步执行）、`get_sync_status` :399、`get_sync_history` :584，另有策略/设备/审计系列命令。
- **持久化/保留**：表 `sync_histories`（[sync_history.rs](file:///d:/OneManager/AxInvest/src-tauri/crates/entities/src/sync_history.rs)：direction/sync_type/result/conflicts/started_at/completed_at）+ sync_change_log/sync_audit_log/sync_device/sync_policy；历史查询带 limit，无自动清理。
- **前端**：[deviceSyncStore.ts](file:///d:/OneManager/AxInvest/src/stores/feature/deviceSyncStore.ts#L424)；UI 为设置面板 [DeviceSyncPanel.tsx](file:///d:/OneManager/AxInvest/src/components/settings/DeviceSyncPanel.tsx)、[SyncHistoryPanel.tsx](file:///d:/OneManager/AxInvest/src/components/settings/SyncHistoryPanel.tsx)（设置页内，无独立路由）。
- **能力**：列表√ 详情× 取消× 重试√（再次发起）暂停× 日志√（conflicts/result）输出√ 进度×。

### 3.7 backup / restore —— 备份恢复　业务域：系统维护

- **后端命令**（[backup.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/backup.rs)）：`list_backups` :24、`create_backup` :35（同步长命令）、`restore_backup` :59，另有 get/upload/download/delete。
- **持久化/保留**：表 `backup_manifests` + 备份文件；手动 delete 管理。**自动备份走 CronJob（`create_backup_task`，已在 1.1 纳管）**；TaskState 另有 auto_backup_handle / webdav_sync_handle / api_server_handle / trajectory_cleanup_handle 等命名长任务句柄。
- **前端**：[backupStore.ts](file:///d:/OneManager/AxInvest/src/stores/feature/backupStore.ts#L47)、[BackupCenter.tsx](file:///d:/OneManager/AxInvest/src/components/settings/BackupCenter.tsx)、TitleBar 快捷入口。
- **能力**：列表√ 详情× 取消× 重试× 暂停× 日志× 输出√（manifest）进度×。

### 3.8 dream / evolution —— 记忆巩固与自进化引擎　业务域：DevTools/系统

- **后端命令**：[dream.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/dream.rs) `dream_consolidate_now` :16、`dream_get_status` :48；[evolution_engine.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/evolution_engine.rs)：`get_all_engine_status` :317、`start_engine` :355、`stop_engine` :427、`get_engine_logs` :489，以及 `run_skill_evolution_generation` :517、`run_dream_consolidation` :639、`run_coevolution_cycle` :1158、`get_coevolution_status` :1211 等一批手动代次运行。
- **持久化/保留**：引擎运行态进程内；统计表 `evolution_execution_stats`。dream 后台服务按设置（dream_enabled / min_interval_hours / min_sessions）系统自动触发。
- **前端**：[evolutionStore.ts](file:///d:/OneManager/AxInvest/src/stores/feature/evolutionStore.ts#L91)、[AdvancedSettings.tsx](file:///d:/OneManager/AxInvest/src/components/settings/AdvancedSettings.tsx#L488)。
- **能力**：列表√（引擎状态/统计）详情√ 取消√（stop_engine）重试√（再跑一代）暂停√（start/stop）日志√ 输出√（stats）进度×。

### 3.9 opc-demand-delivery —— OPC 需求投递　业务域：OPC

- **后端命令**（[demand_discovery.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/demand_discovery.rs)）：`opc_execute_demand_workflow` :598（发起工作流）、`opc_list_deliveries` :747、`opc_get_delivery` :779、`opc_update_delivery` :805、`opc_retry_delivery` :862、`opc_cancel_delivery` :903；扫描类 `run_demand_discovery_cron` :259、能力缺口系列 :367/:395/:438。
- **状态**：投递 `open / pending / completed / failed / cancelled`。
- **持久化/保留**：表 `opc_delivery`、`opc_demand_leads`、`opc_demand_subscriptions`（业务表，永久）；扫描 cron 见 1.1。
- **前端**：DemandDiscoveryPage。**路由**：`/demand-discovery`。
- **能力**：列表√ 详情√ 取消√ 重试√ 暂停× 日志×（last_error）输出√ 进度×。

### 3.10 opc-work-items —— OPC 自运行工作项（状态机）　业务域：OPC

- **后端命令**（[opc.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/opc.rs)）：`opc_kanban_board`（看板聚合列表）、`opc_create_work_item` :721、`opc_work_item_start` :909、`opc_work_item_review` :937、`opc_escalate_work_item` :1002、`opc_work_item_unblock` :1050、`run_self_improving_opc_work_item` :1281。
- **状态**：[opc_work_items.rs](file:///d:/OneManager/AxInvest/src-tauri/crates/entities/src/opc_work_items.rs#L16) phase = `QUEUED / IN_PROGRESS / WAITING_FOR_CHILDREN / BLOCKED / REVIEW / APPROVED / DONE / FAILED / CANCELLED`；`run_id` 可关联 DAG 工作流执行（2.2）。
- **持久化/保留**：表 `opc_work_items`（永久）。
- **前端**：[KanbanTab.tsx](file:///d:/OneManager/AxInvest/src/pages/opc/components/KanbanTab.tsx)。**路由**：`/opc`。
- **能力**：列表√（看板）详情△ 取消×（用状态迁移）重试× 暂停×（REVIEW/BLOCKED 是流程态非暂停）日志× 输出× 进度√（phase）。

### 3.11 fleet —— 多智能体编队调度　业务域：多智能体

- **后端命令**（[fleet/mod.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/fleet/mod.rs)，14 个）：fleet_list :59、fleet_get :73、create :100、`fleet_update_status` :130、delete :145、members 系列 :161/:208/:254/:268/:283、`fleet_reset_daily_tokens` :297、**`fleet_dispatch` :326（真实 agent 回合，经 channel 流式回推）**、fleet_direct_message :480、fleet_list_messages :549。
- **状态**：fleet active/paused/stopped；member 独立状态。
- **持久化/保留**：表 `fleets / fleet_members / fleet_messages`（永久）。
- **前端**：MultiAgentPage；[officeStore.ts](file:///d:/OneManager/AxInvest/src/stores/feature/officeStore.ts#L356) 也调 fleet_dispatch。
- **路由**：`/multi-agent`。
- **能力**：列表√ 详情√ 取消√（置 stopped）重试× 暂停√ 日志√（messages）输出√ 进度×。

### 3.12 sub-agent —— 子 Agent 注册表　业务域：对话/编排

- **后端命令**（[sub_agent/mod.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/sub_agent/mod.rs)）：`sub_agent_list` :13、`sub_agent_get` :22、`sub_agent_get_children` :39、`sub_agent_get_messages` :51。
- **持久化**：注册表在 [trajectory/sub_agent.rs](file:///d:/OneManager/AxInvest/src-tauri/crates/trajectory/src/sub_agent.rs)（**进程内，无实体表**）；生命周期态见 harness `delegation.rs`（SubAgentLifecycleState）。
- **能力**：列表√ 详情√ 取消× 重试× 暂停× 日志√（messages）输出√ 进度×。

### 3.13 reminders —— 提醒（自带调度器）　业务域：主动服务

- **后端命令**（[reminder.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/reminder.rs)）：reminder_create/list/complete/delete + `reminder_cleanup` :268 等 8 个；ReminderManager 进程内按时间触发。
- **持久化/保留**：JSON 文件 `~/.axagent/data/reminders.json`（非 DB），cleanup 命令清理过期项。
- **前端**：[proactiveStore.ts](file:///d:/OneManager/AxInvest/src/stores/feature/proactiveStore.ts#L299)（:374/:397/:411）、SuggestionPanel/PrefetchIndicator。
- **能力**：列表√ 详情× 取消√（delete）重试× 暂停×（complete 办结）日志× 输出× 进度×。

### 3.14 market-sweeps —— 收盘/快照清扫　业务域：行情

- **后端命令**：[market_close_sweep.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/market_close_sweep.rs) `sweep_market_close` :23、`refresh_stock_universe` :51；[stock_analysis.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/stock_analysis.rs) `sweep_daily_snapshots` :4336、`run_daily_snapshot_sweep` :4358（命令与后台 tick 共用）。
- **持久化**：业务表 market_daily_close / market_stock_universe / astock 日线快照；**无运行记录表**，命令返回摘要字符串。
- **触发**：手动 + 系统后台 tick + cron。
- **能力**：列表× 详情× 取消× 重试√（再跑）暂停× 日志× 输出△ 进度×。

### 3.15 scheduler-coordination —— 长时任务协调（编排层，非独立源）　业务域：系统

- [scheduler/mod.rs](file:///d:/OneManager/AxInvest/src-tauri/src/scheduler/mod.rs)：`set_task_priority` :38、`set_budget` :48、`get_budget_usage` :58、`get_task_report` :65（已确认注册于 register_commands.rs :1564-1568）；子模块 queue/gate/report/restore；夜间一次性（recurring=false）CronJob 也入 CronJobStore。
- **结论**：它是 1.1 + 2.1 的调度/预算编排层，**不应作为独立任务源**，但其 TaskReport/BudgetState 是任务中心头部摘要的现成数据源。

### 3.16 其它边缘/不建议纳管项（盘点备查）

| 项 | 证据 | 不纳管理由 |
|---|---|---|
| 单股长时分析 | `cancel_stock_analysis` [stock_analysis.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/stock_analysis.rs#L1163)，运行态在 stock_analyses 行 + 进度事件 | 交互态业务，UI 在投资工作台；如需可只接「取消/状态」 |
| 计划执行 plan | [plan.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/plan.rs) `plan_execute` :1006、`plan_cancel` :1376；plans 表；[planStore.ts](file:///d:/OneManager/AxInvest/src/stores/feature/planStore.ts#L184) | 对话内规划，无列表/历史命令 |
| Wiki 文件夹导入 | `llm_wiki_import_folder` [llm_wiki.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/llm_wiki.rs#L1669) | 命令内同步跑完，文件队列无状态查询 |
| 网关资源同步 | `sync_all_gateway_link_models` :272 / `sync_all_gateway_link_skills` :370（gateway_link.rs） | 一次性同步，同步记录表为审计 |
| Gateway `/api/jobs*` | [crates/gateway/src/handlers/jobs.rs](file:///d:/OneManager/AxInvest/src-tauri/crates/gateway/src/handlers/jobs.rs)（18 个端点） | 是**上游 LLM provider 异步 job 的 HTTP 透传**，非本地任务 |
| 技能/插件安装 | install_skill [install.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/skills/install.rs#L370)、插件命令 | 同步命令，无作业状态机 |
| 行情价格监控 | analysis-engine monitor 后台循环 + price_alerts | 系统常驻服务，无「一次运行」语义 |
| remote-agent 任务 | [remote_agent_task.rs](file:///d:/OneManager/AxInvest/src-tauri/src/runtime/tasks/remote_agent_task.rs)（feature REMOTE_AGENT，状态 connecting/connected/reconnecting/failed/completed） | 编译期 feature，连接态而非作业 |
| Agent 内部任务 DAG | [crates/agent/src/task.rs](file:///d:/OneManager/AxInvest/src-tauri/crates/agent/src/task.rs)、task_decomposer；内置工具 `db_list_tasks` 读的就是 background_tasks | 随会话存活，已被 2.1/2.3 覆盖 |
| 会话压缩 | [compress.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/conversations/compress.rs#L273) `compress_context` :273 | 对话内同步动作 |

---

## 4. 总能力矩阵（源 × 能力）

图例：√ 有命令/可消费　× 无　△ 部分/间接（见各源说明）

| # | 源 key | 列表 | 详情 | 取消 | 重试 | 暂停 | 日志 | 输出 | 进度 | 持久化 |
|---|---|---|---|---|---|---|---|---|---|---|
| 1 | scheduled-cron | √ | √ | ×（删除√） | √（立即执行） | √ | √（历史50条） | √ | × | cron_jobs + cron_job_history |
| 2 | stock-pipeline | √ | √ | × | × | × | ×（仅实时事件） | √ | √（step 事件） | stock_pipeline_runs |
| 3 | time-travel-batch | √ | √ | × | × | × | × | √ | √（阶段/计数轮询） | 内存 + time_travel_batch_runs |
| 4 | replay-sweep | × | × | × | × | × | × | △（同步返回） | × | 无 |
| 5 | background-tasks | √ | √ | √ | ×（字段预留） | × | √ | √ | × | background_tasks + task_events |
| 6 | workflow-dag | √ | √ | √ | √ | √ | √ | √ | √ | workflow_executions 等 3 表 |
| 7 | agent-session | √ | √ | √ | × | √ | × | × | × | agent_sessions + 进程 broker |
| 8 | parallel-execution | √ | √ | √ | × | × | × | √ | △ | **纯内存** |
| 9 | index-jobs | √ | × | √ | √ | × | × | × | √ | index_jobs |
| 10 | fine-tune-jobs | √ | √ | √ | × | × | × | √ | √ | **进程内**（数据集落盘） |
| 11 | tool-executions | √ | × | × | × | × | √ | × | × | tool_executions（随会话删） |
| 12 | skill-execution-stats | √（聚合） | × | × | × | × | × | × | × | trajectory_skills |
| 13 | knowledge-import | × | √ | √ | √ | × | × | × | √ | **纯内存**（前端仅用同步版） |
| 14 | wiki-sync-queue | √ | × | × | △ | × | × | × | × | wiki_sync_queue |
| 15 | quant-backtest | × | √ | × | × | × | × | √ | × | quant_runs（同步阻塞） |
| 16 | rl-training | ×（单例） | √ | × | × | √ | × | √ | √ | **进程内** + checkpoint 文件 |
| 17 | evaluator | × | √ | × | × | × | × | △ | × | 无任务表（报告即返回） |
| 18 | device-sync | √ | × | × | √ | × | √ | √ | × | sync_histories 等 5 表 |
| 19 | backup/restore | √ | × | × | × | × | × | √ | × | backup_manifests |
| 20 | dream/evolution | √ | √ | √ | √ | √ | √ | √ | × | evolution_execution_stats + 进程内 |
| 21 | opc-demand-delivery | √ | √ | √ | √ | × | × | √ | × | opc_delivery / leads |
| 22 | opc-work-items | √ | ×（看板） | ×（迁移态） | × | × | × | × | √（phase） | opc_work_items |
| 23 | fleet | √ | √ | √ | × | √ | √ | √ | × | fleets / members / messages |
| 24 | sub-agent | √ | √ | × | × | × | √ | √ | × | **进程内注册表** |
| 25 | reminders | √ | × | √ | × | ×（办结） | × | × | × | reminders.json 文件 |
| 26 | market-sweeps | × | × | × | √ | × | × | △ | × | 仅业务数据表，无运行记录 |

**按纳管成本分档（适配器视角）**：

- **A 档｜有列表+详情+持久化，直接适配**：scheduled-cron（已接）、background-tasks、workflow-dag、index-jobs、fine-tune-jobs、stock-pipeline、time-travel-batch、opc-demand-delivery、device-sync、fleet、dream/evolution。
- **B 档｜持久化但能力残缺，需后端补命令**：quant-backtest（补列表/异步化）、wiki-sync-queue（补取消/详情）、tool-executions（只读）、opc-work-items（缺 id 详情）、backup（缺进度/详情）。
- **C 档｜纯内存，重启丢失，先补持久化再谈纳管**：parallel-execution、knowledge-import、rl-training、sub-agent。
- **不建议纳管**：skill 统计（聚合）、evaluator/replay-sweep（同步返回）、reminders（领域对象非作业）、market-sweeps（无运行记录）、scheduler-coordination（编排层，做摘要源即可）。

---

## 5. 证伪与落差（实施前必看）

1. **agent-bus 在当前代码库不存在**。对 `agent-bus / agent_bus / registerAgent / sendTask / register_agent / send_task / poll_task / list_tasks` 在 src-tauri、crates、src 全量 grep，无任何 MCP agent-bus 实现。仅有三个同名近缘物：harness `capability_registry.rs` 的 `register_agent_loop`（内部能力注册）、[tools/src/tools/agent.rs](file:///d:/OneManager/AxInvest/src-tauri/crates/tools/src/tools/agent.rs) 的 AGENT_REGISTRY（内存）、内置工具 [task_system.rs](file:///d:/OneManager/AxInvest/src-tauri/crates/tools/src/task_system.rs) 的 `db_list_tasks`（读 background_tasks 表）。**线索中的 agent-bus 适配器应删除**。
2. **两个「零前端」后端源**：index-jobs（2.5，7 个命令无 UI）、parallel-execution（2.4，12 个命令无 UI）。属于已建成未消费的设施，是任务中心最高性价比的接入对象。
3. **knowledge-import 有异步 API 但前端在用同步版**：async/cancel/status/retry 四命令已实现（[knowledge.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/knowledge.rs#L1303) :1303 起），UI 仍走 :1223 同步版。纳管时应先切换消费方。
4. **import_jobs 是预留空表**：[import_jobs.rs](file:///d:/OneManager/AxInvest/src-tauri/crates/entities/src/import_jobs.rs) 注释明确「已建表但无命令写入」，规划中的统一导入追踪，当前不算任务源。
5. **两套 WorkEngine 物理位置易混**：命令 [work_engine.rs](file:///d:/OneManager/AxInvest/src-tauri/src/commands/work_engine.rs) 用的是 `axagent_runtime::work_engine`（wiring）；stock-pipeline 用 `axagent_rt_workflow::work_engine::WorkEngine`；旧命令面 workflows/* 最终也收敛到同一 DAG 引擎与 `workflow_executions` 表。适配器按「执行表」归一，不要按命令文件拆源。
6. **DTO 例外**：CronJob 的 `ExecutionRecord` 无 `rename_all`，历史记录是 snake_case；index_jobs/wiki_sync_queue/quant_runs 实体本身带 camelCase serde。适配器映射层需逐源核对，不能假设全站 camelCase。
7. **保留策略普遍缺失**：除 cron 历史 50 条/任务、tool_executions 随会话级联、index_jobs 提供手动清理、reminders 有 cleanup 外，绝大多数作业表（background_tasks、pipeline、batch、quant_runs、wiki_sync_queue、sync_histories、backup_manifests、opc/fleet 业务表）**没有自动 TTL**。任务中心若做统一列表，需要同时规划归档/清理策略。
8. **既有任务中心的范围**（`tasks.md` 13 个任务）仅覆盖 scheduled-cron + pipeline + batch + replay 四个源；本清单其余 22 个源均在既有 spec 范围外，建议按上文 A/B/C 档分批扩 spec，而非一次性接入。
