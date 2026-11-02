# L1 功能包拆分阻塞面实测（2026-11-03）

本文件由 `node scripts/audit-l1-blockers.mjs` 生成，是 PLAN §0.10 的数字来源。**改判据请改脚本，不要手抄。**

| 度量 | 股票 STOCK | OPC |
|---|---|---|
| 命令条数 / 命令模块数（**插件边界口径**，本脚本 `cmdRe`） | 304 / 20 | 138 / 13 |
| 命令条数 / 模块数（**审计 19 域口径**，`lib/command-domains.mjs`） | 306 / 19（域「股票分析与管线」） | 117 / 10（域「OPC 公司运行时」） |
| **Rust 反向阻塞（待改写，H 桶除外）** | **142 处 / 24 文件** | **0 处 / 0 文件** |
| Rust H 桶（`l1-host-owned-allowlist.json` 逐行点名的宿主持有，不参与「归零」） | 10 处 / 4 文件 | 0 处 / 0 文件 |
| 同上**合计**（= 待改写 + H 桶，与旧 152 口径可比） | 152 处 / 28 文件 | 0 处 / 0 文件 |
| Rust 跨包口（一个功能包的文件调另一个功能包的 crate ⇒ 要走 `call_seam`） | 29 处 / 9 文件 | 0 处 / 0 文件 |
| Rust 域自有文件内引用（随迁出搬走，不算阻塞） | 455 处 / 40 文件 | 16 处 / 2 文件 |
| Cargo.toml 构建边 | 8 | 1 |
| **前端反向阻塞（非域 import 域文件）** | **54 处 / 39 文件** | **28 处 / 11 文件** |
| entities 表文件 | 20 | 28 |
| zh-CN i18n key | 3527 | 1897 |
| 前端域自有文件 / 行数（迁出时的体量） | 293 文件 / 84873 行 | 40 文件 / 12797 行 |
| 注册表命令总数 / 迁出后主壳剩余 | 1543 条 ⇒ 迁完剩 1101 条 | 同左 |

## 股票：142 处 Rust 反向阻塞逐条（每文件前列 8 个行号）

| 文件 | 处数 | 行号 |
|---|---|---|
| `axagent-runtime/crates/tools/src/tools/opc.rs` | 41 | 87, 88, 185, 186, 274, 275, 344, 345 |
| `src-tauri/src/init/services.rs` | 28 | 2283, 2368, 2552, 2598, 2632, 2682, 2688, 3387 |
| `src-tauri/src/init/state.rs` | 10 | 1604, 1678, 1698, 1799, 1828, 1830, 1898, 2415 |
| `src-tauri/src/bin/axagent-validate-backfill.rs` | 8 | 533, 663, 688, 888, 889, 1147, 1152, 1301 |
| `src-tauri/src/commands/evolution_proposal.rs` | 7 | 38, 39, 40, 41, 244, 245, 296 |
| `axagent-runtime/crates/tools/src/tools/finance.rs` | 6 | 9, 13, 1178, 1180, 1189, 1192 |
| `axagent-runtime/crates/tools/src/audit.rs` | 4 | 167, 169, 545, 547 |
| `src-tauri/src/commands/conversations/compress.rs` | 4 | 832, 840, 842, 847 |
| `src-tauri/src/commands/conversations/mod.rs` | 4 | 3052, 3060, 3062, 3067 |
| `src-tauri/src/commands/fleet/mod.rs` | 4 | 858, 914, 978, 1012 |
| `axagent-runtime/crates/tools/src/global_state.rs` | 3 | 63, 68, 73 |
| `axagent-runtime/crates/tools/src/tools/astock_data.rs` | 3 | 110, 119, 135 |
| `src-tauri/src/bin/axagent-batch-rerun.rs` | 3 | 44, 418, 716 |
| `src-tauri/src/commands/background_tasks.rs` | 3 | 8, 350, 916 |
| `src-tauri/src/commands/screenshot_diagnosis.rs` | 3 | 29, 33, 456 |
| `src-tauri/src/init/news_archive_sink.rs` | 2 | 12, 13 |
| `src-tauri/src/market_sim_service.rs` | 2 | 29, 103 |
| `axagent-runtime/crates/tools/src/tools/capability_pack_defs.rs` | 1 | 18 |
| `axagent-runtime/crates/tools/src/tools/industry_constituents.rs` | 1 | 23 |
| `axagent-runtime/crates/tools/src/tools/market_mainline.rs` | 1 | 47 |
| `axagent-runtime/crates/tools/src/tools/stock_factor.rs` | 1 | 21 |
| `src-tauri/src/commands/decision_gates.rs` | 1 | 29 |
| `src-tauri/src/commands/_shared_reco_pool.rs` | 1 | 21 |
| `src-tauri/src/init/daily_snapshot_store.rs` | 1 | 42 |

### 股票：H 桶 10 处（白名单逐行点名豁免；每行都要能拿 anchorText 在盘上复核）

- `src-tauri/src/app_state.rs:244` —— 字段类型即「宿主持有引擎」的证据（res109 A2 标 H→X）。B6 摘字段后该行消失，豁免随之失效并由 --check-allowlist 报「文件缺失/锚点腐烂」⇒ 那时删条目，不是现在。
- `src-tauri/src/app_state.rs:247` —— 同上：监控引擎的宿主持有位。stock.monitor 接缝是**加法式**暴露读/写面，不改变宿主持有该 Arc 这一事实（PLAN §55.6）。
- `src-tauri/src/app_state.rs:257` —— 同上：行情订阅集合的宿主持有位（数据主权在宿主，插件只经 quote.watcher 的 watch/unwatch 改集合内容）。
- `src-tauri/src/app_state.rs:259` —— 同上：交易引擎的宿主持有位（trading.engine 接缝 §51 已交付，执行权仍在宿主）。
- `src-tauri/src/app_state.rs:262` —— 同上：组合信号聚合器的宿主持有位，已并入 stock.monitor 契约面（§55.1），持有关系不变。
- `src-tauri/src/app_state.rs:266` —— 同上：自适应引擎的宿主持有位（res109 把 stock_adaptive_engine 4 处归 P 桶的是**调用点**，本行是 AppState 字段，属持有证据）。
- `src-tauri/src/commands/gateway.rs:579` —— OpenAI 兼容网关的行情发布口 = 宿主对外的数据面（res109 A2 判「不可消」，§E-7）。迁股票插件时这条读的是宿主持有的 client，不是插件内部件。
- `src-tauri/src/commands/gateway.rs:609` —— 同上：网关侧自建轮询流，属宿主对外数据面的实现细节（res109 A2 gateway 行 :609）。
- `src-tauri/src/init/browser_fetcher.rs:16` —— 宿主为 vendor 提供 HTTP 能力的注入口（res109 A2 判「不可消」，§E-6）；MCP / 取数重试扇出同族，属宿主取数设施。
- `src-tauri/src/init/monitor_emitter.rs:25` —— 宿主事件出口。硬口径：告警触发后的工作流回写留在宿主（PLAN §47.2 主权条），接缝只把它补发到 EventDispatchBus，不把发射器交给插件。

### 股票：跨包口 29 处（调用方本身也是待迁功能包的文件）

- `src-tauri/src/commands/opc_capability_pack_actions.rs` 10 处 @13,2248,2249,2255,2509,2593,2733,2756
- `src-tauri/src/commands/opc.rs` 4 处 @15,27,665,668
- `src-tauri/src/commands/opc_capability_pack_runtime.rs` 4 处 @13,18,24,43
- `src-tauri/src/commands/opc_workflows/mod.rs` 3 处 @52,59,656
- `src-tauri/src/commands/opc_workflow_kpi_hook.rs` 3 处 @109,430,432
- `src-tauri/src/commands/opc_learning_hook.rs` 2 处 @64,67
- `src-tauri/src/commands/demand_discovery.rs` 1 处 @46
- `src-tauri/src/commands/opc_capability_pack_logic.rs` 1 处 @18
- `src-tauri/src/commands/opc_data.rs` 1 处 @12

### 股票：前端反向阻塞 54 处
- `src/stores/index.ts` 5 处 @102,104,106,107,108
- `src/pages/TasksCenterPage/OverviewTab.tsx` 3 处 @3,6,7
- `src/types/index.ts` 3 处 @2376,2381,2382
- `src/components/chat/WorkflowAgentCard.tsx` 2 处 @2,11
- `src/components/time-travel/ReplayWorkbench.tsx` 2 处 @1,6
- `src/lib/agentOutput.ts` 2 处 @20,29
- `src/lib/decision-agreement.ts` 2 处 @37,38
- `src/pages/OpcPage.tsx` 2 处 @22,25
- `src/pages/TasksCenterPage/forms/TaskCreateModal.tsx` 2 处 @25,26
- `src/pages/TasksCenterPage/__tests__/OverviewTab.evolutionApproval.test.tsx` 2 处 @2,3
- `src/components/chat/ChatViewMessages.tsx` 1 处 @87
- `src/components/chat/InputArea.tsx` 1 处 @73
- `src/components/chat/MultiLlmVotePanel.tsx` 1 处 @18
- `src/components/layout/ContentArea.tsx` 1 处 @46
- `src/components/office/panels/StrategyRoomMiniBar.tsx` 1 处 @14
- `src/components/office/panels/TrajectoryPanel.tsx` 1 处 @22
- `src/components/office/phaser/__tests__/sceneTemplates.labelKey.test.ts` 1 处 @4
- `src/components/settings/DataVendorsTab.tsx` 1 处 @446
- `src/components/settings/index.ts` 1 处 @30
- `src/components/settings/WorkflowSettings.tsx` 1 处 @5
- `src/components/time-travel/batchTables.tsx` 1 处 @3
- `src/components/time-travel/TimeTravelBatchCard.tsx` 1 处 @4
- `src/hooks/useRightPanel.ts` 1 处 @1
- `src/lib/dataQualityDiagnosis.ts` 1 处 @15
- `src/lib/decisionInputDiagnosis.ts` 1 处 @21
- `src/lib/__tests__/agentOutput.test.ts` 1 处 @16
- `src/lib/__tests__/analysisHistoryFilter.test.ts` 1 处 @12
- `src/lib/__tests__/analystSentiment.test.ts` 1 处 @11
- `src/lib/__tests__/analystTallyRealFixtures.test.ts` 1 处 @12
- `src/main.tsx` 1 处 @44
- `src/pages/ScreenerPage.tsx` 1 处 @1
- `src/pages/ScreenshotDiagnosisPage.tsx` 1 处 @1
- `src/pages/SettingsPage.tsx` 1 处 @71
- `src/pages/TasksCenterPage/adapters/registry.ts` 1 处 @9
- `src/pages/TasksCenterPage/RunDetailDrawer.tsx` 1 处 @15
- `src/pages/TasksCenterPage/__tests__/adapters.test.ts` 1 处 @17
- `src/stores/feature/screenshotDiagnosisStore.ts` 1 处 @19
- `src/theme/__tests__/tokenSingleSource.test.ts` 1 处 @14
- `src/utils/__tests__/workflowNodeLabel.test.ts` 1 处 @1

### 股票：Cargo 构建边
- axagent-quant/crates/analysis-engine/Cargo.toml  → axagent-astock-data
- axagent-quant/crates/company-runtime/Cargo.toml  → axagent-analysis-engine
- axagent-runtime/crates/tools/Cargo.toml  → axagent-astock-data
- axagent-runtime/crates/tools/Cargo.toml  → axagent-analysis-engine
- src-tauri/Cargo.toml  → axagent-astock-data
- src-tauri/Cargo.toml  → axagent-analysis-engine
- src-tauri/Cargo.toml  → axagent-quant
- src-tauri/Cargo.toml  → axagent-market-sim

### 股票：命令模块分布（前 8）
`stock_analysis`(172) · `stock_workflow`(24) · `paper`(20) · `market_mainline`(10) · `paper_portfolio`(9) · `backtest_validation`(8) · `recommendation_cron`(8) · `trade_intent`(8)

### 股票：entities 表文件
`astock_daily_snapshot.rs` · `market_daily_close.rs` · `market_mainlines.rs` · `market_stock_universe.rs` · `market_universe_stats.rs` · `paper_portfolios.rs` · `portfolio_correlation_snapshot.rs` · `portfolio_holdings.rs` · `portfolio_metrics_daily.rs` · `quant_paper_trades.rs` · `quant_runs.rs` · `quant_signals.rs` · `quant_strategies.rs` · `stock_analyses.rs` · `stock_evolution_history.rs` · `stock_execution_reflections.rs` · `stock_pipeline_runs.rs` · `stock_reflections.rs` · `workflow_marketplace.rs` · `workflow_marketplace_review.rs`

## OPC：0 处 Rust 反向阻塞逐条（每文件前列 8 个行号）

| 文件 | 处数 | 行号 |
|---|---|---|

### OPC：H 桶 0 处（白名单逐行点名豁免；每行都要能拿 anchorText 在盘上复核）

- （空：本域尚无点名豁免）

### OPC：跨包口 0 处（调用方本身也是待迁功能包的文件）


### OPC：前端反向阻塞 28 处
- `src/components/layout/ContentArea.tsx` 15 处 @53,56,59,62,65,68,71,74
- `src/components/stock-analysis/DebatePanel.tsx` 2 处 @8,9
- `src/components/stock-analysis/dual-view/DecisionComparisonPanel.tsx` 2 处 @23,24
- `src/components/stock-analysis/RiskMatrix.tsx` 2 处 @19,20
- `src/components/stock-analysis/AnalystReportGrid.tsx` 1 处 @20
- `src/components/stock-analysis/DecisionBanner.tsx` 1 处 @35
- `src/components/stock-analysis/dual-view/CompactDecisionComparison.tsx` 1 处 @26
- `src/components/stock-analysis/ReflectionPanel.tsx` 1 处 @45
- `src/pages/DemandDiscoveryPage.tsx` 1 处 @61
- `src/stores/feature/domainLearningStore.ts` 1 处 @12
- `src/types/index.ts` 1 处 @4

### OPC：Cargo 构建边
- src-tauri/Cargo.toml  → axagent-company-runtime

### OPC：命令模块分布（前 8）
`opc`(57) · `opc_capability_pack_actions`(20) · `opc_capability_pack_runtime`(14) · `demand_discovery`(11) · `opc_demand_discovery`(8) · `agency_expert`(7) · `opc_delivery`(5) · `opc_demand_subscription`(5)

### OPC：entities 表文件
`agency_experts.rs` · `opc_automation_rules.rs` · `opc_blog_posts.rs` · `opc_capability_gap.rs` · `opc_capability_packs.rs` · `opc_contact_submissions.rs` · `opc_content_assets.rs` · `opc_customers.rs` · `opc_delivery.rs` · `opc_demand_leads.rs` · `opc_demand_platforms.rs` · `opc_demand_subscriptions.rs` · `opc_experience_records.rs` · `opc_follow_up_tasks.rs` · `opc_invoices.rs` · `opc_kpi_records.rs` · `opc_landing_pages.rs` · `opc_orgs.rs` · `opc_org_employees.rs` · `opc_org_roles.rs` · `opc_playbooks.rs` · `opc_projects.rs` · `opc_publish_schedules.rs` · `opc_revenue_records.rs` · `opc_rl_experience.rs` · `opc_rl_training_stats.rs` · `opc_talent_templates.rs` · `opc_work_items.rs`

## 判读口径

1. 「反向阻塞」只统计**非域自有文件**；`commands/stock_*` / `commands/opc_*` 等按「粒度 = 能力」与插件同迁，不计入。
2. 行级匹配排除注释行与 `examples/`、`.workbuddy/`、`output/` 目录。
3. 前端归属用文件名/路径正则（`feRe`），命名不规范者会误判，需人工认领异常行。
4. **OPC 的直接 Rust 阻塞为 0 ≠ OPC 干净**：OPC 的工具面物理住在 `axagent-runtime/crates/tools/src/tools/opc.rs`（41 处），
   它 import 的是**股票 crate**（`astock_data` / `analysis_engine`）而非 `company_runtime` ⇒ 这 41 处计在股票名下。
   读法：迁 OPC 时要连带处理 `tools/opc.rs` 与 `tools/capability_pack_defs.rs`，迁股票时要处理 `tools/{finance,audit,global_state,astock_data}.rs`。
5. 本脚本与 §0.10 的口径**以本脚本为准**（早前一次性脚本把 `commands/opc_*` 误并入股票桶，故报 210/70；正确读数是 142 / 0）。
6. **两套命令口径并存且必须并列**：插件边界口径（本脚本）按「迁出后归哪个插件」划分；审计 19 域口径（`scripts/lib/command-domains.mjs`）按模块命名正则划分。
   本次差集（11 个模块被两套规则归到不同边）：agency_expert（边界口径 OPC / 审计口径 —）；backtest_validation（边界口径 STOCK / 审计口径 —）；demand_discovery（边界口径 OPC / 审计口径 STOCK）；dojo_sdk（边界口径 OPC / 审计口径 —）；market_sim（边界口径 STOCK / 审计口径 —）；mover_recall（边界口径 — / 审计口径 STOCK）；mover_recall_analysis（边界口径 — / 审计口径 STOCK）；narrative（边界口径 — / 审计口径 STOCK）；quant_backtest（边界口径 STOCK / 审计口径 —）；time_travel_batch（边界口径 STOCK / 审计口径 —）；wf_des（边界口径 STOCK / 审计口径 —）。
   ⇒ 引用命令数时必须同时说明是哪套口径；PLAN §0.10 用**边界口径**，`AUDIT-capability-inventory` 用**审计口径**。
7. **H 桶 = 白名单逐行点名豁免，不是正则扩面**（PLAN §54 R5）：`node scripts/audit-l1-blockers.mjs --check-allowlist` 会拿每条 `anchorText` 回盘上原文复核，
   找不到 / 重复 / 计入不了 H 桶（锚点在但该行今天不算反向阻塞）三类一律 EXIT=1。
   ⇒ 股票迁移的终态判据是「**待改写归零 + H 桶等于白名单**」，不是「152 打到 0」；合计行与旧 152 口径可比，不许拿它当进度。

## 白名单复扫（10 条登记 / 逐条锚点复核通过）

