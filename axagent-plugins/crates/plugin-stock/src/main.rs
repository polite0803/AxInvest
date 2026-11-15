// SPDX-License-Identifier: AGPL-3.0-only
//! 股票域插件 worker（B4 骨架 + 首个真实能力）。
//!
//! 宿主以 `worker.program` 相对路径 spawn 本进程，双方在 stdin/stdout 上交换长度前缀
//! JSON 帧（`axagent-plugin-proto`）。诊断信息一律走 stderr —— stdout 是协议通道，
//! 混入任何非帧字节都会让宿主读帧失败。
//!
//! **首个能力（B4-c）**：`tool.set` 接缝上的只读工具 `get_stock_quote`。
//! 它是「宿主只读、不持 AppState、不碰凭证」那一档的最小实证：
//! - 工具本身不查库、不读文件、不取时钟；
//! - 行情一律经宿主接缝 `market.quote` 的 `get_quote` op 取（凭证与 provider 都在宿主侧）；
//! - 宿主侧 `RemoteToolSet::load` 会在注册时先调 `list` 取元数据，`exec` 按名执行。
//!
//! 为什么用 `plugin-proto` 的 [`serve`] 而不是自写读循环：插件一旦能主动调宿主，
//! 同一条管道上就同时挂着「我方请求的响应」与「对端发来的新请求」，朴素读一帧写一帧
//! 必然错配。`Peer` 把对端请求进 `deferred` 队列，等响应的同时不丢帧、不死锁。
//!
//! 为什么是纯可执行包（只有 `src/main.rs`、没有 `src/lib.rs`）：分层门禁规则 4 要求每个
//! 库 crate 至少被一份 `Cargo.toml` 引用，而 worker 由宿主 spawn、没有 Cargo 消费方；
//! 门禁对「纯可执行包」已有豁免先例（`crates/schema-gen`），本 crate 照同一形态。

use std::io::{StdinLock, StdoutLock};
use std::process::ExitCode;

use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, CapabilityDecl, CommandFault, CommandFrame, CommandOutcome,
    CommandReply, FrameRequest, FrameResponse, Peer, PluginDeclaration, PluginTaskRunRow,
    ResponseKind, error_codes, is_plugin_task_run_state, ops, serve,
};
use serde_json::{Value, json};

/// 本插件提供的工具名（与宿主登记的工具名逐字一致）。
const TOOL_NAME: &str = "get_stock_quote";
/// 本插件承接的命名命令（manifest `commands[].name` 必须与它逐字一致）。
///
/// 命令面（前端 `invokePluginCommand`）入参一律 camelCase（禁区 13），接缝面（宿主
/// `worker.rs` 既有协议）是 snake_case —— 翻译点只有 `market_read_route` / `kline_seam_args`
/// 两处。工具面（`tools()` 声明 + `exec_tool`）保持 snake_case `stock_code`，因为它的
/// 入参形状与宿主 `stock_analysis_bridge.rs` 的 LLM 工具声明逐字同源，改它等于改智能体面。
const COMMAND_GET_QUOTE: &str = "stock_get_quote";
/// K 线命令（批 2b）：`market.quote` 的 `get_klines` 是多入参 op，不进单标的路由表。
const COMMAND_GET_KLINE: &str = "stock_get_kline";
/// 自然语言意图解析命令：跑 `axagent-stock-core` 的**本体**（不是测试夹具的子集词表）。
const COMMAND_PARSE_INTENT: &str = "parse_analysis_intent";
/// 证据质量驱动权重（B9-1）：纯函数，跑 `analysis-engine` 的权威实现。
///
/// 为什么不在插件里重写一份：权重表/阈值是 LLM 决策链的**数值权威**
/// （`evidence_weight.rs` 的档位与 0.15 阈值都有历史校准记录，见 §86 系列），
/// 复制一份就等于让「插件版」与「宿主历史产物」的算法分叉（禁区 12）。
const COMMAND_EVIDENCE_WEIGHTS: &str = "compute_evidence_weights";
/// `tool.set` 接缝的协议版本；随工具集形状变化时递增。
const TOOL_SET_VERSION: &str = "0.1.0";
/// 工具取数走的宿主接缝：宿主把已有行情能力以入站分派的方式暴露给插件。
const MARKET_SEAM: &str = "market.quote";
/// 任务源取数走的宿主接缝（B8-1b）：工作流执行的运行态只有宿主引擎知道。
const WORKFLOW_RUN_SEAM: &str = "workflow.run";
/// 单股分析记录取数走的宿主接缝（#149 第 1 步）：宿主发起的分析记录在 `stock_analyses` 表里，
/// `workflow.run` 按 owner 隔离看不见。清单声明（`taskSources[]`）留给后续批 —— 本步只把
/// 出站准入（`calls`）与本命令的实现打通。
const ANALYSIS_RECORD_SEAM: &str = "analysis.record";
/// 股票分析数据面接缝（E 批 #240）：成分股 / 逐票 1 月涨跌幅 / 市场主线批量 upsert。
///
/// 这七件能力的实现体分布在 `analysis-engine`（implementor，前三件）与主壳 commands 层
/// （后四件：pipeline 行 / 进化计划 / DES 仿真 DTO）⇒ 跨包口判据见
/// `harness/src/stock_analysis.rs` 模块头。全接缝只有 `batch_upsert_mainlines` 一个写 op。
const STOCK_ANALYSIS_SEAM: &str = "stock.analysis";
/// 自选行情监视列表接缝（本批宿主 `list_watched_quotes` 的取数面）：
/// 自选列表的权威存储在宿主 `quote_watcher_service`，`list_watched` 只读不设闸。
const QUOTE_WATCHER_SEAM: &str = "quote.watcher";
/// 交易引擎接缝（#311）：持仓与交易对比的权威只有宿主持有的 `TradingEngine` 知道。
/// 本批只用两个读 op（`get_positions` / `compare_trade`），`execute_trade` 那个写 op 不碰。
const TRADING_ENGINE_SEAM: &str = "trading.engine";
// ── #311：四条零新 op 的纯智能体工具迁 T2（宿主同名命令同批退役）──
// 四条前端 `invoke` 全零命中，只活在智能体工具面 ⇒ 按 #273 迁不删能力：
// 命令名加 `stock_` 前缀（与既有批一致），入参 camelCase、接缝面 snake_case。
/// 组合风险指标（宿主 `get_portfolio_risk`）：持仓来自接缝，算法跑 `analysis-engine` 权威实现。
const COMMAND_GET_PORTFOLIO_RISK: &str = "stock_get_portfolio_risk";
/// 交易出场价与分析预测对比（宿主 `compare_trade_with_analysis`）。
const COMMAND_COMPARE_TRADE: &str = "stock_compare_trade_with_analysis";
/// 批量板块联动扫描（宿主 `scan_sector_coherence`）：本体索引 + 逐票行情 + 纯函数扫描。
const COMMAND_SCAN_SECTOR_COHERENCE: &str = "stock_scan_sector_coherence";
/// 设置单只股票监控优先级（宿主 `set_quote_watch_priority`）—— **写命令**。
const COMMAND_SET_QUOTE_WATCH_PRIORITY: &str = "stock_set_quote_watch_priority";
// ── F1 余三条（#35 / #321）：宿主同名 Tauri 命令同批退役，走新 op ──
/// 列估值快照原始行（宿主 `list_financial_snapshots`）：读 `financial_snapshots` 表，
/// 与 `stock_get_financials`（vendor 财报）**不等价**，必须独立 op —— 读命令。
const COMMAND_LIST_FINANCIAL_SNAPSHOTS: &str = "stock_list_financial_snapshots";
/// 从东方财富 API 刷新交易日历（宿主 `refresh_trading_calendar`）：读命令；该报表
/// 2026-09-27 起已下线，op 保持与被退役宿主命令等价的失败行为，不修功能。
const COMMAND_REFRESH_TRADING_CALENDAR: &str = "stock_refresh_trading_calendar";
/// 触发每日快照采集（宿主 `sweep_daily_snapshots`）—— **写命令**（网络 + 快照落表）。
/// 插件只是触发口：执行体 `run_daily_snapshot_sweep` 留在宿主，长任务不进 worker 帧。
const COMMAND_SWEEP_DAILY_SNAPSHOTS: &str = "stock_sweep_daily_snapshots";
// ── F2 余四条（#35 / #322）：宿主同名 Tauri 命令同批退役，走 `trading.engine` 新 op ──
// 四 op 的分派臂按嵌 `{"request": {...}}` 层取参（与既有六 op 的平铺取参不同形）；
// 插件侧在 args 里自己嵌层，层内键 snake_case（形状权威在宿主 Input struct）。
/// 组合持仓限额（宿主 `get_position_limits`）：读全局面板变量，零入参 —— 读命令。
const COMMAND_GET_POSITION_LIMITS: &str = "stock_get_position_limits";
/// 更新单条持仓的数量与成本（宿主 `update_portfolio_holding`）—— **写命令**。
const COMMAND_UPDATE_PORTFOLIO_HOLDING: &str = "stock_update_portfolio_holding";
/// 批量导入 VLM 识别的持仓（宿主 `import_portfolio_from_vlm`）—— **写命令**
/// （`replaceExisting` 可选清空旧持仓，不可逆；权限闸在宿主分派层）。
const COMMAND_IMPORT_PORTFOLIO_FROM_VLM: &str = "stock_import_portfolio_from_vlm";
/// 组合压力测试（宿主 `run_portfolio_stress_test`）：N+1 网络取数在宿主跑，零入参 —— 读命令。
/// ⚠ op 名与命令名不同形：op 叫 `run_stress_test`（trading.engine trait 方法名）。
const COMMAND_RUN_PORTFOLIO_STRESS_TEST: &str = "stock_run_portfolio_stress_test";
// ── F6 别名族（#35 / #322）：宿主 Tauri 命令同批退役；别名形状逐字保留（PLAN §174
// 五处不等价）。泛化形邻居（`evolve_node_command` / `save_node_feedback` /
// `get_node_feedbacks` / `get_node_feedback_stats`）**不退役** —— 前端直调。
/// 触发分析师节点自我进化（别名：硬编码 NodeType::Analyst）—— **写命令**（改 prompt/配置）。
/// ⚠ 宿主命令名带 `_command` 后缀，插件命名 = 全名拼接（#273 裁定）。
const COMMAND_EVOLVE_ANALYST: &str = "stock_evolve_analyst_command";
/// 分析师节点进化状态查询（别名，不触发进化）—— 读命令。
const COMMAND_GET_ANALYST_EVOLUTION_STATUS: &str = "stock_get_analyst_evolution_status_command";
/// 通用节点进化状态查询（泛化形：nodeType + nodeId）—— 读命令。
const COMMAND_GET_NODE_EVOLUTION_STATUS: &str = "stock_get_node_evolution_status_command";
/// 保存分析师反馈（别名：13 字段无 node_type，宿主按 Analyst 补）—— **写命令**。
const COMMAND_SAVE_ANALYST_FEEDBACK: &str = "stock_save_analyst_feedback";
/// 分析师反馈历史（别名：单键 analystId 必填）—— 读命令。
const COMMAND_GET_ANALYST_FEEDBACKS: &str = "stock_get_analyst_feedbacks";
/// 分析师反馈统计（别名：含 needs_evolution 判定）—— 读命令。
const COMMAND_GET_ANALYST_FEEDBACK_STATS: &str = "stock_get_analyst_feedback_stats";
// ── #323 T2 五命令批迁（子批②③④）：宿主同名 Tauri 命令同批退役 ──
/// 保存 NeoData token（宿主 `save_neodata_token`）：写脚本缓存 + 共享内存 + 模板变量
/// —— **写命令**（三段持久效应，权限闸在宿主分派层）。
const COMMAND_SAVE_NEODATA_TOKEN: &str = "stock_save_neodata_token";
/// 查询决策回测分析（宿主 `query_decision_backtest`，stock_workflow/misc.rs）—— 读命令。
const COMMAND_QUERY_DECISION_BACKTEST: &str = "stock_query_decision_backtest";
/// 运行组合回测——配置校验面（宿主 `portfolio_backtest_run`，走 `trading.engine`
/// `validate_portfolio_config` op；命令名逐字续宿主名，op 名与命令名不同形照 #322
/// `run_stress_test` 先例）—— 读命令（纯计算，无 DB/行情副作用）。
const COMMAND_PORTFOLIO_BACKTEST_RUN: &str = "stock_portfolio_backtest_run";
// ── #324 T2 F7 回测验证三条迁：宿主同名命令同批退役，走 `stock.analysis` 新 op ──
/// 跑决策回测（宿主 `run_decision_backtest`，backtest_validation.rs）—— **写命令**
/// （写 `decision_validations` 表；执行体留宿主，cron 与 CLI 共用）。
const COMMAND_RUN_DECISION_BACKTEST: &str = "stock_run_decision_backtest";
/// 列出决策验证记录（宿主 `list_decision_validations`）—— 读命令。
const COMMAND_LIST_DECISION_VALIDATIONS: &str = "stock_list_decision_validations";
/// 计算验证报告（宿主 `compute_validation_report`）—— 读命令。
const COMMAND_COMPUTE_VALIDATION_REPORT: &str = "stock_compute_validation_report";
// ── #326 T2 F3 首刀：分析与推荐产出四条迁，零新 op ──
/// 分析情绪深度（宿主 `analyze_sentiment_depth`）—— **纯本地计算**
/// （analysis-engine 直调，无接缝；历史快照经命令面 `historyJson` 预序列化传入）。
/// 宿主档 Caution ⇒ 清单 `workspace-write`。
const COMMAND_ANALYZE_SENTIMENT_DEPTH: &str = "stock_analyze_sentiment_depth";
/// 获取因子分析结果（宿主 `factor_analysis_list`）—— **纯本地计算**，零参数，读命令。
const COMMAND_FACTOR_ANALYSIS_LIST: &str = "stock_factor_analysis_list";
/// 生成基本面分析报告（宿主 `generate_fundamentals_report`）—— 读命令：行情/财务经
/// `market.quote::get_quote` + `get_financials`（现成 op），报告生成本地
/// `FundamentalsAnalyzer::generate`（astock-data 纯函数）。
const COMMAND_GENERATE_FUNDAMENTALS_REPORT: &str = "stock_generate_fundamentals_report";
/// 仅获取基本面报告 Markdown（宿主 `get_fundamentals_report_markdown`，同型轻量变体，
/// §183 取证补漏的 44 档漏项）—— 读命令。
const COMMAND_GET_FUNDAMENTALS_REPORT_MARKDOWN: &str = "stock_get_fundamentals_report_markdown";
/// 行业成分股查询命令（工具 `get_industry_constituents` 的执行面）。
const COMMAND_INDUSTRY_CONSTITUENTS: &str = "stock_get_industry_constituents";
/// 市场主线批量 upsert 命令（工具 `market_mainline_batch_upsert` 的执行面）—— **写命令**。
const COMMAND_MAINLINE_BATCH_UPSERT: &str = "stock_market_mainline_batch_upsert";
/// 最近 N 天市场主线列表（宿主 `market_mainline_list_recent` 退役后的取数面，#304 C3，
/// 前端 `MarketMainlinePanel` 消费）—— 读命令。
const COMMAND_MAINLINE_LIST_RECENT: &str = "stock_market_mainline_list_recent";
/// 逐票 1 月涨跌幅命令（工具 `get_stock_factor` 的执行面）。
const COMMAND_STOCK_FACTOR: &str = "stock_get_stock_factor";
// ── T2 试点批（#248）：四条宿主命令改走本接缝（宿主同名 Tauri 命令待删）──
// 与 E 批三条同形态：`Value` 整块透传，插件侧不裁剪不改名（形状权威在主壳 commands 层）。
/// 管道执行历史（宿主 `get_pipeline_history`，前端 `pipelineStore.ts:27`）。
const COMMAND_PIPELINE_HISTORY: &str = "stock_get_pipeline_history";
/// 单次管道执行详情（宿主 `get_pipeline_run_detail`，前端 `pipelineStore.ts:52`）。
const COMMAND_PIPELINE_RUN_DETAIL: &str = "stock_get_pipeline_run_detail";
// ── #328：两条 DB 直读宿主命令迁 T2（新增 `stock.analysis` 读 op）──
/// 个股最近一次完整分析摘要（宿主 `get_latest_analysis_for_stock`）。
const COMMAND_LATEST_ANALYSIS_FOR_STOCK: &str = "stock_get_latest_analysis_for_stock";
/// 参数调整建议列表（宿主 `list_param_suggestions`）。
const COMMAND_LIST_PARAM_SUGGESTIONS: &str = "stock_list_param_suggestions";
// ── #329：F3 余三条迁 T2（两条写 op 走 `stock.analysis` 新 op；discover 零新 op 编排）──
/// 热门候选发现（宿主 `discover_stock_candidates`）：宇宙两现成 op + 逐票
/// `market.quote::get_quote` + 引擎纯函数 `StockScreener::score_candidate` 打分
/// （打分单一权威；编排判据见 §183 表）。
const COMMAND_DISCOVER_STOCK_CANDIDATES: &str = "stock_discover_stock_candidates";
/// 失效荐股缓存（宿主 `invalidate_recommendation_cache`）：写 op —— 必须落在宿主进程，
/// worker 直调只会清掉插件进程副本（§183 表的裁定依据）。
const COMMAND_INVALIDATE_RECOMMENDATION_CACHE: &str = "stock_invalidate_recommendation_cache";
/// 自改进分析循环（宿主 `run_self_improving_stock_analysis`）：写 op —— 执行体在
/// 宿主 agent crate（consumer 层，插件不可链接）。
const COMMAND_RUN_SELF_IMPROVING: &str = "stock_run_self_improving_stock_analysis";
/// 批量个股最近分析摘要（宿主 `get_latest_analyses_for_stocks`，#330）—— 读 op，
/// 宿主 core 逐票复用单数版 SQL 判据（两版口径只有一处）。
const COMMAND_LATEST_ANALYSES_FOR_STOCKS: &str = "stock_get_latest_analyses_for_stocks";
/// 权重时间线（宿主 `get_evolution_drift_timeline`，#331）—— 读 op。
const COMMAND_EVOLUTION_DRIFT_TIMELINE: &str = "stock_get_evolution_drift_timeline";
/// 决策一致性分数历史（宿主 `get_agreement_score_history`，#331）—— 读 op。
const COMMAND_AGREEMENT_SCORE_HISTORY: &str = "stock_get_agreement_score_history";
/// 节点反馈历史（通用版，宿主 `get_node_feedbacks`，#332）—— 读 op。
const COMMAND_NODE_FEEDBACKS: &str = "stock_get_node_feedbacks";
/// 节点反馈统计（通用版，宿主 `get_node_feedback_stats`，#332）—— 读 op。
const COMMAND_NODE_FEEDBACK_STATS: &str = "stock_get_node_feedback_stats";
/// 月度投资报告（宿主 `generate_monthly_report`，#333）—— 读 op。
const COMMAND_MONTHLY_REPORT: &str = "stock_generate_monthly_report";
/// 宏观经济数据快照（宿主 `macro_data_snapshot`，#333）—— 读 op（每 op 新增于宿主 provider）。
const COMMAND_MACRO_DATA_SNAPSHOT: &str = "stock_macro_data_snapshot";
/// 漏检回灌清单与转正就绪度（宿主 `list_mover_reflow_candidates`，#334）—— 读 op。
const COMMAND_MOVER_REFLOW_CANDIDATES: &str = "stock_list_mover_reflow_candidates";
/// 回测反馈分析（宿主 `analyze_backtest_feedback`，#335）—— 计算型 op（宿主原档 Caution）。
const COMMAND_ANALYZE_BACKTEST_FEEDBACK: &str = "stock_analyze_backtest_feedback";
/// VLM 持仓截图解析（宿主 `parse_vlm_portfolio_screenshot`，#335）—— 读 op（纯解析）。
const COMMAND_PARSE_VLM_PORTFOLIO_SCREENSHOT: &str = "stock_parse_vlm_portfolio_screenshot";
/// 回测运行记录读取（宿主 `quant_run_get`，#336）—— 读 op。
const COMMAND_QUANT_RUN_GET: &str = "stock_quant_run_get";
/// 量化策略列表（宿主 `quant_strategies_list`，#336）—— 读 op。
const COMMAND_QUANT_STRATEGIES_LIST: &str = "stock_quant_strategies_list";
/// 多 run 指标对比（宿主 `quant_metrics_compare`，#336）—— 读 op。
const COMMAND_QUANT_METRICS_COMPARE: &str = "stock_quant_metrics_compare";
/// 注册 Rhai 策略（宿主 `quant_strategy_register_rhai`，#336）—— 写 op（insert/upsert
/// `quant_strategies` 表，op 级要求包有 write）。宿主只留 `quant_backtest_run`：分钟级
/// 长跑，命令帧 30s / 接缝等待 15s 双预算都装不下（见宿主模块注释）。
const COMMAND_QUANT_STRATEGY_REGISTER_RHAI: &str = "stock_quant_strategy_register_rhai";
/// 手动设置 vendor 健康状态（宿主 `set_vendor_status`，#337）—— 写语义 op。
const COMMAND_SET_VENDOR_STATUS: &str = "stock_set_vendor_status";
/// 取消进行中的分析任务（宿主 `cancel_stock_analysis`，#337）—— 写语义 op。
const COMMAND_CANCEL_STOCK_ANALYSIS: &str = "stock_cancel_stock_analysis";
/// 待审批进化计划列表（宿主 `list_pending_evolution_plans`）。
const COMMAND_PENDING_EVOLUTION_PLANS: &str = "stock_list_pending_evolution_plans";
/// Walk-Forward + DES 对比仿真（宿主 `wf_des_integration`，前端 `WfDesTab.tsx:187`）。
const COMMAND_DES_INTEGRATION: &str = "stock_wf_des_integration";
// ── 批 1a（#35）：十二股票 cron 配置命令 ──
// 与试点批同形态：入参是宿主命令参数的 camelCase 平铺 JSON，整块透传给
// `stock.analysis` 接缝；配置归一 / desc 文案 / uuid / 响应形状的权威在主壳
// `commands/recommendation_cron.rs` / `commands/pool_scan.rs` 的无 State 核心里。
/// 创建荐股定时任务（宿主 `create_recommendation_cron`）。
const COMMAND_CREATE_RECOMMENDATION_CRON: &str = "stock_create_recommendation_cron";
/// 列出荐股定时任务（宿主 `list_recommendation_crons`）—— 读 op，无权限闸。
const COMMAND_LIST_RECOMMENDATION_CRONS: &str = "stock_list_recommendation_crons";
/// 启停荐股定时任务（宿主 `toggle_recommendation_cron`）。
const COMMAND_TOGGLE_RECOMMENDATION_CRON: &str = "stock_toggle_recommendation_cron";
/// 删除荐股定时任务（宿主 `delete_recommendation_cron`）。
const COMMAND_DELETE_RECOMMENDATION_CRON: &str = "stock_delete_recommendation_cron";
/// 创建趋势智选定任务（宿主 `create_trend_screening_cron`）。
const COMMAND_CREATE_TREND_SCREENING_CRON: &str = "stock_create_trend_screening_cron";
/// 列出趋势智选定任务（宿主 `list_trend_screening_crons`）—— 读 op。
const COMMAND_LIST_TREND_SCREENING_CRONS: &str = "stock_list_trend_screening_crons";
/// 启停趋势智选定任务（宿主 `toggle_trend_screening_cron`）。
const COMMAND_TOGGLE_TREND_SCREENING_CRON: &str = "stock_toggle_trend_screening_cron";
/// 删除趋势智选定任务（宿主 `delete_trend_screening_cron`）。
const COMMAND_DELETE_TREND_SCREENING_CRON: &str = "stock_delete_trend_screening_cron";
/// 创建候选池扫描定时任务（宿主 `create_pool_scan_cron`）。
const COMMAND_CREATE_POOL_SCAN_CRON: &str = "stock_create_pool_scan_cron";
/// 列出候选池扫描定时任务（宿主 `list_pool_scan_crons`）—— 读 op。
const COMMAND_LIST_POOL_SCAN_CRONS: &str = "stock_list_pool_scan_crons";
/// 启停候选池扫描定时任务（宿主 `toggle_pool_scan_cron`）。
const COMMAND_TOGGLE_POOL_SCAN_CRON: &str = "stock_toggle_pool_scan_cron";
/// 删除候选池扫描定时任务（宿主 `delete_pool_scan_cron`）。
const COMMAND_DELETE_POOL_SCAN_CRON: &str = "stock_delete_pool_scan_cron";
// ── 批 2a（#35）：十一条行情只读命令，一律经宿主 `market.quote` 接缝 ──
// 宿主同名命令（`get_stock_*`）全部 `safety = Safe`，插件清单不声明 requiredPermission
// ⇒ 两档逐条等阶。命令面入参是 camelCase（与前端的 `{ stockCode }` 调用点一致，禁区 13），
// 接缝面键名由路由表翻成 snake_case（宿主 `worker.rs` 的既有协议，不改）。
/// 获取研究报告（宿主 `get_stock_research_reports`）。
const COMMAND_GET_RESEARCH_REPORTS: &str = "stock_get_research_reports";
/// 获取概念板块（宿主 `get_stock_concept_blocks`）。
const COMMAND_GET_CONCEPT_BLOCKS: &str = "stock_get_concept_blocks";
/// 获取公司公告（宿主 `get_stock_announcements`）。
const COMMAND_GET_ANNOUNCEMENTS: &str = "stock_get_announcements";
/// 获取财报披露日历（宿主 `get_earnings_calendar`）。
const COMMAND_GET_EARNINGS_CALENDAR: &str = "stock_get_earnings_calendar";
/// 获取板块成分股（宿主 `get_concept_board_members`，入参是板块代码而非股票代码）。
const COMMAND_GET_CONCEPT_BOARD_MEMBERS: &str = "stock_get_concept_board_members";
/// 获取财联社快讯（宿主 `get_cls_flash`，全局快讯，不吃标的代码）。
const COMMAND_GET_CLS_FLASH: &str = "stock_get_cls_flash";
/// 获取同行业对比（宿主 `get_stock_peers`）。
const COMMAND_GET_PEERS: &str = "stock_get_peers";
/// 获取期权 PCR（宿主 `get_stock_option_pcr`）。
const COMMAND_GET_OPTION_PCR: &str = "stock_get_option_pcr";
/// 获取限售解禁时间表（宿主 `get_lockup_schedule`）。
const COMMAND_GET_LOCKUP_SCHEDULE: &str = "stock_get_lockup_schedule";
/// 获取分红记录（宿主 `get_dividend_records`）。
const COMMAND_GET_DIVIDEND_RECORDS: &str = "stock_get_dividend_records";
/// 获取股票财务数据（宿主 `get_stock_financials`，对比面板使用）。
const COMMAND_GET_FINANCIALS: &str = "stock_get_financials";
// ── 批 2c（#262）：五条全市场榜单/盘面只读命令，全部零入参，同走 `market.quote` 接缝 ──
// 与 `get_cls_flash` 同形（`keys: None`）：榜单是全局数据，不吃标的代码；
// 凭证 / 限流 / provider 选择全在宿主接缝侧，插件只传 op。
/// 获取热门股票榜（宿主 `get_hot_stocks`，零入参）。
const COMMAND_GET_HOT_STOCKS: &str = "stock_get_hot_stocks";
/// 获取行业排名榜（宿主 `get_industry_ranking`，零入参）。
const COMMAND_GET_INDUSTRY_RANKING: &str = "stock_get_industry_ranking";
/// 获取指数行情（宿主 `get_index_quotes`，零入参；⚠ 接缝 op 名与命令名**不同源**，
/// 是 `get_index_quotes_outcome` —— 宿主命令体调的 trait 方法就带 `outcome` 后缀，见路由表）。
const COMMAND_GET_INDEX_QUOTES: &str = "stock_get_index_quotes";
/// 获取全市场龙虎榜（宿主 `get_market_dragon_tiger`，零入参）。
const COMMAND_GET_MARKET_DRAGON_TIGER: &str = "stock_get_market_dragon_tiger";
/// 获取北向资金（宿主 `get_north_bound_flow`，零入参）。
const COMMAND_GET_NORTH_BOUND_FLOW: &str = "stock_get_north_bound_flow";
// ── 候选 A（#35 行情资讯直映批）：十条宿主只读命令迁 T2，全部经 market.quote ──
// 入参形态（多键 / 可选键 / 无复权参）由路由表 `optional` 档、`kline_route` 表与
// `fetch_search_news` 分摊承接；`search_stock` 的宿主 `market` 过滤参不在接缝 op 上
// ⇒ 插件面不开放（与接缝能力对齐，不承诺一条取不到数的通道）。
/// 搜索股票（宿主 `search_stock`，keyword 必填）。
const COMMAND_SEARCH_STOCK: &str = "stock_search_stock";
/// 搜索财经新闻（宿主 `search_news`，keyword 必填 + limit 可选缺省 20 上限 100）。
const COMMAND_SEARCH_NEWS: &str = "stock_search_news";
/// 搜索概念板块（宿主 `search_concept_boards`，keyword 必填）。
const COMMAND_SEARCH_CONCEPT_BOARDS: &str = "stock_search_concept_boards";
/// 获取社交舆情（宿主 `get_social_sentiment`，stockCode 必填）。
const COMMAND_GET_SOCIAL_SENTIMENT: &str = "stock_get_social_sentiment";
/// 获取涨停池（宿主 `get_limit_up_pool`，date 可选；接缝参数名是 `requested_date`）。
const COMMAND_GET_LIMIT_UP_POOL: &str = "stock_get_limit_up_pool";
/// 获取国际个股实时行情（宿主 `get_international_stock_quote`，stockCode 必填）。
const COMMAND_GET_INTERNATIONAL_STOCK_QUOTE: &str = "stock_get_international_stock_quote";
/// 获取国际个股 K 线（宿主 `get_international_stock_kline`；宿主 adj 固定未复权 ⇒ 不收 adjType）。
const COMMAND_GET_INTERNATIONAL_STOCK_KLINE: &str = "stock_get_international_stock_kline";
/// 获取外汇 K 线（宿主 `get_forex_kline`，pair + period + limit）。
const COMMAND_GET_FOREX_KLINE: &str = "stock_get_forex_kline";
/// 获取基准指数 K 线（宿主 `get_benchmark_kline`，benchmarkCode + period + limit）。
const COMMAND_GET_BENCHMARK_KLINE: &str = "stock_get_benchmark_kline";
/// 获取一致预期 EPS（宿主 `get_stock_consensus_eps`，stockCode 必填）。
const COMMAND_GET_STOCK_CONSENSUS_EPS: &str = "stock_get_consensus_eps";
/// 插件贡献的命名命令（manifest `commands[].name` / `taskSources[].command` 必须逐字一致）。
const COMMAND_RECENT_RUNS: &str = "list_recent_workflow_runs";
/// 单股分析记录取数命令（同上：清单声明留给后续批，本步只保证通道可跑）。
const COMMAND_ANALYSIS_RECORDS: &str = "list_stock_analysis_records";
/// 自选行情列表命令（宿主 `list_watched_quotes` 迁移退役）—— 出参为 code 数组。
const COMMAND_LIST_WATCHED_QUOTES: &str = "stock_list_watched_quotes";
/// 逐只并发批量实时行情命令（宿主 `batch_get_quotes` 迁移退役）—— 出参 BatchResult 双 map。
const COMMAND_BATCH_GET_QUOTES: &str = "stock_batch_get_quotes";
/// 逐只并发批量财务报告命令（宿主 `batch_get_financials` 迁移退役）—— 出参 BatchResult 双 map。
const COMMAND_BATCH_GET_FINANCIALS: &str = "stock_batch_get_financials";
/// 一次任务源取数的默认上限：与宿主 `list_recent` 的缺省值一致，
/// 前端泛型适配器不传 limit 时也走这一档（不是插件自己拍的第二个数）。
const DEFAULT_TASK_SOURCE_LIMIT: u32 = 50;

fn main() -> ExitCode {
    match serve(handle) {
        // 对端关管道 = 宿主停机，正常退出（见 `Peer::run`：EOF 不是错误）。
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("[axagent-plugin-stock] 帧通道失败：{e}");
            ExitCode::FAILURE
        },
    }
}

fn handle(
    request: &FrameRequest,
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> FrameResponse {
    match request.op.as_str() {
        ops::DESCRIBE => declaration(),
        ops::SHUTDOWN => FrameResponse::success(json!({ "ok": true })),
        // `tool.set` 的发现面：宿主要在注册时一次性取回工具元数据（`Tool::name()` 是同步借用）。
        "list" => FrameResponse::success(tools()),
        // `tool.set` 的执行面：宿主发 `{name, args}`，执行结果放 `content`（字符串）。
        "exec" => exec_tool(&request.args, &request.call_chain, peer),
        // T2 命令面：宿主发 `CommandFrame`（`pluginId` 绑定 + 命令名 + 入参 + 预算）。
        ops::COMMAND => handle_command(request, peer),
        other => FrameResponse::error(
            error_codes::PLUGIN_UNKNOWN_OP,
            format!("股票插件未实现该操作：{other}"),
        ),
    }
}

/// 处理一条命令帧：身份自检 → 按命令名分发。
///
/// 身份自检是**第二道防线**（第一道在宿主 `WorkerInvoker::execute_command` 写帧前）：
/// 帧上的 `plugin_id` 必须等于 spawn 时注入的 `AXAGENT_PLUGIN_ID`，防宿主侧错配。
fn handle_command(
    request: &FrameRequest,
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> FrameResponse {
    let frame = match CommandFrame::from_frame_request(request) {
        Ok(frame) => frame,
        Err(e) => {
            return FrameResponse::error(
                error_codes::PLUGIN_COMMAND_INVALID_ARGS,
                format!("命令帧解码失败：{e}"),
            );
        },
    };
    let own_id = std::env::var("AXAGENT_PLUGIN_ID").unwrap_or_default();
    if frame.plugin_id != own_id {
        return CommandFault::for_frame(
            &frame,
            error_codes::PLUGIN_COMMAND_ID_MISMATCH,
            format!("帧绑定 `{}` 与本 worker 身份 `{own_id}` 不符，拒绝执行", frame.plugin_id),
        )
        .into_frame_response();
    }

    let outcome = match frame.command.as_str() {
        // 新闻搜索是多键 op（keyword 必填 + limit 可选缺省），套不进下方单键路由表，单独一臂。
        // `stock_get_kline`（候选 A 起）与其余三条 K 线族命令同走 `kline_route` 表
        // （K 线四键入参与单标的路由表不同形），不再独立成臂。
        COMMAND_SEARCH_NEWS => match fetch_search_news(&frame.input, &request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // 纯函数命令：不查库、不调接缝，跑的是 `axagent-stock-core` 的权威实现。
        COMMAND_PARSE_INTENT => match parse_intent(&frame.input) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_EVIDENCE_WEIGHTS => match evidence_weights(&frame.input) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // 任务源取数：行数据来自宿主 `workflow.run` 的 `list_recent`（在跑 + 已结束），
        // 插件只做状态词表翻译与形状收敛（渲染与 run id 拼装归宿主泛型适配器，见 PLAN §97.4）。
        COMMAND_RECENT_RUNS => {
            let rows = recent_workflow_runs(&frame.input, &request.call_chain, peer);
            match rows {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // 单股分析记录取数：行数据来自宿主 `analysis.record` 的 `list`（宿主发起的记录也在内）。
        // 只做「状态词表翻译 + 形状收敛」，行字段与过滤判据见 `stock_analysis_records`。
        COMMAND_ANALYSIS_RECORDS => {
            let rows = stock_analysis_records(&frame.input, &request.call_chain, peer);
            match rows {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // ── 宿主命令迁移批：三条只读命令（list_watched / 批量行情 / 批量财务）──
        // `stock_list_watched_quotes` 走 `quote.watcher` 的 `list_watched`（具名对象解包）；
        // 两条批量命令走 `market.quote` 新增的批量 op（多键入参，仿 `COMMAND_SEARCH_NEWS` 单独成臂）。
        COMMAND_LIST_WATCHED_QUOTES => match fetch_watched_quotes(&request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_BATCH_GET_QUOTES => match batch_quotes_seam_args(&frame.input) {
            Ok(args) => {
                match fetch_market_batch("batch_get_quotes", args, &request.call_chain, peer) {
                    Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                    Err((code, message)) => {
                        CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                    },
                }
            },
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_BATCH_GET_FINANCIALS => match batch_financials_seam_args(&frame.input) {
            Ok(args) => {
                match fetch_market_batch("batch_get_financials", args, &request.call_chain, peer) {
                    Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                    Err((code, message)) => {
                        CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                    },
                }
            },
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // ── E 批（#240）：股票域三条数据面命令，取数一律经宿主 `stock.analysis` 接缝 ──
        // 三个工具（`get_industry_constituents` / `market_mainline_batch_upsert` /
        // `get_stock_factor`）只发命令，**不再直连 analysis-engine**（跨包口归零）。
        // 入参出参都是 `Value` 整块透传：字段形状的权威在 `analysis-engine` 的 DTO，
        // 宿主 provider 是唯一的转换点；插件侧改写字段就是造第二套形状（禁区 12）。
        COMMAND_INDUSTRY_CONSTITUENTS => match call_analysis_seam(
            peer,
            &request.call_chain,
            "industry_constituents",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_STOCK_FACTOR => match call_analysis_seam(
            peer,
            &request.call_chain,
            "stock_factor",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // 唯一写命令：`write` 权限闸在宿主 `required_permission_for` 侧，本层不重复校验
        // （manifest 必须同批声明 `"write"`，否则宿主在收帧时自拒）。
        COMMAND_MAINLINE_BATCH_UPSERT => match call_analysis_seam(
            peer,
            &request.call_chain,
            "batch_upsert_mainlines",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // 读命令（#304 C3）：入参 `{days?}` 整块透传，缺省窗口与上界在宿主 provider 拍。
        COMMAND_MAINLINE_LIST_RECENT => match call_analysis_seam(
            peer,
            &request.call_chain,
            "list_recent_mainlines",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // ── T2 试点批（#248）：四条宿主命令的取数面，全只读/纯计算 ──
        // 与 E 批三条同一透传纪律：字段形状的权威在主壳 commands 层（pipeline 手拼行 /
        // dao 进化模型 / `commands/wf_des.rs` 的 WfDes DTO），插件侧改写即第二套形状。
        COMMAND_PIPELINE_HISTORY => match call_analysis_seam(
            peer,
            &request.call_chain,
            "pipeline_history",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_PIPELINE_RUN_DETAIL => match call_analysis_seam(
            peer,
            &request.call_chain,
            "pipeline_run_detail",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_LATEST_ANALYSIS_FOR_STOCK => match call_analysis_seam(
            peer,
            &request.call_chain,
            "latest_analysis_for_stock",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // #330：批量摘要（读 op；宿主 core 逐票复用单数版判据）。
        COMMAND_LATEST_ANALYSES_FOR_STOCKS => match call_analysis_seam(
            peer,
            &request.call_chain,
            "latest_analyses_for_stocks",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // #331：两条读 op（权重时间线 / 一致性分数历史）。
        COMMAND_EVOLUTION_DRIFT_TIMELINE => match call_analysis_seam(
            peer,
            &request.call_chain,
            "evolution_drift_timeline",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_AGREEMENT_SCORE_HISTORY => match call_analysis_seam(
            peer,
            &request.call_chain,
            "agreement_score_history",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // #332：节点反馈通用版两条读 op。
        COMMAND_NODE_FEEDBACKS => match call_analysis_seam(
            peer,
            &request.call_chain,
            "node_feedbacks",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_NODE_FEEDBACK_STATS => match call_analysis_seam(
            peer,
            &request.call_chain,
            "node_feedback_stats",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // #333：月度报告（读 op）+ 宏观快照（零新 op 直调）。
        COMMAND_MONTHLY_REPORT => match call_analysis_seam(
            peer,
            &request.call_chain,
            "monthly_report",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_MACRO_DATA_SNAPSHOT => match call_analysis_seam(
            peer,
            &request.call_chain,
            "macro_data_snapshot",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // #334：漏检回灌清单（读 op，零入参）。
        COMMAND_MOVER_REFLOW_CANDIDATES => match call_analysis_seam(
            peer,
            &request.call_chain,
            "mover_reflow_candidates",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // #335：回测反馈分析（计算型 op，宿主原档 Caution）。
        COMMAND_ANALYZE_BACKTEST_FEEDBACK => match call_analysis_seam(
            peer,
            &request.call_chain,
            "analyze_backtest_feedback",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // #335：VLM 持仓截图解析（读 op，纯解析）。
        COMMAND_PARSE_VLM_PORTFOLIO_SCREENSHOT => match call_analysis_seam(
            peer,
            &request.call_chain,
            "parse_vlm_portfolio_screenshot",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // #336：quant 四条（3 读 + register_rhai 写，整块透传 camelCase 平铺 JSON）。
        COMMAND_QUANT_RUN_GET => match call_analysis_seam(
            peer,
            &request.call_chain,
            "quant_run_get",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_QUANT_STRATEGIES_LIST => match call_analysis_seam(
            peer,
            &request.call_chain,
            "quant_strategies_list",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_QUANT_METRICS_COMPARE => match call_analysis_seam(
            peer,
            &request.call_chain,
            "quant_metrics_compare",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_QUANT_STRATEGY_REGISTER_RHAI => match call_analysis_seam(
            peer,
            &request.call_chain,
            "quant_strategy_register_rhai",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // #337：两条写语义 op（vendor 状态 / 取消令牌，均走 stock.analysis）。
        COMMAND_SET_VENDOR_STATUS => match call_analysis_seam(
            peer,
            &request.call_chain,
            "set_vendor_status",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_CANCEL_STOCK_ANALYSIS => match call_analysis_seam(
            peer,
            &request.call_chain,
            "cancel_stock_analysis",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_LIST_PARAM_SUGGESTIONS => match call_analysis_seam(
            peer,
            &request.call_chain,
            "list_param_suggestions",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_PENDING_EVOLUTION_PLANS => match call_analysis_seam(
            peer,
            &request.call_chain,
            "pending_evolution_plans",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_DES_INTEGRATION => match call_analysis_seam(
            peer,
            &request.call_chain,
            "des_integration",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        // ── #311：四条零新 op 的纯智能体工具（宿主同名命令同批退役）──
        // 两条走 `trading.engine` 读 op、一条走 `market.quote` + 本地纯函数、
        // 一条走 `quote.watcher` 的写 op。三处可观察行为差见各 helper 的注释。
        COMMAND_GET_PORTFOLIO_RISK => match portfolio_risk(&request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_COMPARE_TRADE => {
            match compare_trade_with_analysis(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_SCAN_SECTOR_COHERENCE => {
            match scan_sector_coherence(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // ── #329：F3 余三条（discover 零新 op 编排；两条写 op 走 stock.analysis 新 op）──
        COMMAND_DISCOVER_STOCK_CANDIDATES => {
            match discover_stock_candidates(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_INVALIDATE_RECOMMENDATION_CACHE => match call_analysis_seam(
            peer,
            &request.call_chain,
            "invalidate_recommendation_cache",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_RUN_SELF_IMPROVING => match call_analysis_seam(
            peer,
            &request.call_chain,
            "run_self_improving_stock_analysis",
            stock_analysis_request(&frame.input),
        ) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_SET_QUOTE_WATCH_PRIORITY => {
            match set_quote_watch_priority(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // ── F1 余三条（#35 / #321）：显式臂两条 + 路由表一条 ──
        // `stock_list_financial_snapshots` / `stock_sweep_daily_snapshots` 走
        // `stock.analysis` 新 op（快照读面 / 采集触发口）；`stock_refresh_trading_calendar`
        // 零入参，与批 2c 同形态进 `market_read_route` 表（`market.quote` 新 op）。
        COMMAND_LIST_FINANCIAL_SNAPSHOTS => {
            match fetch_financial_snapshots(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_SWEEP_DAILY_SNAPSHOTS => {
            match call_analysis_seam(peer, &request.call_chain, "sweep_daily_snapshots", json!({}))
            {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // ── F2 余四条（#35 / #322）：宿主同名 Tauri 命令同批退役，走 trading.engine 新 op ──
        // 两条零入参读命令（限额 / 压测）不收 input；两条写命令平铺 camelCase → 层内
        // snake_case（形状权威在宿主 Input struct，见函数体注释）。
        COMMAND_GET_POSITION_LIMITS => match fetch_position_limits(&request.call_chain, peer) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_UPDATE_PORTFOLIO_HOLDING => {
            match update_portfolio_holding(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_IMPORT_PORTFOLIO_FROM_VLM => {
            match import_portfolio_from_vlm(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_RUN_PORTFOLIO_STRESS_TEST => {
            match run_portfolio_stress_test(&request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // ── F6 别名族（#35 / #322）：六条显式臂，别名形状逐字保留（PLAN §174）──
        // 写 op 两条（evolve_analyst / save_analyst_feedback）的权限闸由宿主分派层
        // required_permission_for 把关；四条读 op 显式 read-only。
        COMMAND_EVOLVE_ANALYST => {
            match evolve_analyst_via_seam(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_GET_ANALYST_EVOLUTION_STATUS => {
            match fetch_analyst_evolution_status(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_GET_NODE_EVOLUTION_STATUS => {
            match fetch_node_evolution_status(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_SAVE_ANALYST_FEEDBACK => {
            match save_analyst_feedback(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_GET_ANALYST_FEEDBACKS => {
            match fetch_analyst_feedbacks(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_GET_ANALYST_FEEDBACK_STATS => {
            match fetch_analyst_feedback_stats(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // ── #323 T2 五命令批迁（子批②③④）：显式臂三条 ──
        // 写命令一条（save_neodata_token，权限闸由宿主分派层 required_permission_for
        // 把关）；读命令两条（query_decision_backtest / portfolio_backtest_run 的
        // 配置校验面）。三条实现体都在宿主进程内（token 缓存 / stock_analyses 表 /
        // quant 配置解析），插件只透传。
        COMMAND_SAVE_NEODATA_TOKEN => {
            match save_neodata_token(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_QUERY_DECISION_BACKTEST => {
            match query_decision_backtest(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_PORTFOLIO_BACKTEST_RUN => {
            match portfolio_backtest_run(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // ── #324 T2 F7 回测验证三条迁：显式臂三条 ──
        // 写命令一条（run_decision_backtest，权限闸由宿主分派层 required_permission_for
        // 把关）；读命令两条（list_decision_validations / compute_validation_report）。
        // 三条实现体都在宿主进程内（decision_validations / reco_picks 表 + 行情 API），
        // 插件只透传。
        COMMAND_RUN_DECISION_BACKTEST => {
            match run_decision_backtest(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_LIST_DECISION_VALIDATIONS => {
            match list_decision_validations(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_COMPUTE_VALIDATION_REPORT => {
            match compute_validation_report(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // ── #326 F3 首刀：分析与推荐产出四条，零新 op ──
        // 前两条纯本地（analysis-engine 直调）；后两条行情/财务走 market.quote 现成 op，
        // 报告生成本地 astock-data 纯函数。四条权限面：sentiment 宿主档 Caution ⇒ 清单
        // workspace-write；其余三条读命令 ⇒ read-only。
        COMMAND_ANALYZE_SENTIMENT_DEPTH => match analyze_sentiment_depth(&frame.input) {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_FACTOR_ANALYSIS_LIST => match factor_analysis_list() {
            Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        COMMAND_GENERATE_FUNDAMENTALS_REPORT => {
            match generate_fundamentals_report(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        COMMAND_GET_FUNDAMENTALS_REPORT_MARKDOWN => {
            match get_fundamentals_report_markdown(&frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            }
        },
        // ── 批 1a（#35）：十二股票 cron 配置命令，一律经 `stock.analysis` 接缝 ──
        // 三条 create 回建好的任务、九条管理面回 `null`（与宿主命令的 `()` 一致）；
        // 插件只透传入参，配置归一/响应形状的权威在宿主无 State 核心里。
        // 十二段转发用表驱动（`cron_seam_op`）而非逐条复制 —— 每个臂的形态完全一致。
        other => match kline_route(other) {
            // K 线族四条（批 2b A 股 + 候选 A 国际/外汇/基准）：多键入参，独立路由表。
            Some(route) => match fetch_kline(route, &frame.input, &request.call_chain, peer) {
                Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                Err((code, message)) => {
                    CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                },
            },
            None => match market_read_route(other) {
                // 批 2a（#35）十一条 + 批 2b quote + 批 2c（#262）五条全市场榜单/盘面命令 +
                // 候选 A 六条行情资讯直映命令，一律经宿主 `market.quote` 接缝，形态由路由表决定。
                Some(route) => {
                    match fetch_market_read(route, &frame.input, &request.call_chain, peer) {
                        Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                        Err((code, message)) => {
                            CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                        },
                    }
                },
                None => match cron_seam_op(other) {
                    Some(op) => match call_analysis_seam(
                        peer,
                        &request.call_chain,
                        op,
                        stock_analysis_request(&frame.input),
                    ) {
                        Ok(value) => CommandOutcome::Reply(CommandReply::for_frame(&frame, value)),
                        Err((code, message)) => {
                            CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
                        },
                    },
                    None => CommandOutcome::Fault(CommandFault::for_frame(
                        &frame,
                        error_codes::PLUGIN_COMMAND_UNKNOWN,
                        format!("股票插件没有命令 `{other}` 的实现"),
                    )),
                },
            },
        },
    };
    outcome
        .into_frame_response()
        .unwrap_or_else(|e| FrameResponse::error(error_codes::PLUGIN_COMMAND_FAILED, e.to_string()))
}

/// 握手声明：只声明**真正实现**的东西 —— 空能力会让宿主注册一条调不通的接缝。
///
/// `calls` 是出站准入（宿主 `handle_call_seam` 会校验本插件是否声明过目标接缝），
/// 少写一条 = 运行时被拒。
fn declaration() -> FrameResponse {
    FrameResponse::success(
        serde_json::to_value(PluginDeclaration {
            proto_version: AXAGENT_PLUGIN_PROTO_VERSION,
            subscribe: Vec::new(),
            calls: vec![
                MARKET_SEAM.to_string(),
                WORKFLOW_RUN_SEAM.to_string(),
                ANALYSIS_RECORD_SEAM.to_string(),
                STOCK_ANALYSIS_SEAM.to_string(),
                QUOTE_WATCHER_SEAM.to_string(),
                TRADING_ENGINE_SEAM.to_string(),
            ],
            capabilities: vec![CapabilityDecl {
                seam: "tool.set".to_string(),
                op: "exec".to_string(),
                version: TOOL_SET_VERSION.to_string(),
            }],
        })
        .expect("插件声明是可序列化的纯数据"),
    )
}

/// 工具清单（`tool.set` 的 `list` 回帧）。字段名是宿主 `RemoteToolDescriptor` 的 camelCase 形态。
fn tools() -> Value {
    json!([{
        "name": TOOL_NAME,
        "description": "获取 A 股个股实时行情（价格/涨跌/成交量等），行情由宿主行情接缝提供",
        "inputSchema": {
            "type": "object",
            "properties": {
                "stock_code": { "type": "string", "description": "6 位股票代码，如 600519" }
            },
            "required": ["stock_code"]
        },
        "category": "integration"
    }])
}

/// 执行一个命名工具。未知工具显式报错（不静默成功）；参数缺失按 `SEAM_CALL_INVALID_ARGS` 报。
fn exec_tool(
    args: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> FrameResponse {
    let name = args.get("name").and_then(Value::as_str).unwrap_or_default();
    if name != TOOL_NAME {
        return FrameResponse::error(
            error_codes::PLUGIN_CALL_FAILED,
            format!("股票插件的工具集中没有名为 `{name}` 的工具"),
        );
    }

    let input = args.get("args").cloned().unwrap_or(Value::Null);
    match fetch_quote(&input, call_chain, peer) {
        Ok((stock_code, quote)) => {
            // 工具出参按宿主 `ToolExecReply` 的约定：`content` 是字符串。
            FrameResponse::success(json!({
                "content": serde_json::to_string(&quote).unwrap_or_else(|_| quote.to_string()),
                "metadata": { "stock_code": stock_code }
            }))
        },
        Err((code, message)) => FrameResponse::error(code, message),
    }
}

/// 纯函数命令：自然语言 → 结构化意图（`ParsedIntent` 的 camelCase JSON）。
fn parse_intent(input: &Value) -> Result<Value, (String, String)> {
    let Some(text) = input.get("input").and_then(Value::as_str) else {
        return Err((
            error_codes::PLUGIN_COMMAND_INVALID_ARGS.to_string(),
            "`input` 必须是字符串".to_string(),
        ));
    };
    serde_json::to_value(axagent_stock_core::intent_parser::parse_analysis_intent(text)).map_err(
        |e| {
            (error_codes::PLUGIN_COMMAND_FAILED.to_string(), format!("意图解析结果序列化失败：{e}"))
        },
    )
}

/// 纯函数命令：`EvidenceWeightRequest`（camelCase）→ `EvidenceWeightReport`。
///
/// 入参解码严格：缺字段 / 类型不符报 `PLUGIN_COMMAND_INVALID_ARGS`，**不**用默认值兜底 ——
/// 权重算错的后果是决策方向被改写，静默降级比报错危险得多（同 §86 的 v133 观察）。
fn evidence_weights(input: &Value) -> Result<Value, (String, String)> {
    use axagent_analysis_engine::evidence_weight::{
        EvidenceWeightRequest, compute_evidence_weights,
    };
    let request: EvidenceWeightRequest = serde_json::from_value(input.clone()).map_err(|e| {
        (
            error_codes::PLUGIN_COMMAND_INVALID_ARGS.to_string(),
            format!("`EvidenceWeightRequest` 解码失败：{e}"),
        )
    })?;
    serde_json::to_value(compute_evidence_weights(request)).map_err(|e| {
        (error_codes::PLUGIN_COMMAND_FAILED.to_string(), format!("权重报告序列化失败：{e}"))
    })
}

/// 任务源取数命令：`PluginTaskRunRow[]` 的 JSON 数组（契约见 plugin-proto）。
///
/// 数据一律经宿主 `workflow.run` 的 `list_recent` op（在跑的 + 已结束的）—— 执行态在宿主
/// 引擎与宿主登记表里，插件不复制一份（禁区 12）。本函数只干两件宿主干不了/不该干的事：
/// 1. **状态词表翻译**：引擎六态（`completed` / `partially_completed` / `cancelled` 英式拼法）
///    → 任务中心七态（`succeeded` / `canceled` 美式拼法）。表外值一律落 `waiting`，
///    **不猜**成某个具体终态（与宿主 `mapRunTrigger` 同一条纪律）。
/// 2. **形状收敛**：只交宿主泛型适配器认得的字段，`raw` 原样带上引擎视图供详情抽屉。
///
/// ⚠ `partially_completed` 归 `succeeded`：它是终态且**确有产出**，判成 `failed` 会把
/// 「跑完但部分节点没成」的批次误标成失败；细分差异留在 `raw.status` 里，宿主不改口径。
fn recent_workflow_runs(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    // `limit` 容忍缺省与非数字：非数字按缺省档处理并**说明**，不静默改成别的值。
    let limit = input
        .get("limit")
        .and_then(Value::as_u64)
        .map(|v| v.clamp(1, 200) as u32)
        .unwrap_or(DEFAULT_TASK_SOURCE_LIMIT);
    let workflow_id = input.get("workflowId").and_then(Value::as_str).map(str::trim);

    let reply = peer
        .call_seam(
            WORKFLOW_RUN_SEAM,
            "list_recent",
            json!({ "limit": limit, "workflow_id": workflow_id }),
            call_chain.to_vec(),
        )
        .map_err(|e| {
            (error_codes::PLUGIN_COMMAND_FAILED.to_string(), format!("工作流执行接缝调用失败：{e}"))
        })?;
    match reply.kind {
        ResponseKind::Success => {
            let page = reply.value.unwrap_or(Value::Null);
            let executions =
                page.get("executions").and_then(Value::as_array).cloned().unwrap_or_default();
            let rows: Vec<Value> = executions.iter().map(task_run_row_from_execution).collect();
            Ok(Value::Array(rows))
        },
        ResponseKind::Error => Err((
            reply.code.unwrap_or_else(|| error_codes::PLUGIN_COMMAND_FAILED.to_string()),
            reply.message.unwrap_or_else(|| "工作流接缝返回错误但未说明".to_string()),
        )),
    }
}

/// 引擎执行视图（camelCase JSON）→ 任务中心一行。
///
/// 先收成 [`PluginTaskRunRow`] 再序列化，而不是手搓 `json!` 键名：行形状的唯一权威在
/// plugin-proto，宿主前端按那份 camelCase 消费（禁区 12）—— 手搓键名漂一个字母不会报错，
/// 只会让任务中心静默少一列。
///
/// ⚠ **不交 `deepLink`**：工作流执行详情今天没有独立站内路由
/// （2026-10-28 已删 `/workflow` 一类旧链，见 `ContentArea.tsx:197`）⇒ 造一个指向 404 的
/// 深链比没有深链更坏。宿主泛型适配器据此把该 run 的 `openSource` 能力关成 false。
///
/// `finishedAt` 只透传宿主的 `finishedAt`，**不由 `updatedAt` 倒推**：宿主对已退出的执行
/// 才盖这个戳（在跑的行没这键），拿「最近一次推进」冒充结束时刻会让耗时列变成编读数。
/// 耗时也不在这里算 —— 宿主泛型适配器有两端点就会算（`resolveDurationMs`），插件再算一份
/// 就是第二套口径。
fn task_run_row_from_execution(execution: &Value) -> Value {
    let status = execution.get("status").and_then(Value::as_str).unwrap_or_default();
    let execution_id = execution.get("executionId").and_then(Value::as_str).unwrap_or_default();
    let workflow_id = execution.get("workflowId").and_then(Value::as_str).unwrap_or_default();
    let state = map_execution_state_to_run_state(status);
    debug_assert!(
        is_plugin_task_run_state(state),
        "状态映射产出了表外值 `{state}`：宿主会静默按未知处理"
    );
    let row = PluginTaskRunRow {
        native_id: execution_id.to_string(),
        // 同一 workflow 会有多条执行 ⇒ 名字必须能区分开，不能只显示 workflow id。
        name: format!("{workflow_id} · {execution_id}"),
        state: state.to_string(),
        // 工作流执行由插件受理，中心一律标 manual：引擎侧没有「定时/系统发起」的列，
        // 猜成 schedule 就是把不知道伪装成读数。
        trigger: Some("manual".to_string()),
        started_at: execution.get("createdAt").and_then(Value::as_i64),
        finished_at: execution.get("finishedAt").and_then(Value::as_i64),
        duration_ms: None,
        deep_link: None,
        raw: Some(execution.clone()),
    };
    serde_json::to_value(row).expect("任务源行记录是可序列化的纯数据")
}

/// 引擎六态 → 任务中心七态（词汇表 = `PLUGIN_TASK_RUN_STATES`，表外一律 `waiting`）。
///
/// 终态那三条分支今天**实测可达**：宿主 `list_recent` 会把已退出的执行放进同一页
/// （#150；此前 `list_active` 只吐 running / paused，那三条只是防御性代码）。
/// 表外值仍落 `waiting` —— 引擎将来加第七态时这里不会误判成某个具体终态。
fn map_execution_state_to_run_state(status: &str) -> &'static str {
    match status {
        "running" => "running",
        "paused" => "paused",
        "completed" | "partially_completed" => "succeeded",
        "failed" => "failed",
        "cancelled" => "canceled",
        _ => "waiting",
    }
}

/// 单股分析记录取数命令：`PluginTaskRunRow[]` 的 JSON 数组。
///
/// 数据一律经宿主 `analysis.record` 的 `list` 取 —— 记录表与「宿主发起的记录」只有宿主知道
/// （`workflow.run` 按 owner 隔离，`list_recent` 一条都看不见宿主发起的分析，判据见
/// `harness/src/analysis_record.rs` 模块头）。本函数只做宿主干不了的两件事：
///
/// 1. **状态词表翻译**：宿主落库值（`completed` / `failed` / `error` / `interrupted` /
///    `canceled` / `cancelled` / `paused` / `queued` / `pending` / `running`）→ 任务中心七态。
///    这张表此前住在宿主适配器 `mapAnalysisState`（`stockAnalysis.adapter.ts`），#149 第 2 步
///    删适配器后**以这里为准**；表外值落 `failed`（沿用宿主适配器的裁定：不得默认成
///    `running`，否则僵死记录会一直显示「运行中」并触发轮询）。
/// 2. **根记录收窄**：`parentAnalysisId != null` 的是时间旅行批次的子行（宿主适配器同样只取
///    根记录，replay 子行由 time-travel 源代表）。⚠ 收窄发生在插件侧 ⇒ 宿主返回的 `total`
///    仍是整表行数，**不能**当作「根记录总数」参与分页算术（接缝口径就是这样，见契约模块头）。
///
/// ⚠ **不交 `deepLink`**：站内路由是宿主的路由契约（`src/lib/investRoutes.ts`），插件侧
/// 抄一份就会随着宿主改路由静默 404。缺省 ⇒ 该 run 暂不可打开；路由拼装归后续批（与
/// `task_run_row_from_execution` 的选择同一条判据）。
fn stock_analysis_records(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    // `limit` / `offset` 原样透传：**不在插件侧钳位**（越界由宿主按契约区间拒收），
    // 缺省也交给宿主 —— 插件再抄一份 50 / 200 就是第二套口径。
    let mut args = serde_json::Map::new();
    if let Some(limit) = input.get("limit").filter(|v| !v.is_null()) {
        args.insert("limit".to_string(), limit.clone());
    }
    if let Some(offset) = input.get("offset").filter(|v| !v.is_null()) {
        args.insert("offset".to_string(), offset.clone());
    }

    let reply = peer
        .call_seam(ANALYSIS_RECORD_SEAM, "list", Value::Object(args), call_chain.to_vec())
        .map_err(|e| {
            (error_codes::PLUGIN_COMMAND_FAILED.to_string(), format!("分析记录接缝调用失败：{e}"))
        })?;
    match reply.kind {
        ResponseKind::Success => {
            let page = reply.value.unwrap_or(Value::Null);
            let records =
                page.get("records").and_then(Value::as_array).cloned().unwrap_or_default();
            let rows: Vec<Value> = records
                .iter()
                .filter(|record| match record.get("parentAnalysisId") {
                    // 只交根记录：`null` / 缺键都算根（接缝对 `None` 的线格式是 `null`）。
                    None | Some(Value::Null) => true,
                    Some(_) => false,
                })
                .map(task_run_row_from_analysis_record)
                .collect();
            Ok(Value::Array(rows))
        },
        // 接缝报错 ⇒ 命令失败并**原样透传错误码**，不得退化成空列表
        // （空数组在任务中心看起来像「今天没活」）。
        ResponseKind::Error => Err((
            reply.code.unwrap_or_else(|| error_codes::PLUGIN_COMMAND_FAILED.to_string()),
            reply.message.unwrap_or_else(|| "分析记录接缝返回错误但未说明".to_string()),
        )),
    }
}

/// 分析记录视图（camelCase JSON）→ 任务中心一行。
///
/// 收成 [`PluginTaskRunRow`] 再序列化（行形状权威在 plugin-proto，不手搓键名）。
///
/// 结束时刻只用**完成时刻** `completedAt`，且仅在终态（succeeded / failed）且
/// `completedAt >= createdAt` 时透出 —— 宿主 `updatedAt` 会被交易意图审核 / 结果回写顶掉
/// （现网 65% 的 completed 行漂移 ≥ 24 小时），拿它算耗时会显示「跑了好几天」。耗时不在
/// 这里算：两端点相减只发生在宿主泛型适配器一处。
fn task_run_row_from_analysis_record(record: &Value) -> Value {
    let status = record.get("status").and_then(Value::as_str).unwrap_or_default();
    let state = map_analysis_state_to_run_state(status);
    debug_assert!(
        is_plugin_task_run_state(state),
        "状态映射产出了表外值 `{state}`：宿主会静默按未知处理"
    );
    let id = record.get("id").and_then(Value::as_str).unwrap_or_default();
    let stock_code = record.get("stockCode").and_then(Value::as_str).unwrap_or_default();
    let stock_name = record.get("stockName").and_then(Value::as_str).unwrap_or_default();
    let created_at = record.get("createdAt").and_then(Value::as_i64);
    let completed_at = record.get("completedAt").and_then(Value::as_i64);
    let finished_at = match (state, created_at, completed_at) {
        ("succeeded" | "failed", Some(started), Some(completed)) if completed >= started => {
            Some(completed)
        },
        _ => None,
    };
    let row = PluginTaskRunRow {
        native_id: id.to_string(),
        // 不含 i18n 前缀：宿主适配器把 `tasksCenter.source.stockAnalysis` 拼在名字前面，
        // 插件侧不持有那套译键（`PluginTaskRunRow::name` 的「不经宿主 i18n」就指这一格）。
        name: format!("{stock_code} {stock_name}").trim().to_string(),
        state: state.to_string(),
        // 记录表没有触发来源列（`config_id` 现网 0 行有值）⇒ 定时发起的分析无法与手动区分。
        // 一律标 manual，与本文件工作流任务源同口径（`:1044` 的注释实证「触发来源不可区分」；原宿主 adapter 已于 #149 第 3 步删除）。
        trigger: Some("manual".to_string()),
        started_at: created_at,
        finished_at,
        duration_ms: None,
        deep_link: None,
        // 详情投影：本插件 `ui.json` 的 `stock-analysis-detail` 用 Table 节点 `id = "rows"`，
        // 宿主泛型适配器把 `run.raw` 原样当 `dataContext` 交给 `PluginSchemaCard`
        // （`pluginTaskSource.adapter.ts` 的 `dataContext: run.raw`）⇒ 键名必须与节点 id 对齐。
        // 记录本身**一个字段不减**地落在 `rows[0]` 里：包一层是为了对上 schema 的数组入口，
        // 不是重新裁剪字段（裁剪等于在插件里造第二套 DTO）。
        raw: Some(json!({ "rows": [record.clone()] })),
    };
    serde_json::to_value(row).expect("任务源行记录是可序列化的纯数据")
}

/// 分析记录落库状态 → 任务中心七态（词汇表 = `PLUGIN_TASK_RUN_STATES`）。
///
/// 逐条对齐宿主适配器的 `mapAnalysisState`（`stockAnalysis.adapter.ts`，大小写不敏感：宿主列
/// 是自由文本）。表外值落 `failed` —— 这不是「已知失败」，而是宿主适配器当时的裁定：
/// **不得默认成 `running`**（那会让僵死记录继续显示「运行中」并触发轮询）。#149 第 2 步删
/// 适配器后本表即唯一权威，改默认档必须同时改这一格的理由。
fn map_analysis_state_to_run_state(status: &str) -> &'static str {
    match status.to_ascii_lowercase().as_str() {
        "running" => "running",
        "paused" => "paused",
        "completed" => "succeeded",
        "failed" | "error" => "failed",
        "interrupted" | "canceled" | "cancelled" => "canceled",
        "queued" | "pending" => "queued",
        // 未知状态不得默认成 running（会让僵死记录一直显示运行中并触发轮询）。
        _ => "failed",
    }
}

/// 取一条个股行情，返回 `(入参里的股票代码, 行情 JSON)`。
///
/// **工具面**（`exec_tool`）的唯一取数实现 —— 命令面 `stock_get_quote` 自批 2b 起改走
/// `market_read_route`，因为两面的入参键不同档（工具面 = 宿主 LLM 工具声明的 snake_case）。
/// 出站走宿主接缝：
/// 凭证 / provider / 限流都在宿主侧，插件只传代码；`call_chain` 必须原样带上入站链，
/// 否则宿主侧的环检测收不到这条边。
fn fetch_quote(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<(String, Value), (String, String)> {
    let Some(stock_code) =
        input.get("stock_code").and_then(Value::as_str).map(str::trim).filter(|c| !c.is_empty())
    else {
        return Err((
            error_codes::SEAM_CALL_INVALID_ARGS.to_string(),
            "`stock_code` 缺失或为空白".to_string(),
        ));
    };

    let reply = peer
        .call_seam(
            MARKET_SEAM,
            "get_quote",
            json!({ "stock_code": stock_code }),
            call_chain.to_vec(),
        )
        .map_err(|e| {
            (error_codes::PLUGIN_CALL_FAILED.to_string(), format!("行情接缝调用失败：{e}"))
        })?;

    match reply.kind {
        ResponseKind::Success => Ok((stock_code.to_string(), reply.value.unwrap_or(Value::Null))),
        ResponseKind::Error => Err((
            reply.code.unwrap_or_else(|| error_codes::PLUGIN_CALL_FAILED.to_string()),
            reply.message.unwrap_or_else(|| "行情接缝返回了错误但没有说明".to_string()),
        )),
    }
}

/// 命令入参 → 接缝 `request` 载荷：**原样透传**，不裁剪、不改名、不补默认值。
///
/// 字段形状（含 `BatchUpsertInput` 的 snake / camel 双写 alias、「缺省 vs 空数组」的区别）
/// 的唯一权威在 `analysis-engine` 的 DTO；插件侧「顺手整理一下」就会变成第二套形状
/// （禁区 12），而且会把「必填缺失」翻译成默认值 —— 那正是写出假主线的入口。
fn stock_analysis_request(input: &Value) -> Value {
    input.clone()
}

/// 行情只读命令的路由：`market.quote` 的 op 名 + 标的入参的键名对照。
///
/// `keys` 是 `(命令面键, 接缝面键)`：命令面 camelCase（禁区 13，与前端调用点一致），
/// 接缝面 snake_case（宿主 `worker.rs` 的既有协议，不改）。`None` = 该 op 不吃标的代码
/// （`get_cls_flash` 是全局快讯）。
#[derive(Clone, Copy)]
struct MarketReadRoute {
    op: &'static str,
    keys: Option<(&'static str, &'static str)>,
    /// 可选键档（§145 未做③）：存在且非空白才透传（camelCase 命令面键， snake_case 接缝键）。
    /// `get_limit_up_pool` 的宿主参数名是 `date`，接缝 op 参数名是 `requested_date` ——
    /// 两名不同源，靠这一档翻译；缺省时不插键，接缝侧落「取当下」。
    optional: Option<(&'static str, &'static str)>,
}

impl MarketReadRoute {
    /// 常规只读路由：无可选键。op 必须逐条写死（不按命令名机械剥前缀）。
    fn read(op: &'static str, keys: Option<(&'static str, &'static str)>) -> Self {
        MarketReadRoute { op, keys, optional: None }
    }
}

/// 插件命令名 → `market.quote` 路由（批 2a 十一条 + 批 2b quote + 批 2c 五条全市场榜单/盘面
/// 命令 + 候选 A 六条行情资讯直映，共二十三条）。
///
/// 表驱动而非逐段复制：转发形态完全一致，只有 op 与入参键不同；但 op 必须逐条写死
/// （不按 `stock_` 前缀机械剥离），否则改名时命令会静默打到相邻的数据面上。
fn market_read_route(command: &str) -> Option<MarketReadRoute> {
    let stock = Some(("stockCode", "stock_code"));
    let keyword = Some(("keyword", "keyword"));
    Some(match command {
        COMMAND_GET_QUOTE => MarketReadRoute::read("get_quote", stock),
        COMMAND_GET_RESEARCH_REPORTS => MarketReadRoute::read("get_research_reports", stock),
        COMMAND_GET_CONCEPT_BLOCKS => MarketReadRoute::read("get_concept_blocks", stock),
        COMMAND_GET_ANNOUNCEMENTS => MarketReadRoute::read("get_announcements", stock),
        COMMAND_GET_EARNINGS_CALENDAR => MarketReadRoute::read("get_earnings_calendar", stock),
        COMMAND_GET_CONCEPT_BOARD_MEMBERS => {
            MarketReadRoute::read("get_concept_board_members", Some(("boardCode", "board_code")))
        },
        COMMAND_GET_CLS_FLASH => MarketReadRoute::read("get_cls_flash", None),
        COMMAND_GET_PEERS => MarketReadRoute::read("get_peers", stock),
        COMMAND_GET_OPTION_PCR => MarketReadRoute::read("get_option_pcr", stock),
        COMMAND_GET_LOCKUP_SCHEDULE => MarketReadRoute::read("get_lockup_schedule", stock),
        COMMAND_GET_DIVIDEND_RECORDS => MarketReadRoute::read("get_dividend_records", stock),
        COMMAND_GET_FINANCIALS => MarketReadRoute::read("get_financials", stock),
        // ── 批 2c（#262）：五条全市场榜单/盘面命令，零入参 ⇒ `keys: None`（与 `get_cls_flash` 同形）。
        // `stock_get_index_quotes` 的 op 是 `get_index_quotes_outcome`，**不按命令名剥前缀**：
        // 宿主命令 `get_index_quotes` 体内调的 trait 方法 / `worker.rs` 分派臂都叫 `*_outcome`
        // （返回 `DataOutcome` 信封而非裸数组）；机械剥成 `get_index_quotes` 会打到不存在的 op 上。
        COMMAND_GET_HOT_STOCKS => MarketReadRoute::read("get_hot_stocks", None),
        COMMAND_GET_INDUSTRY_RANKING => MarketReadRoute::read("get_industry_ranking", None),
        COMMAND_GET_INDEX_QUOTES => MarketReadRoute::read("get_index_quotes_outcome", None),
        COMMAND_GET_MARKET_DRAGON_TIGER => MarketReadRoute::read("get_market_dragon_tiger", None),
        COMMAND_GET_NORTH_BOUND_FLOW => MarketReadRoute::read("get_north_bound_flow", None),
        // ── 候选 A（#35 行情资讯直映批）：五条单必填键 + 一条可选键 ──
        // `search_stock` / `search_concept_boards` / `get_social_sentiment` /
        // `get_international_stock_quote` / `get_consensus_eps` 五条 op 的 as_of 一律缺省
        // （插件命令不开放，与宿主同名命令的调用面等价）。
        COMMAND_SEARCH_STOCK => MarketReadRoute::read("search_stock", keyword),
        COMMAND_SEARCH_CONCEPT_BOARDS => MarketReadRoute::read("search_concept_boards", keyword),
        COMMAND_GET_SOCIAL_SENTIMENT => MarketReadRoute::read("get_social_sentiment", stock),
        COMMAND_GET_INTERNATIONAL_STOCK_QUOTE => {
            MarketReadRoute::read("get_international_quote", stock)
        },
        COMMAND_GET_STOCK_CONSENSUS_EPS => MarketReadRoute::read("get_consensus_eps", stock),
        // `get_limit_up_pool`：宿主参数名 `date`（camelCase 面同名），接缝参数名
        // `requested_date` —— 路由表唯一一条走可选键档的命令。
        COMMAND_GET_LIMIT_UP_POOL => MarketReadRoute {
            op: "get_limit_up_pool",
            keys: None,
            optional: Some(("date", "requested_date")),
        },
        // F1 余三条（#35 / #321）：刷新交易日历，零入参 ⇒ `keys: None`（与批 2c 同形）。
        // `fetch_holiday_calendar` 的东财报表已下线，op 保持与被退役宿主命令等价的失败行为。
        COMMAND_REFRESH_TRADING_CALENDAR => MarketReadRoute::read("refresh_trading_calendar", None),
        _ => return None,
    })
}

/// 调宿主 `market.quote` 接缝取一条只读数据。
///
/// 与 `fetch_quote` 同一纪律：凭证 / provider / 限流都在宿主侧，插件只传标的；
/// `call_chain` 原样带上入站链，否则宿主侧的环检测收不到这条边。
/// `as_of` 不开放 —— 宿主同名命令也不接受该参数，保持调用面等价（多开一个入参
/// 等于让插件命令比宿主命令多一条未验证的时间旅行路径）。
fn fetch_market_read(
    route: MarketReadRoute,
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    let mut args = serde_json::Map::new();
    if let Some((input_key, seam_key)) = route.keys {
        let Some(value) =
            input.get(input_key).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty())
        else {
            return Err((
                error_codes::SEAM_CALL_INVALID_ARGS.to_string(),
                format!("`{input_key}` 缺失或为空白"),
            ));
        };
        args.insert(seam_key.to_string(), json!(value));
    }
    // 可选键档：存在且非空白才插键 —— 缺省 / null / 空白都表示「用接缝侧缺省语义」
    // （如 `get_limit_up_pool` 的「取当下」），与宿主命令不传该参同形。
    if let Some((input_key, seam_key)) = route.optional
        && let Some(value) =
            input.get(input_key).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty())
    {
        args.insert(seam_key.to_string(), json!(value));
    }

    let reply = peer
        .call_seam(MARKET_SEAM, route.op, Value::Object(args), call_chain.to_vec())
        .map_err(|e| {
            (
                error_codes::PLUGIN_CALL_FAILED.to_string(),
                format!("行情接缝（{}）调用失败：{e}", route.op),
            )
        })?;

    match reply.kind {
        ResponseKind::Success => Ok(reply.value.unwrap_or(Value::Null)),
        ResponseKind::Error => Err((
            reply.code.unwrap_or_else(|| error_codes::PLUGIN_CALL_FAILED.to_string()),
            reply.message.unwrap_or_else(|| "行情接缝返回了错误但没有说明".to_string()),
        )),
    }
}

/// K 线族命令面入参 → 接缝 K 线 op 载荷：命令面 camelCase 翻成接缝面 snake_case。
///
/// 拆成纯函数（不碰 `Peer`）是因为多键入参的翻译与拒绝判据需要单测钉住 —— 接缝调用
/// 本身要子进程管道，测不到「缺 `period` 时报什么」。候选 A 把单标的 K 线泛化成
/// K 线族共用：差异只剩「标的键名」（A 股 `stockCode` / 外汇 `pair` / 基准
/// `benchmarkCode`）与 op 名，period / limit 三键判据完全一致。
/// `with_adj = false`（国际/外汇/基准）：宿主同名命令不收复权参 ⇒ 插件面也不收
/// —— 不插 `adj_type` 键，接缝侧 `deserialize_optional_arg` 对缺失键落 `None`。
/// `as_of_date` 不开放 —— 宿主命令有该入参，但接缝 K 线 op 没有 ⇒ 多开一条入参等于
/// 承诺一条接缝给不了的通道。
fn kline_seam_args(
    input: &Value,
    asset: (&str, &str),
    with_adj: bool,
) -> Result<Value, (String, String)> {
    fn invalid(message: String) -> (String, String) {
        (error_codes::SEAM_CALL_INVALID_ARGS.to_string(), message)
    }
    fn required_str(input: &Value, key: &str) -> Result<String, (String, String)> {
        input
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string)
            .ok_or_else(|| invalid(format!("`{key}` 缺失或为空白")))
    }

    let (asset_key, seam_key) = asset;
    let mut args = serde_json::Map::new();
    args.insert(seam_key.to_string(), json!(required_str(input, asset_key)?));
    args.insert("period".to_string(), json!(required_str(input, "period")?));
    let Some(limit) = input.get("limit").and_then(Value::as_u64) else {
        return Err(invalid("`limit` 必须是整数".to_string()));
    };
    args.insert("limit".to_string(), json!(limit));
    if with_adj {
        let adj_type = match input.get("adjType") {
            None | Some(Value::Null) => Value::Null,
            Some(Value::String(raw)) => match raw.as_str() {
                "auto" => Value::Null,
                "none" | "forward" | "backward" => json!(raw),
                other => {
                    return Err(invalid(format!(
                        "`adjType` 只接受 auto / none / forward / backward，收到 `{other}`"
                    )));
                },
            },
            Some(other) => return Err(invalid(format!("`adjType` 必须是字符串，收到 {other}"))),
        };
        args.insert("adj_type".to_string(), adj_type);
    }
    Ok(Value::Object(args))
}

/// K 线族命令名 → `market.quote` 路由（批 2b A 股 K 线 + 候选 A 三条国际/外汇/基准）。
///
/// `stock_get_international_stock_kline` 的 op 是 `get_international_klines` —— 命令名跟
/// 宿主 Tauri 命令（单数），op 跟接缝协议（复数），两套名并存；同理 `get_forex_klines` /
/// `get_benchmark_klines`。
#[derive(Clone, Copy)]
struct KlineRoute {
    op: &'static str,
    /// (命令面 camelCase 标的键， 接缝面 snake_case 标的键)。
    asset: (&'static str, &'static str),
    /// A 股 K 线宿主命令有 `adj` 参 ⇒ 翻译 `adjType`；其余三条宿主固定未复权 ⇒ false。
    with_adj: bool,
}

fn kline_route(command: &str) -> Option<KlineRoute> {
    Some(match command {
        COMMAND_GET_KLINE => {
            KlineRoute { op: "get_klines", asset: ("stockCode", "stock_code"), with_adj: true }
        },
        COMMAND_GET_INTERNATIONAL_STOCK_KLINE => KlineRoute {
            op: "get_international_klines",
            asset: ("stockCode", "stock_code"),
            with_adj: false,
        },
        COMMAND_GET_FOREX_KLINE => {
            KlineRoute { op: "get_forex_klines", asset: ("pair", "pair"), with_adj: false }
        },
        COMMAND_GET_BENCHMARK_KLINE => KlineRoute {
            op: "get_benchmark_klines",
            asset: ("benchmarkCode", "benchmark_code"),
            with_adj: false,
        },
        _ => return None,
    })
}

/// 调宿主 `market.quote` 的 K 线族 op 取一段 K 线（批 2b 起）。
///
/// 与 `fetch_market_read` 同一纪律：凭证 / provider / 限流都在宿主侧，插件只传标的与档位。
fn fetch_kline(
    route: KlineRoute,
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    let args = kline_seam_args(input, route.asset, route.with_adj)?;
    let reply = peer.call_seam(MARKET_SEAM, route.op, args, call_chain.to_vec()).map_err(|e| {
        (
            error_codes::PLUGIN_CALL_FAILED.to_string(),
            format!("行情接缝（{}）调用失败：{e}", route.op),
        )
    })?;

    match reply.kind {
        ResponseKind::Success => Ok(reply.value.unwrap_or(Value::Null)),
        ResponseKind::Error => Err((
            reply.code.unwrap_or_else(|| error_codes::PLUGIN_CALL_FAILED.to_string()),
            reply.message.unwrap_or_else(|| "行情接缝返回了错误但没有说明".to_string()),
        )),
    }
}

/// `search_news` 命令面入参 → 接缝 `search_news` 载荷：keyword 必填，limit 可选。
///
/// limit 缺省 20、上限钳 100，与宿主 `search_news` 命令体的 `unwrap_or(20).min(100)`
/// 同档 —— 插件不造第二个缺省数。接缝 op 的 `limit` 是必填 u32 ⇒ 缺省时也必须显式传，
/// 不能省键。`as_of` 不开放（与宿主命令调用面等价）。
fn search_news_seam_args(input: &Value) -> Result<Value, (String, String)> {
    fn invalid(message: String) -> (String, String) {
        (error_codes::SEAM_CALL_INVALID_ARGS.to_string(), message)
    }
    let keyword = input
        .get("keyword")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| invalid("`keyword` 缺失或为空白".to_string()))?;
    let limit = match input.get("limit") {
        None | Some(Value::Null) => 20,
        Some(raw) => {
            let value =
                raw.as_u64().ok_or_else(|| invalid("`limit` 必须是非负整数".to_string()))?;
            u32::try_from(value)
                .map_err(|_| invalid("`limit` 超出 u32 上限".to_string()))
                .map(|v| v.min(100))?
        },
    };
    Ok(json!({ "keyword": keyword, "limit": limit }))
}

/// 调宿主 `market.quote` 的 `search_news` 取一批新闻（候选 A）。
///
/// 与 `fetch_market_read` 同一纪律：凭证 / provider / 限流都在宿主侧。
fn fetch_search_news(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    let args = search_news_seam_args(input)?;
    let reply =
        peer.call_seam(MARKET_SEAM, "search_news", args, call_chain.to_vec()).map_err(|e| {
            (
                error_codes::PLUGIN_CALL_FAILED.to_string(),
                format!("行情接缝（search_news）调用失败：{e}"),
            )
        })?;

    match reply.kind {
        ResponseKind::Success => Ok(reply.value.unwrap_or(Value::Null)),
        ResponseKind::Error => Err((
            reply.code.unwrap_or_else(|| error_codes::PLUGIN_CALL_FAILED.to_string()),
            reply.message.unwrap_or_else(|| "行情接缝返回了错误但没有说明".to_string()),
        )),
    }
}

/// `batch_get_quotes` 命令面入参 → 接缝 `batch_get_quotes` 载荷：codes 必填，三个数值键可选。
///
/// 纯函数（不碰 `Peer`）：拒绝判据需要单测钉住。`codes` 校验口径与原宿主命令的 serde
/// 反序列化一致（键缺失 / 非字符串数组 ⇒ 拒绝）；可选数值键存在才透传，缺省落到
/// 接缝侧（provider 默认 8s / 不提前终止）。`totalTimeoutMs` 原样透传 —— 接缝与迁移前
/// 一样「接受但不强制」，插件侧不做第二次裁决（禁区 12）。`as_of` 不开放（与宿主
/// 命令调用面等价）。
fn batch_quotes_seam_args(input: &Value) -> Result<Value, (String, String)> {
    fn invalid(message: String) -> (String, String) {
        (error_codes::SEAM_CALL_INVALID_ARGS.to_string(), message)
    }
    let mut args = serde_json::Map::new();
    args.insert("codes".to_string(), json!(parse_string_array(input, "codes")?));
    for (input_key, seam_key) in [
        ("perStockTimeoutMs", "per_stock_timeout_ms"),
        ("totalTimeoutMs", "total_timeout_ms"),
        ("maxFailures", "max_failures"),
    ] {
        match input.get(input_key) {
            None | Some(Value::Null) => {},
            Some(raw) => {
                let value =
                    raw.as_u64().ok_or_else(|| invalid(format!("`{input_key}` 必须是非负整数")))?;
                args.insert(seam_key.to_string(), json!(value));
            },
        }
    }
    Ok(Value::Object(args))
}

/// `batch_get_financials` 命令面入参 → 接缝 `batch_get_financials` 载荷：只有 codes 一键。
fn batch_financials_seam_args(input: &Value) -> Result<Value, (String, String)> {
    Ok(json!({ "codes": parse_string_array(input, "codes")? }))
}

/// 批量命令的字符串数组判据：必须是字符串数组（空数组与迁移前一致地放行 ⇒ 空结果）。
/// `key` 由调用方给（`codes` / `conceptIds`），判据与拒绝码只此一份。
fn parse_string_array(input: &Value, key: &str) -> Result<Vec<String>, (String, String)> {
    let Some(raw) = input.get(key) else {
        return Err((error_codes::SEAM_CALL_INVALID_ARGS.to_string(), format!("`{key}` 缺失")));
    };
    let values: Vec<String> = serde_json::from_value(raw.clone()).map_err(|e| {
        (
            error_codes::SEAM_CALL_INVALID_ARGS.to_string(),
            format!("`{key}` 必须是字符串数组，反序列化失败：{e}"),
        )
    })?;
    Ok(values)
}

/// 调宿主 `market.quote` 的批量 op，整块回传 `BatchResult` 信封。
///
/// 出参**不在这里解构** —— success/failures 双 map 的形状权威在 harness DTO。
fn fetch_market_batch(
    op: &str,
    args: Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    let reply = peer.call_seam(MARKET_SEAM, op, args, call_chain.to_vec()).map_err(|e| {
        (error_codes::PLUGIN_CALL_FAILED.to_string(), format!("行情接缝（{op}）调用失败：{e}"))
    })?;

    match reply.kind {
        ResponseKind::Success => Ok(reply.value.unwrap_or(Value::Null)),
        ResponseKind::Error => Err((
            reply.code.unwrap_or_else(|| error_codes::PLUGIN_CALL_FAILED.to_string()),
            reply.message.unwrap_or_else(|| "行情接缝返回了错误但没有说明".to_string()),
        )),
    }
}

/// 调宿主 `quote.watcher` 的 `list_watched`，解包成 code 数组（原宿主命令的出参形状）。
///
/// 接缝回的是**具名对象** `{codes}`（provider 侧排序，裁定 J6；原宿主命令回裸数组），
/// 解包点仅此一处；形状不符 ⇒ 协议错误，不得吞成空列表掩盖漂移。
fn fetch_watched_quotes(
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    let reply = peer
        .call_seam(QUOTE_WATCHER_SEAM, "list_watched", json!({}), call_chain.to_vec())
        .map_err(|e| {
            (
                error_codes::PLUGIN_CALL_FAILED.to_string(),
                format!("自选监视接缝（list_watched）调用失败：{e}"),
            )
        })?;
    match reply.kind {
        ResponseKind::Success => {
            let value = reply.value.unwrap_or(Value::Null);
            value.get("codes").cloned().filter(Value::is_array).ok_or_else(|| {
                (
                    error_codes::PLUGIN_CALL_FAILED.to_string(),
                    format!("`list_watched` 接缝返回形状异常（应为 {{codes: [...]}}）：{value}"),
                )
            })
        },
        ResponseKind::Error => Err((
            reply.code.unwrap_or_else(|| error_codes::PLUGIN_CALL_FAILED.to_string()),
            reply.message.unwrap_or_else(|| "自选监视接缝返回了错误但没有说明".to_string()),
        )),
    }
}

/// 插件命令名 → `stock.analysis` 接缝 op 名（批 1a 的十二条 cron 配置命令）。
///
/// 表驱动而非十二段复制：每个臂的转发形态完全一致（透传入参、整块回出参），
/// 只有 op 名不同。返回值是 `&'static str`，与宿主 `worker.rs` 的分派臂逐字对应。
fn cron_seam_op(command: &str) -> Option<&'static str> {
    Some(match command {
        COMMAND_CREATE_RECOMMENDATION_CRON => "create_recommendation_cron",
        COMMAND_LIST_RECOMMENDATION_CRONS => "list_recommendation_crons",
        COMMAND_TOGGLE_RECOMMENDATION_CRON => "toggle_recommendation_cron",
        COMMAND_DELETE_RECOMMENDATION_CRON => "delete_recommendation_cron",
        COMMAND_CREATE_TREND_SCREENING_CRON => "create_trend_screening_cron",
        COMMAND_LIST_TREND_SCREENING_CRONS => "list_trend_screening_crons",
        COMMAND_TOGGLE_TREND_SCREENING_CRON => "toggle_trend_screening_cron",
        COMMAND_DELETE_TREND_SCREENING_CRON => "delete_trend_screening_cron",
        COMMAND_CREATE_POOL_SCAN_CRON => "create_pool_scan_cron",
        COMMAND_LIST_POOL_SCAN_CRONS => "list_pool_scan_crons",
        COMMAND_TOGGLE_POOL_SCAN_CRON => "toggle_pool_scan_cron",
        COMMAND_DELETE_POOL_SCAN_CRON => "delete_pool_scan_cron",
        _ => return None,
    })
}

/// 调宿主 `stock.analysis` 接缝 —— 三个工具命令的唯一取数路径（E 批 #240）。
///
/// 入参嵌一层 `{"request": {...}}`，与 `opc.store` 的 `*ListQuery` 同形：宿主
/// `dispatch_seam` 按 `args["request"]` 取载荷（见 `worker.rs` 的三条分派臂）。
/// 出参整块回给命令面**不在这里解构** —— 信封形状归宿主 provider 与域 DTO。
fn call_analysis_seam(
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
    call_chain: &[String],
    op: &str,
    request: Value,
) -> Result<Value, (String, String)> {
    let reply = peer
        .call_seam(STOCK_ANALYSIS_SEAM, op, json!({ "request": request }), call_chain.to_vec())
        .map_err(|e| {
            (
                error_codes::PLUGIN_COMMAND_FAILED.to_string(),
                format!("股票分析接缝（{op}）调用失败：{e}"),
            )
        })?;
    match reply.kind {
        ResponseKind::Success => Ok(reply.value.unwrap_or(Value::Null)),
        // 接缝报错 ⇒ 原样透传错误码：宿主侧已裁定是 invalid_args / permission / 内部错误，
        // 插件再包一层就丢掉宿主给的分类（智能体侧的重试分支依赖它）。
        ResponseKind::Error => Err((
            reply.code.unwrap_or_else(|| error_codes::PLUGIN_COMMAND_FAILED.to_string()),
            reply.message.unwrap_or_else(|| format!("股票分析接缝（{op}）返回错误但未说明")),
        )),
    }
}

/// 命令面必填字符串入参：缺失 / 非字符串 / 空白一律 `SEAM_CALL_INVALID_ARGS`。
///
/// 判据与 `fetch_market_read` 的必填键档逐字一致（trim 后非空），不在同一插件里立第二套
/// 「什么算缺失」。
fn command_str_arg(input: &Value, key: &str) -> Result<String, (String, String)> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            (error_codes::SEAM_CALL_INVALID_ARGS.to_string(), format!("`{key}` 缺失或为空白"))
        })
}

/// 必填数字取参（`command_str_arg` 的数值同形态）。i64 版给整数字段（反馈计数等）。
fn command_f64_arg(input: &Value, key: &str) -> Result<f64, (String, String)> {
    input.get(key).and_then(Value::as_f64).ok_or_else(|| {
        (error_codes::SEAM_CALL_INVALID_ARGS.to_string(), format!("`{key}` 缺失或不是数字"))
    })
}

fn command_i64_arg(input: &Value, key: &str) -> Result<i64, (String, String)> {
    input.get(key).and_then(Value::as_i64).ok_or_else(|| {
        (error_codes::SEAM_CALL_INVALID_ARGS.to_string(), format!("`{key}` 缺失或不是整数"))
    })
}

/// 必填布尔取参（`save_analyst_feedback` 的两个一致性字段用）。
fn command_bool_arg(input: &Value, key: &str) -> Result<bool, (String, String)> {
    input.get(key).and_then(Value::as_bool).ok_or_else(|| {
        (error_codes::SEAM_CALL_INVALID_ARGS.to_string(), format!("`{key}` 缺失或不是布尔"))
    })
}

/// 调宿主 `trading.engine` 接缝（#311 两条读命令的唯一取数路径）。
///
/// 与 `call_analysis_seam` 同形态但**不嵌 `request` 层**：该接缝的分派臂按平铺 snake_case
/// 键取参（`worker.rs` 的 `deserialize_arg` / `deserialize_as_of_arg`）。
fn call_trading_seam(
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
    call_chain: &[String],
    op: &str,
    args: Value,
) -> Result<Value, (String, String)> {
    let reply =
        peer.call_seam(TRADING_ENGINE_SEAM, op, args, call_chain.to_vec()).map_err(|e| {
            (
                error_codes::PLUGIN_COMMAND_FAILED.to_string(),
                format!("交易引擎接缝（{op}）调用失败：{e}"),
            )
        })?;
    match reply.kind {
        ResponseKind::Success => Ok(reply.value.unwrap_or(Value::Null)),
        ResponseKind::Error => Err((
            reply.code.unwrap_or_else(|| error_codes::PLUGIN_COMMAND_FAILED.to_string()),
            reply.message.unwrap_or_else(|| format!("交易引擎接缝（{op}）返回错误但未说明")),
        )),
    }
}

/// 组合风险指标（宿主 `get_portfolio_risk`）：持仓经接缝取，算法跑 `analysis-engine` 权威实现。
///
/// 不开放 `asOf`：宿主原命令调的是 `engine.get_positions()`（无时间旅行入参），
/// 多开一个键就是承诺一条原命令没有的通道（同 `fetch_market_read` 的纪律）。
fn portfolio_risk(
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    use axagent_analysis_engine::portfolio_risk::PortfolioRiskManager;
    use axagent_analysis_engine::trading::PositionSummary;

    let raw = call_trading_seam(peer, call_chain, "get_positions", json!({}))?;
    let positions: Vec<PositionSummary> = serde_json::from_value(raw).map_err(|e| {
        (
            error_codes::PLUGIN_COMMAND_FAILED.to_string(),
            format!("持仓接缝返回形状异常（应为 PositionSummary 数组）：{e}"),
        )
    })?;
    serde_json::to_value(PortfolioRiskManager::compute_from_positions(&positions)).map_err(|e| {
        (error_codes::PLUGIN_COMMAND_FAILED.to_string(), format!("组合风险指标序列化失败：{e}"))
    })
}

/// 交易出场价 vs 分析预测（宿主 `compare_trade_with_analysis`）。
///
/// ⚠ 与宿主原命令的**可观察差异**：接缝 `compare_trade` 走
/// `compare_trade_vs_prediction_opt`，把「交易记录不存在」与「交易前无已完成分析」
/// 合并成 `Ok(None)`；宿主原命令是两条不同文案的 `Err`。插件侧拿不到区分依据 ⇒
/// 统一回一条覆盖两因的错误，**不假装成功**（null 直透会让 LLM 以为对比结果是空）。
fn compare_trade_with_analysis(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    let trade_id = command_str_arg(input, "tradeId")?;
    let value =
        call_trading_seam(peer, call_chain, "compare_trade", json!({ "trade_id": trade_id }))?;
    if value.is_null() {
        return Err((
            error_codes::PLUGIN_COMMAND_FAILED.to_string(),
            "交易记录不存在，或该交易之前没有已完成的分析记录".to_string(),
        ));
    }
    Ok(value)
}

/// 批量板块联动扫描（宿主 `scan_sector_coherence`）。
///
/// 三段与宿主原命令逐字同源：① 本体索引用 `build_sample_index` + `seed_ashare_ontology`
/// （**不是** `get_sector_coherence_report` 那套 DB 完整版索引 —— 两条命令的成员覆盖不同，
/// 换索引就是改结果）；② 逐票行情经 `market.quote::get_quote`，单票失败**跳过不报错**；
/// ③ 扫描与排序跑 `scan_sectors` 权威纯函数（禁区 12：不在插件里复制算法）。
fn scan_sector_coherence(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    use axagent_analysis_engine::concept_index::{build_sample_index, seed_ashare_ontology};
    use axagent_analysis_engine::sector_coherence::{StockQuote, scan_sectors};
    use std::collections::HashMap;

    let concept_ids = parse_string_array(input, "conceptIds")?;

    let mut idx = build_sample_index();
    seed_ashare_ontology(&mut idx);
    let mut quotes_by_concept: HashMap<String, Vec<StockQuote>> = HashMap::new();
    for cid in &concept_ids {
        let members = idx.members(cid);
        if members.is_empty() {
            continue;
        }
        let mut quotes = Vec::with_capacity(members.len());
        for code in &members {
            // 复用工具面同一取数原语（它收 snake_case `stock_code`）；失败与形状不符都跳过，
            // 与宿主原命令的 `if let Ok(q)` 同口径。
            if let Ok((_, value)) = fetch_quote(&json!({ "stock_code": code }), call_chain, peer)
                && let Ok(quote) = serde_json::from_value::<StockQuote>(value)
            {
                quotes.push(quote);
            }
        }
        quotes_by_concept.insert(cid.clone(), quotes);
    }

    let reports = scan_sectors(&idx, &concept_ids, &quotes_by_concept, unix_now_secs());
    Ok(Value::Array(reports.into_iter().filter_map(|r| serde_json::to_value(r).ok()).collect()))
}

/// 热门候选发现（宿主 `discover_stock_candidates`，#329）—— 零新 op 的插件侧重写编排。
///
/// 判据（§183 表）：宇宙 = 热门股前 30 + 行业龙头前 10 + 引擎兜底池（单一权威
/// `recommender::FALLBACK_STOCKS`），行情逐票走 `market.quote::get_quote` 接缝，
/// 打分走引擎纯函数 `StockScreener::score_candidate`（宿主原命令与引擎编排函数已退役，
/// 这份纯函数是唯一权威）。与宿主原命令的可观察差异：
/// - 宇宙源失败时跳过该源（宿主 `if let Ok` 同口径），不报错；
/// - 单票行情失败跳过（宿主 `Err(_) => continue` 同口径）；
/// - 取数走接缝多一跳，时延略增；打分与排序/截断口径逐字同源。
///
/// 出参 = `ScreenResult` 数组（score 降序、前 20），camelCase 与宿主同形。
fn discover_stock_candidates(
    _input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    use axagent_analysis_engine::recommender::FALLBACK_STOCKS;
    // harness 类型一律经引擎 screener 转发取（插件不直接依赖 harness，依赖面判据①）。
    use axagent_analysis_engine::screener::{HotStock, IndustryRank, StockQuote, StockScreener};
    use std::collections::HashSet;

    let mut seen = HashSet::new();
    let mut universe: Vec<(String, String)> = Vec::new();

    // 1. 热门个股前 30（源失败跳过 = 宿主 `if let Ok` 同口径）
    if let Ok(hot_value) = fetch_market_read(
        market_read_route(COMMAND_GET_HOT_STOCKS).expect("热股路由必须在表中"),
        &json!({}),
        call_chain,
        peer,
    ) && let Ok(hot) = serde_json::from_value::<Vec<HotStock>>(hot_value)
    {
        for h in hot.iter().take(30) {
            if seen.insert(h.stock_code.clone()) {
                universe.push((h.stock_code.clone(), h.stock_name.clone()));
            }
        }
    }

    // 2. 行业排名靠前的龙头前 10
    if let Ok(rank_value) = fetch_market_read(
        market_read_route(COMMAND_GET_INDUSTRY_RANKING).expect("行业排名路由必须在表中"),
        &json!({}),
        call_chain,
        peer,
    ) && let Ok(industries) = serde_json::from_value::<Vec<IndustryRank>>(rank_value)
    {
        for ind in industries.iter().take(10) {
            if let (Some(code), Some(name)) = (&ind.leader_code, &ind.leader_name)
                && seen.insert(code.clone())
            {
                universe.push((code.clone(), name.clone()));
            }
        }
    }

    // 3. 回退：引擎兜底池（沪深300 核心成分股）
    if universe.is_empty() {
        for (code, name) in FALLBACK_STOCKS {
            if seen.insert((*code).to_string()) {
                universe.push(((*code).to_string(), (*name).to_string()));
            }
        }
    }

    // 逐票行情 + 纯函数打分；失败跳过 = 宿主 `Err(_) => continue` 同口径
    let mut candidates = Vec::new();
    for (code, name) in &universe {
        if let Ok((_, value)) = fetch_quote(&json!({ "stock_code": code }), call_chain, peer)
            && let Ok(quote) = serde_json::from_value::<StockQuote>(value)
        {
            candidates.push(StockScreener::score_candidate(code, name, &quote));
        }
    }

    candidates.sort_by_key(|r| std::cmp::Reverse(r.score));
    let top: Vec<_> = candidates.into_iter().take(20).collect();
    serde_json::to_value(top).map_err(|e| {
        (error_codes::PLUGIN_CALL_FAILED.to_string(), format!("候选发现序列化失败: {e}"))
    })
}

/// 监控优先级归一（宿主 `set_quote_watch_priority` 的别名宽容）。
///
/// 宿主原命令认 `background` / `bg`，且**任何未知串静默回落 Active**；接缝侧是严格 serde
/// （只认 `active` / `background`，`bg` 直接 `SEAM_CALL_INVALID_ARGS`）⇒ 宽容必须在插件侧
/// 补回，否则 LLM 传 `bg` 从「成功」变「报错」，属行为回退。
fn watch_priority_arg(priority: &str) -> &'static str {
    match priority {
        "background" | "bg" => "background",
        _ => "active",
    }
}

/// 设置单只股票监控优先级（宿主 `set_quote_watch_priority`）—— 本批唯一写命令。
///
/// 引擎的 `watch` 是 insert ⇒ 对已在监控的码重复调用即覆盖优先级，与宿主原命令的
/// `set_priority` 同一语义（裁定 K-6 把 `set_priority` 并入 `watch`）。
///
/// ⚠ 与宿主原命令的**可观察差异**：`watch` 走 `plan_watch_batch`（单次 / 总量双闸，
/// 超限**截断不报错**），而 `set_priority` 是无条件 insert ⇒ 集合已满且该码未在监控时，
/// 原命令会加进去、接缝会静默不加。这里 fail-closed：`applied == 0` 即报错，不假装成功。
/// 出参保持宿主原命令的 `()`（JSON `null`），不透出 `WatchOutcome`。
fn set_quote_watch_priority(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    let stock_code = command_str_arg(input, "stockCode")?;
    let priority = command_str_arg(input, "priority")?;
    let reply = peer
        .call_seam(
            QUOTE_WATCHER_SEAM,
            "watch",
            json!({
                "codes": [stock_code],
                "priority": watch_priority_arg(&priority),
                "replace": false,
            }),
            call_chain.to_vec(),
        )
        .map_err(|e| {
            (
                error_codes::PLUGIN_CALL_FAILED.to_string(),
                format!("自选监视接缝（watch）调用失败：{e}"),
            )
        })?;
    match reply.kind {
        ResponseKind::Success => {
            let value = reply.value.unwrap_or(Value::Null);
            let applied = value.get("applied").and_then(Value::as_u64).unwrap_or(0);
            if applied == 0 {
                return Err((
                    error_codes::PLUGIN_COMMAND_FAILED.to_string(),
                    format!(
                        "监控优先级未生效：`{stock_code}` 未被加入监视集合（集合已满或达单次上限）"
                    ),
                ));
            }
            Ok(Value::Null)
        },
        ResponseKind::Error => Err((
            reply.code.unwrap_or_else(|| error_codes::PLUGIN_CALL_FAILED.to_string()),
            reply.message.unwrap_or_else(|| "自选监视接缝（watch）返回错误但未说明".to_string()),
        )),
    }
}

/// 列估值快照原始行（宿主 `list_financial_snapshots`，F1 余三条之一）。
///
/// 命令面 `stockCode` 必填 + `start` / `end` 可选（camelCase，禁区 13）；接缝面
/// `stock_code` / `start` / `end`（snake_case）—— 两名不同源，靠这一层显式翻译
/// （与 `MarketReadRoute` 的 keys 档同款纪律）。可选键存在且非空白才透传，缺省不插键。
/// 快照行是实体原始行整块透传，出参形状权威在宿主 `financial_snapshots::Model`。
fn fetch_financial_snapshots(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    let stock_code = command_str_arg(input, "stockCode")?;
    let mut request = serde_json::Map::new();
    request.insert("stock_code".to_string(), json!(stock_code));
    if let Some(start) =
        input.get("start").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty())
    {
        request.insert("start".to_string(), json!(start));
    }
    if let Some(end) =
        input.get("end").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty())
    {
        request.insert("end".to_string(), json!(end));
    }
    call_analysis_seam(peer, call_chain, "list_financial_snapshots", Value::Object(request))
}

// ── F2 余四条（#35 / #322）：走 `trading.engine` 新 op ──
// 分派臂按嵌 `{"request": {...}}` 层取参 ⇒ 插件侧在 args 里自己嵌层；层内键
// snake_case（形状权威在宿主 Input struct）。两个零入参 op 传 `json!({})`：
// 分派臂解出 `Null`，宿主 `require_empty_request` 放行（多余载荷会被拒，不是静默忽略）。

/// 组合持仓限额（宿主 `get_position_limits`）：读全局面板变量
/// （`PositionLimits::panel_effective`，面板 `pos_max_*` 回落 20/10/40）。
fn fetch_position_limits(
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    call_trading_seam(peer, call_chain, "get_position_limits", json!({}))
}

/// 更新单条持仓的数量与成本（宿主 `update_portfolio_holding`）—— **写命令**。
///
/// 命令面 `id` / `shares` / `avgCost`（camelCase，禁区 13）；层内键 snake_case。
/// 出参恒 `null`（宿主命令回 `()` 同形）。
fn update_portfolio_holding(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    let id = command_str_arg(input, "id")?;
    let shares = command_f64_arg(input, "shares")?;
    let avg_cost = command_f64_arg(input, "avgCost")?;
    call_trading_seam(
        peer,
        call_chain,
        "update_portfolio_holding",
        json!({ "request": { "id": id, "shares": shares, "avg_cost": avg_cost } }),
    )
}

/// 批量导入 VLM 识别的持仓（宿主 `import_portfolio_from_vlm`）—— **写命令**。
///
/// 命令面 `rawVlmOutput` 必填 + `replaceExisting` 可选（true 会**先清空旧持仓**，
/// 不可逆 —— 权限闸 workspace-write 在宿主分派层）。出参是宿主核心的结果对象
/// （`{"success", "imported", "failed", "stockCodes", "errors"}`）整块透传。
fn import_portfolio_from_vlm(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    let raw_vlm_output = command_str_arg(input, "rawVlmOutput")?;
    let mut request = serde_json::Map::new();
    request.insert("raw_vlm_output".to_string(), json!(raw_vlm_output));
    if let Some(replace) = input.get("replaceExisting").and_then(Value::as_bool) {
        request.insert("replace_existing".to_string(), json!(replace));
    }
    call_trading_seam(
        peer,
        call_chain,
        "import_portfolio_from_vlm",
        json!({ "request": Value::Object(request) }),
    )
}

/// 组合压力测试（宿主 `run_portfolio_stress_test`）：纯计算 op，N+1 网络取数在宿主跑。
/// ⚠ op 名 `run_stress_test` ≠ 命令名（宿主命令带 `portfolio`，trait 方法不带）。
fn run_portfolio_stress_test(
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    call_trading_seam(peer, call_chain, "run_stress_test", json!({}))
}

// ── F6 别名族（#35 / #322）：走 `stock.analysis` 新 op，别名形状逐字保留 ──
// 六 op 的宿主 provider 在 `stock_analysis_service.rs`（权威实现复用无 State 核心）；
// 接缝键 snake_case（provider 头注释同口径）。别名命令与宿主 IPC 面的键名差异只做
// camelCase 平铺（本文件惯例，与宿主 request 嵌层形状无关 —— inputSchema 是权威）。

/// 触发分析师节点自我进化（别名：硬编码 NodeType::Analyst）—— **写命令**。
fn evolve_analyst_via_seam(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    let analyst_id = command_str_arg(input, "analystId")?;
    call_analysis_seam(peer, call_chain, "evolve_analyst", json!({ "analyst_id": analyst_id }))
}

/// 分析师节点进化状态查询（别名，不触发进化）—— 读命令。
fn fetch_analyst_evolution_status(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    let analyst_id = command_str_arg(input, "analystId")?;
    call_analysis_seam(
        peer,
        call_chain,
        "get_analyst_evolution_status",
        json!({ "analyst_id": analyst_id }),
    )
}

/// 通用节点进化状态查询（泛化形：nodeType + nodeId，未知值宿主兜 Other）—— 读命令。
fn fetch_node_evolution_status(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    let node_type = command_str_arg(input, "nodeType")?;
    let node_id = command_str_arg(input, "nodeId")?;
    call_analysis_seam(
        peer,
        call_chain,
        "get_node_evolution_status",
        json!({ "node_type": node_type, "node_id": node_id }),
    )
}

/// 保存分析师反馈（别名：13 字段无 node_type，宿主按 Analyst 补）—— **写命令**。
///
/// 命令面 13 字段 camelCase（可选三键 `bullScore` / `bearScore` / `confidence` 存在才
/// 透传，缺省不插键 —— 宿主侧 serde `default` 收 `None`）；其余 12 键必填。
fn save_analyst_feedback(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    let mut request = serde_json::Map::new();
    request.insert("analyst_id".to_string(), json!(command_str_arg(input, "analystId")?));
    request.insert("report_id".to_string(), json!(command_str_arg(input, "reportId")?));
    request.insert("stock_code".to_string(), json!(command_str_arg(input, "stockCode")?));
    request.insert("execution_id".to_string(), json!(command_str_arg(input, "executionId")?));
    request
        .insert("quality_score".to_string(), json!(command_i64_arg(input, "qualityScore")? as i32));
    request.insert("grade".to_string(), json!(command_str_arg(input, "grade")?));
    request.insert("issue_count".to_string(), json!(command_i64_arg(input, "issueCount")? as i32));
    request
        .insert("warning_count".to_string(), json!(command_i64_arg(input, "warningCount")? as i32));
    request.insert("good_count".to_string(), json!(command_i64_arg(input, "goodCount")? as i32));
    request.insert("checks_json".to_string(), json!(command_str_arg(input, "checksJson")?));
    if let Some(v) = input.get("bullScore").and_then(Value::as_f64) {
        request.insert("bull_score".to_string(), json!(v));
    }
    if let Some(v) = input.get("bearScore").and_then(Value::as_f64) {
        request.insert("bear_score".to_string(), json!(v));
    }
    if let Some(v) = input.get("confidence").and_then(Value::as_f64) {
        request.insert("confidence".to_string(), json!(v));
    }
    request
        .insert("score_consistent".to_string(), json!(command_bool_arg(input, "scoreConsistent")?));
    request.insert(
        "direction_consistent".to_string(),
        json!(command_bool_arg(input, "directionConsistent")?),
    );
    call_analysis_seam(peer, call_chain, "save_analyst_feedback", Value::Object(request))
}

/// 分析师反馈历史（别名：单键 analystId 必填 + limit / onlyIssues 可选）—— 读命令。
fn fetch_analyst_feedbacks(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    let mut request = serde_json::Map::new();
    request.insert("analyst_id".to_string(), json!(command_str_arg(input, "analystId")?));
    if let Some(v) = input.get("limit").and_then(Value::as_u64) {
        request.insert("limit".to_string(), json!(v));
    }
    if let Some(v) = input.get("onlyIssues").and_then(Value::as_bool) {
        request.insert("only_issues".to_string(), json!(v));
    }
    call_analysis_seam(peer, call_chain, "get_analyst_feedbacks", Value::Object(request))
}

/// 分析师反馈统计（别名：含 needs_evolution 判定，阈值在宿主核心）—— 读命令。
fn fetch_analyst_feedback_stats(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    let analyst_id = command_str_arg(input, "analystId")?;
    call_analysis_seam(
        peer,
        call_chain,
        "get_analyst_feedback_stats",
        json!({ "analyst_id": analyst_id }),
    )
}

// ── #323 T2 五命令批迁（子批②③④）：新 op 三条 ──
// 三条实现体都在宿主进程内（token 缓存 / `stock_analyses` 表 / quant 配置解析），
// 插件只做命令面 camelCase → 接缝面 snake_case 的平铺透传。

/// 保存 NeoData token（宿主 `save_neodata_token`）—— **写命令**。
///
/// 命令面 `token` 必填非空（宿主 provider 拒空串）；出参恒 `null`
/// （宿主命令回 `()` 同形）。三段效应（脚本缓存 / 共享内存 / 模板变量）在宿主。
fn save_neodata_token(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    let token = command_str_arg(input, "token")?;
    call_analysis_seam(peer, call_chain, "save_neodata_token", json!({ "token": token }))
}

/// 查询决策回测分析（宿主 `query_decision_backtest`）—— 读命令。
///
/// 命令面 `limit` 可选（存在才透传，缺省 100 / 宿主钳 500）；出参 = 手拼投影数组
/// 整块透传（`stockCode` / `llmAction` / `decisionConfidence` 等，形状权威在宿主）。
fn query_decision_backtest(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    let mut request = serde_json::Map::new();
    if let Some(v) = input.get("limit").and_then(Value::as_u64) {
        request.insert("limit".to_string(), json!(v));
    }
    call_analysis_seam(peer, call_chain, "query_decision_backtest", Value::Object(request))
}

/// 运行组合回测——配置校验面（宿主 `portfolio_backtest_run`）—— 读命令。
///
/// ⚠ op 名 `validate_portfolio_config` ≠ 命令名（命令逐字续宿主名，op 跟
/// trading.engine trait 方法，#322 `run_stress_test` 同款）。命令面 `configJson`
/// 必填（camelCase，禁区 13，与前端 `invoke` 传参一致）；层内键 snake_case，
/// 字符串**原样**透传 —— 它是 `quant::PortfolioConfig` 反序列化形状的唯一权威，
/// 插件侧不拆不验。
fn portfolio_backtest_run(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    let config_json = command_str_arg(input, "configJson")?;
    call_trading_seam(
        peer,
        call_chain,
        "validate_portfolio_config",
        json!({ "request": { "config_json": config_json } }),
    )
}

// ── #324 T2 F7 回测验证三条迁：新 op 三条 ──
// 三条实现体都在宿主进程内（decision_validations / reco_picks 表 + 行情 API 回放），
// 插件只做命令面 camelCase → 接缝面 `{"request": {...}}` 的平铺透传。

/// 跑决策回测（宿主 `run_decision_backtest`）—— **写命令**。
///
/// 命令面六键全可选（camelCase，形状权威在宿主 `RunDecisionBacktestRequest` 的
/// serde 结构，`default` 语义传空对象即全默认）；出参 = `RunDecisionBacktestResponse`
/// 序列化整块透传。执行体留宿主（cron 与 CLI 共用），插件只触发。
fn run_decision_backtest(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    call_analysis_seam(peer, call_chain, "run_decision_backtest", input.clone())
}

/// 列出决策验证记录（宿主 `list_decision_validations`）—— 读命令。
///
/// 命令面五键全可选（存在才透传）；出参 = `DecisionValidationItem` 数组整块透传。
fn list_decision_validations(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    let mut request = serde_json::Map::new();
    if let Some(v) = input.get("stockCode").and_then(Value::as_str) {
        request.insert("stockCode".to_string(), json!(v));
    }
    if let Some(v) = input.get("hitOutcome").and_then(Value::as_str) {
        request.insert("hitOutcome".to_string(), json!(v));
    }
    if let Some(v) = input.get("limit").and_then(Value::as_u64) {
        request.insert("limit".to_string(), json!(v));
    }
    if let Some(v) = input.get("offset").and_then(Value::as_u64) {
        request.insert("offset".to_string(), json!(v));
    }
    if let Some(v) = input.get("generation").and_then(Value::as_str) {
        request.insert("generation".to_string(), json!(v));
    }
    call_analysis_seam(peer, call_chain, "list_decision_validations", Value::Object(request))
}

/// 计算验证报告（宿主 `compute_validation_report`）—— 读命令。
///
/// 命令面 `generation` 可选（某次回放的截止日；缺省 = 实盘出票一代）；出参 =
/// `ValidationReportResponse` 序列化整块透传。
fn compute_validation_report(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    let mut request = serde_json::Map::new();
    if let Some(v) = input.get("generation").and_then(Value::as_str) {
        request.insert("generation".to_string(), json!(v));
    }
    call_analysis_seam(peer, call_chain, "compute_validation_report", Value::Object(request))
}

// ── #326 F3 首刀：分析与推荐产出四条，零新 op ──

/// 分析情绪深度（宿主 `analyze_sentiment_depth` 同义迁移）—— **纯本地**。
///
/// 入参三键必填（camelCase）：`stockCode` / `stockName` / `historyJson`
/// （`Vec<SentimentSnapshot>` 的 JSON 字符串；解析失败报 `解析历史数据失败: {e}`，
/// 与宿主命令体同文案）。出参 = `SentimentReport` 序列化整块。
fn analyze_sentiment_depth(input: &Value) -> Result<Value, (String, String)> {
    let stock_code = input.get("stockCode").and_then(Value::as_str).ok_or_else(|| {
        (error_codes::SEAM_CALL_INVALID_ARGS.to_string(), "缺 `stockCode`".to_string())
    })?;
    let stock_name = input.get("stockName").and_then(Value::as_str).ok_or_else(|| {
        (error_codes::SEAM_CALL_INVALID_ARGS.to_string(), "缺 `stockName`".to_string())
    })?;
    let history_json = input.get("historyJson").and_then(Value::as_str).ok_or_else(|| {
        (error_codes::SEAM_CALL_INVALID_ARGS.to_string(), "缺 `historyJson`".to_string())
    })?;
    let history: Vec<axagent_analysis_engine::sentiment_analysis::SentimentSnapshot> =
        serde_json::from_str(history_json).map_err(|e| {
            (error_codes::SEAM_CALL_INVALID_ARGS.to_string(), format!("解析历史数据失败: {e}"))
        })?;
    let report = axagent_analysis_engine::sentiment_analysis::analyze_sentiment(
        stock_code, stock_name, &history,
    );
    serde_json::to_value(report)
        .map_err(|e| (error_codes::PLUGIN_CALL_FAILED.to_string(), format!("报告序列化失败: {e}")))
}

/// 获取因子分析结果（宿主 `factor_analysis_list` 同义迁移）—— 纯本地，零参数。
///
/// 映射逐字照搬宿主命令体：`category` 走 `format!("{:?}", …)`（枚举变体名 Debug
/// 形态；同 crate 同输出，因子清单口径不漂移）。
fn factor_analysis_list() -> Result<Value, (String, String)> {
    let registry = axagent_analysis_engine::factor_analysis::FactorRegistry::new();
    let factors = registry.all_factors();
    let items: Vec<Value> = factors
        .iter()
        .map(|f| {
            json!({
                "id": f.id,
                "name": f.name,
                "category": format!("{:?}", f.category),
                "higherIsBetter": f.higher_is_better,
                "defaultWeight": f.default_weight,
                "enabled": f.enabled,
            })
        })
        .collect();
    Ok(Value::Array(items))
}

/// 取行情 + 财务（`market.quote::get_quote` / `get_financials`，接缝参数 snake_case）。
///
/// 反序列化目标 = `astock-data::types::{StockQuote, FinancialReport}`——#81 上收后
/// 它们就是 harness 同一类型（astock-data `types.rs` re-export），seam 出参零转换。
fn fetch_quote_and_financials(
    stock_code: &str,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<
    (axagent_astock_data::types::StockQuote, Vec<axagent_astock_data::types::FinancialReport>),
    (String, String),
> {
    let quote_raw =
        fetch_market_batch("get_quote", json!({ "stock_code": stock_code }), call_chain, peer)?;
    let quote: axagent_astock_data::types::StockQuote =
        serde_json::from_value(quote_raw).map_err(|e| {
            (error_codes::PLUGIN_CALL_FAILED.to_string(), format!("行情出参形状异常: {e}"))
        })?;
    let financials_raw = fetch_market_batch(
        "get_financials",
        json!({ "stock_code": stock_code }),
        call_chain,
        peer,
    )?;
    let financials: Vec<axagent_astock_data::types::FinancialReport> =
        serde_json::from_value(financials_raw).map_err(|e| {
            (error_codes::PLUGIN_CALL_FAILED.to_string(), format!("财务出参形状异常: {e}"))
        })?;
    Ok((quote, financials))
}

/// 生成基本面分析报告（宿主 `generate_fundamentals_report` 同义迁移）—— 读命令。
///
/// 入参：`stockCode` 必填、`includeMarkdown` 可选（缺省 `true`，与宿主
/// `unwrap_or(true)` 一致）；出参 = 宿主 `FundamentalsReportEnvelope` 同形
/// `{report, markdown}`（`markdown` 缺省时为 `null`）。
fn generate_fundamentals_report(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    let stock_code = input.get("stockCode").and_then(Value::as_str).ok_or_else(|| {
        (error_codes::SEAM_CALL_INVALID_ARGS.to_string(), "缺 `stockCode`".to_string())
    })?;
    let include_md = input.get("includeMarkdown").and_then(Value::as_bool).unwrap_or(true);
    let (quote, financials) = fetch_quote_and_financials(stock_code, call_chain, peer)?;
    let report = axagent_astock_data::fundamentals_report::FundamentalsAnalyzer::generate(
        stock_code,
        &quote,
        &financials,
    );
    let markdown = if include_md {
        Some(report.to_markdown())
    } else {
        None
    };
    serde_json::to_value(json!({ "report": report, "markdown": markdown }))
        .map_err(|e| (error_codes::PLUGIN_CALL_FAILED.to_string(), format!("报告序列化失败: {e}")))
}

/// 仅获取基本面报告 Markdown（宿主 `get_fundamentals_report_markdown` 同义迁移）——
/// 读命令，轻量变体：只回 `to_markdown()` 字符串，不带结构。
fn get_fundamentals_report_markdown(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<Value, (String, String)> {
    let stock_code = input.get("stockCode").and_then(Value::as_str).ok_or_else(|| {
        (error_codes::SEAM_CALL_INVALID_ARGS.to_string(), "缺 `stockCode`".to_string())
    })?;
    let (quote, financials) = fetch_quote_and_financials(stock_code, call_chain, peer)?;
    let report = axagent_astock_data::fundamentals_report::FundamentalsAnalyzer::generate(
        stock_code,
        &quote,
        &financials,
    );
    Ok(json!(report.to_markdown()))
}

/// 分析时间戳（UNIX 秒）：与宿主原命令的 `chrono::Utc::now().timestamp()` 同量纲。
/// 本 crate 不依赖 chrono ⇒ 用标准库取同一个秒数，不为一个时间戳引第二个时间库。
fn unix_now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        analyze_sentiment_depth, batch_financials_seam_args, batch_quotes_seam_args,
        command_str_arg, cron_seam_op, factor_analysis_list, kline_route, kline_seam_args,
        map_analysis_state_to_run_state, map_execution_state_to_run_state, market_read_route,
        parse_string_array, search_news_seam_args, stock_analysis_request,
        task_run_row_from_analysis_record, task_run_row_from_execution, unix_now_secs,
        watch_priority_arg,
    };
    use axagent_plugin_proto::{PLUGIN_TASK_RUN_STATES, error_codes, is_plugin_task_run_state};
    use serde_json::{Value, json};

    /// 引擎视图 → 行记录：键必须 camelCase，缺省端点必须**不出现**（不是 `null`）。
    ///
    /// 钉这条是因为宿主泛型适配器按「字段在不在」决定要不要显示耗时/是否可打开；
    /// `null` 与「缺省」在 `Option` 反序列化上是两种形状，手搓 `json!` 极易写错。
    #[test]
    fn task_run_row_uses_camel_case_and_omits_absent_endpoints() {
        let execution = json!({
            "executionId": "exec-7",
            "workflowId": "wf-daily",
            "status": "running",
            "currentNodeId": "node-2",
            "ownerPluginId": "stock",
            "createdAt": 1_700_000_000_000_i64,
            "updatedAt": 1_700_000_005_000_i64,
        });
        let row = task_run_row_from_execution(&execution);

        assert_eq!(row["nativeId"], "exec-7");
        assert_eq!(row["name"], "wf-daily · exec-7", "同一 workflow 多条执行 ⇒ 名字必须可区分");
        assert_eq!(row["state"], "running");
        assert_eq!(row["trigger"], "manual");
        assert_eq!(row["startedAt"], 1_700_000_000_000_i64);
        assert_eq!(row["raw"]["currentNodeId"], "node-2", "引擎视图原样留在 raw 供详情");
        for absent in ["started_at", "finishedAt", "durationMs", "deepLink"] {
            // `started_at`（蛇形）在列是专门盯 `rename_all = "camelCase"` 漏掉的情形。
            assert!(row.get(absent).is_none(), "缺省字段 / 蛇形键 `{absent}` 不得进帧：{row}");
        }
    }

    /// 终态行：宿主的 `finishedAt` 必须原样透出（耗时列的唯一依据），但仍不交 `durationMs`。
    ///
    /// 这一条是 #150 的正向断言 —— `list_recent` 之前插件永远拿不到 `finishedAt`，
    /// 「不出现」是当时的正确形状；现在它出现而 `durationMs` 不出现，才说明两端点算耗时
    /// 这件事只发生在宿主泛型适配器一处（`resolveDurationMs`），插件没造第二套口径。
    #[test]
    fn task_run_row_passes_host_finished_at_but_not_duration() {
        let row = task_run_row_from_execution(&json!({
            "executionId": "exec-0",
            "workflowId": "wf-daily",
            "status": "completed",
            "createdAt": 1_699_999_990_000_i64,
            "updatedAt": 1_699_999_999_000_i64,
            "finishedAt": 1_699_999_999_000_i64,
        }));
        assert_eq!(row["state"], "succeeded");
        assert_eq!(row["finishedAt"], 1_699_999_999_000_i64);
        assert!(row.get("durationMs").is_none(), "耗时由宿主按两端点算：{row}");
        assert!(row.get("deepLink").is_none(), "没有站内路由就不交深链：{row}");

        // 在跑行没有该键 ⇒ 不能凭 `updatedAt` 造一个结束时刻
        let running = task_run_row_from_execution(&json!({
            "executionId": "exec-1",
            "workflowId": "wf-daily",
            "status": "running",
            "createdAt": 1_700_000_000_000_i64,
            "updatedAt": 1_700_000_005_000_i64,
        }));
        assert!(running.get("finishedAt").is_none(), "在跑行的耗时是未知，不是 0：{running}");
    }

    /// `createdAt` 缺失 ⇒ `startedAt` 缺省而非 `null`；宿主按「未知」不显示，不编一个 0 点。
    #[test]
    fn task_run_row_drops_started_at_when_engine_view_lacks_created_at() {
        let row = task_run_row_from_execution(&json!({ "executionId": "e", "workflowId": "w" }));
        assert!(row.get("startedAt").is_none(), "缺 createdAt 应产出缺省而非 null：{row}");
        // 表外状态（这里是「没有 status」）落 waiting，不猜成 running。
        assert_eq!(row["state"], "waiting");
        assert_eq!(row["nativeId"], "e", "有 id 时照常透出，不因缺其它字段而丢主键");
    }

    /// 状态翻译：六态逐条对齐七态词表，表外值不得被判成合法状态。
    #[test]
    fn execution_status_maps_into_task_center_vocabulary_only() {
        let cases = [
            ("running", "running"),
            ("paused", "paused"),
            ("completed", "succeeded"),
            // 终态且确有产出 ⇒ 判成 succeeded；判 failed 会把「部分节点没成」误标成失败。
            ("partially_completed", "succeeded"),
            ("failed", "failed"),
            // 引擎用英式 cancelled，中心用美式 canceled：拼法差异必须在插件侧翻掉。
            ("cancelled", "canceled"),
        ];
        for (from, to) in cases {
            let mapped = map_execution_state_to_run_state(from);
            assert_eq!(mapped, to, "`{from}` 应翻成 `{to}`");
            assert!(is_plugin_task_run_state(mapped), "映射产出 `{mapped}` 不在词表内");
        }
        // 表外值（含大小写/拼法漂移）一律 waiting，且词表本身没被改动。
        for unknown in ["", "Running", "canceled ", "created", "unknown"] {
            assert_eq!(map_execution_state_to_run_state(unknown), "waiting");
        }
        assert_eq!(PLUGIN_TASK_RUN_STATES.len(), 7);
    }

    /// 行记录必须是宿主可 `JSON.parse` 的纯数组元素：本测试锁住「命令返回数组」这一形状。
    #[test]
    fn task_run_rows_are_plain_json_objects_for_frontend_parsing() {
        let rows: Vec<Value> =
            [json!({ "executionId": "a", "workflowId": "w", "status": "paused" })]
                .iter()
                .map(task_run_row_from_execution)
                .collect();
        let text = serde_json::to_string(&rows).expect("行数组应可序列化");
        let back: Vec<Value> = serde_json::from_str(&text).expect("宿主 JSON.parse 读得回");
        assert_eq!(back[0]["state"], "paused");
        assert!(back[0].is_object(), "元素必须是对象，宿主泛型适配器按字段取值");
    }

    /// 单股分析记录的状态翻译（#149 第 3 步：宿主 `stockAnalysis.adapter.ts` 删除后，
    /// 这张表是唯一权威 ⇒ 原测试断言搬到这里，不再留在前端）。
    ///
    /// 覆盖点逐条来自被删的 `mapAnalysisState`：大小写不敏感（宿主列是自由文本）、
    /// 七态对齐、以及**表外值落 failed 而不是 running**（默认成 running 会让僵死记录
    /// 一直显示「运行中」并持续轮询）。
    #[test]
    fn analysis_status_maps_into_task_center_vocabulary() {
        let cases = [
            ("running", "running"),
            ("RUNNING", "running"),
            ("completed", "succeeded"),
            ("failed", "failed"),
            ("error", "failed"),
            ("interrupted", "canceled"),
            ("canceled", "canceled"),
            ("cancelled", "canceled"),
            ("paused", "paused"),
            ("queued", "queued"),
            ("pending", "queued"),
            // 表外值：不得默认成 running
            ("weird", "failed"),
            ("", "failed"),
        ];
        for (from, to) in cases {
            let mapped = map_analysis_state_to_run_state(from);
            assert_eq!(mapped, to, "`{from}` 应翻成 `{to}`");
            assert!(is_plugin_task_run_state(mapped), "映射产出 `{mapped}` 不在七态词表内");
        }
    }

    /// 结束时刻只认 `completedAt`，且只在终态透出；`updatedAt` 是编辑时刻，不得冒充完成时刻。
    ///
    /// 现网 65% 的 completed 行 `updatedAt` 被交易意图审核 / 结果回写顶掉（可达数天），
    /// 拿它算耗时会显示「跑了好几天」——这条断言就是那个坑的钉子。
    #[test]
    fn analysis_row_uses_completed_at_only_and_drops_updated_at() {
        let completed = task_run_row_from_analysis_record(&json!({
            "id": "a1",
            "stockCode": "300308",
            "stockName": "中际旭创",
            "status": "completed",
            "createdAt": 1_000_i64,
            "updatedAt": 4_000_i64,
            "completedAt": 4_000_i64,
            "parentAnalysisId": null,
        }));
        assert_eq!(completed["state"], "succeeded");
        assert_eq!(completed["startedAt"], 1_000_i64);
        assert_eq!(completed["finishedAt"], 4_000_i64);

        // completed 但 completedAt 缺失 ⇒ 结束时刻未知（不是拿 updatedAt 顶）
        let no_completed_at = task_run_row_from_analysis_record(&json!({
            "id": "a2",
            "stockCode": "300308",
            "stockName": "中际旭创",
            "status": "completed",
            "createdAt": 1_000_i64,
            "updatedAt": 9_999_999_i64,
            "completedAt": null,
        }));
        assert!(
            no_completed_at.get("finishedAt").is_none(),
            "不得用 updatedAt 冒充：{no_completed_at}"
        );

        // interrupted：算 canceled，且回收时刻不是完成时刻
        let interrupted = task_run_row_from_analysis_record(&json!({
            "id": "a3",
            "status": "interrupted",
            "createdAt": 1_000_i64,
            "updatedAt": 9_999_999_i64,
        }));
        assert_eq!(interrupted["state"], "canceled");
        assert!(interrupted.get("finishedAt").is_none(), "回收时刻不得当完成时刻：{interrupted}");

        // 在跑行：只有开始时间
        let running = task_run_row_from_analysis_record(&json!({
            "id": "a4",
            "status": "running",
            "createdAt": 1_000_i64,
        }));
        assert_eq!(running["state"], "running");
        assert_eq!(running["startedAt"], 1_000_i64);
        assert!(running.get("finishedAt").is_none());
    }

    /// 行形状：名字不带宿主 i18n 前缀、触发来源一律 manual、不交 `deepLink`，
    /// 记录按详情 schema 的节点 id 收进 `raw.rows`（`stock-analysis-detail` 的 Table）。
    ///
    /// 后端 adapter 删除后，「打开」按钮随 `deepLink` 一起消失 —— 这条**正向**钉住
    /// 「不交深链」是当前裁定，而不是漏字段（改裁定要同时改这里与 PLAN §101.5-2）。
    #[test]
    fn analysis_row_shape_and_detail_projection() {
        let record = json!({
            "id": "a5",
            "stockCode": "300308",
            "stockName": "中际旭创",
            "analysisDate": "2026-11-05",
            "status": "running",
            "createdAt": 1_000_i64,
            "updatedAt": 2_000_i64,
            "completedAt": null,
            "analysisKind": "live",
            "parentAnalysisId": null,
        });
        let row = task_run_row_from_analysis_record(&record);

        assert_eq!(row["nativeId"], "a5");
        assert_eq!(row["name"], "300308 中际旭创", "名字由插件拼，不带宿主 i18n 前缀");
        assert_eq!(row["trigger"], "manual", "记录表无触发来源列 ⇒ 与宿主原 adapter 同口径");
        assert!(row.get("deepLink").is_none(), "插件不交深链：{row}");
        assert!(row.get("durationMs").is_none(), "耗时只由宿主按两端点算：{row}");

        let rows = row["raw"]["rows"].as_array().expect("详情投影必须是 rows 数组");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["analysisDate"], "2026-11-05", "记录字段原样保留，不裁剪");
        assert_eq!(rows[0]["analysisKind"], "live");
        assert_eq!(rows[0], record, "投影只包一层数组，不改任何键");
    }

    /// 接缝载荷**逐字透传**：形状权威在 `analysis-engine` 的 DTO，插件侧任何「整理」
    /// 都是第二套形状。这里钉三个容易被顺手改掉的点：snake_case 原样保留、
    /// `archive_missing` 不被补默认值、缺键仍然缺键（补 `null` 会让 `serde` 走到另一分支）。
    #[test]
    fn analysis_request_passes_payload_through_verbatim() {
        let input = json!({
            "mainline_date": "2026-11-09",
            "mainlines": [{ "theme": "AI 算力", "narrative": "测试" }],
        });
        let request = stock_analysis_request(&input);
        assert_eq!(request, input, "载荷必须逐字透传");
        assert!(request.get("archive_missing").is_none(), "缺键不得被补成 false：{request}");
        assert!(request.get("mainlineDate").is_none(), "不得做 snake→camel 改名：{request}");
        assert_eq!(request["mainline_date"], "2026-11-09", "snake_case 必须原样保留");
    }

    /// 出站准入：`calls` 少写一条，宿主 `handle_call_seam` 会在运行时拒掉（`SEAM_CALL_NOT_DECLARED`）。
    /// 这条钉住「声明面与实现面同批」，避免命令臂写了而声明忘了。
    #[test]
    fn declaration_declares_stock_analysis_seam() {
        let response = super::declaration();
        let value = response.value.expect("describe 必须带回声明体");
        let calls: Vec<&str> = value["calls"]
            .as_array()
            .expect("calls 必须是数组")
            .iter()
            .map(|v| v.as_str().unwrap_or_default())
            .collect();
        assert!(
            calls.contains(&super::STOCK_ANALYSIS_SEAM),
            "`calls` 未声明 `{}`：三个 E 批命令会在宿主侧被拒。实际: {calls:?}",
            super::STOCK_ANALYSIS_SEAM
        );
        assert!(
            calls.contains(&super::QUOTE_WATCHER_SEAM),
            "`calls` 未声明 `{}`：`stock_list_watched_quotes` 会在宿主侧被拒。实际: {calls:?}",
            super::QUOTE_WATCHER_SEAM
        );
        assert!(
            calls.contains(&super::TRADING_ENGINE_SEAM),
            "`calls` 未声明 `{}`：#311 两条交易面命令会在宿主侧被拒。实际: {calls:?}",
            super::TRADING_ENGINE_SEAM
        );
    }

    /// 批 1a（#35）十二 cron 命令 → 接缝 op 的映射必须**逐条**正确且互不重复。
    ///
    /// 分派用表驱动（`cron_seam_op`），复制粘贴错一行不会有任何编译错误：命令照样
    /// 回成功，只是打到了另一族的任务表上（例如 `delete_pool_scan_cron` 删掉荐股任务）。
    /// 因此这里既锁「命令 → op」逐字对应，也锁「十二个 op 互不相同」。
    #[test]
    fn cron_command_whitelist_maps_each_command_to_its_own_op() {
        let cases = [
            ("stock_create_recommendation_cron", "create_recommendation_cron"),
            ("stock_list_recommendation_crons", "list_recommendation_crons"),
            ("stock_toggle_recommendation_cron", "toggle_recommendation_cron"),
            ("stock_delete_recommendation_cron", "delete_recommendation_cron"),
            ("stock_create_trend_screening_cron", "create_trend_screening_cron"),
            ("stock_list_trend_screening_crons", "list_trend_screening_crons"),
            ("stock_toggle_trend_screening_cron", "toggle_trend_screening_cron"),
            ("stock_delete_trend_screening_cron", "delete_trend_screening_cron"),
            ("stock_create_pool_scan_cron", "create_pool_scan_cron"),
            ("stock_list_pool_scan_crons", "list_pool_scan_crons"),
            ("stock_toggle_pool_scan_cron", "toggle_pool_scan_cron"),
            ("stock_delete_pool_scan_cron", "delete_pool_scan_cron"),
        ];
        let mut ops: Vec<&str> = Vec::new();
        for (command, expected) in cases {
            let actual = cron_seam_op(command).unwrap_or_else(|| panic!("`{command}` 未映射"));
            assert_eq!(actual, expected, "`{command}` 必须打到自己的 op");
            ops.push(actual);
        }
        ops.sort_unstable();
        ops.dedup();
        assert_eq!(ops.len(), cases.len(), "十二个命令必须映射到十二个不同 op：{ops:?}");
        assert_eq!(cron_seam_op("stock_unknown_cron"), None, "未知命令不得落到任何 op");
    }

    /// 批 2a（#35）十一条 + 批 2b quote + 批 2c（#262）五条榜单/盘面命令 +
    /// 候选 A 六条行情资讯直映命令，共二十三条行情只读命令 → (`market.quote` op, 入参键)
    /// 必须逐条正确。
    ///
    /// 同 `cron_seam_op` 的理由：表里错一行不会有任何编译错误，命令照样回成功，
    /// 只是把「财务数据」打到了「同行业对比」的 op 上。入参键也一并锁住 ——
    /// camel/snake 写反时宿主 `deserialize_arg` 会报缺参，但只有这条测试能指出是哪一格。
    #[test]
    fn market_read_commands_map_to_their_own_ops_and_keys() {
        let stock = Some(("stockCode", "stock_code"));
        let keyword = Some(("keyword", "keyword"));
        let cases = [
            ("stock_get_quote", "get_quote", stock, None),
            ("stock_get_research_reports", "get_research_reports", stock, None),
            ("stock_get_concept_blocks", "get_concept_blocks", stock, None),
            ("stock_get_announcements", "get_announcements", stock, None),
            ("stock_get_earnings_calendar", "get_earnings_calendar", stock, None),
            (
                "stock_get_concept_board_members",
                "get_concept_board_members",
                Some(("boardCode", "board_code")),
                None,
            ),
            ("stock_get_cls_flash", "get_cls_flash", None, None),
            ("stock_get_peers", "get_peers", stock, None),
            ("stock_get_option_pcr", "get_option_pcr", stock, None),
            ("stock_get_lockup_schedule", "get_lockup_schedule", stock, None),
            ("stock_get_dividend_records", "get_dividend_records", stock, None),
            ("stock_get_financials", "get_financials", stock, None),
            // 批 2c（#262）：五条全市场榜单/盘面命令，零入参 ⇒ keys = None；
            // `stock_get_index_quotes` 的 op 带 `_outcome` 后缀（与命令名不同源），逐条钉死。
            ("stock_get_hot_stocks", "get_hot_stocks", None, None),
            ("stock_get_industry_ranking", "get_industry_ranking", None, None),
            ("stock_get_index_quotes", "get_index_quotes_outcome", None, None),
            ("stock_get_market_dragon_tiger", "get_market_dragon_tiger", None, None),
            ("stock_get_north_bound_flow", "get_north_bound_flow", None, None),
            // 候选 A（#35 行情资讯直映批）：五条单必填键 + 一条可选键。
            // `stock_get_international_stock_quote` 的 op 是 `get_international_quote`
            // （命令名跟宿主 Tauri 命令，op 跟接缝协议），`stock_get_consensus_eps` 的 op
            // 是 `get_consensus_eps`（宿主命令名带 stock_ 中缀），两套名并存逐条钉死；
            // `stock_get_limit_up_pool` 的宿主参数名 `date` ≠ 接缝参数名 `requested_date`。
            ("stock_search_stock", "search_stock", keyword, None),
            ("stock_search_concept_boards", "search_concept_boards", keyword, None),
            ("stock_get_social_sentiment", "get_social_sentiment", stock, None),
            ("stock_get_international_stock_quote", "get_international_quote", stock, None),
            ("stock_get_consensus_eps", "get_consensus_eps", stock, None),
            (
                "stock_get_limit_up_pool",
                "get_limit_up_pool",
                None,
                Some(("date", "requested_date")),
            ),
        ];
        let mut ops: Vec<&str> = Vec::new();
        for (command, expected_op, expected_keys, expected_optional) in cases {
            let route = market_read_route(command).unwrap_or_else(|| panic!("`{command}` 未映射"));
            assert_eq!(route.op, expected_op, "`{command}` 必须打自己的 op");
            assert_eq!(route.keys, expected_keys, "`{command}` 的必填入参键不对");
            assert_eq!(route.optional, expected_optional, "`{command}` 的可选入参键不对");
            ops.push(route.op);
        }
        ops.sort_unstable();
        ops.dedup();
        assert_eq!(ops.len(), cases.len(), "二十三条命令必须映射到二十三个不同 op：{ops:?}");

        assert!(market_read_route("stock_unknown_market_read").is_none(), "未知命令不得落到行情表");
        assert!(
            market_read_route("stock_list_recommendation_crons").is_none(),
            "cron 命令不得被行情表吞掉"
        );
        assert!(
            market_read_route("stock_get_kline").is_none(),
            "K 线走 kline_route 表，不得被单标的路由表吞掉"
        );
    }

    /// K 线族四条命令（批 2b A 股 + 候选 A 国际/外汇/基准）→ (op, 标的键, 是否翻译复权档)
    /// 必须逐条正确。国际/外汇/基准的宿主命令不收复权参 ⇒ `with_adj = false`；
    /// 国际 K 线命令名跟宿主（单数）而 op 跟接缝协议（复数），两套名并存逐条钉死。
    #[test]
    fn kline_commands_map_to_their_own_ops_and_asset_keys() {
        let cases = [
            ("stock_get_kline", "get_klines", ("stockCode", "stock_code"), true),
            (
                "stock_get_international_stock_kline",
                "get_international_klines",
                ("stockCode", "stock_code"),
                false,
            ),
            ("stock_get_forex_kline", "get_forex_klines", ("pair", "pair"), false),
            (
                "stock_get_benchmark_kline",
                "get_benchmark_klines",
                ("benchmarkCode", "benchmark_code"),
                false,
            ),
        ];
        let mut ops: Vec<&str> = Vec::new();
        for (command, expected_op, expected_asset, expected_adj) in cases {
            let route = kline_route(command).unwrap_or_else(|| panic!("`{command}` 未映射"));
            assert_eq!(route.op, expected_op, "`{command}` 必须打自己的 op");
            assert_eq!(route.asset, expected_asset, "`{command}` 的标的键不对");
            assert_eq!(route.with_adj, expected_adj, "`{command}` 的复权档开关不对");
            ops.push(route.op);
        }
        ops.sort_unstable();
        ops.dedup();
        assert_eq!(ops.len(), cases.len(), "四条命令必须映射到四个不同 op：{ops:?}");
        assert!(kline_route("stock_unknown_kline").is_none(), "未知命令不得落到 K 线表");
        assert!(kline_route("stock_get_quote").is_none(), "单标的行情命令不得被 K 线表吞掉");
    }

    #[test]
    fn kline_seam_args_translates_camel_input_and_rejects_bad_arms() {
        let full = kline_seam_args(
            &json!({
                "stockCode": " 600519 ",
                "period": "daily",
                "limit": 250,
                "adjType": "forward"
            }),
            ("stockCode", "stock_code"),
            true,
        )
        .expect("完整入参应能翻成接缝载荷");
        assert_eq!(full["stock_code"], "600519", "标的键要翻成 snake 并 trim");
        assert_eq!(full["period"], "daily");
        assert_eq!(full["limit"], 250);
        assert_eq!(full["adj_type"], "forward");
        // 命令面键不得泄漏到接缝载荷里。
        assert!(full.get("stockCode").is_none() && full.get("adjType").is_none());

        // `auto` / 缺省 / `null` 三档都落「未复权」= 接缝侧 `adj_type: null`。
        for input in [
            json!({ "stockCode": "600519", "period": "daily", "limit": 1, "adjType": "auto" }),
            json!({ "stockCode": "600519", "period": "daily", "limit": 1, "adjType": Value::Null }),
            json!({ "stockCode": "600519", "period": "daily", "limit": 1 }),
        ] {
            let args = kline_seam_args(&input, ("stockCode", "stock_code"), true)
                .unwrap_or_else(|e| panic!("{input} 应当合法：{e:?}"));
            assert!(args["adj_type"].is_null(), "`{input}` 的 adj_type 必须落 null：{args}");
        }

        // 拒绝面：必填缺失 / 空白 / limit 非整数 / 复权档越界 —— 一律 INVALID_ARGS，不发帧。
        let bad = [
            json!({ "period": "daily", "limit": 1 }),
            json!({ "stockCode": "  ", "period": "daily", "limit": 1 }),
            json!({ "stockCode": "600519", "limit": 1 }),
            json!({ "stockCode": "600519", "period": "daily" }),
            json!({ "stockCode": "600519", "period": "daily", "limit": -1 }),
            json!({ "stockCode": "600519", "period": "daily", "limit": "250" }),
            json!({ "stockCode": "600519", "period": "daily", "limit": 1, "adjType": "qfq" }),
            json!({ "stockCode": "600519", "period": "daily", "limit": 1, "adjType": 3 }),
        ];
        for input in bad {
            let outcome = kline_seam_args(&input, ("stockCode", "stock_code"), true);
            let Err((code, message)) = outcome else {
                panic!("`{input}` 必须被拒在接缝之前，实际放行：{outcome:?}");
            };
            assert_eq!(
                code,
                error_codes::SEAM_CALL_INVALID_ARGS,
                "`{input}` 的拒绝码不对：{message}"
            );
        }
    }

    /// 候选 A 泛化钉点：K 线族的标的键随路由传入（外汇 `pair` / 基准 `benchmarkCode`），
    /// `with_adj = false` 时不读 `adjType` 也不插 `adj_type` 键 —— 接缝侧
    /// `deserialize_optional_arg` 对缺失键落 `None`，与宿主命令「固定未复权」同形。
    #[test]
    fn kline_seam_args_variants_and_adjless_routes() {
        let forex = kline_seam_args(
            &json!({ "pair": " USD/JPY ", "period": "daily", "limit": 5 }),
            ("pair", "pair"),
            false,
        )
        .expect("外汇入参应能翻成接缝载荷");
        assert_eq!(forex["pair"], "USD/JPY", "外汇标的键要 trim");
        assert_eq!(forex["period"], "daily");
        assert_eq!(forex["limit"], 5);
        assert!(forex.get("adj_type").is_none(), "无复权路由不得插 adj_type 键");
        assert!(forex.get("stock_code").is_none(), "标的键必须随路由传入，不得残留 A 股键名");

        // 命令面就算带了 adjType，无复权路由也不翻译不透传（拒绝一条接缝给不了的通道）。
        let adjless = kline_seam_args(
            &json!({ "benchmarkCode": "HSI", "period": "weekly", "limit": 10, "adjType": "forward" }),
            ("benchmarkCode", "benchmark_code"),
            false,
        )
        .expect("基准入参应能翻成接缝载荷");
        assert_eq!(adjless["benchmark_code"], "HSI");
        assert!(adjless.get("adj_type").is_none(), "adjType 在无复权路由上必须被丢弃");
    }

    /// `search_news` 载荷翻译：keyword 必填，limit 缺省 20 / 上限钳 100 ——
    /// 与宿主 `search_news` 命令体的 `unwrap_or(20).min(100)` 同档，不造第二个缺省数。
    #[test]
    fn search_news_seam_args_defaults_and_clamps_limit() {
        let args = search_news_seam_args(&json!({ "keyword": " 贵州白酒 " }))
            .expect("缺省 limit 应当合法");
        assert_eq!(args["keyword"], "贵州白酒", "keyword 要 trim");
        assert_eq!(args["limit"], 20, "缺省 limit 必须与宿主命令的 20 同档");

        let args = search_news_seam_args(&json!({ "keyword": "新能源", "limit": 100 }))
            .expect("显式 100 应当合法");
        assert_eq!(args["limit"], 100);

        let args = search_news_seam_args(&json!({ "keyword": "新能源", "limit": 500 }))
            .expect("超限入参应当被钳而不是被拒（与宿主命令一致）");
        assert_eq!(args["limit"], 100, "超 100 必须钳到 100");

        // 拒绝面：keyword 缺失 / 空白 / limit 非法 —— 一律 INVALID_ARGS，不发帧。
        let bad = [
            json!({}),
            json!({ "keyword": "   " }),
            json!({ "keyword": "新能源", "limit": -1 }),
            json!({ "keyword": "新能源", "limit": "20" }),
            json!({ "keyword": "新能源", "limit": 1.5 }),
        ];
        for input in bad {
            let outcome = search_news_seam_args(&input);
            let Err((code, message)) = outcome else {
                panic!("`{input}` 必须被拒在接缝之前，实际放行：{outcome:?}");
            };
            assert_eq!(
                code,
                error_codes::SEAM_CALL_INVALID_ARGS,
                "`{input}` 的拒绝码不对：{message}"
            );
        }
    }

    /// `batch_quotes_seam_args`：codes 必填、三可选数值键 camelCase→snake_case 翻译，
    /// 拒绝判据与原宿主 `BatchQuotesRequest` 的 serde 反序列化口径一致。
    #[test]
    fn batch_quotes_seam_args_translates_camel_case_keys() {
        let args = batch_quotes_seam_args(&json!({
            "codes": ["600519", "000001"],
            "perStockTimeoutMs": 5000u64,
            "totalTimeoutMs": 30000u64,
            "maxFailures": 2u64,
        }))
        .expect("完整入参应当合法");
        assert_eq!(args["codes"], json!(["600519", "000001"]));
        assert_eq!(args["per_stock_timeout_ms"], 5000);
        assert_eq!(args["total_timeout_ms"], 30000);
        assert_eq!(args["max_failures"], 2);

        // 只给 codes 时三个可选键**不插**（缺省语义归接缝侧）
        let args =
            batch_quotes_seam_args(&json!({ "codes": ["600519"] })).expect("仅 codes 应当合法");
        assert!(args.get("per_stock_timeout_ms").is_none());
        assert!(args.get("total_timeout_ms").is_none());
        assert!(args.get("max_failures").is_none());

        // 空 codes 数组与迁移前一致地放行（空 ⇒ BatchResult 空，不报 INVALID）
        let args = batch_quotes_seam_args(&json!({ "codes": [] })).expect("空数组应当放行");
        assert_eq!(args["codes"], json!([]));

        // 拒绝面：codes 缺失 / 非数组 / 元素非字符串 / 数值键负数或浮点
        let bad = [
            json!({}),
            json!({ "codes": "600519" }),
            json!({ "codes": [1, 2] }),
            json!({ "codes": ["600519"], "perStockTimeoutMs": -1 }),
            json!({ "codes": ["600519"], "maxFailures": 1.5 }),
        ];
        for input in bad {
            let outcome = batch_quotes_seam_args(&input);
            let Err((code, _)) = outcome else {
                panic!("`{input}` 必须被拒，实际放行：{outcome:?}");
            };
            assert_eq!(code, error_codes::SEAM_CALL_INVALID_ARGS, "`{input}` 拒绝码错误");
        }
    }

    /// `batch_financials_seam_args`：只有 codes 一键，拒绝判据同上。
    #[test]
    fn batch_financials_seam_args_only_takes_codes() {
        let args = batch_financials_seam_args(&json!({ "codes": ["600519"] })).expect("合法入参");
        assert_eq!(args, json!({ "codes": ["600519"] }));
        let Err((code, _)) = batch_financials_seam_args(&json!({})) else {
            panic!("codes 缺失必须被拒");
        };
        assert_eq!(code, error_codes::SEAM_CALL_INVALID_ARGS);
    }

    /// #311：监控优先级的**别名宽容必须留在插件侧**。
    ///
    /// 宿主原命令认 `bg` 且把任何未知串静默回落 Active；接缝是严格 serde（`bg` 直接
    /// INVALID_ARGS）。这条钉住「归一后只剩接缝认得的两个词」，且未知串不许变成错误 ——
    /// 否则 LLM 传 `bg` / 拼错词会从「成功」变「报错」，属行为回退。
    #[test]
    fn watch_priority_arg_keeps_host_alias_tolerance() {
        for alias in ["background", "bg"] {
            assert_eq!(watch_priority_arg(alias), "background", "`{alias}` 必须归一成 background");
        }
        // 正控：接缝认得的另一个词原样保留。
        assert_eq!(watch_priority_arg("active"), "active");
        // 未知串（含大小写变体、空串、中文）一律回落 active，与宿主原命令的 `_ =>` 臂同口径。
        for unknown in ["Active", "BACKGROUND", "", "high", "低", "bg "] {
            assert_eq!(watch_priority_arg(unknown), "active", "`{unknown}` 必须静默回落 active");
        }
    }

    /// #311：必填字符串入参的判据与 `fetch_market_read` 必填键档一致（trim 后非空）。
    #[test]
    fn command_str_arg_trims_and_rejects_blanks() {
        assert_eq!(
            command_str_arg(&json!({ "tradeId": " t-1 " }), "tradeId").expect("合法"),
            "t-1"
        );
        for bad in [
            json!({}),
            json!({ "tradeId": "" }),
            json!({ "tradeId": "   " }),
            json!({ "tradeId": null }),
            json!({ "tradeId": 7 }),
        ] {
            let Err((code, _)) = command_str_arg(&bad, "tradeId") else {
                panic!("`{bad}` 必须被拒");
            };
            assert_eq!(code, error_codes::SEAM_CALL_INVALID_ARGS, "`{bad}` 拒绝码错误");
        }
    }

    /// #311：`conceptIds` 与批量 `codes` 共用同一判据（空数组放行 ⇒ 空结果，与宿主命令一致）。
    #[test]
    fn parse_string_array_serves_codes_and_concept_ids() {
        assert_eq!(
            parse_string_array(&json!({ "conceptIds": [] }), "conceptIds").expect("空数组放行"),
            Vec::<String>::new()
        );
        assert_eq!(
            parse_string_array(&json!({ "conceptIds": ["concept_ai"] }), "conceptIds")
                .expect("合法"),
            vec!["concept_ai".to_string()]
        );
        for bad in [
            json!({}),
            json!({ "conceptIds": "concept_ai" }),
            json!({ "conceptIds": [1] }),
            json!({ "conceptIds": null }),
        ] {
            let Err((code, _)) = parse_string_array(&bad, "conceptIds") else {
                panic!("`{bad}` 必须被拒");
            };
            assert_eq!(code, error_codes::SEAM_CALL_INVALID_ARGS, "`{bad}` 拒绝码错误");
        }
    }

    /// #311 + F1 余三条 + F2/F6（#322）：显式分派臂命令**不得被三张 fallback 路由表吞掉**。
    ///
    /// 负控意义：若有人日后把它们塞进 `market_read_route` / `cron_seam_op`，命令会打到
    /// 相邻数据面上（`stock_set_quote_watch_priority` 尤其危险 —— 它是写命令）。
    #[test]
    fn new_t2_commands_are_not_swallowed_by_fallback_tables() {
        for command in [
            super::COMMAND_GET_PORTFOLIO_RISK,
            super::COMMAND_COMPARE_TRADE,
            super::COMMAND_SCAN_SECTOR_COHERENCE,
            super::COMMAND_SET_QUOTE_WATCH_PRIORITY,
            // F1 余三条的两条显式臂（快照读面 / 快照采集触发口，均走 stock.analysis）。
            super::COMMAND_LIST_FINANCIAL_SNAPSHOTS,
            super::COMMAND_SWEEP_DAILY_SNAPSHOTS,
            // F2 余四条（#322，均走 trading.engine 新 op）。
            super::COMMAND_GET_POSITION_LIMITS,
            super::COMMAND_UPDATE_PORTFOLIO_HOLDING,
            super::COMMAND_IMPORT_PORTFOLIO_FROM_VLM,
            super::COMMAND_RUN_PORTFOLIO_STRESS_TEST,
            // F6 别名族六条（#322，均走 stock.analysis 新 op）。
            super::COMMAND_EVOLVE_ANALYST,
            super::COMMAND_GET_ANALYST_EVOLUTION_STATUS,
            super::COMMAND_GET_NODE_EVOLUTION_STATUS,
            super::COMMAND_SAVE_ANALYST_FEEDBACK,
            super::COMMAND_GET_ANALYST_FEEDBACKS,
            super::COMMAND_GET_ANALYST_FEEDBACK_STATS,
            // #326 F3 首刀四条：两条纯本地 + 两条自行编排 `market.quote` 两现成 op
            // （不经 `market_read_route` 表，故必须仍是显式臂）。
            super::COMMAND_ANALYZE_SENTIMENT_DEPTH,
            super::COMMAND_FACTOR_ANALYSIS_LIST,
            super::COMMAND_GENERATE_FUNDAMENTALS_REPORT,
            super::COMMAND_GET_FUNDAMENTALS_REPORT_MARKDOWN,
            // #328 两条：DB 直读经 `stock.analysis` 新读 op（非行情/K线/cron 三表）。
            super::COMMAND_LATEST_ANALYSIS_FOR_STOCK,
            super::COMMAND_LATEST_ANALYSES_FOR_STOCKS,
            super::COMMAND_EVOLUTION_DRIFT_TIMELINE,
            super::COMMAND_AGREEMENT_SCORE_HISTORY,
            super::COMMAND_NODE_FEEDBACKS,
            super::COMMAND_NODE_FEEDBACK_STATS,
            super::COMMAND_MONTHLY_REPORT,
            super::COMMAND_MACRO_DATA_SNAPSHOT,
            super::COMMAND_MOVER_REFLOW_CANDIDATES,
            // #335 两条：回测反馈分析（计算型）+ VLM 持仓截图解析（读 op）。
            super::COMMAND_ANALYZE_BACKTEST_FEEDBACK,
            super::COMMAND_PARSE_VLM_PORTFOLIO_SCREENSHOT,
            // #336 四条：quant 3 读 + register_rhai 写（均走 stock.analysis）。
            super::COMMAND_QUANT_RUN_GET,
            super::COMMAND_QUANT_STRATEGIES_LIST,
            super::COMMAND_QUANT_METRICS_COMPARE,
            super::COMMAND_QUANT_STRATEGY_REGISTER_RHAI,
            // #337 两条：vendor 状态 / 取消令牌（写语义，均走 stock.analysis）。
            super::COMMAND_SET_VENDOR_STATUS,
            super::COMMAND_CANCEL_STOCK_ANALYSIS,
            super::COMMAND_LIST_PARAM_SUGGESTIONS,
            // #329 三条：discover 零新 op 编排（不走三张路由表），两条写 op 走 stock.analysis。
            super::COMMAND_DISCOVER_STOCK_CANDIDATES,
            super::COMMAND_INVALIDATE_RECOMMENDATION_CACHE,
            super::COMMAND_RUN_SELF_IMPROVING,
        ] {
            assert!(market_read_route(command).is_none(), "`{command}` 不得进行情只读表");
            assert!(kline_route(command).is_none(), "`{command}` 不得进 K 线表");
            assert!(cron_seam_op(command).is_none(), "`{command}` 不得进 cron 表");
        }
        // 正控：三张表本身仍然活着（否则上面的 is_none 全是空洞通过）。
        assert!(market_read_route(super::COMMAND_GET_QUOTE).is_some());
        assert!(kline_route(super::COMMAND_GET_KLINE).is_some());
        assert!(cron_seam_op(super::COMMAND_LIST_RECOMMENDATION_CRONS).is_some());
    }

    /// F1 余三条：`stock_refresh_trading_calendar` 与显式臂两条**必须落在正确的取数路径**
    /// —— refresh 走 `market_read_route` 表（`market.quote` 零入参 op），另两条显式臂走
    /// `stock.analysis`。refresh 若被挪进显式臂而没在表里登记，命令会打
    /// `PLUGIN_COMMAND_UNKNOWN`；反之 list/sweep 若被塞进表，会打到相邻数据面。
    #[test]
    fn f1_three_commands_route_to_their_own_seams() {
        let refresh = market_read_route(super::COMMAND_REFRESH_TRADING_CALENDAR)
            .expect("刷新交易日历必须在 market_read_route 表里");
        assert_eq!(refresh.op, "refresh_trading_calendar");
        assert!(refresh.keys.is_none(), "刷新交易日历是零入参 op，不得有标的键");
        assert!(refresh.optional.is_none(), "刷新交易日历不得有可选键");
    }

    /// #326 F3 首刀：`stock_analyze_sentiment_depth` 的入参键必须是 **camelCase**
    /// （`stockCode`/`stockName`/`historyJson`），且 `historyJson` 按
    /// `SentimentSnapshot`（该结构 `rename_all = "camelCase"`）反序列化。
    ///
    /// 两头都钉：宿主原命令吃的是 snake_case `history_json`，插件侧换 camelCase 后
    /// 若有人把内层快照键也写成 snake（`post_count`），serde 会静默落 `None`/默认值，
    /// 情绪均值恒 0 —— 那种「不报错但算错」最难发现，故用真实两日数据算一遍。
    #[test]
    fn analyze_sentiment_depth_takes_camel_case_keys_and_parses_snapshots() {
        let history = json!([
            { "date": "2026-11-14", "postCount": 12, "sentimentScore": 0.8, "bullRatio": 0.7, "hotRank": 3 },
            { "date": "2026-11-13", "postCount": 9, "sentimentScore": 0.2, "bullRatio": 0.4, "hotRank": 9 },
        ]);
        let input = json!({
            "stockCode": "600000",
            "stockName": "浦发银行",
            "historyJson": history.to_string(),
        });
        let out = analyze_sentiment_depth(&input).expect("合法入参必须成功");
        assert_eq!(out["stockCode"], "600000");
        assert_eq!(out["stockName"], "浦发银行");
        assert_eq!(out["avgSentiment7d"], 0.5, "两日 0.8/0.2 ⇒ 均值 0.5（算错会落 0）");

        // 负控：蛇形键必须被拒（不是静默取默认值），否则调用方传错形状会得到假报告。
        for missing in ["stockCode", "stockName", "historyJson"] {
            let mut bad = input.clone();
            bad.as_object_mut().expect("对象").remove(missing);
            let Err((code, _)) = analyze_sentiment_depth(&bad) else {
                panic!("缺 `{missing}` 必须被拒");
            };
            assert_eq!(code, error_codes::SEAM_CALL_INVALID_ARGS, "缺 `{missing}` 拒绝码错误");
        }
    }

    /// #326：`stock_factor_analysis_list` 零参数、回数组，且键为 camelCase
    /// （`higherIsBetter`/`defaultWeight`）—— 与宿主原命令 `json!` 的逐键照搬一致。
    #[test]
    fn factor_analysis_list_returns_camel_case_items() {
        let Value::Array(items) = factor_analysis_list().expect("因子清单必须可取") else {
            panic!("因子清单必须是数组");
        };
        assert!(!items.is_empty(), "因子注册表不得为空（空表会让工具面静默返回空）");
        let first = &items[0];
        for key in ["id", "name", "category", "higherIsBetter", "defaultWeight", "enabled"] {
            assert!(first.get(key).is_some(), "因子条目缺 `{key}`：{first}");
        }
        for snake in ["higher_is_better", "default_weight"] {
            assert!(first.get(snake).is_none(), "不得回蛇形键 `{snake}`：{first}");
        }
    }

    /// #311：时间戳量纲必须是**秒**（宿主原命令用 `chrono::Utc::now().timestamp()`）。
    /// 写成毫秒会让板块报告的 `timestamp` 字段与历史产物差三个数量级。
    #[test]
    fn unix_now_secs_is_seconds_not_millis() {
        let got = unix_now_secs();
        let direct = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("系统时钟晚于 UNIX 纪元")
            .as_secs() as i64;
        assert!((got - direct).abs() <= 2, "与标准库直取偏差过大：{got} vs {direct}");
        assert!(got < 100_000_000_000, "量纲疑似毫秒：{got}");
    }
}
