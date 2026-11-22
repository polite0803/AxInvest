// SPDX-License-Identifier: AGPL-3.0-only

//! 股票分析数据面接缝（`stock.analysis`）— Trait（DTO 留在域包）。
//!
//! 宿主把二十二条**确定性取数/落库/计算/任务管理**能力经本接缝暴露给插件：
//! `industry_constituents`（按行业取全 A 股成分股）、`batch_upsert_mainlines`（按日批量
//! upsert 市场主线）、`list_recent_mainlines`（列出最近 N 天主线，#304 C3）、
//! `stock_factor`（逐票 1 月涨跌幅，反转因子原料）——E 批三条 + C3 补一条读；
//! `pipeline_history`（管道执行历史）、`pipeline_run_detail`（单次执行详情）、
//! `pending_evolution_plans`（待审批进化计划）、`des_integration`（Walk-Forward + DES
//! 对比仿真）——T2 试点批（#248）四条，插件命名命令 `stock_get_pipeline_history` /
//! `stock_get_pipeline_run_detail` / `stock_list_pending_evolution_plans` /
//! `stock_wf_des_integration` 的取数面；三族定时任务（荐股 / 趋势智选 / 候选池扫描）
//! 的 CRUD 十二条 —— 批 1a（#35），插件命名命令 `stock_create_*_cron` 等，
//! 数据面是宿主 `CronJobStore`（不落 sea-orm 表），见下方 trait 内的段落注释；
//! `latest_analysis_for_stock`（个股最近一次完整分析摘要）、`list_param_suggestions`
//! （最近未处理的参数调整建议）—— #328 两条，原宿主命令直读 sea-orm 表，插件侧
//! 无法自行取数故新增读 op，插件命名命令 `stock_get_latest_analysis_for_stock` /
//! `stock_list_param_suggestions`。
//! `discover_stock_candidates`（热门候选发现——编排留插件侧，打分纯函数
//! `StockScreener::score_candidate` 单一权威）、`invalidate_recommendation_cache`
//! （失效荐股进程内缓存）、`run_self_improving_stock_analysis`（自改进分析循环，
//! 执行体在宿主 agent crate）—— #329 三条，插件命名命令
//! `stock_discover_stock_candidates` / `stock_invalidate_recommendation_cache` /
//! `stock_run_self_improving_stock_analysis`。
//!
//! ## 为什么需要它
//!
//! 这三个能力的实现体在 `analysis-engine`（**implementor**，持 `entities` 与
//! `DatabaseConnection`），而 `tools`（hybrid）的薄包装工具原先直接依赖它 ⇒ 构成
//! **跨包口**：一个功能包的工具体调另一个功能包的实现层。改造方向与 `opc.store`
//! （#36）同族 —— 工具只发插件命令，取数经宿主接缝往返。
//!
//! ## 为什么 payload / 出参都是 `serde_json::Value`
//!
//! 三个 op 的 DTO（`IndustryConstituentsInput` / `IndustryConstituentsReport` /
//! `BatchUpsertInput` / `BatchUpsertResult` / `StockFactorReport` / `StockFactorRow`）的
//! **权威定义在 `analysis-engine`**（implementor，随股票域最终迁进插件仓）。harness 是
//! **foundation**，按依赖铁律不得依赖任何 implementor ⇒ 这里**不能**上收这些结构体。
//! 故本接缝只承诺「JSON 形状与工具出参信封逐字一致」，形状的权威仍在域包；
//! 宿主 provider（wiring）负责 `Value` ↔ 域 DTO 的转换，转换点**唯一**。
//!
//! 这与 `opc.store` 的做法（DTO 上收 harness）不同，判据是**归属**：OPC 的 DTO 已经
//! 是接缝自有契约（没有别的权威），而股票三件各自的 DTO 都另有 implementor 权威 ——
//! 上收会制造第二份定义（禁区 12）。
//!
//! ## 权限闸
//!
//! **十条写 op**：`batch_upsert_mainlines`（改 `market_mainlines` 表：插入 / 更新 /
//! 可选归档当日未提及的主线）+ 批 1a 的九个 cron 管理写 op（3 create + 3 toggle +
//! 3 delete，增删改宿主 `CronJobStore`）。权限登记在 `worker.rs` 的
//! `required_permission_for`，本 trait 不重复校验。其余十个 op（三条 E 批读 / 三条
//! 试点读 / 三个 cron list / `list_recent_mainlines`）是纯读/纯计算面 ⇒ **一条都不许加闸**（给读缝加闸
//! 会让只读插件被无端拒掉，判据同 `opc.store` 模块头）。`des_integration` 虽是重 CPU
//! 仿真，但它不落库、不外发，读/写权限面同样不适用。

use async_trait::async_trait;
use serde_json::Value;

use crate::core_error::{AxAgentError, Result};

/// 股票分析数据面提供者（宿主独占实现：落库与行情表读面都不在插件侧）。
///
/// 实现方：宿主 wiring 层（`src-tauri/src/init/stock_analysis_service.rs`），内部调
/// `analysis-engine` 的权威实现 + 宿主 sea-orm 连接，本层不写 SQL。
/// 默认实现返回「该提供者不支持此数据面」，便于外部实现只覆盖关心的 op
/// （与 `OpcStoreProvider` / `MarketDataProvider` 同款缺省）。
///
/// ⚠ 三个 op 的入参/出参 JSON 形状**必须与工具侧原样一致**（改名即背离「工具名、
/// ToolDef、出参信封逐字不变」的改造红线）：
/// - `industry_constituents`：入参 = `IndustryConstituentsInput`（`{industries: [...]}`，
///   元素为字符串或 `{name, leaderCode?}`）；出参 = `IndustryConstituentsReport`
///   （`{groups, universeTotal, universeWithIndustry}`，camelCase）。
/// - `batch_upsert_mainlines`：入参 = `BatchUpsertInput`（camelCase 主名 +
///   snake_case alias 兼容）；出参 = `BatchUpsertResult`（`{inserted, updated, archived}`）。
/// - `stock_factor`：入参 = `{stock_codes: <宽容形态>, as_of_date?: "YYYY-MM-DD"}`
///   （provider 另收 camel 主名 `stockCodes` / `asOfDate`）—— **与工具入参逐字相同**：
///   字符串数组 / 代码对象数组 / 含 `stocks` 分组的信封。⚠ 归一（`parse_stock_refs`）
///   落在**宿主 provider**，不在插件侧：该函数属 `analysis-engine`（implementor），
///   插件 worker 只透传。出参 =
///   `StockFactorReport`（`{rows, requested, scored, missing, cutoffDate, windowRows}`）。
#[async_trait]
pub trait StockAnalysisProvider: Send + Sync {
    /// 按行业名取全 A 股成分股分组（`industry_constituents` op）—— 读 op，无权限闸。
    async fn industry_constituents(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::industry_constituents 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 按日批量 upsert 市场主线（`batch_upsert_mainlines` op）—— 写 op，要 `write` 权限。
    ///
    /// `archive_missing = true` 会把当日已有但本次未提及的主线置 `status=archived`
    /// ⇒ 不可逆的数据面动作（归档后要靠同主题再 upsert 才回到 active）。
    async fn batch_upsert_mainlines(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::batch_upsert_mainlines 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 列出最近 N 天市场主线（`list_recent_mainlines` op）—— 读 op，无权限闸。
    ///
    /// 入参 `{days?: number}`（camelCase 平铺，缺省 7；宿主 provider 拍上界），
    /// 出参 = `market_mainlines` 实体行数组（形状权威在 analysis-engine，与被退役的
    /// 宿主命令 `market_mainline_list_recent` 原样返回同一 JSON，#304 C3）。
    async fn list_recent_mainlines(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::list_recent_mainlines 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 逐票 1 月涨跌幅（`stock_factor` op）—— 读 op，无权限闸。
    async fn stock_factor(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::stock_factor 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    // ── T2 试点批（#248）：四条宿主命令的取数面 ─────────────────────────────
    // 与上面三条不同，这四条的 DTO 权威**不在 analysis-engine**，而在主壳 commands 层
    // （pipeline 行是 `core.rs` 手拼的 `json!`、进化计划是 dao 的 `EvolutionHistoryModel`、
    // DES 仿真是 `commands/wf_des.rs` 的 `WfDesRequest`/`WfDesReport`）。仍按本接缝的
    // 「Value 透传」铁律处理：形状权威在哪一侧，provider 就照哪一侧透传，本层不上收。
    //
    // 三个查询命令的入参形状与前端 `invoke` 传参逐字一致（Tauri 自动 camelCase 映射）：
    // `pipeline_history` 收 `{limit?}`、`pipeline_run_detail` 收 `{runId}`、
    // `pending_evolution_plans` 收 `{}`；`des_integration` 收 `WfDesRequest` 的 camelCase
    // JSON（provider 侧 `serde_json::from_value` 是唯一形状权威，缺料由它报 Validation）。

    /// 管道执行历史（`pipeline_history` op）—— 读 op，无权限闸。
    async fn pipeline_history(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::pipeline_history 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 单次管道执行详情（`pipeline_run_detail` op）—— 读 op，无权限闸。
    async fn pipeline_run_detail(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::pipeline_run_detail 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 个股最近一次完整分析摘要（`latest_analysis_for_stock` op）—— 读 op，无权限闸。
    ///
    /// 入参 `{stockCode: string, asOfDate?: string}`；出参 = `LatestAnalysisSummary` 的
    /// camelCase JSON，无记录回 `null`。
    async fn latest_analysis_for_stock(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::latest_analysis_for_stock 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 批量个股最近分析摘要（`latest_analyses_for_stocks` op）—— 读 op，无权限闸。
    ///
    /// 入参 `{stockCodes: string[], asOfDate?: string}`；出参 = `{code: Summary|null}`
    /// 的 JSON 对象。逐票复用单数版同一 SQL 判据（宿主 `get_latest_analyses_for_stocks_core`
    /// 循环调 `get_latest_analysis_for_stock_core`）⇒ 两版口径只有一处。
    async fn latest_analyses_for_stocks(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::latest_analyses_for_stocks 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 权重时间线（`evolution_drift_timeline` op）—— 读 op，无权限闸。
    ///
    /// 入参 `{strategyId: string, period: string, limit?: number}`（limit 缺省 60）；
    /// 出参 = `TimelinePoint` 数组（引擎 DTO，camelCase）。
    async fn evolution_drift_timeline(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::evolution_drift_timeline 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 决策一致性分数历史（`agreement_score_history` op）—— 读 op，无权限闸。
    ///
    /// 入参 `{limit?: number}`（缺省 50）；出参 = 行数组，键序
    /// `exitAt / agreementScore / stockCode / stockName / returnPct / wasCorrect`。
    async fn agreement_score_history(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::agreement_score_history 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 节点质量反馈历史（通用版，`node_feedbacks` op）—— 读 op，无权限闸。
    ///
    /// ⚠ 与别名族 `get_analyst_feedbacks`（#322）**不同命令**：本 op 收
    /// `{nodeType?, nodeId?, limit?, onlyIssues?}` 泛化过滤，出参 `NodeFeedbackSummary` 数组。
    async fn node_feedbacks(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::node_feedbacks 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 节点反馈统计（通用版，`node_feedback_stats` op）—— 读 op，无权限闸。
    ///
    /// 入参 `{nodeType: string, nodeId: string}`；出参 `NodeFeedbackStats`（含
    /// `needs_evolution` 与 `consistency_metrics`）。
    async fn node_feedback_stats(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::node_feedback_stats 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 月度投资报告（`monthly_report` op）—— 读 op，无权限闸。
    ///
    /// 入参 `{year: number, month: number}`；出参 = `MonthlyReport` 的 camelCase JSON。
    async fn monthly_report(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::monthly_report 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 宏观经济数据快照（`macro_data_snapshot` op）—— 读 op，无权限闸。
    ///
    /// 入参恒 `{}`（宿主原命令零参）；出参 = `MacroDataSnapshot` 的 camelCase JSON。
    async fn macro_data_snapshot(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::macro_data_snapshot 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 漏检回灌清单与转正就绪度（`mover_reflow_candidates` op）—— 读 op，无权限闸。
    ///
    /// 入参恒 `{}`（宿主原命令零参）；出参 = `MoverReflowView` 的 camelCase JSON
    /// （`items` / `settled` / `required` / `ready`）。
    async fn mover_reflow_candidates(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::mover_reflow_candidates 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 回测反馈分析（`analyze_backtest_feedback` op）—— 计算型 op（宿主原档 Caution）。
    ///
    /// 纯计算（无 IO / 无状态），权威在 `analysis-engine::backtest_feedback`。入参 =
    /// `FeedbackInput` 的 camelCase JSON（`participations` 数组，元素含 `analystId` /
    /// `timeHorizon` / `date` / `stance`（"bullish"|"bearish"|"neutral"）/ `confidence` /
    /// `wasCorrect`）；出参 = `BacktestFeedbackReport` 的 camelCase JSON。
    async fn analyze_backtest_feedback(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::analyze_backtest_feedback 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// VLM 持仓截图解析（`parse_vlm_portfolio_screenshot` op）—— 读 op（宿主原档 Safe）。
    ///
    /// 纯解析（无 IO / 无状态），权威在 `analysis-engine::vlm_import::parse_vlm_output`。
    /// 入参 `{rawVlmOutput: string}`（VLM 文本输出）；出参 = `VlmParseResult` 的
    /// camelCase JSON（`success` / `holdings` / `error` / `sourceType` / `rawOutput`）。
    async fn parse_vlm_portfolio_screenshot(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::parse_vlm_portfolio_screenshot 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 按 runId 读取回测运行记录（`quant_run_get` op）—— 读 op（宿主原档 Safe）。
    ///
    /// 入参 `{runId: string}`；出参 = `quant_runs::Model` 的 JSON。runId 不存在时
    /// 报错（与宿主原命令逐字同语义，前端 catch 后按「记录缺失」呈现）。
    async fn quant_run_get(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::quant_run_get 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 列出所有量化策略（`quant_strategies_list` op）—— 读 op（宿主原档 Safe）。
    ///
    /// 入参恒 `{}`；出参 = `quant_strategies::Model` 行数组的 JSON。
    async fn quant_strategies_list(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::quant_strategies_list 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 多 run 指标对比（`quant_metrics_compare` op）—— 读 op（宿主原档 Safe）。
    ///
    /// 入参 `{runIds: string[]}`；出参 = `MetricsCompareResponse` 的 camelCase JSON
    /// （`runs` / `bestBy`）。
    async fn quant_metrics_compare(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::quant_metrics_compare 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 注册 Rhai 策略（`quant_strategy_register_rhai` op）—— 写 op（宿主原档 Caution，
    /// insert/upsert `quant_strategies` 表）。
    ///
    /// 入参 = `RegisterRhaiRequest` 的 camelCase JSON（`name` / `version` / `description` /
    /// `scriptSource` / `params` / `walkForwardEnabled` / `upsert`）；出参 =
    /// `quant_strategies::Model` 的 JSON。
    async fn quant_strategy_register_rhai(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::quant_strategy_register_rhai 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 手动设置数据源健康状态（`set_vendor_status` op）—— 写 op（宿主原档 Safe 但
    /// 改写 `health_tracker` 运行态 ⇒ op 级要求包有 write，#337）。
    ///
    /// 入参 `{vendor: string, status: string}`（status ∈ healthy/degraded/disabled）；
    /// 出参恒 `null`。无效状态值报错（与宿主原命令同判据）。
    async fn set_vendor_status(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::set_vendor_status 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 设置主行情源（`set_primary_market_vendor` op）—— 写 op（写 settings + 即时重排
    /// `quote`/`klines` 链首；只重排不增删注册源）。入参二选一：
    /// `{vendor: string|null}` 写入并应用；`{applyFromSettings: true}` 只读 settings 应用
    /// （装配/重放已保存的主源，不写库）。空串/全空白归一为 `None`（清除覆盖）。出参恒 `null`。
    async fn set_primary_market_vendor(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::set_primary_market_vendor 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 取消进行中的分析任务（`cancel_stock_analysis` op）—— 写 op（翻转取消令牌，
    /// op 级要求包有 write，#337）。
    ///
    /// 入参 `{analysisId: string}`；出参恒 `null`。令牌不存在（任务已完成）报错
    /// （与宿主原命令同判据）。
    async fn cancel_stock_analysis(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::cancel_stock_analysis 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 查询漏检补分析批次状态（`mover_recall_analysis_batch` op）—— 读 op（宿主原档
    /// Safe；写库副作用 `ensure_stale_reaped` 已上收启动钩子，#338）。
    ///
    /// 入参 `{batchId: string}`；出参 = `MoverBatchState` 的 camelCase JSON（找不到回
    /// `null`；运行态以宿主进程内 `BATCH_RUNS` 为轮询权威）。
    async fn mover_recall_analysis_batch(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::mover_recall_analysis_batch 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 查询漏检补分析批次历史（`mover_recall_analysis_batches` op）—— 读 op（宿主原档
    /// Safe，摘要不含 items，#338）。
    ///
    /// 入参 `{limit?: number, statusFilter?: string}`；出参 = `MoverBatchState` 数组的
    /// camelCase JSON。
    async fn mover_recall_analysis_batches(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::mover_recall_analysis_batches 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 发起漏检补分析批次（`run_mover_recall_analysis` op）—— 写 op（宿主原档
    /// Caution：落批次起点 + 后台跑分析，op 级要求包有 write，#338）。
    ///
    /// 入参 = `RunMoverRecallAnalysisRequest` 的 camelCase JSON（`batchLimit` /
    /// `maxUnexplainedShare` / `runId?`）；出参 = batchId 字符串。fire-and-forget：
    /// 返回只表示已发起，进度用 `mover_recall_analysis_batch` 轮询。
    async fn run_mover_recall_analysis(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::run_mover_recall_analysis 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 自选股列表（`list_watchlist` op）—— 读 op（宿主原命令迁 T2，#342）。
    ///
    /// 入参恒 `{}`；出参形状与被退役的宿主命令逐字一致。
    async fn list_watchlist(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("list_watchlist 未实现（该提供者不支持此数据面）".into()))
    }

    /// 添加自选股（`add_to_watchlist` op）—— 写 op（宿主原命令迁 T2，#342）。
    ///
    /// 入参`{stockCode, stockName, notes?}`；出参形状与被退役的宿主命令逐字一致。
    async fn add_to_watchlist(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("add_to_watchlist 未实现（该提供者不支持此数据面）".into()))
    }

    /// 更新自选股分组（`watchlist_update_group` op）—— 写 op（宿主原命令迁 T2，#342）。
    ///
    /// 入参`{id, groupName}`；出参形状与被退役的宿主命令逐字一致。
    async fn watchlist_update_group(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "watchlist_update_group 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 自选股分组列表（`watchlist_list_groups` op）—— 读 op（宿主原命令迁 T2，#342）。
    ///
    /// 入参恒 `{}`；出参形状与被退役的宿主命令逐字一致。
    async fn watchlist_list_groups(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("watchlist_list_groups 未实现（该提供者不支持此数据面）".into()))
    }

    /// 保存自选股分组（`watchlist_save_groups` op）—— 写 op（宿主原命令迁 T2，#342）。
    ///
    /// 入参`{groups: string[]}`；出参形状与被退役的宿主命令逐字一致。
    async fn watchlist_save_groups(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("watchlist_save_groups 未实现（该提供者不支持此数据面）".into()))
    }

    /// 创建股票定时分析任务（`create_stock_cron` op）—— 写 op（宿主原命令迁 T2，#342）。
    ///
    /// 入参`{stockCode, stockName, cronExpression}`；出参形状与被退役的宿主命令逐字一致。
    async fn create_stock_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("create_stock_cron 未实现（该提供者不支持此数据面）".into()))
    }

    /// 股票定时任务列表（`list_stock_crons` op）—— 读 op（宿主原命令迁 T2，#342）。
    ///
    /// 入参恒 `{}`；出参形状与被退役的宿主命令逐字一致。
    async fn list_stock_crons(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("list_stock_crons 未实现（该提供者不支持此数据面）".into()))
    }

    /// 开关股票定时任务（`toggle_stock_cron` op）—— 写 op（宿主原命令迁 T2，#342）。
    ///
    /// 入参`{id, enabled}`；出参形状与被退役的宿主命令逐字一致。
    async fn toggle_stock_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("toggle_stock_cron 未实现（该提供者不支持此数据面）".into()))
    }

    /// 创建决策校验定时任务（`create_validate_decisions_cron` op）—— 写 op（宿主原命令迁 T2，#342）。
    ///
    /// 入参`{cronExpression?, minConfidenceThreshold?, reflectionDepth?, enabled?}`；出参形状与被退役的宿主命令逐字一致。
    async fn create_validate_decisions_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "create_validate_decisions_cron 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 决策校验定时任务列表（`list_validate_decisions_crons` op）—— 读 op（宿主原命令迁 T2，#342）。
    ///
    /// 入参恒 `{}`；出参形状与被退役的宿主命令逐字一致。
    async fn list_validate_decisions_crons(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "list_validate_decisions_crons 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 开关决策校验定时任务（`toggle_validate_decisions_cron` op）—— 写 op（宿主原命令迁 T2，#342）。
    ///
    /// 入参`{id, enabled}`；出参形状与被退役的宿主命令逐字一致。
    async fn toggle_validate_decisions_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "toggle_validate_decisions_cron 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 更新决策校验定时任务配置（`update_validate_decisions_cron` op）—— 写 op（宿主原命令迁 T2，#342）。
    ///
    /// 入参`{id, cronExpression, minConfidenceThreshold, reflectionDepth}`；出参形状与被退役的宿主命令逐字一致。
    async fn update_validate_decisions_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "update_validate_decisions_cron 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 创建批量反思定时任务（`create_batch_reflection_cron` op）—— 写 op（宿主原命令迁 T2，#342）。
    ///
    /// 入参`{cronExpression?, period?, dueOnly?, maxCount?, enabled?}`；出参形状与被退役的宿主命令逐字一致。
    async fn create_batch_reflection_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "create_batch_reflection_cron 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 创建自选股扫描定时任务（`create_watchlist_scan_cron` op）—— 写 op（宿主原命令迁 T2，#342）。
    ///
    /// 入参`{cronExpression, enabled?}`；出参形状与被退役的宿主命令逐字一致。
    async fn create_watchlist_scan_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "create_watchlist_scan_cron 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 持仓列表（含实时行情盈亏）（`list_portfolio` op）—— 读 op（宿主原命令迁 T2，#343）。
    async fn list_portfolio(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("list_portfolio 未实现（该提供者不支持此数据面）".into()))
    }

    /// 添加持仓（`add_portfolio_holding` op）—— 写 op（宿主原命令迁 T2，#343）。
    async fn add_portfolio_holding(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("add_portfolio_holding 未实现（该提供者不支持此数据面）".into()))
    }

    /// 移除持仓（`remove_portfolio_holding` op）—— 写 op（宿主原命令迁 T2，#343）。
    async fn remove_portfolio_holding(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "remove_portfolio_holding 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 组合两两相关性快照（`get_portfolio_correlations` op）—— 读 op（宿主原命令迁 T2，#343）。
    async fn get_portfolio_correlations(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "get_portfolio_correlations 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 开关交易功能（`toggle_trading_enabled` op）—— 写 op（宿主原命令迁 T2，#343）。
    async fn toggle_trading_enabled(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "toggle_trading_enabled 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 批量导入交易记录 CSV（`import_trades` op）—— 写 op（宿主原命令迁 T2，#343）。
    async fn import_trades(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("import_trades 未实现（该提供者不支持此数据面）".into()))
    }

    /// 拒绝待审批进化计划（`reject_evolution_plan` op）—— 写 op（宿主原命令迁 T2，#343）。
    async fn reject_evolution_plan(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("reject_evolution_plan 未实现（该提供者不支持此数据面）".into()))
    }

    /// 回滚进化版本到上一已接受版本（`rollback_evolution` op）—— 写 op（宿主原命令迁 T2，#343）。
    async fn rollback_evolution(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("rollback_evolution 未实现（该提供者不支持此数据面）".into()))
    }

    /// as-of 降级条目总数（`get_asof_degradation_count` op）—— 读 op（宿主原命令迁 T2，#344）。
    async fn get_asof_degradation_count(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "get_asof_degradation_count 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// as-of 降级日志快照（`get_asof_degradation_log` op）—— 读 op（宿主原命令迁 T2，#344）。
    async fn get_asof_degradation_log(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "get_asof_degradation_log 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 市场状态文案（`get_market_status` op）—— 读 op（宿主原命令迁 T2，#344）。
    async fn get_market_status(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("get_market_status 未实现（该提供者不支持此数据面）".into()))
    }

    /// 所有数据源健康状态（`get_vendor_health_all` op）—— 读 op（宿主原命令迁 T2，#344）。
    async fn get_vendor_health_all(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("get_vendor_health_all 未实现（该提供者不支持此数据面）".into()))
    }

    /// vendor fallback 日志（`get_vendor_fallback_log` op）—— 读 op（宿主原命令迁 T2，#344）。
    async fn get_vendor_fallback_log(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "get_vendor_fallback_log 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 核心接口日级健康记录（`get_interface_health_history` op）—— 读 op（宿主原命令迁 T2，#344）。
    async fn get_interface_health_history(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "get_interface_health_history 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 估值参数（`get_valuation_params` op）—— 读 op（宿主原命令迁 T2，#344）。
    async fn get_valuation_params(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("get_valuation_params 未实现（该提供者不支持此数据面）".into()))
    }

    /// 保存估值参数（`save_valuation_params` op）—— 写 op（宿主原命令迁 T2，#344）。
    async fn save_valuation_params(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("save_valuation_params 未实现（该提供者不支持此数据面）".into()))
    }

    /// 模板代际对照（代码版本 vs 库内播种）（`get_stock_template_version_status` op）—— 读 op（宿主原命令迁 T2，#344）。
    async fn get_stock_template_version_status(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "get_stock_template_version_status 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 历史分析列表（`list_stock_analyses` op）—— 读 op（宿主原命令迁 T2，#344）。
    async fn list_stock_analyses(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("list_stock_analyses 未实现（该提供者不支持此数据面）".into()))
    }

    /// 最近分析摘要（`get_recent_analyses` op）—— 读 op（宿主原命令迁 T2，#344）。
    async fn get_recent_analyses(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("get_recent_analyses 未实现（该提供者不支持此数据面）".into()))
    }

    /// 反思记录列表（`list_reflections` op）—— 读 op（宿主原命令迁 T2，#344）。
    async fn list_reflections(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("list_reflections 未实现（该提供者不支持此数据面）".into()))
    }

    /// 单条分析详情（`get_stock_analysis` op）—— 读 op（宿主原命令迁 T2，#344）。
    async fn get_stock_analysis(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("get_stock_analysis 未实现（该提供者不支持此数据面）".into()))
    }

    /// 决策证据对账审计（`audit_decision_evidence` op）—— 读 op（宿主原命令迁 T2，#344）。
    async fn audit_decision_evidence(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "audit_decision_evidence 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 重命名分析记录（`rename_stock_analysis` op）—— 写 op（宿主原命令迁 T2，#344）。
    async fn rename_stock_analysis(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("rename_stock_analysis 未实现（该提供者不支持此数据面）".into()))
    }

    /// 价格告警列表（`list_price_alerts` op）—— 读 op（宿主原命令迁 T2，#344）。
    async fn list_price_alerts(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("list_price_alerts 未实现（该提供者不支持此数据面）".into()))
    }

    /// 创建价格告警（落库并进监控引擎）（`create_price_alert` op）—— 写 op（宿主原命令迁 T2，#344）。
    async fn create_price_alert(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("create_price_alert 未实现（该提供者不支持此数据面）".into()))
    }

    /// 实时监控运行读数（`monitor_status` op）—— 读 op（宿主原命令迁 T2，#344）。
    async fn monitor_status(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("monitor_status 未实现（该提供者不支持此数据面）".into()))
    }

    /// T+0 配置（`get_t0_config` op）—— 读 op（宿主原命令迁 T2，#344）。
    async fn get_t0_config(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("get_t0_config 未实现（该提供者不支持此数据面）".into()))
    }

    /// 更新 T+0 配置（`set_t0_config` op）—— 写 op（宿主原命令迁 T2，#344）。
    async fn set_t0_config(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("set_t0_config 未实现（该提供者不支持此数据面）".into()))
    }

    /// 组合监控仪表盘（`get_portfolio_dashboard` op）—— 读 op（宿主原命令迁 T2，#344）。
    async fn get_portfolio_dashboard(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "get_portfolio_dashboard 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 刷新组合监控快照（写 portfolio_metrics_daily + correlation_snapshot）（`refresh_portfolio_metrics` op）—— 写 op（宿主原命令迁 T2，#344）。
    async fn refresh_portfolio_metrics(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "refresh_portfolio_metrics 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 开仓限制校验（`check_position_limits` op）—— 读 op（宿主原命令迁 T2，#344）。
    async fn check_position_limits(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("check_position_limits 未实现（该提供者不支持此数据面）".into()))
    }

    /// 接受待审批进化计划（执行进化并回写）（`approve_evolution_plan` op）—— 写 op（宿主原命令迁 T2，#344）。
    async fn approve_evolution_plan(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "approve_evolution_plan 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// mover 核查轮次列表（`list_mover_recall_runs` op）—— 读 op（宿主原命令迁 T2，#344）。
    async fn list_mover_recall_runs(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "list_mover_recall_runs 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 按轮次读回核查结果（`get_mover_recall_run_view` op）—— 读 op（宿主原命令迁 T2，#344）。
    async fn get_mover_recall_run_view(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "get_mover_recall_run_view 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 漏检事件补分析报告与逐档反思（`get_mover_recall_event_review` op）—— 读 op（宿主原命令迁 T2，#344）。
    async fn get_mover_recall_event_review(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "get_mover_recall_event_review 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 参数变更台账（`list_param_changes` op）—— 读 op（宿主原命令迁 T2，#344）。
    async fn list_param_changes(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("list_param_changes 未实现（该提供者不支持此数据面）".into()))
    }

    /// 参数变更到期评估（写评估结论）（`evaluate_param_changes` op）—— 写 op（宿主原命令迁 T2，#344）。
    async fn evaluate_param_changes(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "evaluate_param_changes 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// `parse_trades_csv`（宿主命令迁 T2，#345）—— 读 op。
    async fn parse_trades_csv(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("parse_trades_csv 未实现（该提供者不支持此数据面）".into()))
    }

    /// `evolve_node_command`（宿主命令迁 T2，#345）—— 写 op。
    async fn evolve_node_command(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("evolve_node_command 未实现（该提供者不支持此数据面）".into()))
    }

    /// `save_node_feedback`（宿主命令迁 T2，#345）—— 写 op。
    async fn save_node_feedback(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("save_node_feedback 未实现（该提供者不支持此数据面）".into()))
    }

    /// `serenity_validation_summary`（宿主命令迁 T2，#345）—— 读 op。
    async fn serenity_validation_summary(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "serenity_validation_summary 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// `create_decision_backtest_cron`（宿主命令迁 T2，#345）—— 写 op。
    async fn create_decision_backtest_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "create_decision_backtest_cron 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// `get_evolution_drift_dashboard`（宿主命令迁 T2，#345）—— 读 op。
    async fn get_evolution_drift_dashboard(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "get_evolution_drift_dashboard 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// `get_evolution_history`（宿主命令迁 T2，#345）—— 读 op。
    async fn get_evolution_history(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("get_evolution_history 未实现（该提供者不支持此数据面）".into()))
    }

    /// `stock_evolution_recalc`（宿主命令迁 T2，#345）—— 写 op。
    async fn stock_evolution_recalc(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "stock_evolution_recalc 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// `get_mover_reflow_merge`（宿主命令迁 T2，#345）—— 读 op。
    async fn get_mover_reflow_merge(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "get_mover_reflow_merge 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// `set_mover_reflow_merge`（宿主命令迁 T2，#345）—— 写 op。
    async fn set_mover_reflow_merge(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "set_mover_reflow_merge 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// `get_mover_asof_summary`（宿主命令迁 T2，#345）—— 读 op。
    async fn get_mover_asof_summary(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "get_mover_asof_summary 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// `get_reco_board_scope`（宿主命令迁 T2，#345）—— 读 op。
    async fn get_reco_board_scope(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("get_reco_board_scope 未实现（该提供者不支持此数据面）".into()))
    }

    /// `set_reco_board_scope`（宿主命令迁 T2，#345）—— 写 op。
    async fn set_reco_board_scope(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("set_reco_board_scope 未实现（该提供者不支持此数据面）".into()))
    }

    /// `get_reco_pool_batch`（宿主命令迁 T2，#345）—— 读 op。
    async fn get_reco_pool_batch(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("get_reco_pool_batch 未实现（该提供者不支持此数据面）".into()))
    }

    /// `get_reco_strategy_weights`（宿主命令迁 T2，#346）—— 读 op。
    async fn get_reco_strategy_weights(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "get_reco_strategy_weights 未实现（该提供者不支持此数据面）".into(),
        ))
    }
    /// `manual_recalc_strategy_weights`（宿主命令迁 T2，#346）—— 写 op。
    async fn manual_recalc_strategy_weights(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "manual_recalc_strategy_weights 未实现（该提供者不支持此数据面）".into(),
        ))
    }
    /// `recalc_reco_loop_weights`（宿主命令迁 T2，#346）—— 写 op。
    async fn recalc_reco_loop_weights(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "recalc_reco_loop_weights 未实现（该提供者不支持此数据面）".into(),
        ))
    }
    /// `apply_param_suggestions`（宿主命令迁 T2，#346）—— 写 op。
    async fn apply_param_suggestions(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "apply_param_suggestions 未实现（该提供者不支持此数据面）".into(),
        ))
    }
    /// `rollback_param_change`（宿主命令迁 T2，#346）—— 写 op。
    async fn rollback_param_change(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("rollback_param_change 未实现（该提供者不支持此数据面）".into()))
    }
    /// `apply_reco_weights`（宿主命令迁 T2，#346）—— 写 op。
    async fn apply_reco_weights(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("apply_reco_weights 未实现（该提供者不支持此数据面）".into()))
    }
    /// `calibrate_portfolio_mgr_params`（宿主命令迁 T2，#346）—— 写 op。
    async fn calibrate_portfolio_mgr_params(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "calibrate_portfolio_mgr_params 未实现（该提供者不支持此数据面）".into(),
        ))
    }
    /// `optimize_scoring_weights`（宿主命令迁 T2，#346）—— 写 op。
    async fn optimize_scoring_weights(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "optimize_scoring_weights 未实现（该提供者不支持此数据面）".into(),
        ))
    }
    /// `compute_valuation_band`（宿主命令迁 T2，#346）—— 读 op。
    async fn compute_valuation_band(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "compute_valuation_band 未实现（该提供者不支持此数据面）".into(),
        ))
    }
    /// `list_stock_tools`（宿主命令迁 T2，#346）—— 读 op。
    async fn list_stock_tools(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("list_stock_tools 未实现（该提供者不支持此数据面）".into()))
    }
    /// `compute_what_if`（宿主命令迁 T2，#346）—— 读 op。
    async fn compute_what_if(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("compute_what_if 未实现（该提供者不支持此数据面）".into()))
    }

    /// `reco_ic_stats`（宿主命令迁 T2，#347）—— 读 op。
    async fn reco_ic_stats(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("reco_ic_stats 未实现（该提供者不支持此数据面）".into()))
    }

    /// `get_reco_signal_history`（宿主命令迁 T2，#347）—— 读 op。
    async fn get_reco_signal_history(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "get_reco_signal_history 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// `get_cached_recommendation`（宿主命令迁 T2，#347）—— 读 op。
    async fn get_cached_recommendation(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "get_cached_recommendation 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// `save_eastmoney_proxy`（宿主命令迁 T2，#347）—— 写 op。
    async fn save_eastmoney_proxy(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("save_eastmoney_proxy 未实现（该提供者不支持此数据面）".into()))
    }

    /// `list_reco_history`（宿主命令迁 T2，#347）—— 读 op。
    async fn list_reco_history(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("list_reco_history 未实现（该提供者不支持此数据面）".into()))
    }

    /// `get_reco_detail`（宿主命令迁 T2，#347）—— 读 op。
    async fn get_reco_detail(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("get_reco_detail 未实现（该提供者不支持此数据面）".into()))
    }

    /// `record_decision_trust`（宿主命令迁 T2，#347）—— 写 op。
    async fn record_decision_trust(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("record_decision_trust 未实现（该提供者不支持此数据面）".into()))
    }

    /// `reflection_stats`（宿主命令迁 T2，#347）—— 读 op。
    async fn reflection_stats(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("reflection_stats 未实现（该提供者不支持此数据面）".into()))
    }

    /// `list_time_travel_batches`（宿主命令迁 T2，#347）—— 读 op。
    async fn list_time_travel_batches(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "list_time_travel_batches 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// `start_task`（宿主长任务迁 T2，#349）—— 写 op（发起后台任务）。
    ///
    /// 入参 `{ task, input }`；task 必须命中宿主白名单（见宿主 service 派发表）。
    /// 返回 `{ taskId }`；执行体跑在宿主进程（spawn_tracked 台账）。
    async fn start_task(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("start_task 未实现（该提供者不支持此数据面）".into()))
    }

    /// `poll_task`（宿主长任务迁 T2，#349）—— 读 op。
    ///
    /// 入参 `{ taskId }`；返回 `{ status, output, result?, finishedAt? }`。
    async fn poll_task(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("poll_task 未实现（该提供者不支持此数据面）".into()))
    }

    /// `cancel_task`（宿主长任务迁 T2，#349）—— 写 op（协作式取消，与宿主 `stop_background_task` 同源）。
    async fn cancel_task(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("cancel_task 未实现（该提供者不支持此数据面）".into()))
    }

    /// `generate_stock_report`（宿主命令迁 T2，#349）—— 写 op（读库 + 行情网络 + HTML 渲染落盘）。
    async fn generate_stock_report(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("generate_stock_report 未实现（该提供者不支持此数据面）".into()))
    }

    /// `get_time_travel_batch`（宿主命令迁 T2，#349）—— 读 op（历史表权威；`ensure_stale_reaped` 不跟随，同 #338/#347 判据）。
    async fn get_time_travel_batch(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider("get_time_travel_batch 未实现（该提供者不支持此数据面）".into()))
    }

    /// 参数调整建议列表（`list_param_suggestions` op）—— 读 op，无权限闸。
    ///
    /// 入参 `{stockCode?: string}`；出参 = 建议行数组。
    async fn list_param_suggestions(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::list_param_suggestions 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 热门候选发现（`discover_stock_candidates` op）—— 读 op，无权限闸。
    ///
    /// 打分权威在 `analysis-engine::screener::StockScreener::score_candidate`（纯函数）；
    /// 宇宙来源（热门股/行业龙头/兜底池）与逐票行情由插件编排（`market.quote` 接缝），
    /// 故宿主 provider **不实现**本方法（worker 分派臂不存在，命令直接缺省报错）。
    /// 出参 = `ScreenResult` 数组的 camelCase JSON。
    async fn discover_stock_candidates(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::discover_stock_candidates 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 失效荐股缓存（`invalidate_recommendation_cache` op）—— 写 op。
    ///
    /// ⚠ 必须落在宿主进程：`analysis_engine::recommender::invalidate_cache()` 失效的是
    /// **当前进程**的进程内缓存，worker 直调只会清掉插件进程副本，而宿主的
    /// `recommend_stocks_multi` 等活消费方用的是宿主副本（§183 表的裁定依据）。
    /// 入参为空对象；出参 = `null`。
    async fn invalidate_recommendation_cache(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::invalidate_recommendation_cache 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 自改进分析循环（`run_self_improving_stock_analysis` op）—— 写 op。
    ///
    /// 执行体在宿主 agent crate（consumer 层，插件不可链接）：`SelfImprovementExecutor`
    /// + `StockAnalysisRound`。入参 `{task: string, maxRounds?: number}`；出参 =
    /// `{text, totalRounds, finalScore, confidence, strengths, gaps}`，中途出错时返回
    /// `{partial: true, ...}`（与宿主原命令同口径）。
    async fn run_self_improving_stock_analysis(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::run_self_improving_stock_analysis 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }
    /// 待审批进化计划列表（`pending_evolution_plans` op）—— 读 op，无权限闸。
    async fn pending_evolution_plans(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::pending_evolution_plans 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// Walk-Forward + DES 对比仿真（`des_integration` op）—— 纯计算 op，无权限闸。
    ///
    /// ⚠ 重 CPU（秒级）：宿主 provider 必须 `spawn_blocking` 执行，不许在 async
    /// executor 上同步跑；实现体内部自建 tokio runtime（`commands/wf_des.rs` 的同步
    /// 命令形态），**只能**在 blocking 线程上调，否则构成嵌套 runtime。
    async fn des_integration(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::des_integration 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    // ── 批 1a（#35）：十二股票 cron 管理命令的管理面 ─────────────────────────
    // 荐股 / 趋势智选 / 候选池扫描三族定时任务的 CRUD：create / list / toggle /
    // delete 各 3 条（`stock-recommendation` / `trend-screening` / `pool-scan`）。
    // 数据面是宿主 `CronJobStore`（runtime-core），**不落 sea-orm 表**；配置归一
    // （`RecoCronConfig` / `PoolScanConfig` 默认值与校验）、desc 文案、uuid 生成、
    // task_type 标记与响应形状（`RecoCronJobResponse` / `CronJobResponse`）的权威
    // 都在主壳 commands 层（`commands/recommendation_cron.rs` / `pool_scan.rs` 的
    // 无 State 核心函数）⇒ 继续按本接缝的「Value 透传」铁律：形状权威在主壳，
    // 本层不上收（同上面四条试点 op 的判据）。
    //
    // 入参 = 命令参数的 camelCase JSON（Tauri 自动映射面，如
    // `{cronExpression, periods, minConfidence, topN}`）；`list` 收 `{}` 或缺省；
    // `toggle` 收 `{id, enabled}`；`delete` 收 `{id}`；两个 create 收各自 Config
    // 参数（缺省值补齐在核心函数）。出参 = 命令原样返回值（create/list 返回
    // 响应数组或对象，toggle/delete 返回 null）。
    //
    // ## 权限闸
    //
    // **9 写**（3 create + 3 toggle + 3 delete，增删改宿主 CronJobStore）在
    // `worker.rs` 的 `required_permission_for` 登记为 `Write`；**3 读**（list）
    // 无闸（给读缝加闸会让只读插件被无端拒掉）。

    /// 创建荐股定时任务（`create_recommendation_cron` op）—— 写 op，要 `write` 权限。
    async fn create_recommendation_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::create_recommendation_cron 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 列出荐股定时任务（`list_recommendation_crons` op）—— 读 op，无权限闸。
    async fn list_recommendation_crons(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::list_recommendation_crons 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 启停荐股定时任务（`toggle_recommendation_cron` op）—— 写 op，要 `write` 权限。
    async fn toggle_recommendation_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::toggle_recommendation_cron 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 删除荐股定时任务（`delete_recommendation_cron` op）—— 写 op，要 `write` 权限。
    async fn delete_recommendation_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::delete_recommendation_cron 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 创建趋势智选定时任务（`create_trend_screening_cron` op）—— 写 op，要 `write` 权限。
    async fn create_trend_screening_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::create_trend_screening_cron 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 列出趋势智选定时任务（`list_trend_screening_crons` op）—— 读 op，无权限闸。
    async fn list_trend_screening_crons(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::list_trend_screening_crons 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 启停趋势智选定时任务（`toggle_trend_screening_cron` op）—— 写 op，要 `write` 权限。
    async fn toggle_trend_screening_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::toggle_trend_screening_cron 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 删除趋势智选定时任务（`delete_trend_screening_cron` op）—— 写 op，要 `write` 权限。
    async fn delete_trend_screening_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::delete_trend_screening_cron 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 创建候选池扫描定时任务（`create_pool_scan_cron` op）—— 写 op，要 `write` 权限。
    async fn create_pool_scan_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::create_pool_scan_cron 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 列出候选池扫描定时任务（`list_pool_scan_crons` op）—— 读 op，无权限闸。
    async fn list_pool_scan_crons(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::list_pool_scan_crons 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 启停候选池扫描定时任务（`toggle_pool_scan_cron` op）—— 写 op，要 `write` 权限。
    async fn toggle_pool_scan_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::toggle_pool_scan_cron 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 删除候选池扫描定时任务（`delete_pool_scan_cron` op）—— 写 op，要 `write` 权限。
    async fn delete_pool_scan_cron(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::delete_pool_scan_cron 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    // ── F1 余三条（#35）：estimation 快照读面 + 快照采集触发面 ────────────────
    // `list_financial_snapshots` 读宿主本地 `financial_snapshots` 表原始行——
    // 与 `market.quote::get_financials`（vendor 财报）**不等价**，形状权威是
    // entities 的 `financial_snapshots::Model` ⇒ 照 `list_recent_mainlines` 先例
    // 「实体行数组整块透传」，harness 不上收 DTO。`sweep_daily_snapshots` 是
    // **触发口**：执行体 `run_daily_snapshot_sweep` 留宿主（还被后台定时服务复用），
    // 长任务不搬进 worker 帧。入参 snake_case 接缝风格（同 `opc.store`）。

    /// 列估值快照原始行（`list_financial_snapshots` op）—— 读 op，无权限闸。
    ///
    /// 入参 `{"stock_code": str, "start": str?, "end": str?}`（`start`/`end` 为
    /// `YYYY-MM-DD` 快照日期闭区间，缺省不筛）；出参 = `financial_snapshots`
    /// 实体行数组（按 `snapshot_date` 升序，与被退役的宿主命令原样同形状）。
    async fn list_financial_snapshots(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::list_financial_snapshots 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 触发每日快照采集（`sweep_daily_snapshots` op）—— 写 op，要 `write` 权限。
    ///
    /// 入参 `{}`；出参 = 采集总结串（与被退役的宿主命令同形状）。归属日 =
    /// 最近一个**已完成**交易日的计算留宿主（盘中触发绝不把半日数据当收盘值落盘）。
    async fn sweep_daily_snapshots(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::sweep_daily_snapshots 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    // ── F6 分析师别名族六条（#35，#322）：进化与反馈面 ─────────────────────────
    // 实现权威 `evolve_node` / `get_node_evolution_status` / `NodeEvolutionStatus` /
    // `NodeType` 都在 `analysis-engine`（implementor）⇒ 照本接缝「Value 透传」铁律，
    // harness 不上收 DTO。六条全部要宿主 DB（进化统计读写 `node_quality_feedbacks`
    // 表）⇒ 插件进程够不着，必须走 op。**别名入参形状逐字保留**（§174 五处不等价）：
    // 两条 analyst 别名收单键、`get_node_evolution_status` 是泛化形本体收双键、
    // `save_analyst_feedback` 13 字段无 nodeType、`get_analyst_feedbacks` 把 Option
    // 强制成必填、`get_analyst_feedback_stats` 平铺单参——入参 schema 给 LLM 的形状
    // 由插件命名命令照宿主别名命令逐字保留，op 帧入参走 snake_case 接缝风格。
    // 权限闸登记在 `worker.rs` 的 `required_permission_for`：`evolve_analyst` 与
    // `save_analyst_feedback` 是写 op（要 `write`），四条 get_* 是读 ⇒ 无闸。

    /// 触发分析师节点自我进化（`evolve_analyst` op）—— 写 op，要 `write` 权限。
    ///
    /// 入参 `{"analyst_id": str}`（op 帧）；出参 = `NodeEvolutionStatus` 原样 JSON。
    /// 宿主 provider 委托 `evolve_node(db, &NodeType::Analyst, &analyst_id)`——
    /// 分析师别名硬编码 `NodeType::Analyst`，不得顺手开放成泛化形。
    async fn evolve_analyst(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::evolve_analyst 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取分析师节点进化状态（`get_analyst_evolution_status` op）—— 读 op，无权限闸。
    ///
    /// 入参 `{"analyst_id": str}`；出参 = `NodeEvolutionStatus` 原样 JSON（不触发进化）。
    async fn get_analyst_evolution_status(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::get_analyst_evolution_status 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 获取任意节点进化状态（`get_node_evolution_status` op）—— 读 op，无权限闸。
    ///
    /// 入参 `{"node_type": str, "node_id": str}`；出参 = `NodeEvolutionStatus` 原样
    /// JSON。**这是泛化形本体**（F6 六条里唯一的泛化形，宿主命令带 `_command` 后缀
    /// 但不是别名）；未知 `node_type` 由宿主 provider 照 `parse_node_type` 兜底
    /// `NodeType::Other`（宽容语义，与宿主命令逐字一致）。
    async fn get_node_evolution_status(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::get_node_evolution_status 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 保存分析师质量反馈（`save_analyst_feedback` op）—— 写 op，要 `write` 权限。
    ///
    /// 入参 = 13 字段请求体的 snake_case 帧形态（无 `node_type`——宿主 provider 按
    /// 分析师别名语义补 `Analyst`）；出参 = 与被退役的宿主命令逐字同形状。
    /// ⚠ 载荷 null 键是既有写侧行为（§175 已裁定属载荷卫生、读口全按缺键处理），
    /// 本 op 照原样透传，不加读侧特判。
    async fn save_analyst_feedback(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::save_analyst_feedback 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取指定分析师的质量反馈历史（`get_analyst_feedbacks` op）—— 读 op，无权限闸。
    ///
    /// 入参 = 别名请求体（**把泛化形的 `Option` 字段强制成必填**，§174 入参不等价
    /// 第 2 处——别名形状逐字保留，不得回落成可选）；出参 = 反馈历史数组原样 JSON。
    async fn get_analyst_feedbacks(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::get_analyst_feedbacks 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取分析师反馈统计（`get_analyst_feedback_stats` op）—— 读 op，无权限闸。
    ///
    /// 入参 `{"analyst_id": str}` 平铺单参（别名形状，与泛化形的平铺双参不同）；
    /// 出参 = 统计对象原样 JSON（判据「是否需要触发进化」的读数）。
    async fn get_analyst_feedback_stats(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::get_analyst_feedback_stats 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    // ── #323 T2 五命令批迁（子批②③）：token 写入面 + 决策回测读面 ──────────────
    // 宿主命令 `save_neodata_token` / `query_decision_backtest` 退役后的接缝面。
    // 两者的实现都只存在于宿主进程（token 三段效应动宿主文件/内存/DB；决策回测读
    // `stock_analyses` 表）⇒ 插件够不着，必须走 op。`save_neodata_token` 是写 op
    // （写脚本缓存 + 改共享内存 + 改 `workflow_template` 变量），权限闸登记在
    // `worker.rs` 的 `required_permission_for`；`query_decision_backtest` 纯读 ⇒ 无闸。

    /// 保存 NeoData token（`save_neodata_token` op）—— 写 op，要 `write` 权限。
    ///
    /// 入参 `{"token": str}`（非空，op 帧收 snake_case 接缝风格）；出参 `null`
    /// （与被退役的宿主命令回 `()` 同形状）。三段效应**逐字下沉**宿主 provider：
    /// ① `save_token_to_cache` 写 Python 脚本缓存；② `astock_client.neodata_token`
    /// 共享内存立即生效（无需重启）；③ `workflow_template` 的 `stock-analysis`
    /// 模板变量 `vendor_neodata_token` 替换或新增（`is_secret = true`）。
    async fn save_neodata_token(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::save_neodata_token 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 查询决策回测分析（`query_decision_backtest` op）—— 读 op，无权限闸。
    ///
    /// 入参 `{"limit": u32?}`（缺省 100，宿主 provider 钳 500）；出参 = 手拼投影数组
    /// **整块透传**（`stockCode` / `stockName` / `analysisDate` / `formulaAction` /
    /// `llmAction` / `outcome` / `decisionConfidence` / `userTrustDecision`）——形状
    /// 权威在被退役的宿主命令（`stock_workflow/misc.rs`），照 `list_financial_snapshots`
    /// 先例 harness 不上收 DTO。查询面：`Outcome` / `DecisionAction` 双 `IS NOT NULL`
    /// 过滤 + `AnalysisDate` 降序（与被退役的宿主命令逐字等价）。
    async fn query_decision_backtest(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::query_decision_backtest 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    // ── #324 T2 F7 回测验证三条迁（§160 分档表 F7）────────────────────────────
    // 宿主命令 `run_decision_backtest` / `list_decision_validations` /
    // `compute_validation_report`（`commands/backtest_validation.rs`）退役后的接缝面。
    // 三条实现体都只在宿主进程（decision_validations / reco_picks 表 + 行情 API 回放）
    // ⇒ 插件够不着，必须走 op。`run_decision_backtest` 是写 op（写 decision_validations
    // 行，权限闸登记在 `worker.rs` 的 `required_permission_for`）；两条读 op 无闸。
    // 三个静态门（check-generation-scope-registry / check-validation-window-authority /
    // check-reco-generation）按「文件+fn 名」钉实现体 ⇒ 宿主侧抽 State 化保号留原文件。

    /// 跑决策回测（`run_decision_backtest` op）—— **写 op**，要 `write` 权限。
    ///
    /// 入参 `{"request": {tPlusNList?, excludeSynthetic?, maxPicks?, dryRun?,
    /// periodFilter?, includeReplay?}}` 全可选（camelCase，形状权威在宿主
    /// `RunDecisionBacktestRequest` 的 serde 结构，`default` 语义 `{}` = 全默认）；
    /// 出参 = `RunDecisionBacktestResponse` 序列化整块透传（harness 不上收 DTO）。
    /// 执行体 `run_decision_backtest_inner` 留宿主（cron 与 CLI `axagent-validate-backfill`
    /// 共用同一实现，接缝只做触发口，照 `sweep_daily_snapshots` 先例）。
    async fn run_decision_backtest(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::run_decision_backtest 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 列出决策验证记录（`list_decision_validations` op）—— 读 op，无权限闸。
    ///
    /// 入参 `{"stockCode"?, "hitOutcome"?, "limit"?, "offset"?, "generation"?}`
    /// 全可选；出参 = `DecisionValidationItem` 数组整块透传（harness 不上收 DTO）。
    /// 代际口径：默认只列实盘验证记录，`generation` = 某次回放的截止日（显式传入，
    /// 两套样本不并排）—— 与被退役的宿主命令逐字等价。
    async fn list_decision_validations(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::list_decision_validations 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 计算验证报告（`compute_validation_report` op）—— 读 op，无权限闸。
    ///
    /// 入参 `{"generation"?}`（None = 实盘出票一代）；出参 =
    /// `ValidationReportResponse` 序列化整块透传（harness 不上收 DTO）。分母筛样
    /// 按荐股代际常量（statistics 类，门钉），T+N 窗口按 `is_validation_window`
    /// 现场推导（B3 权威门钉）—— 两判据都在宿主实现体里，接缝只透传。
    async fn compute_validation_report(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "StockAnalysisProvider::compute_validation_report 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 缺省实现必须显式报「不支持此数据面」，不得静默返回空值
    /// （空值会被工具侧读成「全部缺席但查询成功」，那是另一种谎）。
    struct Stub;

    #[async_trait]
    impl StockAnalysisProvider for Stub {}

    #[test]
    fn default_impls_reject_explicitly() {
        let stub = Stub;
        // harness 内既有异步断言一律用 `futures::executor::block_on`（见
        // `analysis_record.rs` / `opc_store.rs` 的测试），不为一条断言引入异步测试属性。
        let cases = [
            (
                "industry_constituents",
                futures::executor::block_on(stub.industry_constituents(Value::Null)),
            ),
            (
                "batch_upsert_mainlines",
                futures::executor::block_on(stub.batch_upsert_mainlines(Value::Null)),
            ),
            (
                "list_recent_mainlines",
                futures::executor::block_on(stub.list_recent_mainlines(Value::Null)),
            ),
            ("stock_factor", futures::executor::block_on(stub.stock_factor(Value::Null))),
            ("list_financial_snapshots", {
                futures::executor::block_on(stub.list_financial_snapshots(Value::Null))
            }),
            (
                "sweep_daily_snapshots",
                futures::executor::block_on(stub.sweep_daily_snapshots(Value::Null)),
            ),
            // F6 别名族六 op（#322）：缺省实现同样必须显式报错，不得返回空值
            // （空数组会被读成「该分析师没有任何反馈」，那是另一种谎）。
            ("evolve_analyst", futures::executor::block_on(stub.evolve_analyst(Value::Null))),
            (
                "get_analyst_evolution_status",
                futures::executor::block_on(stub.get_analyst_evolution_status(Value::Null)),
            ),
            (
                "get_node_evolution_status",
                futures::executor::block_on(stub.get_node_evolution_status(Value::Null)),
            ),
            (
                "save_analyst_feedback",
                futures::executor::block_on(stub.save_analyst_feedback(Value::Null)),
            ),
            (
                "get_analyst_feedbacks",
                futures::executor::block_on(stub.get_analyst_feedbacks(Value::Null)),
            ),
            (
                "get_analyst_feedback_stats",
                futures::executor::block_on(stub.get_analyst_feedback_stats(Value::Null)),
            ),
            ("pipeline_history", futures::executor::block_on(stub.pipeline_history(Value::Null))),
            (
                "pipeline_run_detail",
                futures::executor::block_on(stub.pipeline_run_detail(Value::Null)),
            ),
            (
                "pending_evolution_plans",
                futures::executor::block_on(stub.pending_evolution_plans(Value::Null)),
            ),
            ("des_integration", futures::executor::block_on(stub.des_integration(Value::Null))),
            // 批 1a（#35）十二 cron 管理 op：默认实现同样必须显式报错，不得返回空值
            // （空数组会被前端读成「这类定时任务一个都没有」，那是另一种谎）。
            (
                "create_recommendation_cron",
                futures::executor::block_on(stub.create_recommendation_cron(Value::Null)),
            ),
            (
                "list_recommendation_crons",
                futures::executor::block_on(stub.list_recommendation_crons(Value::Null)),
            ),
            (
                "toggle_recommendation_cron",
                futures::executor::block_on(stub.toggle_recommendation_cron(Value::Null)),
            ),
            (
                "delete_recommendation_cron",
                futures::executor::block_on(stub.delete_recommendation_cron(Value::Null)),
            ),
            (
                "create_trend_screening_cron",
                futures::executor::block_on(stub.create_trend_screening_cron(Value::Null)),
            ),
            (
                "list_trend_screening_crons",
                futures::executor::block_on(stub.list_trend_screening_crons(Value::Null)),
            ),
            (
                "toggle_trend_screening_cron",
                futures::executor::block_on(stub.toggle_trend_screening_cron(Value::Null)),
            ),
            (
                "delete_trend_screening_cron",
                futures::executor::block_on(stub.delete_trend_screening_cron(Value::Null)),
            ),
            (
                "create_pool_scan_cron",
                futures::executor::block_on(stub.create_pool_scan_cron(Value::Null)),
            ),
            (
                "list_pool_scan_crons",
                futures::executor::block_on(stub.list_pool_scan_crons(Value::Null)),
            ),
            (
                "toggle_pool_scan_cron",
                futures::executor::block_on(stub.toggle_pool_scan_cron(Value::Null)),
            ),
            (
                "delete_pool_scan_cron",
                futures::executor::block_on(stub.delete_pool_scan_cron(Value::Null)),
            ),
            // #323 T2 五命令批迁（子批②③）：默认实现同样必须显式报错
            // （空值会被读成「token 已保存 / 回测记录为空」，那是另一种谎）。
            (
                "save_neodata_token",
                futures::executor::block_on(stub.save_neodata_token(Value::Null)),
            ),
            (
                "query_decision_backtest",
                futures::executor::block_on(stub.query_decision_backtest(Value::Null)),
            ),
            // #324 T2 F7 回测验证三条迁：默认实现同样必须显式报错
            // （空值会被读成「回测已跑 / 记录为空 / 报告为空」，那是另一种谎）。
            (
                "run_decision_backtest",
                futures::executor::block_on(stub.run_decision_backtest(Value::Null)),
            ),
            (
                "list_decision_validations",
                futures::executor::block_on(stub.list_decision_validations(Value::Null)),
            ),
            (
                "compute_validation_report",
                futures::executor::block_on(stub.compute_validation_report(Value::Null)),
            ),
            // #328 两条 DB 直读迁：默认实现同样必须显式报错
            // （空值会被读成「该股没有分析记录 / 没有任何参数建议」，那是另一种谎）。
            (
                "latest_analysis_for_stock",
                futures::executor::block_on(stub.latest_analysis_for_stock(Value::Null)),
            ),
            (
                "list_param_suggestions",
                futures::executor::block_on(stub.list_param_suggestions(Value::Null)),
            ),
            (
                "latest_analyses_for_stocks",
                futures::executor::block_on(stub.latest_analyses_for_stocks(Value::Null)),
            ),
            (
                "evolution_drift_timeline",
                futures::executor::block_on(stub.evolution_drift_timeline(Value::Null)),
            ),
            (
                "agreement_score_history",
                futures::executor::block_on(stub.agreement_score_history(Value::Null)),
            ),
            ("node_feedbacks", futures::executor::block_on(stub.node_feedbacks(Value::Null))),
            (
                "node_feedback_stats",
                futures::executor::block_on(stub.node_feedback_stats(Value::Null)),
            ),
            ("monthly_report", futures::executor::block_on(stub.monthly_report(Value::Null))),
            (
                "macro_data_snapshot",
                futures::executor::block_on(stub.macro_data_snapshot(Value::Null)),
            ),
            (
                "mover_reflow_candidates",
                futures::executor::block_on(stub.mover_reflow_candidates(Value::Null)),
            ),
            // #335 两条：纯计算 op（回测反馈分析 / VLM 截图解析），默认实现同样必须
            // 显式报错（空对象入参不能被读成「解析成功 / 报告为空」）。
            (
                "analyze_backtest_feedback",
                futures::executor::block_on(stub.analyze_backtest_feedback(Value::Null)),
            ),
            (
                "parse_vlm_portfolio_screenshot",
                futures::executor::block_on(stub.parse_vlm_portfolio_screenshot(Value::Null)),
            ),
            // #336 四条（quant 3 读 + register_rhai 写）：默认实现同样必须显式报错
            // （空对象入参不能被读成「记录不存在 / 列表为空 / 注册成功」）。
            ("quant_run_get", futures::executor::block_on(stub.quant_run_get(Value::Null))),
            (
                "quant_strategies_list",
                futures::executor::block_on(stub.quant_strategies_list(Value::Null)),
            ),
            (
                "quant_metrics_compare",
                futures::executor::block_on(stub.quant_metrics_compare(Value::Null)),
            ),
            (
                "quant_strategy_register_rhai",
                futures::executor::block_on(stub.quant_strategy_register_rhai(Value::Null)),
            ),
            // #337 两条（写语义）：默认实现同样必须显式报错（空对象入参不能被读成
            // 「vendor 状态已改 / 取消已下发」）。
            ("set_vendor_status", futures::executor::block_on(stub.set_vendor_status(Value::Null))),
            (
                "set_primary_market_vendor",
                futures::executor::block_on(stub.set_primary_market_vendor(Value::Null)),
            ),
            (
                "cancel_stock_analysis",
                futures::executor::block_on(stub.cancel_stock_analysis(Value::Null)),
            ),
            ("parse_trades_csv", futures::executor::block_on(stub.parse_trades_csv(Value::Null))),
            (
                "evolve_node_command",
                futures::executor::block_on(stub.evolve_node_command(Value::Null)),
            ),
            (
                "save_node_feedback",
                futures::executor::block_on(stub.save_node_feedback(Value::Null)),
            ),
            (
                "serenity_validation_summary",
                futures::executor::block_on(stub.serenity_validation_summary(Value::Null)),
            ),
            (
                "create_decision_backtest_cron",
                futures::executor::block_on(stub.create_decision_backtest_cron(Value::Null)),
            ),
            (
                "get_evolution_drift_dashboard",
                futures::executor::block_on(stub.get_evolution_drift_dashboard(Value::Null)),
            ),
            (
                "get_evolution_history",
                futures::executor::block_on(stub.get_evolution_history(Value::Null)),
            ),
            (
                "stock_evolution_recalc",
                futures::executor::block_on(stub.stock_evolution_recalc(Value::Null)),
            ),
            (
                "get_mover_reflow_merge",
                futures::executor::block_on(stub.get_mover_reflow_merge(Value::Null)),
            ),
            (
                "set_mover_reflow_merge",
                futures::executor::block_on(stub.set_mover_reflow_merge(Value::Null)),
            ),
            (
                "get_mover_asof_summary",
                futures::executor::block_on(stub.get_mover_asof_summary(Value::Null)),
            ),
            (
                "get_reco_board_scope",
                futures::executor::block_on(stub.get_reco_board_scope(Value::Null)),
            ),
            (
                "set_reco_board_scope",
                futures::executor::block_on(stub.set_reco_board_scope(Value::Null)),
            ),
            (
                "get_reco_pool_batch",
                futures::executor::block_on(stub.get_reco_pool_batch(Value::Null)),
            ),
            (
                "get_reco_strategy_weights",
                futures::executor::block_on(stub.get_reco_strategy_weights(Value::Null)),
            ),
            (
                "manual_recalc_strategy_weights",
                futures::executor::block_on(stub.manual_recalc_strategy_weights(Value::Null)),
            ),
            (
                "recalc_reco_loop_weights",
                futures::executor::block_on(stub.recalc_reco_loop_weights(Value::Null)),
            ),
            (
                "apply_param_suggestions",
                futures::executor::block_on(stub.apply_param_suggestions(Value::Null)),
            ),
            (
                "rollback_param_change",
                futures::executor::block_on(stub.rollback_param_change(Value::Null)),
            ),
            (
                "apply_reco_weights",
                futures::executor::block_on(stub.apply_reco_weights(Value::Null)),
            ),
            (
                "calibrate_portfolio_mgr_params",
                futures::executor::block_on(stub.calibrate_portfolio_mgr_params(Value::Null)),
            ),
            (
                "optimize_scoring_weights",
                futures::executor::block_on(stub.optimize_scoring_weights(Value::Null)),
            ),
            (
                "compute_valuation_band",
                futures::executor::block_on(stub.compute_valuation_band(Value::Null)),
            ),
            ("list_stock_tools", futures::executor::block_on(stub.list_stock_tools(Value::Null))),
            ("compute_what_if", futures::executor::block_on(stub.compute_what_if(Value::Null))),
            ("reflection_stats", futures::executor::block_on(stub.reflection_stats(Value::Null))),
            (
                "record_decision_trust",
                futures::executor::block_on(stub.record_decision_trust(Value::Null)),
            ),
            ("get_reco_detail", futures::executor::block_on(stub.get_reco_detail(Value::Null))),
            ("list_reco_history", futures::executor::block_on(stub.list_reco_history(Value::Null))),
            (
                "save_eastmoney_proxy",
                futures::executor::block_on(stub.save_eastmoney_proxy(Value::Null)),
            ),
            (
                "get_cached_recommendation",
                futures::executor::block_on(stub.get_cached_recommendation(Value::Null)),
            ),
            (
                "get_reco_signal_history",
                futures::executor::block_on(stub.get_reco_signal_history(Value::Null)),
            ),
            ("reco_ic_stats", futures::executor::block_on(stub.reco_ic_stats(Value::Null))),
            (
                "list_time_travel_batches",
                futures::executor::block_on(stub.list_time_travel_batches(Value::Null)),
            ),
            // #349 批：异步任务三 op + 同步面两条（默认实现必须显式报错）。
            ("start_task", futures::executor::block_on(stub.start_task(Value::Null))),
            ("poll_task", futures::executor::block_on(stub.poll_task(Value::Null))),
            ("cancel_task", futures::executor::block_on(stub.cancel_task(Value::Null))),
            (
                "generate_stock_report",
                futures::executor::block_on(stub.generate_stock_report(Value::Null)),
            ),
            (
                "get_time_travel_batch",
                futures::executor::block_on(stub.get_time_travel_batch(Value::Null)),
            ),
            // #344 批 B18：29 条（默认实现必须显式报错）。
            (
                "get_asof_degradation_count",
                futures::executor::block_on(stub.get_asof_degradation_count(Value::Null)),
            ),
            (
                "get_asof_degradation_log",
                futures::executor::block_on(stub.get_asof_degradation_log(Value::Null)),
            ),
            ("get_market_status", futures::executor::block_on(stub.get_market_status(Value::Null))),
            (
                "get_vendor_health_all",
                futures::executor::block_on(stub.get_vendor_health_all(Value::Null)),
            ),
            (
                "get_vendor_fallback_log",
                futures::executor::block_on(stub.get_vendor_fallback_log(Value::Null)),
            ),
            (
                "get_interface_health_history",
                futures::executor::block_on(stub.get_interface_health_history(Value::Null)),
            ),
            (
                "get_valuation_params",
                futures::executor::block_on(stub.get_valuation_params(Value::Null)),
            ),
            (
                "save_valuation_params",
                futures::executor::block_on(stub.save_valuation_params(Value::Null)),
            ),
            (
                "get_stock_template_version_status",
                futures::executor::block_on(stub.get_stock_template_version_status(Value::Null)),
            ),
            (
                "list_stock_analyses",
                futures::executor::block_on(stub.list_stock_analyses(Value::Null)),
            ),
            (
                "get_recent_analyses",
                futures::executor::block_on(stub.get_recent_analyses(Value::Null)),
            ),
            ("list_reflections", futures::executor::block_on(stub.list_reflections(Value::Null))),
            (
                "get_stock_analysis",
                futures::executor::block_on(stub.get_stock_analysis(Value::Null)),
            ),
            (
                "audit_decision_evidence",
                futures::executor::block_on(stub.audit_decision_evidence(Value::Null)),
            ),
            (
                "rename_stock_analysis",
                futures::executor::block_on(stub.rename_stock_analysis(Value::Null)),
            ),
            ("list_price_alerts", futures::executor::block_on(stub.list_price_alerts(Value::Null))),
            (
                "create_price_alert",
                futures::executor::block_on(stub.create_price_alert(Value::Null)),
            ),
            ("monitor_status", futures::executor::block_on(stub.monitor_status(Value::Null))),
            ("get_t0_config", futures::executor::block_on(stub.get_t0_config(Value::Null))),
            ("set_t0_config", futures::executor::block_on(stub.set_t0_config(Value::Null))),
            (
                "get_portfolio_dashboard",
                futures::executor::block_on(stub.get_portfolio_dashboard(Value::Null)),
            ),
            (
                "refresh_portfolio_metrics",
                futures::executor::block_on(stub.refresh_portfolio_metrics(Value::Null)),
            ),
            (
                "check_position_limits",
                futures::executor::block_on(stub.check_position_limits(Value::Null)),
            ),
            (
                "approve_evolution_plan",
                futures::executor::block_on(stub.approve_evolution_plan(Value::Null)),
            ),
            (
                "list_mover_recall_runs",
                futures::executor::block_on(stub.list_mover_recall_runs(Value::Null)),
            ),
            (
                "get_mover_recall_run_view",
                futures::executor::block_on(stub.get_mover_recall_run_view(Value::Null)),
            ),
            (
                "get_mover_recall_event_review",
                futures::executor::block_on(stub.get_mover_recall_event_review(Value::Null)),
            ),
            (
                "list_param_changes",
                futures::executor::block_on(stub.list_param_changes(Value::Null)),
            ),
            (
                "evaluate_param_changes",
                futures::executor::block_on(stub.evaluate_param_changes(Value::Null)),
            ),
            // #343 批 B17：组合/交易 6 + 进化审批 2，默认实现必须显式报错。
            ("list_portfolio", futures::executor::block_on(stub.list_portfolio(Value::Null))),
            (
                "add_portfolio_holding",
                futures::executor::block_on(stub.add_portfolio_holding(Value::Null)),
            ),
            (
                "remove_portfolio_holding",
                futures::executor::block_on(stub.remove_portfolio_holding(Value::Null)),
            ),
            (
                "get_portfolio_correlations",
                futures::executor::block_on(stub.get_portfolio_correlations(Value::Null)),
            ),
            (
                "toggle_trading_enabled",
                futures::executor::block_on(stub.toggle_trading_enabled(Value::Null)),
            ),
            ("import_trades", futures::executor::block_on(stub.import_trades(Value::Null))),
            (
                "reject_evolution_plan",
                futures::executor::block_on(stub.reject_evolution_plan(Value::Null)),
            ),
            (
                "rollback_evolution",
                futures::executor::block_on(stub.rollback_evolution(Value::Null)),
            ),
            // #342 批 A15：watchlist 5 + cron 9（四处读十写），默认实现同样必须显式报错
            // （空入参不能被读成「列表为空 / 已创建」）。
            ("list_watchlist", futures::executor::block_on(stub.list_watchlist(Value::Null))),
            ("add_to_watchlist", futures::executor::block_on(stub.add_to_watchlist(Value::Null))),
            (
                "watchlist_update_group",
                futures::executor::block_on(stub.watchlist_update_group(Value::Null)),
            ),
            (
                "watchlist_list_groups",
                futures::executor::block_on(stub.watchlist_list_groups(Value::Null)),
            ),
            (
                "watchlist_save_groups",
                futures::executor::block_on(stub.watchlist_save_groups(Value::Null)),
            ),
            ("create_stock_cron", futures::executor::block_on(stub.create_stock_cron(Value::Null))),
            ("list_stock_crons", futures::executor::block_on(stub.list_stock_crons(Value::Null))),
            ("toggle_stock_cron", futures::executor::block_on(stub.toggle_stock_cron(Value::Null))),
            (
                "create_validate_decisions_cron",
                futures::executor::block_on(stub.create_validate_decisions_cron(Value::Null)),
            ),
            (
                "list_validate_decisions_crons",
                futures::executor::block_on(stub.list_validate_decisions_crons(Value::Null)),
            ),
            (
                "toggle_validate_decisions_cron",
                futures::executor::block_on(stub.toggle_validate_decisions_cron(Value::Null)),
            ),
            (
                "update_validate_decisions_cron",
                futures::executor::block_on(stub.update_validate_decisions_cron(Value::Null)),
            ),
            (
                "create_batch_reflection_cron",
                futures::executor::block_on(stub.create_batch_reflection_cron(Value::Null)),
            ),
            (
                "create_watchlist_scan_cron",
                futures::executor::block_on(stub.create_watchlist_scan_cron(Value::Null)),
            ),
            // #338 三条（mover_recall_analysis：两读一写）：默认实现同样必须显式报错
            // （空对象入参不能被读成「批次不存在 / 历史为空 / 已发起」）。
            (
                "mover_recall_analysis_batch",
                futures::executor::block_on(stub.mover_recall_analysis_batch(Value::Null)),
            ),
            (
                "mover_recall_analysis_batches",
                futures::executor::block_on(stub.mover_recall_analysis_batches(Value::Null)),
            ),
            (
                "run_mover_recall_analysis",
                futures::executor::block_on(stub.run_mover_recall_analysis(Value::Null)),
            ),
            // #329 三条：默认实现同样必须显式报错（discover 的宿主 provider 不实现——
            // 编排在插件侧；后两条是写 op，空对象入参不能被读成「缓存已失效 / 循环完成」）。
            (
                "discover_stock_candidates",
                futures::executor::block_on(stub.discover_stock_candidates(Value::Null)),
            ),
            (
                "invalidate_recommendation_cache",
                futures::executor::block_on(stub.invalidate_recommendation_cache(Value::Null)),
            ),
            (
                "run_self_improving_stock_analysis",
                futures::executor::block_on(stub.run_self_improving_stock_analysis(Value::Null)),
            ),
        ];
        for (name, result) in cases {
            let err = result.expect_err(&format!("{name} 缺省实现应报错"));
            assert!(err.to_string().contains("未实现"), "{name} 实际: {err}");
        }
    }
}
