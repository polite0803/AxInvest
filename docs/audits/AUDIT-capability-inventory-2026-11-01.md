# AUDIT-capability-inventory-2026-11-01 — AxInvest 功能点全集清点（一次扫全）

> 生成方式：`node .tmp/full-inventory.cjs`（本文件由脚本直接落盘，非手抄）。
> 权威骨架 = `src-tauri/src/register_commands.rs`（build.rs 生成的命令注册表）；
> 交叉源 = 前端 `src/**` 的 invoke 消费方、`commands/**` 的代码行与 `axagent_*` 引用、全仓 `pub trait` 与其实现者分布、`app_state.rs`/`init/state.rs` 的 `Arc<dyn T>` 全局持有面。
> 口径唯一：层级判据见 `PLAN-shell-plus-plugins.md` §0.9.4。

## 1. 总量

| 面 | 数量 |
|---|---|
| Tauri 命令（注册表逐条） | **1541** |
| 命令模块 | **164** |
| 功能域（下表） | **19** |
| `pub trait`（全仓） | **316**（harness 233） |
| L2 提供方接缝：已建 | 14 |
| L2 提供方接缝：待建 | 19 |
| L2 候选（有外部实现者，无键控分发点） | 198 |
| L3 策略位（0 外部实现者，壳层全局持有） | 4 |
| L3 策略位（仅内置实现） | 52 |
| 空接口（全仓无实现） | 29 |
| L0 内核/实现层接口 | 0 |

## 2. 功能域全集（含体量、后端 crate、前端消费方）

| # | 功能域 | 命令模块 | 命令数 | 代码行 | 后端 crate | 前端消费方（目录/页/store） | 跨面共享消费方 | 被别的面消费的命令数 | 层级判定 |
|---|---|---|---|---|---|---|---|---|---|
| 1 | 智能体引擎与编排 | 22 个（共 184 条命令） | 184 | 11,505 | agent_macro crypto dao entities harness providers trajectory kit +6 | lib、page:QuickBarPage、store:agentRoleStore、store:agentStore、store:appConfigStore、store:cacheStore、store:conversationStore、store:conversationStoreEvents +18 | 15 | 84 | L0 公共底座（被 ≥8 个别的面的消费方共享） |
| 2 | 对话与消息 | 10 个（共 57 条命令） | 57 | 3,614 | agent_macro dao harness entities analysis_engine astock_data rt_workflow search +1 | lib、page:PersonaPage、page:QuickBarPage、store:categoryStore、store:compressStore、store:conversationStore、store:conversationStoreEvents、store:conversationStoreSend +9 | 11 | 40 | L0 公共底座（被 ≥8 个别的面的消费方共享） |
| 3 | 设置 / 安全 / 应用外壳 | 13 个（共 101 条命令） | 101 | 5,567 | agent agent_macro dao harness storage crypto entities kit +5 | lib、store:appConfigStore、store:conversationStoreEvents、store:memoryStore、store:nudgeStore、store:onboardingStore、store:proactiveStore、store:searchStore +7 | 10 | 51 | L0 公共底座（被 ≥8 个别的面的消费方共享） |
| 4 | 工作流引擎与编辑器 | 11 个（共 83 条命令） | 83 | 7,204 | agent_macro market_sim quant dao harness rt_workflow runtime tools +3 | lib、page:DemandDiscoveryPage、page:WorkflowMarketplace、store:approvalStore、store:stockAnalysisStore、store:workEngineStore、store:workflowEditorStore、store:workflowStore +5 | 10 | 49 | L0 公共底座（被 ≥8 个别的面的消费方共享） |
| 5 | 股票分析与管线 | 19 个（共 306 条命令） | 306 | 18,040 | agent_macro analysis_engine dao entities harness astock_data rt_workflow tools +4 | lib、page:DemandDiscoveryPage、store:crossMarketStore、store:paperPortfolioStore、store:paperStore、store:pipelineStore、store:readingListStore、store:stockAnalysisStore +5 | 7 | 167 | L1 功能包 · 深耦合（跨面共享消费方 7） |
| 6 | 知识库 / RAG / Wiki | 11 个（共 131 条命令） | 131 | 11,827 | agent_macro entities runtime dao document_parser harness search tools +5 | lib、page:QuickBarPage、page:WikiGraphPage、store:dynamicUIStore、store:knowledgeGraphStore、store:knowledgeSourceStore、store:knowledgeStore、store:sourceStore +4 | 7 | 33 | L1 功能包 · 深耦合（跨面共享消费方 7） |
| 7 | 技能与工具 | 13 个（共 78 条命令） | 78 | 3,974 | harness tools agent agent_macro dao entities runtime trajectory +3 | lib、page:SkillsPage、page:WorkflowMarketplace、store:artifactStore、store:capabilityStore、store:decompositionStore、store:localToolStore、store:proactiveStore +7 | 7 | 18 | L1 功能包 · 深耦合（跨面共享消费方 7） |
| 8 | 数据与文件 | 8 个（共 58 条命令） | 58 | 2,946 | agent_macro dao harness storage crypto kit migration | lib、store:artifactStore、store:backupStore、store:knowledgeStore、store:userProfileStore、ui:chat、ui:files、ui:layout +2 | 7 | 42 | L1 功能包 · 深耦合（跨面共享消费方 7） |
| 9 | 终端 / 桌面控制 / 多媒体 | 12 个（共 92 条命令） | 92 | 3,096 | agent_macro kit harness providers dao device tools credential +4 | lib、store:deviceSyncStore、store:screenshotDiagnosisStore、store:terminalStore、ui:chat、ui:layout、ui:settings、ui:terminal | 4 | 28 | L1 功能包 · 深耦合（跨面共享消费方 4） |
| 10 | 模型提供方 | 5 个（共 71 条命令） | 71 | 4,464 | agent agent_macro crypto dao entities harness kit providers +2 | page:QuickBarPage、store:llmWikiStore、store:proactiveStore、store:providerStore、ui:llm-wiki、ui:settings、ui:wiki | 4 | 28 | L1 功能包 · 深耦合（跨面共享消费方 4） |
| 11 | 轨迹 / 反思 / 进化 / RL | 10 个（共 86 条命令） | 86 | 4,540 | agent_macro harness trajectory agent crypto dao runtime tools +3 | page:LearningGraphPage、store:evaluatorStore、store:evolutionStore、store:fineTuneStore、store:rlTrainingStore、store:tracerStore、ui:chat、ui:layout | 3 | 16 | L1 功能包 · 深耦合（跨面共享消费方 3） |
| 12 | MCP 与插件体系 | 4 个（共 49 条命令） | 49 | 2,187 | agent_macro dao harness mcp tools runtime plugins | store:agentStore、store:mcpStore、store:platformStore、store:pluginStore、ui:plugin、ui:settings | 3 | 6 | L1 功能包 · 深耦合（跨面共享消费方 3） |
| 13 | OPC 公司运行时（口径已废，盘上未收口） | 10 个（共 117 条命令） | 117 | 7,575 | agent agent_macro analysis_engine company_runtime entities harness trajectory dao +5 | lib、page:DemandDiscoveryPage | 2 | 79 | L1 功能包 · 浅耦合（2） |
| 14 | 任务调度与后台执行 | 4 个（共 22 条命令） | 22 | 1,474 | agent_macro astock_data dao entities harness kit trajectory | lib、store:schedulerStore、ui:chat | 2 | 4 | L1 功能包 · 浅耦合（2） |
| 15 | 通知与定时 | 3 个（共 21 条命令） | 21 | 1,027 | agent_macro entities runtime runtime_core | lib、ui:settings | 2 | 16 | L1 功能包 · 浅耦合（2） |
| 16 | 量化与仿真 | 3 个（共 18 条命令） | 18 | 2,596 | agent_macro analysis_engine astock_data entities harness runtime_core market_sim quant | lib、ui:stock-analysis | 2 | 9 | L1 功能包 · 浅耦合（2） |
| 17 | 仪表盘与动态 UI | 1 个（共 5 条命令） | 5 | 290 | agent_macro dao entities harness plugins | page:DashboardPage、ui:plugin、ui:settings | 2 | 2 | L1 功能包 · 浅耦合（2） |
| 18 | 网关与互联 | 2 个（共 44 条命令） | 44 | 1,738 | agent_macro astock_data crypto dao gateway harness mcp storage +1 | lib、store:gatewayLinkStore、store:gatewayStore、ui:gateway | 1 | 3 | L1 功能包 · 浅耦合（1） |
| 19 | 上下文与提示词工程 | 3 个（共 18 条命令） | 18 | 507 | agent_macro dao entities harness | store:contextSourceStore、store:promptTemplateStore | 0 | 0 | L1 功能包 · 无前端 invoke 交叉（最易独立） |

### 2.1 判据（机械可复算，见脚本 `verdictOf`）

- 「跨面共享消费方」= 同一个前端文件（组件目录 / 页面 / store）既 invoke 了本域的某个命令，又 invoke 了**别的域**的命令。
- 「被别的面消费的命令数」= 本域中由这些共享消费方调用过的命令条数。
- 判层：共享消费方 ≥8 ⇒ **L0 公共底座**（拆走会让多个面同时失去能力）；3–7 ⇒ L1 但深耦合；1–2 ⇒ L1 浅耦合；0 ⇒ 最易独立。
- 这个度量只看**前端消费面**的交叉，不看后端 crate 依赖；两者都硬耦合的域，看第 3 节 `后端 crate` 列是否与其它域重叠。

## 3. 命令模块逐条清单（164 个，按命令数降序）

| 模块 | 命令数 | 代码行 | 路径 | 后端 crate | 前端消费方 |
|---|---|---|---|---|---|
| `stock_analysis` | 172 | 8,596 | src-tauri/src/commands/stock_analysis.rs | agent agent_macro analysis_engine astock_data dao | lib、store:crossMarketStore、store:stockAnalysisStore、store:timeAnchorStore、ui:quant、ui:settings |
| `opc` | 57 | 1,589 | src-tauri/src/commands/opc.rs | agent agent_macro analysis_engine company_runtime entities | lib |
| `memory` | 54 | 2,350 | src-tauri/src/commands/memory.rs | agent agent_macro crypto dao entities | store:conversationStoreEvents、store:memoryStore、store:nudgeStore、store:sourceStore、ui:chat、ui:memory |
| `knowledge` | 39 | 4,033 | src-tauri/src/commands/knowledge.rs | agent_macro dao document_parser entities harness | page:QuickBarPage、store:knowledgeStore、store:sourceStore、ui:settings |
| `wiki` | 36 | 3,033 | src-tauri/src/commands/wiki.rs | agent_macro dao entities harness search | page:WikiGraphPage、store:sourceStore、store:wikiStore、ui:settings、ui:wiki |
| `skills` | 33 | 5 | src-tauri/src/commands/skills/mod.rs | — | lib、store:skillExtensionStore、store:skillStore、ui:settings、ui:skill |
| `llm_wiki` | 29 | 1,728 | src-tauri/src/commands/llm_wiki.rs | agent agent_macro crypto dao entities | page:QuickBarPage、store:llmWikiStore、ui:llm-wiki、ui:settings、ui:wiki |
| `device_sync` | 26 | 647 | src-tauri/src/commands/device_sync.rs | agent_macro device harness | store:deviceSyncStore |
| `gateway` | 26 | 1,252 | src-tauri/src/commands/gateway.rs | agent_macro astock_data crypto dao gateway | lib、store:gatewayStore、ui:gateway |
| `stock_workflow` | 24 | 13 | src-tauri/src/commands/stock_workflow/mod.rs | — | lib、store:stockAnalysisStore、ui:stock-analysis |
| `conversations` | 23 | 3 | src-tauri/src/commands/conversations/mod.rs | — | page:PersonaPage、page:QuickBarPage、store:compressStore、store:conversationStore、store:conversationStoreEvents、store:conversationStoreSend |
| `work_engine` | 21 | 1,085 | src-tauri/src/commands/work_engine.rs | agent_macro dao harness rt_workflow runtime | store:approvalStore、store:workEngineStore、store:workflowStore、ui:stock-analysis、ui:workflow |
| `agent_advanced` | 20 | 1,145 | src-tauri/src/commands/agent_advanced.rs | agent_macro | store:cacheStore |
| `opc_capability_pack_actions` | 20 | 3,486 | src-tauri/src/commands/opc_capability_pack_actions.rs | agent_macro analysis_engine dao entities harness | lib |
| `paper` | 20 | 428 | src-tauri/src/commands/paper.rs | agent_macro dao harness search | store:paperStore、store:readingListStore |
| `agent` | 19 | 6 | src-tauri/src/commands/agent/mod.rs | — | page:QuickBarPage、store:agentStore、store:conversationStore、store:conversationStoreEvents、store:streamStore、ui:chat |
| `evolution_engine` | 18 | 1,521 | src-tauri/src/commands/evolution_engine.rs | agent_macro dao harness runtime tools | store:evolutionStore |
| `gateway_link` | 18 | 486 | src-tauri/src/commands/gateway_link.rs | agent_macro crypto dao harness | store:gatewayLinkStore |
| `local_model` | 18 | 1,705 | src-tauri/src/commands/local_model.rs | agent_macro dao harness kit search | ui:settings |
| `providers` | 18 | 871 | src-tauri/src/commands/providers.rs | agent_macro crypto dao harness kit | store:proactiveStore、store:providerStore、ui:settings |
| `mcp` | 17 | 772 | src-tauri/src/commands/mcp.rs | agent_macro dao harness mcp tools | store:agentStore、store:mcpStore |
| `dynamic_ui` | 16 | 1,085 | src-tauri/src/commands/dynamic_ui.rs | agent_macro entities runtime | lib、store:dynamicUIStore |
| `fine_tune` | 16 | 575 | src-tauri/src/commands/fine_tune.rs | agent agent_macro crypto dao harness | store:fineTuneStore |
| `fleet` | 16 | 2 | src-tauri/src/commands/fleet/mod.rs | — | store:officeStore、ui:office |
| `tracer` | 16 | 707 | src-tauri/src/commands/tracer.rs | agent agent_macro telemetry trajectory | store:tracerStore、ui:chat、ui:layout |
| `workflow_template` | 16 | 3,544 | src-tauri/src/commands/workflow_template.rs | agent_macro dao entities harness kit | lib、page:WorkflowMarketplace、store:stockAnalysisStore、store:workflowEditorStore、store:workflowStore、ui:settings |
| `platform_integration` | 15 | 455 | src-tauri/src/commands/platform_integration.rs | agent_macro dao harness runtime | store:platformStore |
| `user_profile` | 15 | 602 | src-tauri/src/commands/user_profile.rs | agent_macro trajectory | store:styleStore、store:userProfileStore、ui:chat |
| `opc_capability_pack_runtime` | 14 | 747 | src-tauri/src/commands/opc_capability_pack_runtime.rs | agent_macro analysis_engine dao | lib |
| `scheduled_task` | 14 | 675 | src-tauri/src/commands/scheduled_task.rs | agent_macro entities runtime runtime_core | lib、ui:settings |
| `desktop` | 13 | 299 | src-tauri/src/commands/desktop.rs | agent_macro dao | lib、ui:layout、ui:settings |
| `agent_nudge` | 12 | 230 | src-tauri/src/commands/agent_nudge.rs | agent_macro trajectory | store:nudgeStore、store:proactiveStore |
| `parallel_execution` | 12 | 161 | src-tauri/src/commands/parallel_execution.rs | agent_macro trajectory | — |
| `plugin` | 12 | 595 | src-tauri/src/commands/plugin.rs | agent_macro harness plugins | store:pluginStore、ui:plugin、ui:settings |
| `prompt_templates` | 12 | 211 | src-tauri/src/commands/prompt_templates.rs | agent_macro dao harness | store:promptTemplateStore |
| `browser` | 11 | 355 | src-tauri/src/commands/browser.rs | agent_macro kit | ui:chat |
| `demand_discovery` | 11 | 937 | src-tauri/src/commands/demand_discovery.rs | agent_macro analysis_engine dao entities harness | page:DemandDiscoveryPage |
| `evaluator` | 11 | 564 | src-tauri/src/commands/evaluator.rs | agent agent_macro crypto dao harness | store:evaluatorStore |
| `proactive` | 11 | 720 | src-tauri/src/commands/proactive.rs | agent_macro dao trajectory | store:appConfigStore、store:proactiveStore、ui:chat |
| `backup` | 10 | 479 | src-tauri/src/commands/backup.rs | agent_macro dao harness storage | store:backupStore、ui:layout、ui:settings |
| `capability` | 10 | 1,171 | src-tauri/src/commands/capability.rs | agent agent_macro dao entities harness | store:capabilityStore、ui:settings |
| `cognitive` | 10 | 3,309 | src-tauri/src/commands/cognitive.rs | agent_macro dao entities harness orchestrator | store:conversationStoreSend、store:evolutionStore、store:taskShapeApprovalStore |
| `computer_control` | 10 | 119 | src-tauri/src/commands/computer_control.rs | agent_macro kit | ui:chat |
| `files_page` | 10 | 1,038 | src-tauri/src/commands/files_page.rs | agent_macro dao harness storage | lib、store:knowledgeStore、store:userProfileStore、ui:chat、ui:files、ui:shared |
| `knowledge_source` | 10 | 1,334 | src-tauri/src/commands/knowledge_source.rs | agent_macro dao entities harness kit | store:knowledgeSourceStore |
| `market_mainline` | 10 | 191 | src-tauri/src/commands/market_mainline.rs | agent_macro analysis_engine tools | ui:stock-analysis |
| `personality` | 10 | 623 | src-tauri/src/commands/personality.rs | agent agent_macro dao harness trajectory | lib |
| `workflows` | 10 | 1 | src-tauri/src/commands/workflows/mod.rs | — | lib、page:DemandDiscoveryPage、store:workflowEditorStore、store:workflowStore、ui:chat、ui:stock-analysis |
| `agent_analytics` | 9 | 201 | src-tauri/src/commands/agent_analytics.rs | agent_macro trajectory | store:executionStore、store:nudgeStore、store:trajectoryStore、ui:chat |
| `paper_portfolio` | 9 | 166 | src-tauri/src/commands/paper_portfolio.rs | agent_macro analysis_engine entities | store:paperPortfolioStore |
| `webdav` | 9 | 813 | src-tauri/src/commands/webdav.rs | agent_macro crypto dao harness kit | ui:layout、ui:settings |
| `backtest_validation` | 8 | 1,567 | src-tauri/src/commands/backtest_validation.rs | agent_macro analysis_engine astock_data entities harness | ui:stock-analysis |
| `cloud_workspace` | 8 | 422 | src-tauri/src/commands/cloud_workspace.rs | agent_macro storage | ui:settings |
| `file_browser` | 8 | 425 | src-tauri/src/commands/file_browser.rs | agent_macro kit storage | lib |
| `files` | 8 | 178 | src-tauri/src/commands/files.rs | agent_macro dao harness kit storage | ui:chat |
| `opc_demand_discovery` | 8 | 222 | src-tauri/src/commands/opc_demand_discovery.rs | agent_macro dao harness tools | page:DemandDiscoveryPage |
| `plan` | 8 | 1,899 | src-tauri/src/commands/plan.rs | agent agent_macro crypto dao entities | store:planStore |
| `pty` | 8 | 135 | src-tauri/src/commands/pty.rs | agent_macro runtime | store:terminalStore |
| `recommendation_cron` | 8 | 370 | src-tauri/src/commands/recommendation_cron.rs | agent_macro analysis_engine runtime_core | — |
| `reminder` | 8 | 276 | src-tauri/src/commands/reminder.rs | agent_macro trajectory | store:proactiveStore |
| `rl_training` | 8 | 545 | src-tauri/src/commands/rl_training.rs | agent agent_macro entities harness trajectory | store:rlTrainingStore |
| `screenshot_diagnosis` | 8 | 495 | src-tauri/src/commands/screenshot_diagnosis.rs | agent_macro analysis_engine entities harness | store:screenshotDiagnosisStore |
| `storage` | 8 | 361 | src-tauri/src/commands/storage.rs | agent_macro crypto dao storage | lib、ui:settings |
| `trade_intent` | 8 | 193 | src-tauri/src/commands/trade_intent.rs | agent_macro analysis_engine dao entities | — |
| `workflow_ai` | 8 | 5 | src-tauri/src/commands/workflow_ai/mod.rs | — | store:workflowEditorStore、store:workflowStore、ui:stock-analysis |
| `workflow_reflection` | 8 | 330 | src-tauri/src/commands/workflow_reflection.rs | agent_macro dao harness | lib |
| `agency_expert` | 7 | 937 | src-tauri/src/commands/agency_expert.rs | agent_macro crypto dao entities harness | lib、store:expertStore、ui:settings、ui:workflow |
| `agent_profile` | 7 | 291 | src-tauri/src/commands/agent_profile.rs | agent_macro dao entities harness kit | store:agentStore、ui:chat |
| `index_jobs` | 7 | 277 | src-tauri/src/commands/index_jobs.rs | agent_macro dao | — |
| `knowledge_graph` | 7 | 596 | src-tauri/src/commands/knowledge_graph.rs | agent_macro dao harness kit runtime | store:knowledgeGraphStore、ui:settings |
| `messages` | 7 | 171 | src-tauri/src/commands/messages.rs | agent_macro dao harness | store:compressStore、store:conversationStore、store:proactiveStore、store:stockWorkflowChatBridge、ui:chat |
| `search` | 7 | 372 | src-tauri/src/commands/search.rs | agent_macro crypto dao entities harness | store:proactiveStore、store:searchStore、ui:settings |
| `analyst_feedback` | 6 | 1,092 | src-tauri/src/commands/analyst_feedback.rs | agent_macro analysis_engine dao entities harness | — |
| `conversation_categories` | 6 | 118 | src-tauri/src/commands/conversation_categories.rs | agent_macro dao harness | store:categoryStore |
| `evolution` | 6 | 313 | src-tauri/src/commands/evolution.rs | agent_macro trajectory | store:evolutionStore、ui:chat、ui:layout |
| `marketplace` | 6 | 133 | src-tauri/src/commands/marketplace.rs | agent_macro dao harness | page:WorkflowMarketplace |
| `reflection` | 6 | 90 | src-tauri/src/commands/reflection.rs | agent agent_macro | ui:chat |
| `screen_vision` | 6 | 263 | src-tauri/src/commands/screen_vision.rs | agent agent_macro kit providers | ui:chat |
| `skill_decomposition` | 6 | 1,040 | src-tauri/src/commands/skill_decomposition.rs | agent_macro crypto dao entities harness | page:SkillsPage、store:decompositionStore、store:workflowEditorStore、ui:settings |
| `theme` | 6 | 77 | src-tauri/src/commands/theme.rs | agent_macro runtime | store:themeStore |
| `webhook` | 6 | 139 | src-tauri/src/commands/webhook.rs | agent_macro runtime | ui:settings |
| `workflow_tool` | 6 | 239 | src-tauri/src/commands/workflow_tool.rs | agent_macro dao entities harness tools | ui:workflow |
| `agent_role` | 5 | 292 | src-tauri/src/commands/agent_role.rs | agent_macro dao entities harness kit | store:agentRoleStore、ui:settings、ui:workflow |
| `background_tasks` | 5 | 967 | src-tauri/src/commands/background_tasks.rs | agent_macro astock_data dao entities harness | lib、ui:chat |
| `branches` | 5 | 163 | src-tauri/src/commands/branches.rs | agent_macro dao harness | store:conversationStore |
| `conditional_order` | 5 | 308 | src-tauri/src/commands/conditional_order.rs | agent_macro analysis_engine astock_data | ui:stock-analysis |
| `context_sources` | 5 | 96 | src-tauri/src/commands/context_sources.rs | agent_macro dao harness | store:contextSourceStore |
| `dashboard` | 5 | 290 | src-tauri/src/commands/dashboard.rs | agent_macro dao entities harness plugins | page:DashboardPage、ui:plugin、ui:settings |
| `db_config` | 5 | 297 | src-tauri/src/commands/db_config.rs | agent_macro crypto dao | ui:settings |
| `local_models` | 5 | 60 | src-tauri/src/commands/local_models.rs | agent_macro dao search | ui:settings |
| `market_sim` | 5 | 357 | src-tauri/src/commands/market_sim.rs | agent_macro market_sim quant | ui:stock-analysis |
| `migration` | 5 | 83 | src-tauri/src/commands/migration.rs | agent_macro migration | — |
| `mover_recall` | 5 | 1,837 | src-tauri/src/commands/mover_recall.rs | agent_macro analysis_engine astock_data dao harness | ui:stock-analysis |
| `narrative` | 5 | 202 | src-tauri/src/commands/narrative.rs | agent_macro dao entities | lib |
| `opc_delivery` | 5 | 164 | src-tauri/src/commands/opc_delivery.rs | agent_macro dao harness | lib、page:DemandDiscoveryPage |
| `opc_demand_subscription` | 5 | 357 | src-tauri/src/commands/opc_demand_subscription.rs | dao harness runtime_core | page:DemandDiscoveryPage |
| `plugin_profile` | 5 | 365 | src-tauri/src/commands/plugin_profile.rs | agent_macro harness | — |
| `profile` | 5 | 83 | src-tauri/src/commands/profile.rs | agent_macro runtime | ui:settings |
| `quant_backtest` | 5 | 672 | src-tauri/src/commands/quant_backtest.rs | agent_macro astock_data entities harness quant | lib |
| `retrieval_feedback` | 5 | 177 | src-tauri/src/commands/retrieval_feedback.rs | agent_macro dao | ui:chat |
| `smart_router` | 5 | 92 | src-tauri/src/commands/smart_router.rs | agent_macro | — |
| `terminal` | 5 | 284 | src-tauri/src/commands/terminal.rs | agent_macro kit | ui:terminal |
| `tool_recommender` | 5 | 160 | src-tauri/src/commands/tool_recommender.rs | agent agent_macro | store:recommendationStore |
| `workflow_ai_apply` | 5 | 938 | src-tauri/src/commands/workflow_ai_apply.rs | agent_macro dao harness | store:workflowEditorStore、ui:stock-analysis |
| `analyst_evolution` | 4 | 178 | src-tauri/src/commands/analyst_evolution.rs | agent_macro analysis_engine | ui:stock-analysis |
| `artifacts` | 4 | 85 | src-tauri/src/commands/artifacts.rs | agent_macro dao harness | store:artifactStore、ui:chat |
| `local_tool` | 4 | 259 | src-tauri/src/commands/local_tool.rs | agent_macro dao harness tools | store:localToolStore、store:proactiveStore、ui:chat、ui:settings |
| `market_close_sweep` | 4 | 243 | src-tauri/src/commands/market_close_sweep.rs | agent_macro analysis_engine astock_data | ui:stock-analysis |
| `pool_scan` | 4 | 905 | src-tauri/src/commands/pool_scan.rs | agent_macro analysis_engine astock_data entities harness | — |
| `scheduler` | 4 | 5 | src-tauri/src/scheduler/mod.rs | — | store:schedulerStore |
| `sources` | 4 | 345 | src-tauri/src/commands/sources.rs | agent_macro dao harness search tools | store:sourceStore、ui:settings |
| `stock_evolution_approval` | 4 | 340 | src-tauri/src/commands/stock_evolution_approval.rs | agent_macro analysis_engine dao | store:stockAnalysisStore |
| `sub_agent` | 4 | 1 | src-tauri/src/commands/sub_agent/mod.rs | — | ui:office、ui:settings |
| `unified_knowledge` | 4 | 281 | src-tauri/src/commands/unified_knowledge.rs | agent_macro dao harness search | — |
| `workflow_execution_stats` | 4 | 115 | src-tauri/src/commands/workflow_execution_stats.rs | agent_macro dao harness | — |
| `agent_insight` | 3 | 51 | src-tauri/src/commands/agent_insight.rs | agent_macro trajectory | store:nudgeStore |
| `agent_session` | 3 | 85 | src-tauri/src/commands/agent_session.rs | agent_macro harness | lib |
| `app_config` | 3 | 358 | src-tauri/src/commands/app_config.rs | agent agent_macro dao harness | store:appConfigStore |
| `conversations_search` | 3 | 852 | src-tauri/src/commands/conversations_search.rs | agent_macro harness | store:searchStore |
| `decision_gates` | 3 | 131 | src-tauri/src/commands/decision_gates.rs | agent_macro analysis_engine | ui:settings |
| `dojo_sdk` | 3 | 581 | src-tauri/src/commands/dojo_sdk.rs | agent harness | — |
| `dream` | 3 | 74 | src-tauri/src/commands/dream.rs | agent_macro trajectory | — |
| `error_classification` | 3 | 90 | src-tauri/src/commands/error_classification.rs | agent agent_macro harness | — |
| `master_key` | 3 | 311 | src-tauri/src/commands/master_key.rs | agent_macro crypto dao | ui:settings |
| `mover_recall_analysis` | 3 | 1,949 | src-tauri/src/commands/mover_recall_analysis.rs | agent_macro analysis_engine astock_data dao harness | lib、ui:stock-analysis |
| `onboarding` | 3 | 321 | src-tauri/src/commands/onboarding.rs | agent_macro crypto dao harness | store:onboardingStore |
| `opc_demand_implement` | 3 | 299 | src-tauri/src/commands/opc_demand_implement.rs | dao harness | page:DemandDiscoveryPage |
| `opc_workflows` | 3 | 36 | src-tauri/src/commands/opc_workflows/mod.rs | — | — |
| `session_share` | 3 | 137 | src-tauri/src/commands/session_share.rs | agent_macro | — |
| `sessions_export` | 3 | 418 | src-tauri/src/commands/sessions_export.rs | agent_macro dao harness | lib |
| `skills_hub` | 3 | 267 | src-tauri/src/commands/skills_hub.rs | agent_macro dao entities trajectory | page:SkillsPage |
| `stock_pipeline` | 3 | 3 | src-tauri/src/commands/stock_pipeline/mod.rs | — | lib、store:pipelineStore |
| `time_travel_batch` | 3 | 1,557 | src-tauri/src/commands/time_travel_batch.rs | agent_macro analysis_engine astock_data dao entities | lib、ui:time-travel |
| `approval_rules` | 2 | 40 | src-tauri/src/commands/approval_rules.rs | harness tools | ui:settings |
| `generated_tool` | 2 | 62 | src-tauri/src/commands/generated_tool.rs | agent_macro dao | store:localToolStore |
| `hooks_config` | 2 | 66 | src-tauri/src/commands/hooks_config.rs | agent_macro dao | ui:settings |
| `image_gen_settings` | 2 | 166 | src-tauri/src/commands/image_gen_settings.rs | agent_macro credential | ui:chat、ui:settings |
| `message_continuation` | 2 | 105 | src-tauri/src/commands/message_continuation.rs | agent_macro entities | store:streamStore |
| `multi_agent` | 2 | 249 | src-tauri/src/commands/multi_agent.rs | agent agent_macro dao harness kit | store:multiAgentStore |
| `orchestrator` | 2 | 280 | src-tauri/src/commands/orchestrator.rs | agent_macro harness orchestrator runtime | — |
| `quickbar` | 2 | 90 | src-tauri/src/commands/quickbar.rs | agent_macro | lib、page:QuickBarPage |
| `research` | 2 | 473 | src-tauri/src/commands/research.rs | agent agent_macro crypto dao harness | — |
| `settings` | 2 | 133 | src-tauri/src/commands/settings.rs | agent_macro dao harness storage telemetry | lib、store:settingsStore |
| `workflow_ai_diagnose` | 2 | 512 | src-tauri/src/commands/workflow_ai_diagnose.rs | agent_macro crypto dao entities harness | store:workflowEditorStore、ui:workflow |
| `workflow_yaml` | 2 | 67 | src-tauri/src/commands/workflow_yaml.rs | agent_macro rt_workflow | ui:workflow |
| `chart_generator` | 1 | 144 | src-tauri/src/commands/chart_generator.rs | agent_macro harness providers | lib |
| `context_breakdown` | 1 | 200 | src-tauri/src/commands/context_breakdown.rs | agent_macro dao entities | — |
| `crash_report` | 1 | 10 | src-tauri/src/commands/crash_report.rs | agent_macro | — |
| `dependency_metrics` | 1 | 37 | src-tauri/src/commands/dependency_metrics.rs | agent_macro harness | — |
| `document_export` | 1 | 90 | src-tauri/src/commands/document_export.rs | tools | lib、ui:chat |
| `health` | 1 | 168 | src-tauri/src/commands/health.rs | agent_macro | ui:settings |
| `image_gen` | 1 | 99 | src-tauri/src/commands/image_gen.rs | agent_macro credential providers | ui:chat |
| `knowledge_integration` | 1 | 193 | src-tauri/src/knowledge_integration.rs | dao harness search | — |
| `learning_graph` | 1 | 114 | src-tauri/src/commands/learning_graph.rs | agent_macro entities trajectory | page:LearningGraphPage |
| `nl_to_cron` | 1 | 213 | src-tauri/src/commands/nl_to_cron.rs | agent_macro | — |
| `opc_demand_capability` | 1 | 284 | src-tauri/src/commands/opc_demand_capability.rs | agent_macro dao harness tools | page:DemandDiscoveryPage |
| `opc_learning_hook` | 1 | 391 | src-tauri/src/commands/opc_learning_hook.rs | agent_macro analysis_engine orchestrator | — |
| `provider_balance` | 1 | 100 | src-tauri/src/commands/provider_balance.rs | agent_macro crypto dao entities runtime_core | — |
| `sandbox` | 1 | 18 | src-tauri/src/commands/sandbox.rs | agent_macro kit | lib、store:artifactStore |
| `scheduler::restore` | 1 | 341 | src-tauri/src/scheduler/restore.rs | dao entities harness | store:schedulerStore |
| `skill_workflow` | 1 | 172 | src-tauri/src/commands/skill_workflow.rs | agent_macro dao entities harness | store:workflowEditorStore |
| `tray` | 1 | 142 | src-tauri/src/tray.rs | — | — |
| `universe_scan` | 1 | 89 | src-tauri/src/commands/universe_scan.rs | agent_macro analysis_engine astock_data dao | ui:stock-analysis |
| `wf_des` | 1 | 368 | src-tauri/src/commands/wf_des.rs | agent_macro market_sim quant | ui:quant |

## 4. trait 分类全集（逐条归属，可复核）

### 4.1 L2-候选(有外部实现者无键控点) — 198 条

`AgencyExpertRepository`（harness/src/repositories.rs:443） · `AgentProfileRepository`（harness/src/repositories.rs:425） · `AgentRoleRepository`（harness/src/repositories.rs:461） · `AgentSessionBroker`（harness/src/agent_session_broker.rs:52） · `AgentSessionRepository`（harness/src/agent_session_repo.rs:16） · `AgentStreamReporter`（harness/src/streaming.rs:60） · `AnalyticsService`（crates/analysis-engine/src/opc/analytics.rs:335） · `ApiClient`（harness/src/runtime_types/conversation.rs:77） · `ApprovalRuleStore`（harness/src/approval_rules.rs:87） · `AskUserBridge`（harness/src/tool.rs:124） · `AsyncEmbedFn`（crates/search/src/rag.rs:1957） · `BenchEvaluator`（crates/runtime/src/benchmarks/mod.rs:99） · `BrowserHttpFetch`（crates/astock-data/src/vendors/browser_eastmoney.rs:40） · `CacheLayer`（crates/rt-workflow/src/work_engine/cache_layer.rs:43） · `CapabilityFilter`（harness/src/capability_filter.rs:210） · `CapabilityIndexer`（harness/src/capability_indexer.rs:29） · `CapabilityPassport`（harness/src/capability.rs:815） · `CapabilityRanker`（harness/src/capability_ranker.rs:63） · `CapabilityRetriever`（harness/src/capability_retriever.rs:131） · `ConsolidationDataProvider`（harness/src/dream.rs:197） · `ContentAssetService`（crates/analysis-engine/src/opc/content_asset.rs:61） · `ContentBlockExt`（crates/runtime-core/src/session.rs:434） · `ConversationMessageExt`（crates/runtime-core/src/session.rs:318） · `ConversationRepository`（harness/src/repositories.rs:130） · `CronDeliverySink`（harness/src/cron_delivery.rs:130） · `CronJobPersistence`（harness/src/cron_persistence.rs:61） · `CronJobStore`（harness/src/tool_service.rs:37） · `CustomerService`（crates/analysis-engine/src/opc/customer.rs:174） · `DailySnapshotStore`（crates/astock-data/src/daily_snapshot.rs:210） · `DatabaseQueryService`（harness/src/database_query_service.rs:16） · `DecomposerLlmClient`（crates/agent/src/task_decomposer.rs:29） · `DelegateTaskRunner`（harness/src/tools_ext.rs:82） · `DemandLlmBridge`（crates/tools/src/tools/demand_llm.rs:27） · `DeviceManager`（harness/src/device_sync.rs:339） · `DocumentParser`（harness/src/search_sources.rs:62） · `DreamConsolidator`（harness/src/dream.rs:229） · `EmbeddingProvider`（harness/src/rag_provider.rs:7） · `EntityGraphProvider`（harness/src/knowledge_graph.rs:49） · `EventBus`（harness/src/event_bus.rs:143） · `EventBusSubscription`（harness/src/event_bus.rs:128） · `EventSubscriber`（harness/src/typed_event.rs:106） · `EvolutionArtifactValidator`（harness/src/workflow_evolution.rs:302） · `ExecutionFeedbackSink`（harness/src/workflow_evolution.rs:756） · `FeatureFlagProvider`（harness/src/feature_flag_provider.rs:8） · `FeedbackDataLake`（harness/src/feedback_data_lake.rs:217） · `FinanceService`（crates/analysis-engine/src/opc/finance.rs:41） · `FinancialSnapshotLike`（crates/astock-data/src/valuation_band.rs:305） · `FleetIntentLlm`（harness/src/fleet.rs:426） · `FleetRepository`（harness/src/fleet.rs:154） · `GatewayKeyRepository`（harness/src/platform_adapter.rs:59） · `GatewayRequestLogRepository`（harness/src/platform_adapter.rs:102） · `GeneratedToolRepository`（harness/src/repositories.rs:193） · `GuardianBridge`（harness/src/tool_access.rs:22） · `GuardianReviewer`（crates/agent/src/guardian.rs:194） · `HarnessCache`（harness/src/cache_interceptor.rs:21） · `HarnessInterceptor`（harness/src/interceptor.rs:111） · `HarnessToolExecutor`（harness/src/contracts.rs:16） · `HasProviderRegistry`（harness/src/has_provider_registry.rs:25） · `HealthCheck`（crates/runtime/src/health_check.rs:70） · `HookProgressReporter`（harness/src/runtime_types/hooks.rs:153） · `ImageGenProvider`（crates/providers/src/image_gen.rs:64） · `Introspector`（crates/dao/src/reconcile/introspect/mod.rs:70） · `InvoiceService`（crates/analysis-engine/src/opc/invoice.rs:137） · `IrRenderer`（harness/src/ir_renderer.rs:18） · `KitHtmlCleaner`（harness/src/kit_bridge.rs:44） · `KitMarkdownParser`（harness/src/kit_bridge.rs:38） · `KitSlashCommandProcessor`（harness/src/kit_bridge.rs:121） · `KitTokenBudgetTracker`（harness/src/kit_bridge.rs:89） · `KnowledgeDocumentRepository`（harness/src/repositories.rs:570） · `KnowledgeEntityRepository`（harness/src/repositories.rs:516） · `KnowledgeFlowRepository`（harness/src/repositories.rs:534） · `KnowledgeInterfaceRepository`（harness/src/repositories.rs:552） · `KnowledgeSource`（harness/src/search_sources.rs:25） · `LlmContentGenerator`（crates/agent/src/research_agent.rs:51） · `LlmEvaluator`（crates/analysis-engine/src/opc/agent.rs:54） · `LlmEvolutionProvider`（harness/src/trajectory_types.rs:513） · `LlmExecutionService`（harness/src/llm_execution.rs:42） · `LlmInferencePort`（crates/orchestrator/src/capability_pack_learning.rs:40） · `LlmJudge`（harness/src/trajectory_types.rs:524） · `LlmReasoningProvider`（crates/agent/src/react_engine.rs:88） · `LlmSampler`（crates/runtime/src/mcp_server.rs:223） · `LlmTextGradProvider`（harness/src/trajectory_types.rs:534） · `LlmToolProvider`（harness/src/trajectory_types.rs:472） · `LoopCheckpointRepository`（harness/src/repositories.rs:375） · `MarketBatchQuery`（crates/astock-data/src/batch.rs:134） · `MarketDataStreamer`（harness/src/market_data.rs:246） · `MarketplaceScanner`（crates/tools/src/tools/marketplace_scanner.rs:617） · `McpClientService`（harness/src/mcp_service.rs:54） · `McpServerStore`（harness/src/mcp_service.rs:48） · `MemoryProvider`（crates/trajectory/src/memory_provider.rs:63） · `MemoryRepository`（harness/src/repositories.rs:281） · `MemorySource`（harness/src/search_sources.rs:38） · `MemoryStore`（harness/src/memory.rs:370） · `MessageRepository`（harness/src/repositories.rs:157） · `MigrationRunner`（harness/src/tools_ext.rs:17） · `MissionDecomposer`（crates/orchestrator/src/decomposer.rs:25） · `ModelTierResolver`（harness/src/route_bridge.rs:70） · `ModuleSwitch`（crates/runtime/src/module_switch.rs:56） · `MonitorEventEmitter`（crates/analysis-engine/src/monitor.rs:23） · `NewsArchiveSink`（crates/astock-data/src/lib.rs:91） · `NoteBacklinkRepository`（harness/src/wiki_dtos.rs:234） · `NoteRepository`（harness/src/wiki_dtos.rs:198） · `NotificationChannel`（harness/src/notification_channel.rs:195） · `OpCapabilityPackVendor`（crates/analysis-engine/src/opc/vendors.rs:19） · `OpcAutomationService`（crates/analysis-engine/src/opc/automation.rs:298） · `PermissionChecker`（harness/src/runtime_types/permission_enforcer.rs:15） · `PermissionPolicyExt`（crates/runtime-core/src/permissions.rs:11） · `PermissionPrompter`（harness/src/runtime_types/permissions.rs:92） · `Persistence`（harness/src/persistence_mod.rs:19） · `PlatformAdapter`（harness/src/platform_adapter.rs:152） · `PlatformConfigExt`（crates/rt-messaging/src/message_gateway/platform_config.rs:10） · `PlatformConfigRepository`（harness/src/repositories.rs:105） · `Plugin`（crates/plugins/src/core.rs:44） · `PluginAgentProvider`（harness/src/tools_ext.rs:42） · `PluginHook`（harness/src/plugin_hook.rs:159） · `PluginLifecycle`（crates/runtime/src/plugin_lifecycle.rs:183） · `PrmLlmProvider`（harness/src/trajectory_types.rs:560） · `ProjectService`（crates/analysis-engine/src/opc/project.rs:132） · `ProviderRegistry`（harness/src/registry.rs:19） · `ProviderRepository`（harness/src/platform_adapter.rs:24） · `PublishScheduleService`（crates/analysis-engine/src/opc/publish_schedule.rs:56） · `RecommendStrategy`（crates/analysis-engine/src/recommender/strategy.rs:58） · `RelevanceCalculator`（crates/agent/src/fact_checker.rs:95） · `RemoteStorage`（harness/src/device_sync.rs:840） · `ReplayContextExt`（crates/trajectory/src/trajectory_impl.rs:14） · `ReplayExecutor`（harness/src/replay_executor.rs:222） · `RerankBackend`（crates/search/src/reranker.rs:56） · `ResearchStateResultProcessor`（crates/agent/src/search_provider.rs:215） · `ResponseNormalizer`（harness/src/response_normalizer.rs:17） · `RewardCategoryExt`（crates/trajectory/src/trajectory_impl.rs:76） · `RlExperienceStore`（harness/src/rl.rs:304） · `RuntimeMutationAccess`（harness/src/runtime_types/runtime_mutation.rs:49） · `SandboxExecutor`（crates/trajectory/src/skill_evolution.rs:485） · `SandboxToolTester`（crates/trajectory/src/auto_tool.rs:148） · `SearchProvider`（crates/agent/src/search_provider.rs:95） · `SecureStore`（crates/kit/src/secure_store.rs:9） · `SelfImprovingRound`（harness/src/self_improving_loop.rs:86） · `SemanticValidator`（crates/agent/src/self_verifier.rs:88） · `SessionEventSink`（harness/src/session_events.rs:189） · `SessionExt`（crates/runtime-core/src/session.rs:45） · `Sessionlike`（crates/runtime/src/connection_pool.rs:267） · `SettingsRepository`（harness/src/platform_adapter.rs:52） · `SettingsSource`（harness/src/search_sources.rs:54） · `SharedBlackboard`（harness/src/multi_agent.rs:93） · `SignalService`（harness/src/device_sync.rs:647） · `SimAgent`（crates/market-sim/src/agent/traits.rs:232） · `SiteService`（crates/analysis-engine/src/opc/site.rs:168） · `SkillEvolutionHook`（harness/src/skill_evolution_hook.rs:18） · `StockStore`（harness/src/stock_service.rs:19） · `StorageBackend`（harness/src/storage_backend.rs:46） · `StoredFileRepository`（harness/src/repositories.rs:498） · `Strategy`（harness/src/strategy_contract.rs:440） · `SubTaskDispatcher`（harness/src/orchestration_dispatch.rs:87） · `SubTaskHandler`（crates/runtime/src/subtask_dispatcher.rs:42） · `SyncEngine`（harness/src/device_sync.rs:372） · `SyncStorage`（harness/src/device_sync.rs:676） · `TaskExecutorImpl`（crates/agent/src/task_executor.rs:92） · `TaskShapeClassifier`（harness/src/task_shape.rs:234） · `TaskShapeLlmClassifier`（harness/src/task_shape.rs:275） · `TelemetrySink`（crates/telemetry/src/lib.rs:234） · `TerminalBackend`（crates/runtime/src/terminal/backend_trait.rs:54） · `TerminalState`（crates/rt-workflow/src/work_engine/engine/node_state.rs:44） · `ThinkScrubber`（crates/runtime-core/src/guard_traits.rs:65） · `TokenCost`（crates/runtime-core/src/usage.rs:27） · `TokenUsageSink`（crates/runtime-core/src/guard_traits.rs:85） · `ToolCallGuardrail`（crates/runtime-core/src/guard_traits.rs:42） · `ToolExecutionAudit`（harness/src/tool_audit.rs:12） · `ToolExecutionRepository`（harness/src/repositories.rs:252） · `ToolExecutor`（harness/src/runtime_types/conversation.rs:88） · `ToolExecutorAccess`（crates/tools/src/tools/rpc.rs:21） · `ToolRanker`（harness/src/tool.rs:578） · `ToolRegistry`（harness/src/registry.rs:29） · `TraceExporter`（crates/telemetry/src/exporter.rs:23） · `TraceStorage`（crates/telemetry/src/storage.rs:50） · `TrajectoryBuilderExt`（crates/trajectory/src/trajectory_impl.rs:44） · `TrajectoryRepository`（harness/src/repositories.rs:590） · `TrajectoryRewardEngine`（harness/src/rl.rs:238） · `TransportHandler`（crates/rt-messaging/src/message_gateway.rs:168） · `TransportProvider`（crates/providers/src/transport/mod.rs:86） · `UnifiedKnowledgeSource`（harness/src/search_sources.rs:163） · `VectorSearch`（crates/agent/src/query_engine.rs:46） · `WeChatWebhookHandler`（harness/src/messaging_webhook.rs:14） · `WebhookEventSink`（harness/src/webhook_subscription.rs:170） · `WebhookPersistence`（harness/src/webhook_subscription.rs:128） · `WebhookSubscriptionService`（harness/src/webhook_subscription.rs:94） · `WhatsAppWebhookHandler`（harness/src/messaging_webhook.rs:37） · `WikiOperationRepository`（harness/src/wiki_dtos.rs:190） · `WikiPageRepository`（harness/src/wiki_dtos.rs:168） · `WikiRepository`（harness/src/wiki_dtos.rs:124） · `WikiSource`（harness/src/search_sources.rs:44） · `WikiSourceRepository`（harness/src/wiki_dtos.rs:148） · `WorkflowDagExecutor`（harness/src/workflow_evolution.rs:279） · `WorkflowExecutionRepository`（harness/src/repositories.rs:331） · `WorkflowGenomeLoader`（harness/src/workflow_evolution.rs:230） · `WorkflowLifecycleHook`（harness/src/workflow_lifecycle.rs:33） · `WorkflowLlmMutator`（harness/src/workflow_evolution.rs:246） · `WorkflowTemplateRepo`（harness/src/workflow_template_repo.rs:19） · `WorkflowTemplateRepository`（harness/src/repositories.rs:402）

### 4.2 L3-策略位(仅内置实现) — 52 条

`AssemblyBuilder`（harness/src/assembly_builder.rs:66） · `BenchmarkRunner`（harness/src/benchmark.rs:61） · `BrowserController`（harness/src/browser.rs:31） · `CapabilityRouter`（harness/src/capability_router.rs:134） · `ChunkProvider`（harness/src/indexer.rs:39） · `CircuitBreaker`（harness/src/circuit_breaker.rs:44） · `CodeVerifierPort`（harness/src/code_verifier.rs:115） · `ContentFilter`（harness/src/content_filter.rs:37） · `ConversationRuntimeHost`（harness/src/runtime_types/conversation.rs:132） · `DevExperienceProvider`（harness/src/dev_experience.rs:40） · `DocumentIndexer`（harness/src/indexer.rs:49） · `DojoExtension`（harness/src/dojo_extension.rs:215） · `EntityExtractor`（harness/src/knowledge_graph.rs:117） · `FsmPersistence`（harness/src/business_state_machine.rs:760） · `GatewayService`（harness/src/gateway_service.rs:21） · `GraphContextFormatter`（harness/src/knowledge_graph.rs:221） · `HookEventFirer`（harness/src/tool_service.rs:70） · `InputSanitizer`（harness/src/tool.rs:585） · `MarketplaceService`（harness/src/marketplace.rs:53） · `MemoryLifecycleHook`（harness/src/memory.rs:289） · `MemoryScanner`（harness/src/scanner.rs:42） · `ModelCascadeExecutor`（harness/src/model_cascade.rs:474） · `ModelKnowledgeProvider`（harness/src/model_knowledge.rs:12） · `NpmRegistryService`（harness/src/npm_registry.rs:16） · `ObservabilityProvider`（harness/src/observability.rs:17） · `OutputSanitizer`（harness/src/tool.rs:542） · `PlannerAdapter`（harness/src/planner.rs:18） · `PlatformManager`（harness/src/platform_manager.rs:14） · `PromptGuard`（harness/src/prompt_guard.rs:21） · `RAGProvider`（harness/src/rag_provider.rs:86） · `RLEngine`（harness/src/rl.rs:222） · `RLTrainer`（harness/src/rl.rs:261） · `RarRecaller`（harness/src/rar_recaller.rs:95） · `RateLimiter`（harness/src/rate_limiter.rs:31） · `RelevanceSignalExt`（harness/src/graph_dtos.rs:248） · `RerankProvider`（harness/src/rag_provider.rs:40） · `ReversibleEffect`（harness/src/reversible_effect.rs:19） · `RhaiEngineAdapter`（harness/src/rhai_engine.rs:20） · `RouteEngine`（harness/src/route_engine.rs:194） · `SelfRagProvider`（harness/src/rag_provider.rs:56） · `ServiceRegistryProvider`（harness/src/service_registry.rs:540） · `SessionTracer`（harness/src/session_tracer.rs:16） · `SsrFGuard`（harness/src/ssrf_guard.rs:32） · `StyleApplier`（harness/src/style.rs:105） · `StyleExtractor`（harness/src/style.rs:94） · `StyleVectorizer`（harness/src/style.rs:111） · `ToolAccessControl`（harness/src/tool_access.rs:41） · `ToolMetricsCollector`（harness/src/tool_metrics.rs:24） · `TrajectoryService`（harness/src/trajectory_service.rs:27） · `UserProfileService`（harness/src/profile.rs:547） · `VectorStoreProvider`（harness/src/rag_provider.rs:21） · `WorkflowHookSink`（harness/src/workflow_hook_sink.rs:27）

### 4.3 空接口(全仓无实现) — 29 条

`AuditRecorder`（harness/src/audit_trail.rs:29） · `AxAgentPlugin`（crates/tools/src/plugin_sdk.rs:87） · `BatchVendor`（crates/astock-data/src/batch.rs:120） · `CacheService`（harness/src/cache_service.rs:12） · `CapabilityCircuitBreaker`（harness/src/capability_circuit.rs:19） · `CapabilityCompleter`（harness/src/capability_circuit.rs:66） · `CapabilityHotSwapper`（harness/src/capability_circuit.rs:118） · `ChannelAdapter`（harness/src/channel_adapter.rs:25） · `CompletionProvider`（crates/runtime/src/shell_completer.rs:120） · `ContextEngine`（harness/src/context_engine.rs:24） · `CustomRuleChecker`（crates/trajectory/src/constitution.rs:102） · `DatabaseInitializer`（harness/src/repositories.rs:302） · `DojoSdkExecutor`（crates/astock-data/src/mcp_tools.rs:23） · `DualRegistry`（harness/src/dual_registry.rs:129） · `HarnessApiClient`（harness/src/contracts.rs:40） · `HasCapabilityRegistry`（harness/src/capability_registry.rs:1030） · `HookService`（harness/src/hook_service.rs:18） · `InferenceEngine`（harness/src/inference_engine.rs:46） · `LayeredPromptEngine`（harness/src/layered_prompt_engine.rs:174） · `MarketplaceCatalogService`（harness/src/marketplace.rs:162） · `MultiAgentCoordination`（harness/src/multi_agent.rs:155） · `PlatformMessageHandler`（harness/src/platform_manager.rs:24） · `RouterSelfUpdateManager`（harness/src/dual_registry.rs:192） · `SelfReferenceCircuitBreaker`（harness/src/capability_circuit.rs:174） · `SessionRepository`（harness/src/repositories.rs:235） · `SkillDirsProvider`（harness/src/repositories.rs:316） · `StockDataService`（crates/analysis-engine/src/stock_data_service.rs:20） · `SystemConfigStore`（harness/src/dual_registry.rs:100） · `SystemPrivilegedPipeline`（harness/src/dual_registry.rs:300）

### 4.4 L2-待建接缝 — 19 条

`Agent`（harness/src/agent.rs:61） · `AnyNodeState`（crates/rt-workflow/src/work_engine/engine/node_state.rs:281） · `BackgroundTaskRepository`（harness/src/repositories.rs:478） · `CapabilityPackAdapter`（harness/src/capability_pack_orchestration/mod.rs:38） · `CommandHandler`（src-tauri/src/commands/agent/command_bridge.rs:1125） · `CredentialService`（harness/src/credential_service.rs:23） · `CryptoService`（harness/src/platform_adapter.rs:122） · `KitSkillDirs`（harness/src/kit_bridge.rs:97） · `MarketDataProvider`（harness/src/market_data.rs:192） · `NodeExecutorTrait`（crates/rt-workflow/src/work_engine/node_executor_trait.rs:224） · `OpcDataService`（crates/analysis-engine/src/opc/data_service.rs:115） · `Oracle`（crates/market-sim/src/oracle.rs:59） · `PathEncoder`（harness/src/path_vars.rs:11） · `PromptProvider`（harness/src/prompt_provider.rs:29） · `RAGSource`（crates/search/src/rag.rs:45） · `SeamInvoker`（harness/src/seam_invoker.rs:29） · `StockVendor`（crates/astock-data/src/vendors/mod.rs:7） · `Tool`（harness/src/tool.rs:424） · `VaultSource`（harness/src/vault.rs:113）

### 4.5 L2-已建接缝 — 14 条

`AgentTurnRunner`（harness/src/agent_turn_runner.rs:125） · `BusinessRuleEvaluator`（harness/src/business_rules.rs:71） · `ContextContributor`（harness/src/context_contributor.rs:22） · `MessagePlatformAdapter`（harness/src/message_adapter.rs:83） · `PlatformMessageCallback`（harness/src/platform_callback.rs:21） · `ProviderAdapter`（harness/src/provider.rs:25） · `SessionLogInvariant`（harness/src/session_log_invariant.rs:141） · `SessionStateStore`（harness/src/session_state.rs:123） · `ToolSetProvider`（harness/src/registry.rs:130） · `WebhookDispatch`（harness/src/webhook_subscription.rs:154） · `WorkflowEvolver`（harness/src/workflow_evolution.rs:172） · `WorkflowOptimizer`（harness/src/workflow_optimization.rs:80） · `WorkflowReflector`（harness/src/workflow_reflection.rs:190） · `WorkflowSandbox`（harness/src/workflow_evolution.rs:263）

### 4.6 L3-策略位(壳层全局持有) — 4 条

`ClusterRouter`（harness/src/cluster_router.rs:225） · `CognitiveRouter`（harness/src/cognitive_router.rs:489） · `DomainRouter`（harness/src/domain_router.rs:536） · `RarRouter`（harness/src/rar_router.rs:327）

## 5. 判读口径与已知偏差

0. **第 2 节的「跨面共享消费方 / 被别的面消费的命令数」只度量拆分的顺序成本，不度量可行性。**
   `WorkEngine` 是底座（`rt-workflow/Cargo.toml` 的 axagent-* 依赖仅 crypto dao harness kit providers telemetry，**不含任何业务域 crate**），
   域以客户端方式调它（`cognitive.rs:1249` 调 `run_workflow`、`stock_workflow/core.rs:2035` 调 `cancel_workflow`）⇒ **「基于工作流」不构成拆不动的理由**。
   股票分析拆不干净的真实原因见 `PLAN-shell-plus-plugins.md` §0.8.5：引擎 `agent_executor.rs:4385` 的 `PREFETCHABLE_TOOLS` 硬编码十余个 `get_stock_*` 工具名、
   `:1232-1246` 从上下文取 `stock_code` 预取、`:854` prompt 文本写死 `search_stock`；另有一条空壳执行器
   `document_parser_executor.rs`（59 行，只回显 `"status":"parsed"`，不接 `DocumentParser` 接缝）。
1. 命令模块的「代码行」按该模块目录内全部 `.rs` 求和，含注释与测试外的内联 `#[cfg(test)]`。
2. 域簇用命令模块名的正则归并（脚本内 `DOMAIN_RULES`，逐条可见）。**归并是命名级的，命名不规范者会落「其它」**，
   那张表的行需要人工认领，但**不影响命令数总量闭合**（1541 条已分配）。
3. trait 分类里「L2 候选（无键控点）」是**中间态**：它有外部实现者，但我没找到按字符串键选实现的站点 ⇒ 只能算分层解耦，不能算可插拔。
4. 「空接口」= 全仓零实现 ⇒ 属债不属能力，不得计入插件数。
5. `domain.pack`／OPC 已按产品口径从能力清单剔除，但盘上仍在（见 PLAN §0.8.4 / 任务 #32），故本清点仍如实列出其体量。
