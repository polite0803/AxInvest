// SPDX-License-Identifier: AGPL-3.0-only

//! 内置工具实现

use std::collections::HashSet;
use std::sync::Arc;

pub mod agent;
pub mod agent_memory;
pub mod apply_patch;
pub mod arxiv_scanner;
pub mod bash;
pub mod batch_missing;
pub mod browser;
pub mod capability_browse;
pub mod capability_load;
pub mod capability_pack_defs;
pub mod capability_shared;
pub mod capability_view;
pub mod ci;
#[cfg(feature = "computer-use")]
pub mod computer_use;
pub mod context;
pub mod cron;
pub mod database;
pub mod demand_llm;
pub mod devops;
pub mod document;
pub mod domain_mcp;
pub mod export;
pub mod file_edit;
pub mod file_read;
pub mod file_system;
pub mod file_write;
pub mod finance;
pub mod git;
pub mod github_discussions_scanner;
pub mod github_issue_scanner;
pub mod glob;
pub mod grep;
pub mod hacker_news_scanner;
pub mod huggingface_scanner;
pub mod integration;
pub mod knowledge;
pub mod lsp;
pub mod market_mainline;
// 阶段 2「候选池改趋势→全关联股票」：行业→全成分股确定性查询工具
pub mod industry_constituents;
pub mod marketplace_scanner;
pub mod media;
pub mod media_delivery;
pub mod messaging;
pub mod migration_tool;
pub mod misc;
pub mod monitor;
pub mod multi_agent;
pub mod narrative_structure;
pub mod network;
pub mod obsidian;
pub mod ocr;
pub mod ontology_query;
pub mod opc;
pub mod opc_demand_scan;
pub mod package_ecosystem_scanner;
pub mod personality;
pub mod plan;
pub mod product_hunt_scanner;
pub mod push_notification;
pub mod reddit_scanner;
pub mod repl;
pub mod rpc;
pub mod run_workflow;
pub mod save_as_workflow;
pub mod scan_policy;
pub mod scanner_common;
pub mod shared_path;
pub mod skill;
pub mod stackoverflow_scanner;
pub mod stock_factor;
pub mod storage;
pub mod system_info;
pub mod task_system;
pub mod testing;
pub mod todo_write;
pub mod twitter_scanner;
pub mod web_fetch;
pub mod web_search;
pub mod workspace;
pub mod worktree;
pub mod xianyu_scanner;
pub mod zhubajie_scanner;

// 新增扫描器
pub mod csdn_scanner;
pub mod dribbble_scanner;
pub mod linkedin_scanner;
pub mod upwork_scanner;
pub mod zhihu_scanner;

pub use todo_write::{AskUserQuestionTool, NotebookEditTool};

/// 内置工具实例清单 —— [`register_all`] 与 `tool.set` 接缝共用同一份定义。
///
/// 顺序即注册顺序（同名后者覆盖前者），与各段原先的拼接顺序一致。
pub fn builtin_tool_instances() -> Vec<std::sync::Arc<dyn crate::Tool>> {
    let mut tools: Vec<std::sync::Arc<dyn crate::Tool>> = vec![
        // ── 核心文件操作 ──
        std::sync::Arc::new(file_read::FileReadTool),
        std::sync::Arc::new(file_write::FileWriteTool),
        std::sync::Arc::new(file_edit::FileEditTool),
        std::sync::Arc::new(file_system::ListDirectoryTool),
        std::sync::Arc::new(file_system::DeleteFileTool),
        std::sync::Arc::new(file_system::CreateDirectoryTool),
        std::sync::Arc::new(file_system::FileExistsTool),
        std::sync::Arc::new(file_system::GetFileInfoTool),
        std::sync::Arc::new(file_system::MoveFileTool),
        std::sync::Arc::new(glob::GlobTool),
        std::sync::Arc::new(grep::GrepTool),
        // ── Shell 和网络 ──
        std::sync::Arc::new(bash::BashTool),
        std::sync::Arc::new(web_fetch::WebFetchTool),
        std::sync::Arc::new(web_search::WebSearchTool),
        // ── 网络工具 ──
        std::sync::Arc::new(network::HttpRequestTool),
        std::sync::Arc::new(network::PingTool),
        std::sync::Arc::new(network::DnsLookupTool),
        std::sync::Arc::new(network::JsonApiTool),
        std::sync::Arc::new(network::RssReaderTool),
        std::sync::Arc::new(network::GraphQLTool),
        std::sync::Arc::new(network::WebSocketTool),
        // ── 任务和提问 ──
        std::sync::Arc::new(todo_write::TodoWriteTool),
        std::sync::Arc::new(todo_write::AskUserQuestionTool),
        std::sync::Arc::new(todo_write::NotebookEditTool),
        // ── Agent 和 Skill ──
        std::sync::Arc::new(agent::AgentTool),
        std::sync::Arc::new(multi_agent::DelegateTaskTool),
        std::sync::Arc::new(skill::SkillsListTool),
        std::sync::Arc::new(skill::SkillViewTool),
        std::sync::Arc::new(skill::SkillReferenceTool),
        // ── 能力渐进式披露 L1：定义层按需展开（面向护照全集，与 SkillView 互补）──
        std::sync::Arc::new(capability_view::CapabilityViewTool),
        // ── 能力渐进式披露 L1.5：加载层（写会话状态 + 激活工具，有副作用）──
        std::sync::Arc::new(capability_load::CapabilityLoadTool),
        // ── 领域本体查询：按 domain 路由查类/关系/分档/权重（P1 §4.3.3，一次注册全域可见）──
        std::sync::Arc::new(ontology_query::OntologyQueryTool),
        // ── 能力持久化：把已加载能力组装为工作流模板 ──
        std::sync::Arc::new(save_as_workflow::SaveAsWorkflowTool),
        // ── 工作流执行入口：认知编排命中 Workflow 能力后由 agent 发起执行（T3）──
        std::sync::Arc::new(run_workflow::RunWorkflowTool),
        // ── 能力渐进式披露 L0：导航层（能力树逐层下钻）──
        std::sync::Arc::new(capability_browse::CapabilityBrowseTool),
        // ── OPC 需求发现：按关键词扫描平台并评估入库（demand-discovery 模板 Loop 体）──
        std::sync::Arc::new(opc_demand_scan::OpcDiscoverLeadsTool),
        // ── 叙事结构引擎：文学创作工作流的确定性结构注入 / 落库（替代 lc-structure-injector LLM 节点）──
        std::sync::Arc::new(narrative_structure::NarrativeChapterInstructionsTool),
        std::sync::Arc::new(narrative_structure::NarrativeStructurePersistTool),
        std::sync::Arc::new(skill::SkillBundleListTool),
        std::sync::Arc::new(skill::SkillBundleCreateTool),
        std::sync::Arc::new(skill::SkillBundleLoadTool),
        std::sync::Arc::new(skill::SkillBundleDeleteTool),
        // ── 计划模式 ──
        std::sync::Arc::new(plan::EnterPlanModeTool),
        std::sync::Arc::new(plan::ExitPlanModeTool),
        std::sync::Arc::new(plan::VerifyPlanExecutionTool),
        // ── 人格系统 ──
        std::sync::Arc::new(personality::PersonalityTool),
        // ── 桌面控制 ──
        #[cfg(feature = "computer-use")]
        std::sync::Arc::new(computer_use::ComputerUseTool),
        // ── 浏览器 ──
        std::sync::Arc::new(browser::BrowserNavigateTool),
        std::sync::Arc::new(browser::BrowserScreenshotTool),
        std::sync::Arc::new(browser::BrowserClickTool),
        std::sync::Arc::new(browser::BrowserFillTool),
        std::sync::Arc::new(browser::BrowserTypeTool),
        std::sync::Arc::new(browser::BrowserExtractTextTool),
        std::sync::Arc::new(browser::BrowserExtractAllTool),
        std::sync::Arc::new(browser::BrowserGetContentTool),
        std::sync::Arc::new(browser::BrowserSelectTool),
        std::sync::Arc::new(browser::BrowserWaitForTool),
        // ── 定时任务 ──
        std::sync::Arc::new(cron::CronCreateTool),
        std::sync::Arc::new(cron::CronDeleteTool),
        std::sync::Arc::new(cron::CronListTool),
        // ── Task 系统 ──
        std::sync::Arc::new(task_system::TaskCreateTool),
        std::sync::Arc::new(task_system::TaskGetTool),
        std::sync::Arc::new(task_system::TaskListTool),
        std::sync::Arc::new(task_system::TaskStopTool),
        std::sync::Arc::new(task_system::TaskUpdateTool),
        std::sync::Arc::new(task_system::TaskOutputTool),
        // ── Worktree ──
        std::sync::Arc::new(worktree::EnterWorktreeTool),
        std::sync::Arc::new(worktree::ExitWorktreeTool),
        // ── 系统工具 ──
        std::sync::Arc::new(batch_missing::SleepTool),
        std::sync::Arc::new(batch_missing::ConfigTool),
        std::sync::Arc::new(batch_missing::ReviewArtifactTool),
        std::sync::Arc::new(batch_missing::TerminalCaptureTool),
        std::sync::Arc::new(skill::DiscoverSkillsTool),
        // ── Skills Hub — 本地隔离区管理 ──
        std::sync::Arc::new(skill::SkillHubReviewTool),
        std::sync::Arc::new(skill::SkillEnvCheckTool),
        std::sync::Arc::new(skill::SkillConfigTool),
        // ── 消息和文件 ──
        std::sync::Arc::new(batch_missing::BriefTool),
        std::sync::Arc::new(batch_missing::SendUserFileTool),
        std::sync::Arc::new(batch_missing::SubscribePRTool),
        std::sync::Arc::new(batch_missing::WorkflowTool),
        std::sync::Arc::new(agent::RemoteTriggerTool),
        std::sync::Arc::new(agent::SuggestBackgroundPRTool),
        // ── 通信: SendMessage + ListPeers + Team ──
        std::sync::Arc::new(messaging::SendMessageTool),
        std::sync::Arc::new(messaging::ListPeersTool),
        std::sync::Arc::new(messaging::TeamCreateTool),
        std::sync::Arc::new(messaging::TeamDeleteTool),
        // ── 开发工具 ──
        std::sync::Arc::new(lsp::LSPTool),
        std::sync::Arc::new(repl::REPLTool),
        // ── 监控和上下文 ──
        std::sync::Arc::new(monitor::MonitorTool),
        std::sync::Arc::new(context::CtxInspectTool),
        std::sync::Arc::new(context::SnipTool),
        std::sync::Arc::new(context::ContextResolveTool),
        std::sync::Arc::new(context::ContextRemainingTool),
        // ── 知识库 ──
        std::sync::Arc::new(knowledge::ListKnowledgeBasesTool),
        std::sync::Arc::new(knowledge::SearchKnowledgeTool),
        std::sync::Arc::new(knowledge::CreateKnowledgeEntityTool),
        std::sync::Arc::new(knowledge::CreateKnowledgeFlowTool),
        std::sync::Arc::new(knowledge::CreateKnowledgeInterfaceTool),
        std::sync::Arc::new(knowledge::AddKnowledgeDocumentTool),
        // ── 存储管理 ──
        std::sync::Arc::new(storage::GetStorageInfoTool),
        std::sync::Arc::new(storage::ListStorageFilesTool),
        std::sync::Arc::new(storage::UploadStorageFileTool),
        std::sync::Arc::new(storage::DownloadStorageFileTool),
        std::sync::Arc::new(storage::DeleteStorageFileTool),
        // ── 系统信息 ──
        std::sync::Arc::new(system_info::GetSystemInfoTool),
        std::sync::Arc::new(system_info::ListProcessesTool),
        // ── Git ──
        std::sync::Arc::new(git::GitStatusTool),
        std::sync::Arc::new(git::GitDiffTool),
        std::sync::Arc::new(git::GitCommitTool),
        std::sync::Arc::new(git::GitLogTool),
        std::sync::Arc::new(git::GitBranchTool),
        std::sync::Arc::new(git::GitReviewTool),
        // ── 原子补丁 ──
        std::sync::Arc::new(apply_patch::ApplyPatchTool),
        // ── OCR ──
        std::sync::Arc::new(ocr::OcrImageTool),
        std::sync::Arc::new(ocr::OcrDetectLangsTool),
        // ── Obsidian Vault 集成（9 个语义化工具，参考 DeepTutor） ──
        // 通过 ToolContext.extra["vault_kb_id"] 注入已绑定的 ConnectedVault KB
        std::sync::Arc::new(obsidian::ObsidianSearchTool),
        std::sync::Arc::new(obsidian::ObsidianReadTool),
        std::sync::Arc::new(obsidian::ObsidianListTool),
        std::sync::Arc::new(obsidian::ObsidianBacklinksTool),
        std::sync::Arc::new(obsidian::ObsidianLinksTool),
        std::sync::Arc::new(obsidian::ObsidianTagsTool),
        std::sync::Arc::new(obsidian::ObsidianCreateNoteTool),
        std::sync::Arc::new(obsidian::ObsidianAppendTool),
        std::sync::Arc::new(obsidian::ObsidianSetPropertyTool),
        // ── 导出与格式 ──
        std::sync::Arc::new(document::ExportWordTool),
        std::sync::Arc::new(document::RenderMarkdownTool),
        std::sync::Arc::new(document::ExportPdfTool),
        std::sync::Arc::new(document::ExportXlsxTool),
        std::sync::Arc::new(document::ExportPptxTool),
        std::sync::Arc::new(document::ReadXlsxTool),
        std::sync::Arc::new(document::ReadPptxTool),
        std::sync::Arc::new(export::PdfInfoTool),
        std::sync::Arc::new(export::DetectEncodingTool),
        // ── 远程文件 ──
        std::sync::Arc::new(misc::RemoteFileUploadTool),
        std::sync::Arc::new(misc::RemoteFileListTool),
        std::sync::Arc::new(misc::RemoteFileDeleteTool),
        // ── 缓存管理 ──
        std::sync::Arc::new(misc::CacheInfoTool),
        std::sync::Arc::new(misc::CacheClearTool),
        // ── 工作区记忆 ──
        std::sync::Arc::new(workspace::WorkspaceReadTool),
        std::sync::Arc::new(workspace::WorkspaceWriteTool),
        // ── Agent 记忆 ──
        std::sync::Arc::new(agent_memory::SessionSearchTool),
        std::sync::Arc::new(agent_memory::MemoryFlushTool),
        std::sync::Arc::new(agent_memory::AgentCheckpointTool),
        std::sync::Arc::new(agent_memory::AgentStatusTool),
        std::sync::Arc::new(agent_memory::AgentRememberTool),
        // MemoryRecall: 主动按任务语义检索文件级长期记忆(.axagent/memory/)
        std::sync::Arc::new(agent_memory::MemoryRecallTool),
        // ── AI 媒体 ──
        std::sync::Arc::new(media::GenerateImageTool),
        std::sync::Arc::new(media::GenerateChartConfigTool),
        std::sync::Arc::new(media::SequentialThinkingTool),
        std::sync::Arc::new(media::Base64ImageTool),
        // ── 媒体智能投递 ──
        std::sync::Arc::new(media_delivery::MediaDetectTool),
        std::sync::Arc::new(media_delivery::MediaDeliverTool),
        std::sync::Arc::new(media_delivery::MediaPreviewTool),
        // ── 外部集成 ──
        std::sync::Arc::new(integration::DifyListBasesTool),
        std::sync::Arc::new(integration::DifySearchTool),
        // ── 通知 ──
        std::sync::Arc::new(push_notification::PushNotificationTool),
        // ── 数据库管理 ──
        std::sync::Arc::new(database::DatabaseQueryTool),
        std::sync::Arc::new(database::DatabaseListTablesTool),
        std::sync::Arc::new(database::DatabaseMigrationStatusTool),
        // ── 测试运行 ──
        std::sync::Arc::new(testing::RunTestsTool),
        std::sync::Arc::new(testing::RunLinterTool),
        std::sync::Arc::new(testing::RunTestCoverageTool),
        // ── CI/CD ──
        std::sync::Arc::new(ci::CiStatusTool),
        std::sync::Arc::new(ci::CiTriggerTool),
        std::sync::Arc::new(ci::CiListWorkflowsTool),
        // ── DevOps ──
        std::sync::Arc::new(devops::SecurityAuditTool),
        std::sync::Arc::new(devops::DeadCodeDetectTool),
        std::sync::Arc::new(devops::BundleAnalyzeTool),
        std::sync::Arc::new(devops::IssueCreateTool),
        std::sync::Arc::new(devops::IssueListTool),
        // ── RPC ──
        std::sync::Arc::new(rpc::RpcTool),
        std::sync::Arc::new(rpc::RpcCallTool),
        // ── 迁移工具 ──
        std::sync::Arc::new(migration_tool::MigrationTool),
        // ── 金融分析（finance.rs，29 个：风险模型 / 信号 / 数据清洗 / 技术指标 / 蒙特卡洛 / 行情 API）──
        // [2026-09-03 接线恢复] finance.rs 此前无 mod 声明 + 未挂 register_all，整模块从未编译。
        std::sync::Arc::new(finance::CalcMaxDrawdownTool),
        std::sync::Arc::new(finance::CalcSharpeRatioTool),
        std::sync::Arc::new(finance::CalcVarTool),
        std::sync::Arc::new(finance::CalcPEPercentileTool),
        std::sync::Arc::new(finance::CalcPEGTool),
        std::sync::Arc::new(finance::CalcKellyTool),
        std::sync::Arc::new(finance::CalcRiskParityTool),
        std::sync::Arc::new(finance::DetectMACrossTool),
        std::sync::Arc::new(finance::DetectBreakoutTool),
        std::sync::Arc::new(finance::CleanOutliersTool),
        std::sync::Arc::new(finance::CleanFillMissingTool),
        std::sync::Arc::new(finance::AdjustPricesTool),
        std::sync::Arc::new(finance::ComputeATRTool),
        std::sync::Arc::new(finance::ComputeKDJTool),
        std::sync::Arc::new(finance::ComputeOBVTool),
        std::sync::Arc::new(finance::CalcBetaTool),
        std::sync::Arc::new(finance::DetectEarningsTool),
        std::sync::Arc::new(finance::DetectPledgeRiskTool),
        std::sync::Arc::new(finance::CalcCorrMatrixTool),
        std::sync::Arc::new(finance::RunMonteCarloTool),
        std::sync::Arc::new(finance::AnalyzeIndustryTool),
        std::sync::Arc::new(finance::DetectLimitUpTool),
        std::sync::Arc::new(finance::DetectCandlestickPatternsTool),
        std::sync::Arc::new(finance::DetectDivergenceTool),
        // B7b：五个数据 API 工具（get_research_reports / get_concept_blocks /
        // get_north_bound_flow / get_market_dragon_tiger / get_cls_flash）已改为
        // 域工具定义 + domain_tools 执行槽位，不再由 tools crate 注册（见 finance.rs 注释）。
    ];

    // ── 业务域工具（L1）：并入内置清单 ──
    // 定义由域侧经 `domain_tools::register_tool_definitions` 在启动早期注入；
    // 本段只做通用包装，工具层不认识任何具体域（PLAN §54.3 B5）。
    // 注册后 register_all_capabilities 的通用逻辑自动派生 tool:{name} 护照
    // （domain=Finance + tool_ref），DiscoverSkills / extra_tools 注入链路即刻生效。
    // 内部跳过与 finance.rs 重名的 3 个工具，避免 HashMap 静默覆盖。
    tools.extend(domain_mcp::domain_mcp_tool_instances());

    // ── 市场主线工具（daily-market-events 模板的 persist 落地工具）──
    //
    // 为什么单独列一段：该工具名由 `seed_daily_market_events.rs` 的 Agent 节点
    // `ToolDef { name: "market_mainline_batch_upsert" }` 声明，并在提示词里要求模型调用。
    // 而工作流的工具解析（`init/services.rs` 注入的 `ToolResolver`）判据是
    // `reg.list_all_tool_names()` ∪ `reg.mcp.mcp_tools` —— `#[agent_command]`
    // 元数据**不在其中**（它只喂 `command_bridge` 的 `execute_tauri_command` 索引）。
    // 故未注册时该名字必然解析为 None，调用被 `core.rs` 的 Failed 分支
    // `emit degraded: true` 静默吞掉（节点仍 completed、结果为空）。
    //
    // ⚠️ 与 `seed_daily_market_events.rs` 的 `ToolDef.name` **逐字同名**，
    //    改名必须同步模板并递增 `TEMPLATE_VERSION`（有单测锚定，见
    //    `market_mainline::tests::tool_name_matches_seed_template_declaration`）。
    tools.push(std::sync::Arc::new(market_mainline::MarketMainlineBatchUpsertTool));

    // ── 行业成分股工具（趋势智选候选池数据面，阶段 2）──
    //
    // 工作流 ToolResolver 判据是 register_all 注册集 ∪ mcp_tools ⇒ 未注册则
    // 快速链 t-constituents / 原链聚合注入全部「工具未注册」。实现委托
    // `axagent_analysis_engine::industry_constituents`，DB 走 `get_sea_db()` 槽位
    // （同 market_mainline 先例）。只读、幂等。
    tools.push(std::sync::Arc::new(industry_constituents::IndustryConstituentsTool));

    // ── 逐票 1 月反转因子工具（趋势智选打分重构数据面，阶段 3）──
    // 实现委托 `axagent_analysis_engine::stock_factor`，DB 走 `get_sea_db()` 槽位。
    tools.push(std::sync::Arc::new(stock_factor::StockFactorTool));

    // ── OPC 业务工具（一人公司：发票 / 客户 / 项目 / 站点内容 / KPI / 发布计划）──
    //
    // 为什么单独列一段：`opc.rs` 这 27 个 `impl Tool` 此前**只被
    // 域包工具白名单（现 `crates/tools/src/tools/capability_pack_defs.rs`，2026-09-16 下沉）
    // 的 `opc_tool_defs()` 用来产出
    // ToolDef schema**，从未进入本注册表。后果是 `init/services.rs` 的 ToolResolver
    // 判定 `known = reg.list_all_tool_names().contains(name) || mcp_tools...` 恒为
    // false —— 工作流 AgentNode 只要声明 `OpcCreateBlogPost` 之类，执行时就必然拿到
    // 「工具 '{name}' 未注册」（派发点 `agent_executor.rs` 的 `execute_tool`）。
    // 本段补上注册后，该 resolver 的 `known` 分支即刻接通，**无需改引擎 / resolver**。
    //
    // 与 `opc_demand_scan::OpcDiscoverLeadsTool`（上方 :144 已注册）不重复：
    // 那段是 demand-discovery 的 Loop 体工具，与本段 26 个无交集。
    //
    // ⚠️ `OpcSendNotification` **已退役**（缺陷 #184，2026-11 本批）—— 原先它「故意未列入」，
    // 理由是 `opc.rs` 的 `get_notify_tx()` 用 `expect("OPC notify tx not initialized")`
    // 同步 panic，而 `set_opc_notify_tx` 全仓零调用者 ⇒ 一旦注册即「可达且必 panic」。
    // 本批裁定并落地**退役**而非补生产者，两个独立理由：
    //   ① `OpcNotification` 全仓**零接收端**（没有 `Receiver`、没有通知 worker 消费它）⇒
    //      即便注入 `tx`，`UnboundedSender::send` 也恒 `Err` ⇒ 只是把「必 panic」换成
    //      「必失败」，与任务 #115 判 `alert_tx`「写进一条没人听的频道」同一形态；
    //      要让它真能用得先建通知 worker + 渠道接线，属新功能，不在缺陷修复授权内。
    //   ② 工具执行路径里的同步 panic 本身不可留（上层只能捕获 `Err`，捕不了 panic）。
    // ⇒ 实现体、channel、`set_opc_notify_tx` / `get_notify_tx` 均已从 `opc.rs` 删除，
    //   `capability_pack_defs::opc_tool_defs` 的候选同步摘除 ⇒ **本段永远不会有它**，
    //   不存在「待加回」这回事了（原「修好 panic 后加回」的口径作废）。
    //   域包 / 专家清单里残留的 `td("OpcSendNotification")` 声明是**行为面**遗留（需逐文件
    //   递增 `TEMPLATE_VERSION` 才会重导模板），登记给 OPC→L1 插件迁移（任务 #36），本批不动。
    //   历史登记见 output/opc-tool-wiring-impl.md 的 DEF-OPC-NOTIFY-01（现已闭合）。
    // 元素类型必须显式标注：异构的 `vec![Arc::new(A), Arc::new(B), ..]` 会按
    // **首元素**推成 `Vec<Arc<A>>`，而不是 `Vec<Arc<dyn Tool>>`（原样传给
    // `registry.register_all` 时由形参兜底，改走 `tools.extend` 后没人兜底）。
    let opc_tools: Vec<std::sync::Arc<dyn crate::Tool>> = vec![
        // 发票 / 客户 / 项目
        std::sync::Arc::new(opc::OpcListInvoicesTool),
        std::sync::Arc::new(opc::OpcCreateInvoiceTool),
        std::sync::Arc::new(opc::OpcTransitionInvoiceTool),
        std::sync::Arc::new(opc::OpcListCustomersTool),
        std::sync::Arc::new(opc::OpcCreateCustomerTool),
        std::sync::Arc::new(opc::OpcListProjectsTool),
        std::sync::Arc::new(opc::OpcCreateProjectTool),
        std::sync::Arc::new(opc::OpcAddMilestoneTool),
        std::sync::Arc::new(opc::OpcGetDashboardTool),
        // 站点内容（博客 / 落地页 / 联系人）
        std::sync::Arc::new(opc::OpcListLandingPagesTool),
        std::sync::Arc::new(opc::OpcListBlogPostsTool),
        std::sync::Arc::new(opc::OpcListContactsTool),
        std::sync::Arc::new(opc::OpcCreateLandingPageTool),
        std::sync::Arc::new(opc::OpcCreateBlogPostTool),
        // 指标 / 知识源 / 财务
        std::sync::Arc::new(opc::OpcRecordKpiTool),
        std::sync::Arc::new(opc::OpcListKpisTool),
        std::sync::Arc::new(opc::OpcSearchWikiTool),
        std::sync::Arc::new(opc::OpcGetFinancialReportTool),
        // 内容资产（域包模板已引用，见 capability_pack.rs 的 opc_tool_defs）
        std::sync::Arc::new(opc::OpcCreateContentAssetTool),
        std::sync::Arc::new(opc::OpcListContentAssetsTool),
        std::sync::Arc::new(opc::OpcUpdateContentAssetTool),
        std::sync::Arc::new(opc::OpcDeleteContentAssetTool),
        // 发布计划（草稿 → 已发布的唯一程序化入口之一）
        std::sync::Arc::new(opc::OpcCreatePublishScheduleTool),
        std::sync::Arc::new(opc::OpcListPublishSchedulesTool),
        std::sync::Arc::new(opc::OpcCancelPublishScheduleTool),
        std::sync::Arc::new(opc::OpcProcessDueSchedulesTool),
    ];
    tools.extend(opc_tools);
    tools
}

/// 注册所有内置工具到注册表。
///
/// 工具清单来自 [`builtin_tool_instances`] —— 与 `tool.set` 接缝的内置实现
/// 共用同一份定义，故内置工具集可被外部实现整体替换。
pub fn register_all(registry: &mut crate::registry::ToolRegistry) {
    registry.register_all(builtin_tool_instances());

    let available_toolsets: HashSet<String> =
        registry.list_all().iter().map(|t| format!("{:?}", t.category).to_lowercase()).collect();
    skill::set_available_toolsets(available_toolsets);

    rpc::set_tool_executor(std::sync::Arc::new(registry.clone()));
}

/// 内置工具集 —— `tool.set` 接缝的内置实现。
///
/// wiring 层在启动装配时把它注册进能力注册表（
/// [`axagent_harness::CapabilityRegistry::register_tool_set`]），
/// 工具注册表初始化时再从接缝取回；外部实现注册同一接缝即可整体替换内置工具集。
pub struct BuiltinToolSet;

impl axagent_harness::ToolSetProvider for BuiltinToolSet {
    fn tools(&self) -> Vec<std::sync::Arc<dyn crate::Tool>> {
        builtin_tool_instances()
    }
}

/// 注入跨层依赖的 trait object。
/// 在 wiring 层（runtime/gateway）启动时调用一次。
pub fn init_extensions(
    migration_runner: Arc<dyn axagent_harness::MigrationRunner>,
    plugin_agent_provider: Arc<dyn axagent_harness::PluginAgentProvider>,
    delegate_task_runner: Option<Arc<dyn axagent_harness::DelegateTaskRunner>>,
) {
    migration_tool::set_migration_runner(migration_runner);
    agent::set_plugin_agent_provider(plugin_agent_provider);
    if let Some(runner) = delegate_task_runner {
        multi_agent::set_delegate_task_runner(runner);
    }
}

// ── OpcSendNotification 退役事实钉桩（缺陷 #184）──
// ⚠ 必须保持在文件末尾：插在中间会触发 clippy::items_after_test_module。
#[cfg(test)]
mod opc_send_notification_retired_tests {
    /// 退役判据 ①：内置工具实例集里**永远不该**再出现 `OpcSendNotification`。
    ///
    /// 它原先「故意未列入」，本批把实现体一并删了。若有人日后「顺手加回」，
    /// 这条测试会红 —— 加回的前提是先建通知 worker（有真实接收端），
    /// 而不是把一条 `expect` 型 panic 重新接进工具执行路径。
    #[test]
    fn builtin_tool_instances_excludes_retired_opc_send_notification() {
        let instances = super::builtin_tool_instances();
        // 正控先行：内置实例集必须**真的起来了**（含一个在活的 OPC 工具），否则下面那条
        // 「不含退役名」的断言会因为「函数坏掉 / 返回空集」而假绿 —— 本仓既有教训：
        // 「断言不发生」的负控必须与同族正控配对。
        assert!(
            instances.iter().any(|t| t.name() == "OpcListCustomers"),
            "正控失效：应能在内置实例集里找到在活的 `OpcListCustomers`"
        );
        let hit = instances.iter().any(|t| t.name() == "OpcSendNotification");
        assert!(!hit, "`OpcSendNotification` 已退役（#184）：不得重新进入内置工具注册集");
    }

    /// 退役判据 ②：域包工具白名单也不得为它产出 ToolDef —— 产出 ToolDef 等于
    /// 让 LLM 看见一个不存在的能力（历史形态：LLM 调得到、执行必失败）。
    ///
    /// 同时正向对照：白名单里仍活的 `OpcListCustomers` 必须照常产出，
    /// 否则本断言会因「整个函数坏掉」而假绿。
    #[test]
    fn opc_tool_defs_skips_retired_name_but_keeps_live_ones() {
        use crate::tools::capability_pack_defs::opc_tool_defs;
        let names: Vec<String> =
            ["OpcSendNotification", "OpcListCustomers"].iter().map(|s| (*s).to_string()).collect();
        let produced: Vec<String> = opc_tool_defs(&names).into_iter().map(|d| d.name).collect();
        assert!(
            !produced.iter().any(|n| n == "OpcSendNotification"),
            "退役工具不应产出 ToolDef，实际产出：{produced:?}"
        );
        assert!(
            produced.iter().any(|n| n == "OpcListCustomers"),
            "正向对照失效：在活的工具应产出 ToolDef，实际产出：{produced:?}"
        );
    }

    /// 退役判据 ③：工具执行路径里不得再有「未初始化即 panic」这类同步 panic 串。
    /// 逐字锚定原 panic 消息，防止有人只删调用点、把 `expect` 留在源码里。
    ///
    /// ⚠ 判据只作用在**代码行**上：上面的退役说明注释必然要引用被删掉的串原文
    /// （`expect("OPC notify tx not initialized")`、`set_opc_notify_tx`），不剥行注释就是
    /// 自证失败（2026-11-06 实测：本用例因自己的注释红了一次）。
    /// 固有边界：本判据只剥 `//` 起始行，若将来把这两个串包进 `/* */` 块注释，会漏判 ——
    /// 那是「源码文本判据」这一形态的天花板，要更强判据得改行为级（构造实例并调用）。
    #[test]
    fn opc_module_has_no_uninitialized_channel_panic() {
        let src = code_lines_only(include_str!("opc.rs"));
        assert!(
            !src.contains("OPC notify tx not initialized"),
            "`opc.rs` 不应再包含 OPC channel 的 panic 串（#184 已退役）"
        );
        assert!(
            !src.contains("set_opc_notify_tx"),
            "`opc.rs` 不应再保留零调用者的注入入口 `set_opc_notify_tx`"
        );
    }

    /// 只留非注释行（判据 ③ 的靶面预处理）。
    fn code_lines_only(src: &str) -> String {
        src.lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// `code_lines_only` 自身的正负控：剥注释这件事不能只靠肉眼认定 ——
    /// 正控证明注释里的串确实被剥掉，负控证明代码行里的同名串**不会**被剥（否则判据 ③
    /// 会因为预处理过头而永远绿）。用中性假 token 取材：若拿真串当样本，字符串字面量本身
    /// 就是一行代码，判据会被自己的样本喂假。
    #[test]
    fn code_lines_only_strips_comments_but_keeps_code() {
        const TOKEN: &str = "ZZZ_SENTINEL_TOKEN";
        let sample = format!("// {TOKEN} in a comment\n    let x = \"{TOKEN}\";\n");
        let kept = code_lines_only(&sample);
        assert_eq!(
            kept.matches(TOKEN).count(),
            1,
            "注释里的一处必须被剥掉、代码里的一处必须留下，实际保留 {kept:?}"
        );
        assert!(kept.trim_start().starts_with("let x"), "剩下的应是代码行：{kept}");
    }
}
