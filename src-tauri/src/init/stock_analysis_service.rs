// SPDX-License-Identifier: AGPL-3.0-only

//! `stock.analysis` 接缝的**宿主侧实现**（wiring 层）。
//!
//! 契约在 `axagent-core/crates/harness/src/stock_analysis.rs`（七个 op：五个读/纯计算 +
//! 一个写）。前三个 op 的实现体在 `analysis-engine`（implementor，持 `entities`）；后四个
//! （T2 试点批 #248）的形状权威在主壳 commands 层 —— pipeline 两行直接调
//! `commands/stock_pipeline/core.rs` 抽出的**无 State 核心**，进化计划走 dao 的
//! `list_pending_asc`，DES 仿真把入参交回 `commands/wf_des.rs` 的权威实现。本文件是宿主
//! 与接缝之间**唯一**的 `Value` ↔ 域 DTO 转换点。
//!
//! ## 为什么转换点必须唯一
//!
//! harness 是 foundation，按依赖铁律不得依赖 `analysis-engine`（implementor）⇒ 接缝上
//! 只能传 `serde_json::Value`，DTO 权威仍在域包。于是「JSON → DTO」这一步必须只出现在
//! 一处：本文件。插件侧、worker 分派层都只透传，任何一层多做一次字段改写都是第二套形状
//! （禁区 12），而第二套形状的失败形态是**静默**的（字段名漂一个字母不会报错，
//! `serde` 的 `#[serde(default)]` 会把它当成「调用方没给」）。
//!
//! ## 为什么不在这层写 SQL
//!
//! 三个权威函数（`fetch_industry_constituents` / `batch_upsert_mainlines` /
//! `fetch_stock_chg_1m`）已经是 `analysis-engine` 的**唯一实现**，宿主 IPC 命令
//! （`src/commands/` 侧）与工作流工具此前都指向它们。本层再拼一次 `Entity::find()` 就会
//! 出现两份列清单 / 两套归一逻辑（同 `opc_store_service.rs` 模块头的判据）。
//!
//! ## 与工具出参信封的关系
//!
//! ⚠ **接缝出参不是工具出参信封**：`{status, data, detail}` / `{inserted, updated,
//! archived}` 这层包装归**工具**（`axagent-runtime/crates/tools/src/tools/`），本层只回域
//! DTO 的 JSON（`{groups, universeTotal, …}` / `{rows, requested, …}`）。判据是「信封是
//! 工具面契约，换调用通道（工作流工具 / 智能体命令 / 前端 invoke）时形状不变的是信封之外的
//! 数据面」。把信封也搬进接缝，会让另一个消费方（插件命令的直接调用者）拿到一个它不认识的
//! `data` 层套娃。
//!
//! ## 闸
//!
//! - `industry_constituents`：空 `industries` ⇒ `Err(Validation)`。判据沿用工具侧原文
//!   （「至少给一个行业名」）—— 空清单换来的是「全宇宙空结果」，那看起来像「这些行业没有
//!   成分股」，是另一种谎。
//! - `stock_factor`：入参是**宽容形态**的代码清单（字符串数组 / 代码对象数组 /
//!   `{stocks: [...]}` 信封），与工具入参逐字相同 —— `parse_stock_refs` 的权威在
//!   `analysis-engine`（implementor），工具侧（consumer）不得依赖它，故归一只能在此处。
//!   归一后为空 ⇒ `Err(Validation)`。同判据。
//! - `batch_upsert_mainlines`：**不做字段级闸**，一律交给 `BatchUpsertInput` 的 `serde`
//!   （它的 `#[serde(default)]` + `alias` 是唯一权威，且缺 `mainline_date` 必须报错 ——
//!   见 `market_mainline.rs` 的 `missing_required_field_is_an_error`）。本层预判一次
//!   「字段在不在」就会与该 DTO 的演进脱钩。

use async_trait::async_trait;
// F6 别名族（#322）：analyst 进化/反馈的形状权威 —— evolve 两函数在 analysis-engine
// （implementor），反馈三族走泛化形命令抽出的无 State 核心（前端仍直调，同一份实现）。
use crate::commands::analyst_evolution::parse_node_type;
use crate::commands::analyst_feedback::{
    GetNodeFeedbacksRequest, SaveNodeFeedbackRequest, get_node_feedback_stats_core,
    get_node_feedbacks_core, save_node_feedback_core,
};
use axagent_analysis_engine::industry_constituents::{
    IndustryConstituentsInput, fetch_industry_constituents,
};
use axagent_analysis_engine::market_mainline::{
    BatchUpsertInput, batch_upsert_mainlines, list_recent_mainlines,
};
use axagent_analysis_engine::node_evolution::{evolve_node, get_node_evolution_status};
use axagent_analysis_engine::node_quality::NodeType;
use axagent_analysis_engine::stock_factor::{fetch_stock_chg_1m, parse_stock_refs};
// T2 试点批（#248）：pipeline 两行的形状权威在主壳 commands 层 —— 无 State 核心是
// IPC 命令与本接缝的**同一份**列清单（第二份 `json!` 就是第二套形状，禁区 12）。
use crate::commands::pool_scan::{
    PoolCreateRequest, create_pool_scan_cron_core, delete_pool_scan_cron_core,
    list_pool_scan_crons_core, toggle_pool_scan_cron_core,
};
use crate::commands::recommendation_cron::{
    CronIdRequest, CronToggleRequest, RecoCreateRequest, TrendCreateRequest,
    create_recommendation_cron_core, create_trend_screening_cron_core,
    delete_recommendation_cron_core, delete_trend_screening_cron_core,
    list_recommendation_crons_core, list_trend_screening_crons_core,
    toggle_recommendation_cron_core, toggle_trend_screening_cron_core,
};
use crate::commands::stock_analysis::{
    agreement_score_history_core, evolution_drift_timeline_core,
    get_latest_analyses_for_stocks_core, get_latest_analysis_for_stock_core,
    list_param_suggestions_core, monthly_report_core, run_daily_snapshot_sweep,
    snapshot_sweep_date,
};
use crate::commands::stock_pipeline::core::{pipeline_history_rows, pipeline_run_detail_row};
use crate::commands::wf_des::{WfDesRequest, wf_des_integration};
use axagent_agent::self_improvement_executor::{SelfImprovementConfig, SelfImprovementExecutor};
use axagent_analysis_engine::stock_analysis_round::StockAnalysisRound;
use axagent_astock_data::AStockClient;
use axagent_dao::repo::stock_evolution_history as evo_hist;
use axagent_entities::{financial_snapshots, stock_analyses};
use axagent_harness::DatabaseConnection;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::stock_analysis::StockAnalysisProvider;
use axagent_runtime_core::CronJobStore;
use dashmap::DashMap;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// `list_recent_mainlines` op 的缺省窗口（宿主命令 `market_mainline_list_recent` 原值，#304 C3）。
const DEFAULT_RECENT_DAYS: usize = 7;
/// `list_recent_mainlines` op 的入参上界（防单次查询拖全表；同源：被退役的宿主命令）。
const MAX_RECENT_DAYS: usize = 90;

/// `stock_factor` op 的入参：**宽容形态原样**（与工具入参逐字相同）。
///
/// 归一必须在这里做、也只在这里做：`parse_stock_refs` 的权威实现在
/// `analysis-engine`（implementor），工具侧（`axagent-runtime/crates/tools`，consumer）
/// 不得依赖它 —— 那正是本批要消掉的跨包口。工具只做「明显缺料」闸，宽容扁平化
/// （字符串数组 / 对象数组 / `{stocks: [...]}` 信封）归本层。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StockFactorRequest {
    /// 宽容形态的股票代码清单，交给 `parse_stock_refs` 归一。
    ///
    /// 主名 camelCase（禁区 13），但工具与工作流模板传的是 snake_case 的 `stock_codes`
    /// （插件侧原样透传、不改名）⇒ 必须收下 alias，否则整个读 op 会因「缺字段」失败。
    #[serde(alias = "stock_codes")]
    stock_codes: Value,
    /// 可选截止日（`YYYY-MM-DD`）；缺省 = 各票序列最新。
    #[serde(default, alias = "as_of_date")]
    as_of_date: Option<String>,
}

/// `stock.analysis` 的宿主实现。
pub struct HostStockAnalysis {
    /// 与 `AppState.harness.db()` **同一个**连接池句柄（Clone 是句柄语义，不是第二套连接）。
    db: DatabaseConnection,
    /// 与 `AppState.cron_job_store` **同一个**实例（`Arc` 共享，不是第二份任务表）——
    /// 定时任务的读改必须与调度器、宿主命令看到同一份状态，否则插件建的任务永不触发。
    cron_job_store: Arc<CronJobStore>,
    /// F1 余三条（#35）：`sweep_daily_snapshots` 是触发口，真正的采集走 vendor 网络
    /// ⇒ 执行体 `run_daily_snapshot_sweep` 仍留宿主（后台定时服务同源复用），本层只传客户端。
    astock_client: Arc<AStockClient>,
    /// 与 `AppState.agent_cancel_tokens` **同一个**实例（`Arc` 共享）—— `cancel_stock_analysis`
    /// 翻转的就是运行中分析任务在看的那个令牌，必须是同一份，否则插件取消的令牌没人读。
    agent_cancel_tokens: Arc<DashMap<String, Arc<AtomicBool>>>,
    /// 与 `AppState.time_travel_batch_lock` **同一把**锁（`Arc` 共享）—— `#338` 的
    /// `run_mover_recall_analysis` 与时间旅行批次共用全局 as-of 批次互斥；插件发起必须
    /// 与宿主其他入口抢同一把，否则两批 as-of 回放会并跑。
    time_travel_batch_lock: Arc<tokio::sync::Mutex<()>>,
    /// 与 `AppState.work_engine` **同一个**实例 —— 漏检补分析批次的执行链走工作流引擎。
    work_engine: Arc<axagent_runtime::work_engine::WorkEngine>,
    /// 与 `StockDomainState.trading_engine` **同一个** `Arc`（#344：组合仪表盘 /
    /// 快照刷新 / 开仓校验三个 op 读它取持仓）。
    trading_engine: Arc<tokio::sync::RwLock<axagent_analysis_engine::trading::TradingEngine>>,
    /// 与 `StockDomainState.stock_monitor` **同一个** `Arc<OnceLock>`（#344：监控读数 /
    /// T+0 配置 / 价格告警入引擎；引擎惰性装配 ⇒ 用共享 OnceLock 而非当前值快照）。
    stock_monitor: Arc<std::sync::OnceLock<Arc<axagent_analysis_engine::monitor::RealtimeMonitor>>>,
    /// 与 `StockDomainState.stock_adaptive_engine` **同一个** `Arc`（#344：进化审批执行）。
    stock_adaptive_engine: Arc<axagent_analysis_engine::stock_adaptive_engine::StockAdaptiveEngine>,
    /// Tauri 句柄（#349）：`start_task` 经 `spawn_tracked_impl` 写台账并 emit 任务事件。
    /// 离线 bin（`create_app_state`）无句柄 ⇒ `None`（台账照写、事件不发，见 #349）。
    app_handle: Option<tauri::AppHandle>,
    /// 与 `AppState.vector_store` **同一个**实例（#349：手动反思任务走 RAG）。
    vector_store: Arc<axagent_search::vector_store::VectorStore>,
    /// 与 `AppState.trajectory_storage` **同一个**实例（#349：手动反思持久化 trajectory）。
    trajectory_storage: Arc<axagent_trajectory::TrajectoryStorage>,
    /// 与 `AppState.harness.master_key()` 同源（#349：反思链 RAG 写入）。按值持有（Copy）。
    master_key: [u8; 32],
}

impl HostStockAnalysis {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: DatabaseConnection,
        cron_job_store: Arc<CronJobStore>,
        astock_client: Arc<AStockClient>,
        agent_cancel_tokens: Arc<DashMap<String, Arc<AtomicBool>>>,
        time_travel_batch_lock: Arc<tokio::sync::Mutex<()>>,
        work_engine: Arc<axagent_runtime::work_engine::WorkEngine>,
        trading_engine: Arc<tokio::sync::RwLock<axagent_analysis_engine::trading::TradingEngine>>,
        stock_monitor: Arc<
            std::sync::OnceLock<Arc<axagent_analysis_engine::monitor::RealtimeMonitor>>,
        >,
        stock_adaptive_engine: Arc<
            axagent_analysis_engine::stock_adaptive_engine::StockAdaptiveEngine,
        >,
        app_handle: Option<tauri::AppHandle>,
        vector_store: Arc<axagent_search::vector_store::VectorStore>,
        trajectory_storage: Arc<axagent_trajectory::TrajectoryStorage>,
        master_key: [u8; 32],
    ) -> Self {
        Self {
            db,
            cron_job_store,
            astock_client,
            agent_cancel_tokens,
            time_travel_batch_lock,
            work_engine,
            trading_engine,
            stock_monitor,
            stock_adaptive_engine,
            app_handle,
            vector_store,
            trajectory_storage,
            master_key,
        }
    }

    /// 无参 op 的入参闸：只收 `null` / `{}`。
    fn require_no_params(op: &str, input: &Value) -> Result<()> {
        if !input.is_null() && !input.as_object().is_some_and(|o| o.is_empty()) {
            return Err(AxAgentError::Validation(format!(
                "stock.analysis::{op} 不收参数（期望 {{}} 或缺省）"
            )));
        }
        Ok(())
    }

    /// 核心错误（`String`）→ 接缝错误。核心与宿主命令共用，错误文案原样透出。
    fn cron_err(op: &str, e: String) -> AxAgentError {
        AxAgentError::Provider(format!("stock.analysis::{op} 失败: {e}"))
    }

    /// 响应序列化错误 → 接缝错误。
    fn to_value(op: &str, value: &impl serde::Serialize) -> Result<Value> {
        serde_json::to_value(value)
            .map_err(|e| AxAgentError::Provider(format!("stock.analysis::{op} 序列化失败: {e}")))
    }

    /// JSON 入参 → 请求结构（错误文案带 op 与期望形状）。
    fn parse<T: serde::de::DeserializeOwned>(op: &str, expect: &str, input: Value) -> Result<T> {
        serde_json::from_value(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::{op} 参数解析失败: {e}。期望 {expect}"
            ))
        })
    }
}

// ── Trait 实现 ────────────────────────────────────────────────────────────

#[async_trait]
impl StockAnalysisProvider for HostStockAnalysis {
    /// 读 op：按行业名取全 A 股成分股分组。
    async fn industry_constituents(&self, input: Value) -> Result<Value> {
        let parsed: IndustryConstituentsInput = serde_json::from_value(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::industry_constituents 参数解析失败: {e}。期望 \
                 {{industries: [...]}}（元素为行业名字符串或 {{name, leaderCode}}）"
            ))
        })?;
        // 闸在 IO 之前：空清单换来的「全宇宙空结果」会被读成「这些行业没有成分股」。
        if parsed.industries.is_empty() {
            return Err(AxAgentError::Validation(
                "stock.analysis::industry_constituents industries 为空：至少给一个行业名".into(),
            ));
        }
        let report = fetch_industry_constituents(&self.db, &parsed).await.map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::industry_constituents 查询失败: {e}"))
        })?;
        serde_json::to_value(&report).map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::industry_constituents 序列化失败: {e}"))
        })
    }

    /// 写 op：按日批量 upsert 市场主线（`archive_missing = true` 会归档当日未提及的主线）。
    ///
    /// 权限闸**不在本层**（`worker.rs` 的 `required_permission_for` 已登记 `Write`，
    /// 本 trait 的文档也写明不重复校验）。本层只做形状转换与错误归类。
    async fn batch_upsert_mainlines(&self, input: Value) -> Result<Value> {
        // **不做字段级预判**：`BatchUpsertInput` 的 serde 是形状唯一权威（含 snake/camel
        // 双写 alias 与「缺 mainline_date 必须失败」）。预判会与该 DTO 的演进脱钩。
        let params: BatchUpsertInput = serde_json::from_value(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::batch_upsert_mainlines 参数解析失败: {e}。期望字段: \
                 mainline_date(YYYY-MM-DD) / mainlines(数组) / archive_missing(可选 bool)"
            ))
        })?;
        let result = batch_upsert_mainlines(&self.db, params).await.map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::batch_upsert_mainlines 写入失败: {e}"))
        })?;
        serde_json::to_value(&result).map_err(|e| {
            AxAgentError::Provider(format!(
                "stock.analysis::batch_upsert_mainlines 序列化失败: {e}"
            ))
        })
    }

    /// 读 op：列出最近 N 天市场主线（宿主 `market_mainline_list_recent` 的接缝面，
    /// #304 C3 —— 前端 `MarketMainlinePanel` 经插件命名命令取数）。
    ///
    /// 缺省窗口与上界从被退役的宿主命令**原样搬来**（7 / clamp 1..=90）：上界在这里拍，
    /// 插件侧不重复设置；其余天数原样放行给 SQL limit（引擎函数内部每日再限 20 条）。
    async fn list_recent_mainlines(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct RecentRequest {
            days: Option<usize>,
        }
        let request = serde_json::from_value::<RecentRequest>(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::list_recent_mainlines 参数解析失败: {e}。期望 {{days?: number}}"
            ))
        })?;
        let days = request.days.unwrap_or(DEFAULT_RECENT_DAYS).clamp(1, MAX_RECENT_DAYS);
        let rows = list_recent_mainlines(&self.db, days).await.map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::list_recent_mainlines 查询失败: {e}"))
        })?;
        serde_json::to_value(&rows).map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::list_recent_mainlines 序列化失败: {e}"))
        })
    }

    /// 读 op：逐票 1 月涨跌幅（反转因子原料）。
    async fn stock_factor(&self, input: Value) -> Result<Value> {
        let request: StockFactorRequest = serde_json::from_value(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::stock_factor 参数解析失败: {e}。期望 {{stockCodes: ..., \
                 asOfDate?: \"YYYY-MM-DD\"}}（stockCodes 为宽容形态：字符串数组 / \
                 代码对象数组 / 含 stocks 分组的信封）"
            ))
        })?;
        // 归一一律在本层做（`parse_stock_refs` 属 implementor，工具侧够不着）。
        let codes = parse_stock_refs(&request.stock_codes);
        // 闸在 IO 之前：空清单换来的「0 只票、全部缺席」会被读成「这些票都没数据」。
        if codes.is_empty() {
            return Err(AxAgentError::Validation(
                "stock.analysis::stock_factor codes 为空：无法打分（缺席要显式报错，不得伪装成空结果）"
                    .into(),
            ));
        }
        let report =
            fetch_stock_chg_1m(&self.db, &codes, request.as_of_date.as_deref()).await.map_err(
                |e| AxAgentError::Provider(format!("stock.analysis::stock_factor 查询失败: {e}")),
            )?;
        serde_json::to_value(&report).map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::stock_factor 序列化失败: {e}"))
        })
    }

    // ── T2 试点批（#248）：四条宿主命令的取数面（全只读/纯计算，无权限闸）──────
    // 入参形状与前端 `invoke` 传参逐字一致（Tauri 自动 camelCase 映射）；出参就是宿主
    // 命令原样返回的那个 JSON（权威在同一处，见模块头）。

    /// 读 op：管道执行历史（宿主 `get_pipeline_history` 的接缝面）。
    ///
    /// 缺省 limit 与宿主命令逐字一致（20），插件侧不拍第二个数。
    async fn pipeline_history(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct HistoryRequest {
            limit: Option<u64>,
        }
        let request = serde_json::from_value::<HistoryRequest>(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::pipeline_history 参数解析失败: {e}。期望 {{limit?: number}}"
            ))
        })?;
        let rows =
            pipeline_history_rows(&self.db, request.limit.unwrap_or(20)).await.map_err(|e| {
                AxAgentError::Provider(format!("stock.analysis::pipeline_history 查询失败: {e}"))
            })?;
        Ok(Value::Array(rows))
    }

    /// 读 op：单次管道执行详情（宿主 `get_pipeline_run_detail` 的接缝面）。
    async fn pipeline_run_detail(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct RunDetailRequest {
            run_id: String,
        }
        let request = serde_json::from_value::<RunDetailRequest>(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::pipeline_run_detail 参数解析失败: {e}。期望 {{runId: string}}"
            ))
        })?;
        pipeline_run_detail_row(&self.db, &request.run_id).await.map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::pipeline_run_detail 查询失败: {e}"))
        })
    }

    /// 读 op：个股最近一次完整分析摘要（宿主 `get_latest_analysis_for_stock` 的接缝面）。
    ///
    /// 入参 `{stockCode: string, asOfDate?: string}`；出参 = `LatestAnalysisSummary` 的
    /// camelCase JSON，无记录回 `null`（不是 `{}` —— 消费端按 null 判「没有」）。
    async fn latest_analysis_for_stock(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct LatestRequest {
            stock_code: String,
            as_of_date: Option<String>,
        }
        let request = serde_json::from_value::<LatestRequest>(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::latest_analysis_for_stock 参数解析失败: {e}。期望 \
                 {{stockCode: string, asOfDate?: string}}"
            ))
        })?;
        let summary = get_latest_analysis_for_stock_core(
            &self.db,
            &request.stock_code,
            request.as_of_date.as_deref(),
        )
        .await
        .map_err(|e| {
            AxAgentError::Provider(format!(
                "stock.analysis::latest_analysis_for_stock 查询失败: {e}"
            ))
        })?;
        serde_json::to_value(summary)
            .map_err(|e| AxAgentError::Provider(format!("最新分析摘要序列化失败: {e}")))
    }

    /// 读 op：批量个股最近分析摘要（宿主 `get_latest_analyses_for_stocks` 的接缝面）。
    ///
    /// 入参 `{stockCodes: string[], asOfDate?: string}`；出参 = `{code: Summary|null}`
    /// 对象。逐票复用单数版 core（见宿主 core 注释）⇒ 与单数版同一 SQL 判据。
    async fn latest_analyses_for_stocks(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct LatestBatchRequest {
            stock_codes: Vec<String>,
            as_of_date: Option<String>,
        }
        let request = serde_json::from_value::<LatestBatchRequest>(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::latest_analyses_for_stocks 参数解析失败: {e}。期望                  {{stockCodes: string[], asOfDate?: string}}"
            ))
        })?;
        let summaries = get_latest_analyses_for_stocks_core(
            &self.db,
            &request.stock_codes,
            request.as_of_date.as_deref(),
        )
        .await
        .map_err(|e| {
            AxAgentError::Provider(format!(
                "stock.analysis::latest_analyses_for_stocks 查询失败: {e}"
            ))
        })?;
        serde_json::to_value(summaries)
            .map_err(|e| AxAgentError::Provider(format!("批量分析摘要序列化失败: {e}")))
    }

    /// 读 op：权重时间线（宿主 `get_evolution_drift_timeline` 的接缝面）。
    ///
    /// 入参 `{strategyId, period, limit?}`（limit 缺省 60）；出参 = `TimelinePoint` 数组。
    async fn evolution_drift_timeline(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct TimelineRequest {
            strategy_id: String,
            period: String,
            limit: Option<u32>,
        }
        let request = serde_json::from_value::<TimelineRequest>(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::evolution_drift_timeline 参数解析失败: {e}。期望                  {{strategyId: string, period: string, limit?: number}}"
            ))
        })?;
        let points = evolution_drift_timeline_core(
            &self.db,
            &request.strategy_id,
            &request.period,
            request.limit,
        )
        .await
        .map_err(|e| {
            AxAgentError::Provider(format!(
                "stock.analysis::evolution_drift_timeline 查询失败: {e}"
            ))
        })?;
        serde_json::to_value(points)
            .map_err(|e| AxAgentError::Provider(format!("权重时间线序列化失败: {e}")))
    }

    /// 读 op：决策一致性分数历史（宿主 `get_agreement_score_history` 的接缝面）。
    ///
    /// 入参 `{limit?: number}`（缺省 50）；出参 = 行数组（键序见 core 注释）。
    async fn agreement_score_history(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct AgreementHistoryRequest {
            limit: Option<u32>,
        }
        let request = serde_json::from_value::<AgreementHistoryRequest>(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::agreement_score_history 参数解析失败: {e}。期望 {{limit?: number}}"
            ))
        })?;
        let rows = agreement_score_history_core(&self.db, request.limit).await.map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::agreement_score_history 查询失败: {e}"))
        })?;
        Ok(Value::Array(rows))
    }

    /// 读 op：节点质量反馈历史（通用版，`node_feedbacks` op；#332）。
    ///
    /// ⚠ 与别名族 `get_analyst_feedbacks` 不同命令：本 op 收泛化过滤
    /// `{nodeType?, nodeId?, limit?, onlyIssues?}`，不强制 `analyst`。
    async fn node_feedbacks(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Input {
            #[serde(default)]
            node_type: Option<String>,
            #[serde(default)]
            node_id: Option<String>,
            #[serde(default)]
            limit: Option<u64>,
            #[serde(default)]
            only_issues: Option<bool>,
        }
        let req: Input =
            Self::parse("node_feedbacks", "{nodeType?, nodeId?, limit?, onlyIssues?}", input)?;
        let rows = get_node_feedbacks_core(
            &self.db,
            GetNodeFeedbacksRequest {
                node_type: req.node_type,
                node_id: req.node_id,
                limit: req.limit,
                only_issues: req.only_issues,
            },
        )
        .await
        .map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::node_feedbacks 查询失败: {e}"))
        })?;
        serde_json::to_value(rows).map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::node_feedbacks 序列化失败: {e}"))
        })
    }

    /// 读 op：节点反馈统计（通用版，`node_feedback_stats` op；#332）。
    async fn node_feedback_stats(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Input {
            node_type: String,
            node_id: String,
        }
        let req: Input = Self::parse("node_feedback_stats", "{nodeType, nodeId}", input)?;
        let stats = get_node_feedback_stats_core(&self.db, req.node_type, req.node_id)
            .await
            .map_err(|e| {
                AxAgentError::Provider(format!("stock.analysis::node_feedback_stats 查询失败: {e}"))
            })?;
        serde_json::to_value(stats).map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::node_feedback_stats 序列化失败: {e}"))
        })
    }

    /// 读 op：月度投资报告（宿主 `generate_monthly_report` 的接缝面；#333）。
    async fn monthly_report(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct MonthlyRequest {
            year: i32,
            month: u32,
        }
        let req: MonthlyRequest = Self::parse("monthly_report", "{year, month}", input)?;
        let report = monthly_report_core(&self.db, req.year, req.month).await.map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::monthly_report 生成失败: {e}"))
        })?;
        serde_json::to_value(report).map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::monthly_report 序列化失败: {e}"))
        })
    }

    /// 读 op：宏观经济数据快照（宿主 `macro_data_snapshot` 的接缝面；#333）。
    ///
    /// 宿主原命令只 `MacroDataClient::new().snapshot()`、无 State 依赖 ⇒ 此处逐字下沉到
    /// provider（⚠ 取数发生在宿主进程，与宿主原命令同进程同源）。
    async fn macro_data_snapshot(&self, _input: Value) -> Result<Value> {
        let client = axagent_astock_data::macro_data::MacroDataClient::new();
        let snap = client.snapshot().await;
        serde_json::to_value(snap).map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::macro_data_snapshot 序列化失败: {e}"))
        })
    }

    /// 读 op：漏检回灌清单与转正就绪度（宿主 `list_mover_reflow_candidates` 的接缝面；#334）。
    async fn mover_reflow_candidates(&self, _input: Value) -> Result<Value> {
        let view = crate::commands::mover_recall::mover_reflow_candidates_core(&self.db)
            .await
            .map_err(|e| {
                AxAgentError::Provider(format!(
                    "stock.analysis::mover_reflow_candidates 查询失败: {e}"
                ))
            })?;
        serde_json::to_value(view).map_err(|e| {
            AxAgentError::Provider(format!(
                "stock.analysis::mover_reflow_candidates 序列化失败: {e}"
            ))
        })
    }

    /// 计算型 op：回测反馈分析（宿主 `analyze_backtest_feedback` 的接缝面；#335）。
    ///
    /// 纯计算，权威在 commands 层 core；入参 = `FeedbackInput` 的 camelCase JSON。
    async fn analyze_backtest_feedback(&self, input: Value) -> Result<Value> {
        let parsed = serde_json::from_value::<
            axagent_analysis_engine::backtest_feedback::FeedbackInput,
        >(input)
        .map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::analyze_backtest_feedback 参数解析失败: {e}。期望 \
                 {{participations: [{{analystId, timeHorizon, date, stance, confidence, \
                 wasCorrect}}]}}"
            ))
        })?;
        let report =
            crate::commands::stock_analysis::analyze_backtest_feedback_core(parsed.participations);
        serde_json::to_value(report).map_err(|e| {
            AxAgentError::Provider(format!(
                "stock.analysis::analyze_backtest_feedback 序列化失败: {e}"
            ))
        })
    }

    /// 读 op：VLM 持仓截图解析（宿主 `parse_vlm_portfolio_screenshot` 的接缝面；#335）。
    ///
    /// 纯解析，权威在 commands 层 core；入参 `{rawVlmOutput: string}`。
    async fn parse_vlm_portfolio_screenshot(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct VlmParseRequest {
            raw_vlm_output: String,
        }
        let request = serde_json::from_value::<VlmParseRequest>(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::parse_vlm_portfolio_screenshot 参数解析失败: {e}。期望 \
                 {{rawVlmOutput: string}}"
            ))
        })?;
        let result = crate::commands::stock_analysis::parse_vlm_portfolio_screenshot_core(
            &request.raw_vlm_output,
        );
        serde_json::to_value(result).map_err(|e| {
            AxAgentError::Provider(format!(
                "stock.analysis::parse_vlm_portfolio_screenshot 序列化失败: {e}"
            ))
        })
    }

    /// 读 op：按 runId 读回测记录（宿主 `quant_run_get` 的接缝面；#336）。
    ///
    /// 权威在 commands 层 core；入参 `{runId: string}`，runId 不存在时按宿主原语义报错。
    async fn quant_run_get(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct QuantRunGetRequest {
            run_id: String,
        }
        let request = serde_json::from_value::<QuantRunGetRequest>(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::quant_run_get 参数解析失败: {e}。期望 {{runId: string}}"
            ))
        })?;
        let run = crate::commands::quant_backtest::quant_run_get_core(&self.db, &request.run_id)
            .await
            .map_err(AxAgentError::Provider)?;
        serde_json::to_value(run).map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::quant_run_get 序列化失败: {e}"))
        })
    }

    /// 读 op：量化策略列表（宿主 `quant_strategies_list` 的接缝面；#336）。
    ///
    /// 入参恒 `{}`；出参 = `quant_strategies::Model` 行数组。
    async fn quant_strategies_list(&self, _input: Value) -> Result<Value> {
        let rows = crate::commands::quant_backtest::quant_strategies_list_core(&self.db)
            .await
            .map_err(AxAgentError::Provider)?;
        serde_json::to_value(rows).map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::quant_strategies_list 序列化失败: {e}"))
        })
    }

    /// 读 op：多 run 指标对比（宿主 `quant_metrics_compare` 的接缝面；#336）。
    ///
    /// 入参 `{runIds: string[]}`；出参 = `MetricsCompareResponse` 的 camelCase JSON。
    async fn quant_metrics_compare(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct QuantMetricsCompareRequest {
            run_ids: Vec<String>,
        }
        let request = serde_json::from_value::<QuantMetricsCompareRequest>(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::quant_metrics_compare 参数解析失败: {e}。期望 \
                     {{runIds: string[]}}"
            ))
        })?;
        let response =
            crate::commands::quant_backtest::quant_metrics_compare_core(&self.db, request.run_ids)
                .await
                .map_err(AxAgentError::Provider)?;
        serde_json::to_value(response).map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::quant_metrics_compare 序列化失败: {e}"))
        })
    }

    /// 写 op：注册 Rhai 策略（宿主 `quant_strategy_register_rhai` 的接缝面；#336）。
    ///
    /// insert/upsert `quant_strategies` 表；入参 = `RegisterRhaiRequest` 的 camelCase JSON。
    async fn quant_strategy_register_rhai(&self, input: Value) -> Result<Value> {
        let request =
            serde_json::from_value::<crate::commands::quant_backtest::RegisterRhaiRequest>(input)
                .map_err(|e| {
                AxAgentError::Validation(format!(
                    "stock.analysis::quant_strategy_register_rhai 参数解析失败: {e}。期望 \
                 {{name, version, description?, scriptSource, params, walkForwardEnabled, upsert}}"
                ))
            })?;
        let model =
            crate::commands::quant_backtest::quant_strategy_register_rhai_core(&self.db, request)
                .await
                .map_err(AxAgentError::Provider)?;
        serde_json::to_value(model).map_err(|e| {
            AxAgentError::Provider(format!(
                "stock.analysis::quant_strategy_register_rhai 序列化失败: {e}"
            ))
        })
    }

    /// 写 op：手动设置 vendor 健康状态（宿主 `set_vendor_status` 的接缝面；#337）。
    ///
    /// 与宿主原命令同判据：status ∈ healthy/degraded/disabled，非法值报错不落任何状态。
    async fn set_vendor_status(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct VendorStatusRequest {
            vendor: String,
            status: String,
        }
        let request = serde_json::from_value::<VendorStatusRequest>(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::set_vendor_status 参数解析失败: {e}。期望 {{vendor, status}}"
            ))
        })?;
        use axagent_astock_data::vendor_health::VendorStatus;
        let st = match request.status.as_str() {
            "healthy" => VendorStatus::Healthy,
            "degraded" => VendorStatus::Degraded,
            "disabled" => VendorStatus::Disabled,
            other => {
                return Err(AxAgentError::Validation(format!(
                    "stock.analysis::set_vendor_status: 无效的 vendor 状态: \
                     {other}（可选: healthy/degraded/disabled）"
                )));
            },
        };
        self.astock_client.health_tracker.set_vendor_status(&request.vendor, st).await;
        tracing::info!(
            "[P3-B5] vendor '{}' 状态已手动设置为 '{}'（经 stock.analysis 接缝）",
            request.vendor,
            request.status
        );
        Ok(Value::Null)
    }

    /// 写 op：设置主行情源（原宿主 `stock_set_market_data_primary_vendor` 命令的业务体）。
    ///
    /// 两种入参：`{vendor}` 写 settings 并即时重排；`{applyFromSettings: true}` 只读 settings
    /// 重排（装配/重放已保存的主源，不重复落库）。空串/全空白归一为 `None`（清除覆盖），
    /// 与宿主原命令逐字一致；只重排不注册新源，未知 vendor 不报错（由 `AStockClient` 裁定）。
    async fn set_primary_market_vendor(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct PrimaryVendorRequest {
            #[serde(default)]
            vendor: Option<String>,
            #[serde(default)]
            apply_from_settings: bool,
        }
        let request = serde_json::from_value::<PrimaryVendorRequest>(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::set_primary_market_vendor 参数解析失败: {e}。期望 \
                 {{vendor}} 或 {{applyFromSettings: true}}"
            ))
        })?;
        let mut settings =
            axagent_dao::repo::settings::get_settings(&self.db).await.map_err(|e| {
                AxAgentError::Provider(format!(
                    "stock.analysis::set_primary_market_vendor 读取设置失败: {e}"
                ))
            })?;
        let effective = if request.apply_from_settings {
            settings.market_data_primary_vendor.clone()
        } else {
            let normalized = request.vendor.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
            settings.market_data_primary_vendor = normalized.clone();
            axagent_dao::repo::settings::save_settings(&self.db, &settings).await.map_err(|e| {
                AxAgentError::Provider(format!(
                    "stock.analysis::set_primary_market_vendor 保存设置失败: {e}"
                ))
            })?;
            normalized
        };
        self.astock_client.set_primary_market_vendor(effective.as_deref());
        tracing::info!(
            target: "astock-data::priority",
            primary = ?effective,
            from_settings = request.apply_from_settings,
            "主行情源已更新（quote/klines 链首重排）"
        );
        Ok(Value::Null)
    }

    /// 写 op：取消进行中的分析任务（宿主 `cancel_stock_analysis` 的接缝面；#337）。
    ///
    /// 与宿主原命令同判据：令牌不存在（任务不存在或已完成）报错，不伪造成功。
    async fn cancel_stock_analysis(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct CancelRequest {
            analysis_id: String,
        }
        let request = serde_json::from_value::<CancelRequest>(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::cancel_stock_analysis 参数解析失败: {e}。期望 {{analysisId}}"
            ))
        })?;
        if let Some(token) = self.agent_cancel_tokens.get(&request.analysis_id) {
            token.store(true, Ordering::Relaxed);
            tracing::info!("cancel_stock_analysis: 已设置取消令牌 {}", request.analysis_id);
            Ok(Value::Null)
        } else {
            Err(AxAgentError::Validation(format!(
                "stock.analysis::cancel_stock_analysis: 分析任务不存在或已完成: {}",
                request.analysis_id
            )))
        }
    }

    /// 读 op：漏检补分析批次状态（宿主 `get_mover_recall_analysis_batch` 的接缝面；#338）。
    ///
    /// 纯读（reap 已上收启动钩子）；入参 `{batchId: string}`，找不到回 `null`。
    async fn mover_recall_analysis_batch(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct BatchQuery {
            batch_id: String,
        }
        let request = serde_json::from_value::<BatchQuery>(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::mover_recall_analysis_batch 参数解析失败: {e}。期望 {{batchId}}"
            ))
        })?;
        let state = crate::commands::mover_recall_analysis::mover_recall_analysis_batch_core(
            &self.db,
            &request.batch_id,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Ok(state.map(|s| serde_json::to_value(s).unwrap_or(Value::Null)).unwrap_or(Value::Null))
    }

    /// 读 op：漏检补分析批次历史（宿主 `list_mover_recall_analysis_batches` 的接缝面；#338）。
    ///
    /// 纯读；入参 `{limit?, statusFilter?}`；出参 = 摘要数组。
    async fn mover_recall_analysis_batches(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct BatchesQuery {
            limit: Option<u64>,
            status_filter: Option<String>,
        }
        let request = serde_json::from_value::<BatchesQuery>(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::mover_recall_analysis_batches 参数解析失败: {e}。期望 \
                 {{limit?, statusFilter?}}"
            ))
        })?;
        let rows = crate::commands::mover_recall_analysis::mover_recall_analysis_batches_core(
            &self.db,
            request.limit,
            request.status_filter.as_deref(),
        )
        .await
        .map_err(AxAgentError::Provider)?;
        serde_json::to_value(rows).map_err(|e| {
            AxAgentError::Provider(format!(
                "stock.analysis::mover_recall_analysis_batches 序列化失败: {e}"
            ))
        })
    }

    /// 写 op：发起漏检补分析批次（宿主 `run_mover_recall_analysis` 的接缝面；#338）。
    ///
    /// fire-and-forget：与调度器入口共用 `launch`（同一把 as-of 批次锁、同一份
    /// 进程内 `BATCH_RUNS`），返回 batchId 只表示已发起。
    async fn run_mover_recall_analysis(&self, input: Value) -> Result<Value> {
        let request = serde_json::from_value::<
            crate::commands::mover_recall_analysis::RunMoverRecallAnalysisRequest,
        >(input)
        .map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::run_mover_recall_analysis 参数解析失败: {e}。期望 \
                 {{batchLimit, maxUnexplainedShare, runId?}}"
            ))
        })?;
        let batch_id = crate::commands::mover_recall_analysis::launch(
            &self.time_travel_batch_lock,
            self.db.clone(),
            self.astock_client.clone(),
            self.work_engine.clone(),
            request,
            "manual",
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Ok(Value::String(batch_id))
    }

    // ── #342 批 A15：watchlist 5 + cron 9（宿主命令体逐字下沉 commands 层 core）──

    /// 读 op：自选股列表（`list_watchlist`）。
    async fn list_watchlist(&self, _input: Value) -> Result<Value> {
        let rows = crate::commands::stock_analysis::list_watchlist_core(&self.db)
            .await
            .map_err(AxAgentError::Provider)?;
        Self::to_value("list_watchlist", &rows)
    }

    /// 写 op：添加自选股（`add_to_watchlist`）。
    async fn add_to_watchlist(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct AddRequest {
            stock_code: String,
            stock_name: String,
            notes: Option<String>,
        }
        let req =
            Self::parse::<AddRequest>("add_to_watchlist", "{stockCode, stockName, notes?}", input)?;
        let model = crate::commands::stock_analysis::add_to_watchlist_core(
            &self.db,
            req.stock_code,
            req.stock_name,
            req.notes,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("add_to_watchlist", &model)
    }

    /// 写 op：更新自选股分组（`watchlist_update_group`）。
    async fn watchlist_update_group(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct UpdateGroupRequest {
            id: String,
            group_name: String,
        }
        let req =
            Self::parse::<UpdateGroupRequest>("watchlist_update_group", "{id, groupName}", input)?;
        crate::commands::stock_analysis::watchlist_update_group_core(
            &self.db,
            req.id,
            req.group_name,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Ok(Value::Null)
    }

    /// 读 op：自选股分组列表（`watchlist_list_groups`）。
    async fn watchlist_list_groups(&self, _input: Value) -> Result<Value> {
        let rows = crate::commands::stock_analysis::watchlist_list_groups_core(&self.db)
            .await
            .map_err(AxAgentError::Provider)?;
        Self::to_value("watchlist_list_groups", &rows)
    }

    /// 写 op：保存自选股分组（`watchlist_save_groups`）。
    async fn watchlist_save_groups(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct SaveGroupsRequest {
            groups: Vec<String>,
        }
        let req =
            Self::parse::<SaveGroupsRequest>("watchlist_save_groups", "{groups: [string]}", input)?;
        crate::commands::stock_analysis::watchlist_save_groups_core(&self.db, req.groups)
            .await
            .map_err(AxAgentError::Provider)?;
        Ok(Value::Null)
    }

    /// 写 op：创建股票定时分析任务（`create_stock_cron`）。
    async fn create_stock_cron(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct CreateStockCronRequest {
            stock_code: String,
            stock_name: String,
            cron_expression: String,
        }
        let req = Self::parse::<CreateStockCronRequest>(
            "create_stock_cron",
            "{stockCode, stockName, cronExpression}",
            input,
        )?;
        let job = crate::commands::stock_analysis::create_stock_cron_core(
            &self.cron_job_store,
            req.stock_code,
            req.stock_name,
            req.cron_expression,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("create_stock_cron", &job)
    }

    /// 读 op：股票定时任务列表（`list_stock_crons`）。
    async fn list_stock_crons(&self, _input: Value) -> Result<Value> {
        let jobs = crate::commands::stock_analysis::list_stock_crons_core(&self.cron_job_store)
            .await
            .map_err(AxAgentError::Provider)?;
        Self::to_value("list_stock_crons", &jobs)
    }

    /// 写 op：开关股票定时任务（`toggle_stock_cron`）。
    async fn toggle_stock_cron(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ToggleRequest {
            id: String,
            enabled: bool,
        }
        let req = Self::parse::<ToggleRequest>("toggle_stock_cron", "{id, enabled}", input)?;
        crate::commands::stock_analysis::toggle_stock_cron_core(
            &self.cron_job_store,
            req.id,
            req.enabled,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Ok(Value::Null)
    }

    /// 写 op：创建决策校验定时任务（`create_validate_decisions_cron`）。
    async fn create_validate_decisions_cron(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct CreateValidateCronRequest {
            cron_expression: Option<String>,
            min_confidence_threshold: Option<i32>,
            reflection_depth: Option<String>,
            enabled: Option<bool>,
        }
        let req = Self::parse::<CreateValidateCronRequest>(
            "create_validate_decisions_cron",
            "{cronExpression?, minConfidenceThreshold?, reflectionDepth?, enabled?}",
            input,
        )?;
        let job = crate::commands::stock_analysis::create_validate_decisions_cron_core(
            &self.cron_job_store,
            req.cron_expression,
            req.min_confidence_threshold,
            req.reflection_depth,
            req.enabled,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("create_validate_decisions_cron", &job)
    }

    /// 读 op：决策校验定时任务列表（`list_validate_decisions_crons`）。
    async fn list_validate_decisions_crons(&self, _input: Value) -> Result<Value> {
        let jobs = crate::commands::stock_analysis::list_validate_decisions_crons_core(
            &self.cron_job_store,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("list_validate_decisions_crons", &jobs)
    }

    /// 写 op：开关决策校验定时任务（`toggle_validate_decisions_cron`）。
    async fn toggle_validate_decisions_cron(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ToggleRequest {
            id: String,
            enabled: bool,
        }
        let req =
            Self::parse::<ToggleRequest>("toggle_validate_decisions_cron", "{id, enabled}", input)?;
        crate::commands::stock_analysis::toggle_validate_decisions_cron_core(
            &self.cron_job_store,
            req.id,
            req.enabled,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Ok(Value::Null)
    }

    /// 写 op：更新决策校验定时任务配置（`update_validate_decisions_cron`）。
    async fn update_validate_decisions_cron(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct UpdateValidateCronRequest {
            id: String,
            cron_expression: String,
            min_confidence_threshold: i32,
            reflection_depth: String,
        }
        let req = Self::parse::<UpdateValidateCronRequest>(
            "update_validate_decisions_cron",
            "{id, cronExpression, minConfidenceThreshold, reflectionDepth}",
            input,
        )?;
        let job = crate::commands::stock_analysis::update_validate_decisions_cron_core(
            &self.cron_job_store,
            req.id,
            req.cron_expression,
            req.min_confidence_threshold,
            req.reflection_depth,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("update_validate_decisions_cron", &job)
    }

    /// 写 op：创建批量反思定时任务（`create_batch_reflection_cron`）。
    async fn create_batch_reflection_cron(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct CreateBatchReflectionCronRequest {
            cron_expression: Option<String>,
            period: Option<axagent_analysis_engine::recommender::Period>,
            due_only: Option<bool>,
            max_count: Option<u32>,
            enabled: Option<bool>,
        }
        let req = Self::parse::<CreateBatchReflectionCronRequest>(
            "create_batch_reflection_cron",
            "{cronExpression?, period?, dueOnly?, maxCount?, enabled?}",
            input,
        )?;
        let job = crate::commands::stock_analysis::create_batch_reflection_cron_core(
            &self.cron_job_store,
            req.cron_expression,
            req.period,
            req.due_only,
            req.max_count,
            req.enabled,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("create_batch_reflection_cron", &job)
    }

    /// 写 op：创建自选股扫描定时任务（`create_watchlist_scan_cron`）。
    async fn create_watchlist_scan_cron(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct CreateWatchlistScanCronRequest {
            cron_expression: String,
            enabled: Option<bool>,
        }
        let req = Self::parse::<CreateWatchlistScanCronRequest>(
            "create_watchlist_scan_cron",
            "{cronExpression, enabled?}",
            input,
        )?;
        let job = crate::commands::stock_analysis::create_watchlist_scan_cron_core(
            &self.cron_job_store,
            req.cron_expression,
            req.enabled,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("create_watchlist_scan_cron", &job)
    }

    // ── #343 批 B17：组合/交易 6 + 进化审批 2（宿主命令体下沉 commands 层 core）──

    /// 读 op：持仓列表（`list_portfolio`，含实时行情盈亏）。
    async fn list_portfolio(&self, _input: Value) -> Result<Value> {
        let rows =
            crate::commands::stock_analysis::list_portfolio_core(&self.db, &self.astock_client)
                .await
                .map_err(AxAgentError::Provider)?;
        Self::to_value("list_portfolio", &rows)
    }

    /// 写 op：添加持仓（`add_portfolio_holding`）。
    async fn add_portfolio_holding(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct AddHoldingRequest {
            stock_code: String,
            stock_name: String,
            shares: f64,
            avg_cost: f64,
        }
        let req = Self::parse::<AddHoldingRequest>(
            "add_portfolio_holding",
            "{stockCode, stockName, shares, avgCost}",
            input,
        )?;
        let model = crate::commands::stock_analysis::add_portfolio_holding_core(
            &self.db,
            req.stock_code,
            req.stock_name,
            req.shares,
            req.avg_cost,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("add_portfolio_holding", &model)
    }

    /// 写 op：移除持仓（`remove_portfolio_holding`）。
    async fn remove_portfolio_holding(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct IdRequest {
            id: String,
        }
        let req = Self::parse::<IdRequest>("remove_portfolio_holding", "{id}", input)?;
        crate::commands::stock_analysis::remove_portfolio_holding_core(&self.db, req.id)
            .await
            .map_err(AxAgentError::Provider)?;
        Ok(Value::Null)
    }

    /// 读 op：组合相关性快照（`get_portfolio_correlations`）。
    async fn get_portfolio_correlations(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct CorrRequest {
            as_of_date: Option<String>,
        }
        let req = Self::parse::<CorrRequest>("get_portfolio_correlations", "{asOfDate?}", input)?;
        let rows = crate::commands::stock_analysis::get_portfolio_correlations_core(
            &self.db,
            req.as_of_date,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("get_portfolio_correlations", &rows)
    }

    /// 写 op：交易功能开关（`toggle_trading_enabled`）。
    async fn toggle_trading_enabled(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ToggleTradingRequest {
            enabled: bool,
        }
        let req =
            Self::parse::<ToggleTradingRequest>("toggle_trading_enabled", "{enabled}", input)?;
        crate::commands::stock_analysis::toggle_trading_enabled_core(&self.db, req.enabled)
            .await
            .map_err(AxAgentError::Provider)?;
        Ok(Value::Null)
    }

    /// 写 op：批量导入交易记录（`import_trades`，含交易开关闸）。
    async fn import_trades(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ImportTradesRequest {
            file_path: String,
        }
        let req = Self::parse::<ImportTradesRequest>("import_trades", "{filePath}", input)?;
        let summary = crate::commands::stock_analysis::import_trades_core(&self.db, req.file_path)
            .await
            .map_err(AxAgentError::Provider)?;
        Self::to_value("import_trades", &summary)
    }

    /// 写 op：拒绝进化计划（`reject_evolution_plan`）。
    async fn reject_evolution_plan(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct RejectPlanRequest {
            history_id: String,
            reason: String,
            reviewer: Option<String>,
        }
        let req = Self::parse::<RejectPlanRequest>(
            "reject_evolution_plan",
            "{historyId, reason, reviewer?}",
            input,
        )?;
        let row = crate::commands::stock_evolution_approval::reject_evolution_plan_core(
            &self.db,
            req.history_id,
            req.reason,
            req.reviewer,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("reject_evolution_plan", &row)
    }

    /// 写 op：回滚进化版本（`rollback_evolution`）。
    async fn rollback_evolution(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct RollbackRequest {
            history_id: Option<String>,
            reviewer: Option<String>,
        }
        let req =
            Self::parse::<RollbackRequest>("rollback_evolution", "{historyId?, reviewer?}", input)?;
        let row = crate::commands::stock_evolution_approval::rollback_evolution_core(
            &self.db,
            req.history_id,
            req.reviewer,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("rollback_evolution", &row)
    }

    // ── #344 批 B18：29 条（统计/数据源/分析读写/监控/T+0/价格告警/组合引擎/进化审批/mover/参数台账）──

    /// 读 op：as-of 降级条目总数（`get_asof_degradation_count`）。
    async fn get_asof_degradation_count(&self, _input: Value) -> Result<Value> {
        Self::to_value(
            "get_asof_degradation_count",
            &crate::commands::stock_analysis::get_asof_degradation_count_core(),
        )
    }

    /// 读 op：as-of 降级日志快照（`get_asof_degradation_log`）。
    async fn get_asof_degradation_log(&self, _input: Value) -> Result<Value> {
        Self::to_value(
            "get_asof_degradation_log",
            &crate::commands::stock_analysis::get_asof_degradation_log_core(),
        )
    }

    /// 读 op：市场状态文案（`get_market_status`）。
    async fn get_market_status(&self, _input: Value) -> Result<Value> {
        crate::commands::stock_analysis::get_market_status_core()
            .await
            .map_err(AxAgentError::Provider)
    }

    /// 读 op：所有数据源健康状态（`get_vendor_health_all`）。
    async fn get_vendor_health_all(&self, _input: Value) -> Result<Value> {
        let rows = crate::commands::stock_analysis::get_vendor_health_all_core(&self.astock_client)
            .await
            .map_err(AxAgentError::Provider)?;
        Self::to_value("get_vendor_health_all", &rows)
    }

    /// 读 op：vendor fallback 日志（`get_vendor_fallback_log`）。
    async fn get_vendor_fallback_log(&self, _input: Value) -> Result<Value> {
        let rows =
            crate::commands::stock_analysis::get_vendor_fallback_log_core(&self.astock_client)
                .await
                .map_err(AxAgentError::Provider)?;
        Self::to_value("get_vendor_fallback_log", &rows)
    }

    /// 读 op：接口健康度历史（`get_interface_health_history`）。
    async fn get_interface_health_history(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct DaysRequest {
            days: Option<u32>,
        }
        let req = Self::parse::<DaysRequest>("get_interface_health_history", "{days?}", input)?;
        let history = crate::commands::stock_analysis::get_interface_health_history_core(
            &self.astock_client,
            req.days,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("get_interface_health_history", &history)
    }

    /// 读 op：估值参数（`get_valuation_params`）。
    async fn get_valuation_params(&self, _input: Value) -> Result<Value> {
        let params = crate::commands::stock_analysis::get_valuation_params_core(&self.db)
            .await
            .map_err(AxAgentError::Provider)?;
        Self::to_value("get_valuation_params", &params)
    }

    /// 写 op：保存估值参数（`save_valuation_params`）。
    async fn save_valuation_params(&self, input: Value) -> Result<Value> {
        let params = Self::parse::<crate::commands::stock_analysis::ValuationParams>(
            "save_valuation_params",
            "ValuationParams（camelCase）",
            input,
        )?;
        crate::commands::stock_analysis::save_valuation_params_core(&self.db, params)
            .await
            .map_err(AxAgentError::Provider)?;
        Ok(Value::Null)
    }

    /// 读 op：模板代际对照（`get_stock_template_version_status`）。
    async fn get_stock_template_version_status(&self, _input: Value) -> Result<Value> {
        let status =
            crate::commands::stock_analysis::get_stock_template_version_status_core(&self.db)
                .await
                .map_err(AxAgentError::Provider)?;
        Self::to_value("get_stock_template_version_status", &status)
    }

    /// 读 op：历史分析列表（`list_stock_analyses`）。
    async fn list_stock_analyses(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ListAnalysesRequest {
            limit: u32,
            offset: u32,
        }
        let req =
            Self::parse::<ListAnalysesRequest>("list_stock_analyses", "{limit, offset}", input)?;
        let rows = crate::commands::stock_analysis::list_stock_analyses_core(
            &self.db, req.limit, req.offset,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("list_stock_analyses", &rows)
    }

    /// 读 op：最近分析摘要（`get_recent_analyses`）。
    async fn get_recent_analyses(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct RecentRequest {
            limit: Option<u32>,
        }
        let req = Self::parse::<RecentRequest>("get_recent_analyses", "{limit?}", input)?;
        let rows = crate::commands::stock_analysis::get_recent_analyses_core(&self.db, req.limit)
            .await
            .map_err(AxAgentError::Provider)?;
        Self::to_value("get_recent_analyses", &rows)
    }

    /// 读 op：反思记录列表（`list_reflections`）。
    async fn list_reflections(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct ReflectionsRequest {
            stock_code: Option<String>,
            limit: Option<u32>,
            original_analysis_id: Option<String>,
        }
        let req = Self::parse::<ReflectionsRequest>(
            "list_reflections",
            "{stockCode?, limit?, originalAnalysisId?}",
            input,
        )?;
        let rows = crate::commands::stock_analysis::list_reflections_core(
            &self.db,
            req.stock_code,
            req.limit,
            req.original_analysis_id,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("list_reflections", &rows)
    }

    /// 读 op：单条分析详情（`get_stock_analysis`）。
    async fn get_stock_analysis(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct OneAnalysisRequest {
            analysis_id: String,
        }
        let req = Self::parse::<OneAnalysisRequest>("get_stock_analysis", "{analysisId}", input)?;
        crate::commands::stock_analysis::get_stock_analysis_core(&self.db, req.analysis_id)
            .await
            .map_err(AxAgentError::Provider)
    }

    /// 读 op：决策证据对账审计（`audit_decision_evidence`）。
    async fn audit_decision_evidence(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct AuditRequest {
            analysis_id: String,
        }
        let req = Self::parse::<AuditRequest>("audit_decision_evidence", "{analysisId}", input)?;
        let report = crate::commands::stock_analysis::audit_decision_evidence_core(
            &self.db,
            req.analysis_id,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("audit_decision_evidence", &report)
    }

    /// 写 op：重命名分析记录（`rename_stock_analysis`）。
    async fn rename_stock_analysis(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct RenameRequest {
            analysis_id: String,
            new_name: String,
        }
        let req =
            Self::parse::<RenameRequest>("rename_stock_analysis", "{analysisId, newName}", input)?;
        crate::commands::stock_analysis::rename_stock_analysis_core(
            &self.db,
            req.analysis_id,
            req.new_name,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Ok(Value::Null)
    }

    /// 读 op：价格告警列表（`list_price_alerts`）。
    async fn list_price_alerts(&self, _input: Value) -> Result<Value> {
        let rows = crate::commands::stock_analysis::list_price_alerts_core(&self.db)
            .await
            .map_err(AxAgentError::Provider)?;
        Self::to_value("list_price_alerts", &rows)
    }

    /// 写 op：创建价格告警（`create_price_alert`，落库 + 进监控引擎同源）。
    async fn create_price_alert(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct CreateAlertRequest {
            stock_code: String,
            stock_name: String,
            condition: String,
            target_price: f64,
        }
        let req = Self::parse::<CreateAlertRequest>(
            "create_price_alert",
            "{stockCode, stockName, condition, targetPrice}",
            input,
        )?;
        // OnceLock::get 返回 Option<&Arc<..>>；upsert 助手签名正是该形态。
        let monitor = self.stock_monitor.get();
        let model = crate::commands::stock_analysis::create_price_alert_core(
            &self.db,
            monitor,
            req.stock_code,
            req.stock_name,
            req.condition,
            req.target_price,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("create_price_alert", &model)
    }

    /// 读 op：实时监控运行读数（`monitor_status`）。
    async fn monitor_status(&self, _input: Value) -> Result<Value> {
        let status = crate::commands::stock_analysis::monitor_status_core(self.stock_monitor.get())
            .await
            .map_err(AxAgentError::Provider)?;
        Self::to_value("monitor_status", &status)
    }

    /// 读 op：T+0 配置（`get_t0_config`）。
    async fn get_t0_config(&self, _input: Value) -> Result<Value> {
        let cfg = crate::commands::stock_analysis::get_t0_config_core(self.stock_monitor.get())
            .await
            .map_err(AxAgentError::Provider)?;
        Self::to_value("get_t0_config", &cfg)
    }

    /// 写 op：T+0 配置（`set_t0_config`）。
    async fn set_t0_config(&self, input: Value) -> Result<Value> {
        let cfg = Self::parse::<axagent_analysis_engine::monitor::TZeroConfig>(
            "set_t0_config",
            "TZeroConfig（camelCase）",
            input,
        )?;
        crate::commands::stock_analysis::set_t0_config_core(self.stock_monitor.get(), cfg)
            .await
            .map_err(AxAgentError::Provider)?;
        Ok(Value::Null)
    }

    /// 读 op：组合监控仪表盘（`get_portfolio_dashboard`）。
    async fn get_portfolio_dashboard(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct DashboardRequest {
            as_of_date: Option<String>,
        }
        let req = Self::parse::<DashboardRequest>("get_portfolio_dashboard", "{asOfDate?}", input)?;
        let dashboard = crate::commands::stock_analysis::get_portfolio_dashboard_core(
            &self.db,
            &self.astock_client,
            &self.trading_engine,
            req.as_of_date,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("get_portfolio_dashboard", &dashboard)
    }

    /// 写 op：刷新组合监控快照（`refresh_portfolio_metrics`）。
    async fn refresh_portfolio_metrics(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct RefreshRequest {
            as_of_date: Option<String>,
        }
        let req = Self::parse::<RefreshRequest>("refresh_portfolio_metrics", "{asOfDate?}", input)?;
        crate::commands::stock_analysis::refresh_portfolio_metrics_core(
            &self.db,
            &self.astock_client,
            &self.trading_engine,
            req.as_of_date,
        )
        .await
        .map_err(AxAgentError::Provider)
    }

    /// 读 op：开仓限制校验（`check_position_limits`）。
    async fn check_position_limits(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct LimitsRequest {
            stock_code: String,
            proposed_shares: i32,
            proposed_price: f64,
        }
        let req = Self::parse::<LimitsRequest>(
            "check_position_limits",
            "{stockCode, proposedShares, proposedPrice}",
            input,
        )?;
        crate::commands::stock_analysis::check_position_limits_core(
            &self.trading_engine,
            req.stock_code,
            req.proposed_shares,
            req.proposed_price,
        )
        .await
        .map_err(AxAgentError::Provider)
    }

    /// 写 op：接受进化计划（`approve_evolution_plan`，执行进化并回写）。
    async fn approve_evolution_plan(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ApproveRequest {
            history_id: String,
            reviewer: Option<String>,
        }
        let req = Self::parse::<ApproveRequest>(
            "approve_evolution_plan",
            "{historyId, reviewer?}",
            input,
        )?;
        let row = crate::commands::stock_evolution_approval::approve_evolution_plan_core(
            &self.db,
            &self.stock_adaptive_engine,
            req.history_id,
            req.reviewer,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("approve_evolution_plan", &row)
    }

    /// 读 op：mover 核查轮次列表（`list_mover_recall_runs`）。
    async fn list_mover_recall_runs(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct RunsRequest {
            limit: Option<u32>,
        }
        let req = Self::parse::<RunsRequest>("list_mover_recall_runs", "{limit?}", input)?;
        let rows = crate::commands::mover_recall::list_mover_recall_runs_core(&self.db, req.limit)
            .await
            .map_err(AxAgentError::Provider)?;
        Self::to_value("list_mover_recall_runs", &rows)
    }

    /// 读 op：按轮次读回核查结果（`get_mover_recall_run_view`）。
    async fn get_mover_recall_run_view(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct RunViewRequest {
            run_id: String,
        }
        let req = Self::parse::<RunViewRequest>("get_mover_recall_run_view", "{runId}", input)?;
        let view =
            crate::commands::mover_recall::get_mover_recall_run_view_core(&self.db, req.run_id)
                .await
                .map_err(AxAgentError::Provider)?;
        Ok(view.map(|v| serde_json::to_value(v).unwrap_or(Value::Null)).unwrap_or(Value::Null))
    }

    /// 读 op：漏检事件补分析报告与逐档反思（`get_mover_recall_event_review`）。
    async fn get_mover_recall_event_review(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ReviewRequest {
            stock_code: String,
            anchor_date: String,
            period: String,
            chain: Option<String>,
        }
        let req = Self::parse::<ReviewRequest>(
            "get_mover_recall_event_review",
            "{stockCode, anchorDate, period, chain?}",
            input,
        )?;
        let review = crate::commands::mover_recall::get_mover_recall_event_review_core(
            &self.db,
            req.stock_code,
            req.anchor_date,
            req.period,
            req.chain,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Ok(review.map(|v| serde_json::to_value(v).unwrap_or(Value::Null)).unwrap_or(Value::Null))
    }

    /// 读 op：参数变更台账（`list_param_changes`）。
    async fn list_param_changes(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct ParamChangesRequest {
            limit: Option<u64>,
        }
        let req = Self::parse::<ParamChangesRequest>("list_param_changes", "{limit?}", input)?;
        let rows = crate::commands::stock_analysis::list_param_changes_core(&self.db, req.limit)
            .await
            .map_err(AxAgentError::Provider)?;
        Self::to_value("list_param_changes", &rows)
    }

    /// 写 op：参数变更到期评估（`evaluate_param_changes`，落评估结论）。
    async fn evaluate_param_changes(&self, _input: Value) -> Result<Value> {
        crate::commands::stock_analysis::evaluate_param_changes_core(&self.db)
            .await
            .map_err(AxAgentError::Provider)
    }

    // ── #345 批 B19：16 条（2 零新 op 之外 14 条新 op）──

    /// 读 op：解析 CSV 交易记录（`parse_trades_csv`，纯函数）。
    async fn parse_trades_csv(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ParseCsvRequest {
            file_path: String,
        }
        let req = Self::parse::<ParseCsvRequest>("parse_trades_csv", "{filePath}", input)?;
        let rows = axagent_analysis_engine::trade_import::parse_csv(&req.file_path)
            .map_err(AxAgentError::Provider)?;
        Self::to_value("parse_trades_csv", &rows)
    }

    /// 写 op：通用节点进化（`evolve_node_command`）。
    async fn evolve_node_command(&self, input: Value) -> Result<Value> {
        let req = Self::parse::<crate::commands::analyst_evolution::NodeEvolveRequest>(
            "evolve_node_command",
            "NodeEvolveRequest（{nodeType, nodeId}，camelCase）",
            input,
        )?;
        let node_type = crate::commands::analyst_evolution::parse_node_type(&req.node_type);
        let status = axagent_analysis_engine::evolve_node(&self.db, &node_type, &req.node_id)
            .await
            .map_err(AxAgentError::Provider)?;
        Self::to_value("evolve_node_command", &status)
    }

    /// 写 op：保存节点反馈（`save_node_feedback`）。
    async fn save_node_feedback(&self, input: Value) -> Result<Value> {
        let req = Self::parse::<crate::commands::analyst_feedback::SaveNodeFeedbackRequest>(
            "save_node_feedback",
            "SaveNodeFeedbackRequest（camelCase）",
            input,
        )?;
        let id = crate::commands::analyst_feedback::save_node_feedback_core(&self.db, req)
            .await
            .map_err(|e| {
                AxAgentError::Provider(format!("stock.analysis::save_node_feedback 落库失败: {e}"))
            })?;
        Ok(Value::String(id))
    }

    /// 读 op：智选后验摘要（`serenity_validation_summary`）。
    async fn serenity_validation_summary(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct GenRequest {
            generation: Option<String>,
        }
        let req = Self::parse::<GenRequest>("serenity_validation_summary", "{generation?}", input)?;
        let summary = crate::commands::backtest_validation::serenity_validation_summary_core(
            &self.db,
            req.generation,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("serenity_validation_summary", &summary)
    }

    /// 写 op：创建决策回测定时任务（`create_decision_backtest_cron`）。
    async fn create_decision_backtest_cron(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct CreateDcCronRequest {
            cron_expression: Option<String>,
            t_plus_n_list: Option<Vec<i32>>,
            max_picks: Option<u32>,
            period_filter: Option<String>,
            enabled: Option<bool>,
        }
        let req = Self::parse::<CreateDcCronRequest>(
            "create_decision_backtest_cron",
            "{cronExpression?, tPlusNList?, maxPicks?, periodFilter?, enabled?}",
            input,
        )?;
        let job = crate::commands::backtest_validation::create_decision_backtest_cron_core(
            &self.cron_job_store,
            req.cron_expression,
            req.t_plus_n_list,
            req.max_picks,
            req.period_filter,
            req.enabled,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("create_decision_backtest_cron", &job)
    }

    /// 读 op：进化漂移仪表盘（`get_evolution_drift_dashboard`）。
    async fn get_evolution_drift_dashboard(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct AsOfRequest {
            as_of_date: Option<String>,
        }
        let req =
            Self::parse::<AsOfRequest>("get_evolution_drift_dashboard", "{asOfDate?}", input)?;
        let d = crate::commands::stock_analysis::get_evolution_drift_dashboard_core(
            &self.db,
            req.as_of_date,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("get_evolution_drift_dashboard", &d)
    }

    /// 读 op：进化历史（`get_evolution_history`）。
    async fn get_evolution_history(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct LimitRequest {
            limit: Option<u32>,
        }
        let req = Self::parse::<LimitRequest>("get_evolution_history", "{limit?}", input)?;
        let rows = crate::commands::stock_analysis::get_evolution_history_core(&self.db, req.limit)
            .await
            .map_err(AxAgentError::Provider)?;
        Self::to_value("get_evolution_history", &rows)
    }

    /// 写 op：演化漂移重算（`stock_evolution_recalc`）。
    async fn stock_evolution_recalc(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct AsOfRequest {
            as_of_date: Option<String>,
        }
        let req = Self::parse::<AsOfRequest>("stock_evolution_recalc", "{asOfDate?}", input)?;
        crate::commands::stock_analysis::stock_evolution_recalc_core(&self.db, req.as_of_date)
            .await
            .map_err(AxAgentError::Provider)
    }

    /// 读 op：回灌并入开关（`get_mover_reflow_merge`）。
    async fn get_mover_reflow_merge(&self, _input: Value) -> Result<Value> {
        let v = crate::commands::stock_analysis::get_mover_reflow_merge_core(&self.db)
            .await
            .map_err(AxAgentError::Provider)?;
        Ok(Value::String(v))
    }

    /// 写 op：回灌并入开关（`set_mover_reflow_merge`）。
    async fn set_mover_reflow_merge(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct MergeRequest {
            value: String,
        }
        let req = Self::parse::<MergeRequest>("set_mover_reflow_merge", "{value}", input)?;
        let v = crate::commands::stock_analysis::set_mover_reflow_merge_core(&self.db, req.value)
            .await
            .map_err(AxAgentError::Provider)?;
        Ok(Value::String(v))
    }

    /// 读 op：最新一轮荐股链 as-of 摘要（`get_mover_asof_summary`）。
    async fn get_mover_asof_summary(&self, _input: Value) -> Result<Value> {
        let v = crate::commands::stock_analysis::get_mover_asof_summary_core(&self.db)
            .await
            .map_err(AxAgentError::Provider)?;
        Ok(v.unwrap_or(Value::Null))
    }

    /// 读 op：扫描板块范围（`get_reco_board_scope`）。
    async fn get_reco_board_scope(&self, _input: Value) -> Result<Value> {
        let v = crate::commands::stock_analysis::get_reco_board_scope_core(&self.db)
            .await
            .map_err(AxAgentError::Provider)?;
        Ok(Value::String(v))
    }

    /// 写 op：保存扫描板块范围（`set_reco_board_scope`）。
    async fn set_reco_board_scope(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ScopeRequest {
            scope: String,
        }
        let req = Self::parse::<ScopeRequest>("set_reco_board_scope", "{scope}", input)?;
        crate::commands::stock_analysis::set_reco_board_scope_core(&self.db, req.scope)
            .await
            .map_err(AxAgentError::Provider)?;
        Ok(Value::Null)
    }

    /// 读 op：候选池批次留痕（`get_reco_pool_batch`）。
    async fn get_reco_pool_batch(&self, _input: Value) -> Result<Value> {
        let v = crate::commands::stock_analysis::get_reco_pool_batch_core(&self.db)
            .await
            .map_err(AxAgentError::Provider)?;
        Ok(v.map(|b| serde_json::to_value(b).unwrap_or(Value::Null)).unwrap_or(Value::Null))
    }

    /// 读 op：时间旅行批次历史（宿主 `list_time_travel_batches` 的接缝面，#347）。
    ///
    /// 入参 `{limit?, statusFilter?, asOf?}`；出参 = `BatchRunState[]`（摘要，不含明细）。
    async fn list_time_travel_batches(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct ListRequest {
            limit: Option<u64>,
            status_filter: Option<String>,
            as_of: Option<String>,
        }
        let req = Self::parse::<ListRequest>(
            "list_time_travel_batches",
            "{limit?, statusFilter?, asOf?}",
            input,
        )?;
        let rows = crate::commands::time_travel_batch::list_time_travel_batches_core(
            &self.db,
            req.limit,
            req.status_filter,
            req.as_of,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("list_time_travel_batches", &rows)
    }

    // ── #347 批 B22：8 条（IC/信号史/缓存读 + 代理写 + 荐股历史三件 + 反思统计）──
    /// 读 op：荐股 IC 统计（宿主 `reco_ic_stats` 的接缝面，#347）。
    async fn reco_ic_stats(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct IcRequest {
            generation: Option<String>,
        }
        let req = Self::parse::<IcRequest>("reco_ic_stats", "{generation?}", input)?;
        let stats = crate::commands::stock_analysis::reco_ic_stats_core(&self.db, req.generation)
            .await
            .map_err(AxAgentError::Provider)?;
        Self::to_value("reco_ic_stats", &stats)
    }

    /// 读 op：最近一次荐股的策略信号明细（宿主 `get_reco_signal_history` 的接缝面，#347）。
    async fn get_reco_signal_history(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct SigRequest {
            strategy_id: String,
        }
        let req = Self::parse::<SigRequest>("get_reco_signal_history", "{strategyId}", input)?;
        let rows = crate::commands::stock_analysis::get_reco_signal_history_core(
            &self.db,
            &self.astock_client,
            req.strategy_id,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("get_reco_signal_history", &rows)
    }

    /// 读 op：最新荐股缓存（宿主 `get_cached_recommendation` 的接缝面，#347）。
    async fn get_cached_recommendation(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct CachedRequest {
            period: axagent_analysis_engine::recommender::Period,
            as_of_date: Option<String>,
        }
        let req = Self::parse::<CachedRequest>(
            "get_cached_recommendation",
            "{period, asOfDate?}",
            input,
        )?;
        let cached = crate::commands::stock_analysis::get_cached_recommendation_core(
            &self.db,
            req.period,
            req.as_of_date,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("get_cached_recommendation", &cached)
    }

    /// 写 op：东财代理热更新 + 持久化（宿主 `save_eastmoney_proxy` 的接缝面，#347）。
    ///
    /// 写语义两段：热更新共享 `AStockClient` 运行态 + 写 `workflow_template` 模板变量。
    async fn save_eastmoney_proxy(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ProxyRequest {
            proxy_url: String,
        }
        let req = Self::parse::<ProxyRequest>("save_eastmoney_proxy", "{proxyUrl}", input)?;
        crate::commands::stock_analysis::save_eastmoney_proxy_core(
            &self.db,
            &self.astock_client,
            req.proxy_url,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Ok(Value::Null)
    }

    /// 读 op：荐股历史列表（宿主 `list_reco_history` 的接缝面，#347）。
    async fn list_reco_history(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct ListRequest {
            style_filter: Option<String>,
            exclude_styles: Option<String>,
            limit: Option<u32>,
            offset: Option<u32>,
            generation: Option<String>,
        }
        let req = Self::parse::<ListRequest>(
            "list_reco_history",
            "{styleFilter?, excludeStyles?, limit?, offset?, generation?}",
            input,
        )?;
        let rows = crate::commands::stock_workflow::reco_history::list_reco_history_core(
            &self.db,
            req.style_filter,
            req.exclude_styles,
            req.limit,
            req.offset,
            req.generation,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("list_reco_history", &rows)
    }

    /// 读 op：荐股历史明细（宿主 `get_reco_detail` 的接缝面，#347）。
    async fn get_reco_detail(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct DetailRequest {
            generated_at: String,
            style_filter: Option<String>,
            exclude_styles: Option<String>,
        }
        let req = Self::parse::<DetailRequest>(
            "get_reco_detail",
            "{generatedAt, styleFilter?, excludeStyles?}",
            input,
        )?;
        let rows = crate::commands::stock_workflow::reco_history::get_reco_detail_core(
            &self.db,
            req.generated_at,
            req.style_filter,
            req.exclude_styles,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("get_reco_detail", &rows)
    }

    /// 写 op：决策信任记录（宿主 `record_decision_trust` 的接缝面，#347）。
    async fn record_decision_trust(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct TrustRequest {
            analysis_id: String,
            trust_model: String,
        }
        let req = Self::parse::<TrustRequest>(
            "record_decision_trust",
            "{analysisId, trustModel}",
            input,
        )?;
        let out = crate::commands::stock_workflow::misc::record_decision_trust_core(
            &self.db,
            req.analysis_id,
            req.trust_model,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("record_decision_trust", &out)
    }

    /// 读 op：反思命中率统计（宿主 `reflection_stats` 的接缝面，#347）。
    async fn reflection_stats(&self, input: Value) -> Result<Value> {
        Self::require_no_params("reflection_stats", &input)?;
        let stats =
            crate::commands::stock_workflow::reflection_stats::reflection_stats_core(&self.db)
                .await
                .map_err(AxAgentError::Provider)?;
        Self::to_value("reflection_stats", &stats)
    }

    // ── #349 批：异步任务 seam（start/poll/cancel）+ 同步面两条 ──

    /// 写 op：发起宿主长任务（#349）。入参 `{ task, input }`，task 必须命中白名单。
    ///
    /// 执行体跑在**宿主进程**（`spawn_tracked_impl` 台账：tokio::spawn + 任务中心可见 +
    /// 协作式取消），插件侧立即拿 `{ taskId }`；经 `poll_task` 轮询终态、`cancel_task` 取消。
    /// 执行体天然持有宿主全部单例 ⇒ LLM / 向量库 / 工作流引擎无需经接缝透传。
    async fn start_task(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct StartRequest {
            task: String,
            #[serde(default)]
            input: Value,
        }
        let req = Self::parse::<StartRequest>("start_task", "{task, input?}", input)?;
        let inner = req.input;
        let accepted = match req.task.as_str() {
            "backtest_reco_strategies" => {
                #[derive(Deserialize, Default)]
                #[serde(rename_all = "camelCase")]
                struct Req {
                    as_of_date: Option<String>,
                }
                let r = Self::parse::<Req>(
                    "start_task.backtest_reco_strategies",
                    "{asOfDate?}",
                    inner,
                )?;
                let app = self.app_handle.clone();
                let db = self.db.clone();
                let dbh = db.clone();
                let client = self.astock_client.clone();
                let app2 = app.clone();
                crate::commands::background_tasks::spawn_tracked_impl(
                    app,
                    &dbh,
                    "荐股策略历史回测对比",
                    "reco-strategy-backtest",
                    "最近一次荐股的正负样本两组策略信号历史回溯对比",
                    async move {
                        crate::commands::stock_analysis::backtest_reco_strategies_task(
                            app2,
                            &db,
                            &client,
                            r.as_of_date,
                        )
                        .await
                    },
                    |_| "荐股策略回测对比完成".to_string(),
                )
                .await?
            },
            "replay_tool_chain" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Req {
                    params: crate::commands::stock_analysis::ReplayToolChainRequest,
                }
                let r = Self::parse::<Req>("start_task.replay_tool_chain", "{params}", inner)?;
                let app = self.app_handle.clone();
                let db = self.db.clone();
                let dbh = db.clone();
                crate::commands::background_tasks::spawn_tracked_impl(
                    app,
                    &dbh,
                    "工具链重放",
                    "replay-tool-chain",
                    "What-If 工具链重放（多源行情 + 评分推演）",
                    async move {
                        crate::commands::stock_analysis::replay_tool_chain_task(r.params).await
                    },
                    |_| "工具链重放完成".to_string(),
                )
                .await?
            },
            "quick_backtest" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Req {
                    request: crate::commands::stock_analysis::QuickBacktestRequest,
                }
                let r = Self::parse::<Req>("start_task.quick_backtest", "{request}", inner)?;
                let app = self.app_handle.clone();
                let db = self.db.clone();
                let dbh = db.clone();
                let client = self.astock_client.clone();
                crate::commands::background_tasks::spawn_tracked_impl(
                    app,
                    &dbh,
                    "快速回测",
                    "quick-backtest",
                    "单票多采样点决策回测",
                    async move {
                        crate::commands::stock_analysis::quick_backtest_task(&client, r.request)
                            .await
                    },
                    |_| "快速回测完成".to_string(),
                )
                .await?
            },
            "screen_stocks" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Req {
                    criteria: axagent_analysis_engine::screener::ScreenCriteria,
                }
                let r = Self::parse::<Req>("start_task.screen_stocks", "{criteria}", inner)?;
                let app = self.app_handle.clone();
                let db = self.db.clone();
                let dbh = db.clone();
                let client = self.astock_client.clone();
                crate::commands::background_tasks::spawn_tracked_impl(
                    app,
                    &dbh,
                    "选股扫描",
                    "screen-stocks",
                    "按条件全市场选股扫描",
                    async move {
                        crate::commands::stock_analysis::screen_stocks_task(
                            &db, &client, r.criteria,
                        )
                        .await
                    },
                    |_| "选股扫描完成".to_string(),
                )
                .await?
            },
            "analyze_mover_recall" => {
                #[derive(Deserialize, Default)]
                #[serde(rename_all = "camelCase")]
                struct Req {
                    to: Option<String>,
                }
                let r = Self::parse::<Req>("start_task.analyze_mover_recall", "{to?}", inner)?;
                let app = self.app_handle.clone();
                let db = self.db.clone();
                let dbh = db.clone();
                crate::commands::background_tasks::spawn_tracked_impl(
                    app,
                    &dbh,
                    "妖股漏检核查",
                    "mover-recall",
                    "全 A 股窗口涨幅达标漏检逐票核查",
                    async move {
                        crate::commands::mover_recall::analyze_mover_recall_task(&db, r.to).await
                    },
                    |_| "妖股漏检核查完成".to_string(),
                )
                .await?
            },
            "run_universe_code_scan" => {
                #[derive(Deserialize, Default)]
                #[serde(rename_all = "camelCase")]
                struct Req {
                    shard: Option<u32>,
                    shards: Option<u32>,
                }
                let r = Self::parse::<Req>(
                    "start_task.run_universe_code_scan",
                    "{shard?, shards?}",
                    inner,
                )?;
                let app = self.app_handle.clone();
                let db = self.db.clone();
                let dbh = db.clone();
                let client = self.astock_client.clone();
                crate::commands::background_tasks::spawn_tracked_impl(
                    app,
                    &dbh,
                    "编码段盲扫",
                    "universe-scan",
                    "只登记市场清单，不产事件",
                    async move {
                        crate::commands::universe_scan::run_universe_code_scan_task(
                            &db, &client, r.shard, r.shards,
                        )
                        .await
                    },
                    |_| "编码段盲扫完成".to_string(),
                )
                .await?
            },
            "preview_adjust_reco_weights" => {
                #[derive(Deserialize, Default)]
                #[serde(rename_all = "camelCase")]
                struct Req {
                    as_of_date: Option<String>,
                }
                let r = Self::parse::<Req>(
                    "start_task.preview_adjust_reco_weights",
                    "{asOfDate?}",
                    inner,
                )?;
                let app = self.app_handle.clone();
                let db = self.db.clone();
                let dbh = db.clone();
                let client = self.astock_client.clone();
                crate::commands::background_tasks::spawn_tracked_impl(
                    app,
                    &dbh,
                    "权重调整预览",
                    "reco-weight-preview",
                    "荐股权重调整的历史回测预览（长跑）",
                    async move {
                        crate::commands::stock_analysis::preview_adjust_reco_weights_task(
                            &db,
                            &client,
                            r.as_of_date,
                        )
                        .await
                    },
                    |_| "权重调整预览完成".to_string(),
                )
                .await?
            },
            "check_vendor_health" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Req {
                    vendor: String,
                }
                let r = Self::parse::<Req>("start_task.check_vendor_health", "{vendor}", inner)?;
                let app = self.app_handle.clone();
                let db = self.db.clone();
                let dbh = db.clone();
                let client = self.astock_client.clone();
                crate::commands::background_tasks::spawn_tracked_impl(
                    app,
                    &dbh,
                    "vendor 健康检查",
                    "vendor-health",
                    "数据源接口健康探测",
                    async move {
                        crate::commands::stock_analysis::check_vendor_health_task(
                            &db, &client, r.vendor,
                        )
                        .await
                    },
                    |_| "vendor 健康检查完成".to_string(),
                )
                .await?
            },
            "batch_get_klines" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Req {
                    codes: Vec<String>,
                    period: String,
                    limit: u32,
                }
                let r = Self::parse::<Req>(
                    "start_task.batch_get_klines",
                    "{codes, period, limit}",
                    inner,
                )?;
                let app = self.app_handle.clone();
                let db = self.db.clone();
                let dbh = db.clone();
                let client = self.astock_client.clone();
                crate::commands::background_tasks::spawn_tracked_impl(
                    app,
                    &dbh,
                    "批量 K 线获取",
                    "batch-klines",
                    "逐票批量 K 线拉取（批量网络）",
                    async move {
                        crate::commands::stock_analysis::batch_get_klines_task(
                            &client, r.codes, r.period, r.limit,
                        )
                        .await
                    },
                    |_| "批量 K 线获取完成".to_string(),
                )
                .await?
            },
            "backtest_analysis" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Req {
                    stock_code: String,
                    analysis_date: String,
                    decision_action: String,
                    decision_confidence: f64,
                    holding_days: u32,
                    as_of_date: Option<String>,
                }
                let r = Self::parse::<Req>(
                    "start_task.backtest_analysis",
                    "{stockCode, analysisDate, decisionAction, decisionConfidence, holdingDays, asOfDate?}",
                    inner,
                )?;
                let app = self.app_handle.clone();
                let db = self.db.clone();
                let dbh = db.clone();
                let client = self.astock_client.clone();
                crate::commands::background_tasks::spawn_tracked_impl(
                    app,
                    &dbh,
                    "决策回测",
                    "backtest",
                    "单票单决策后验回测",
                    async move {
                        crate::commands::stock_analysis::backtest_analysis_task(
                            &db,
                            &client,
                            r.stock_code,
                            r.analysis_date,
                            r.decision_action,
                            r.decision_confidence,
                            r.holding_days,
                            r.as_of_date,
                        )
                        .await
                    },
                    |_| "决策回测完成".to_string(),
                )
                .await?
            },
            "backtest_all_history" => {
                #[derive(Deserialize, Default)]
                #[serde(rename_all = "camelCase")]
                struct Req {
                    holding_days: u32,
                    scope: Option<String>,
                }
                let r = Self::parse::<Req>(
                    "start_task.backtest_all_history",
                    "{holdingDays, scope?}",
                    inner,
                )?;
                let app = self.app_handle.clone();
                let db = self.db.clone();
                let dbh = db.clone();
                let client = self.astock_client.clone();
                crate::commands::background_tasks::spawn_tracked_impl(
                    app,
                    &dbh,
                    "全历史回测统计",
                    "backtest",
                    "全部已完成分析的批量回测统计（长跑）",
                    async move {
                        crate::commands::stock_analysis::backtest_all_history_task(
                            &db,
                            &client,
                            r.holding_days,
                            r.scope,
                        )
                        .await
                    },
                    |_| "全历史回测统计完成".to_string(),
                )
                .await?
            },
            "run_replay_backtest" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Req {
                    items: Vec<crate::commands::stock_analysis::ReplaySweepItem>,
                    holding_days: u32,
                }
                let r = Self::parse::<Req>(
                    "start_task.run_replay_backtest",
                    "{items, holdingDays}",
                    inner,
                )?;
                let app = self.app_handle.clone();
                let db = self.db.clone();
                let dbh = db.clone();
                let client = self.astock_client.clone();
                crate::commands::background_tasks::spawn_tracked_impl(
                    app,
                    &dbh,
                    "逐样本回放回测",
                    "backtest",
                    "逐样本 as-of 回放回测（长跑）",
                    async move {
                        crate::commands::stock_analysis::run_replay_backtest_task(
                            &client,
                            r.items,
                            r.holding_days,
                        )
                        .await
                    },
                    |_| "逐样本回放回测完成".to_string(),
                )
                .await?
            },
            "run_reflection_now" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Req {
                    stock_code: String,
                    stock_name: String,
                    as_of_date: String,
                    actual_outcome: Option<String>,
                    reflection_depth: Option<String>,
                    horizon: Option<String>,
                }
                let r = Self::parse::<Req>(
                    "start_task.run_reflection_now",
                    "{stockCode, stockName, asOfDate, actualOutcome?, reflectionDepth?, horizon?}",
                    inner,
                )?;
                let app = self.app_handle.clone();
                let db = self.db.clone();
                let dbh = db.clone();
                let client = self.astock_client.clone();
                let engine = self.work_engine.clone();
                let vs = self.vector_store.clone();
                let ts = self.trajectory_storage.clone();
                let mk = self.master_key;
                crate::commands::background_tasks::spawn_tracked_impl(
                    app,
                    &dbh,
                    "手动复盘反思",
                    "reflection",
                    "单票行情快照手动反思（四档快照 + RAG）",
                    async move {
                        crate::commands::stock_analysis::run_reflection_now_task(
                            &db,
                            &client,
                            &engine,
                            &vs,
                            &ts,
                            &mk,
                            r.stock_code,
                            r.stock_name,
                            r.as_of_date,
                            r.actual_outcome,
                            r.reflection_depth,
                            r.horizon,
                        )
                        .await
                    },
                    |_| "手动反思完成".to_string(),
                )
                .await?
            },
            "trigger_stock_evolution" => {
                #[derive(Deserialize, Default)]
                #[serde(rename_all = "camelCase")]
                struct Req {
                    reason: Option<String>,
                    template_id: Option<String>,
                }
                let r = Self::parse::<Req>(
                    "start_task.trigger_stock_evolution",
                    "{reason?, templateId?}",
                    inner,
                )?;
                let app = self.app_handle.clone();
                let db = self.db.clone();
                let dbh = db.clone();
                let adaptive = self.stock_adaptive_engine.clone();
                crate::commands::background_tasks::spawn_tracked_impl(
                    app,
                    &dbh,
                    "股票进化触发",
                    "stock-evolution",
                    "手动触发股票自适应进化（长跑）",
                    async move {
                        crate::commands::stock_analysis::trigger_stock_evolution_task(
                            &adaptive,
                            r.reason,
                            r.template_id,
                        )
                        .await
                    },
                    |_| "股票进化触发完成".to_string(),
                )
                .await?
            },
            "recommend_stocks_all_periods" => {
                #[derive(Deserialize, Default)]
                #[serde(rename_all = "camelCase")]
                struct Req {
                    as_of_date: Option<String>,
                }
                let r = Self::parse::<Req>(
                    "start_task.recommend_stocks_all_periods",
                    "{asOfDate?}",
                    inner,
                )?;
                let app = self.app_handle.clone();
                let db = self.db.clone();
                let dbh = db.clone();
                let client = self.astock_client.clone();
                crate::commands::background_tasks::spawn_tracked_impl(
                    app,
                    &dbh,
                    "四档荐股",
                    "reco",
                    "全四档智能荐股（种子刷新 + 长链）",
                    async move {
                        crate::commands::stock_analysis::recommend_stocks_all_periods_task(
                            &db,
                            &client,
                            r.as_of_date,
                        )
                        .await
                    },
                    |_| "四档荐股完成".to_string(),
                )
                .await?
            },
            "recommend_stocks" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Req {
                    period: axagent_analysis_engine::recommender::Period,
                    as_of_date: Option<String>,
                }
                let r = Self::parse::<Req>(
                    "start_task.recommend_stocks",
                    "{period, asOfDate?}",
                    inner,
                )?;
                let app = self.app_handle.clone();
                let db = self.db.clone();
                let dbh = db.clone();
                let client = self.astock_client.clone();
                crate::commands::background_tasks::spawn_tracked_impl(
                    app,
                    &dbh,
                    "智能荐股",
                    "reco",
                    "单档智能荐股（种子刷新 + 长链）",
                    async move {
                        crate::commands::stock_analysis::recommend_stocks_task(
                            &db,
                            &client,
                            r.period,
                            r.as_of_date,
                        )
                        .await
                    },
                    |_| "荐股完成".to_string(),
                )
                .await?
            },
            "generate_daily_review" => {
                Self::require_no_params("start_task.generate_daily_review", &inner)?;
                let app = self.app_handle.clone();
                let db = self.db.clone();
                let dbh = db.clone();
                let client = self.astock_client.clone();
                crate::commands::background_tasks::spawn_tracked_impl(
                    app,
                    &dbh,
                    "每日复盘生成",
                    "daily-review",
                    "自选股 + 当日告警逐票行情复盘生成",
                    async move {
                        crate::commands::stock_analysis::generate_daily_review_task(&db, &client)
                            .await
                    },
                    |_| "每日复盘生成完成".to_string(),
                )
                .await?
            },
            "run_time_travel_batch" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Req {
                    request: crate::commands::time_travel_batch::TimeTravelBatchRequest,
                }
                let r = Self::parse::<Req>("start_task.run_time_travel_batch", "{request}", inner)?;
                let app = self.app_handle.clone();
                let db = self.db.clone();
                let dbh = db.clone();
                let client = self.astock_client.clone();
                let engine = self.work_engine.clone();
                let vs = self.vector_store.clone();
                let ts = self.trajectory_storage.clone();
                let lock = self.time_travel_batch_lock.clone();
                let mk = self.master_key;
                crate::commands::background_tasks::spawn_tracked_impl(
                    app,
                    &dbh,
                    "时间旅行批次复盘",
                    "time-travel",
                    "as-of 封闭世界双链回放 + 四档反思（长跑）",
                    async move {
                        crate::commands::time_travel_batch::run_time_travel_batch_inner(
                            &db, &client, &engine, &vs, &mk, &ts, &lock, r.request,
                        )
                        .await
                    },
                    |_| "时间旅行批次完成".to_string(),
                )
                .await?
            },
            other => {
                return Err(AxAgentError::Validation(format!(
                    "start_task 未知任务种类 `{other}`（白名单见宿主 service 派发表）"
                )));
            },
        };
        Self::to_value("start_task", &accepted)
    }

    /// 读 op：轮询宿主长任务（#349）。入参 `{ taskId }`。
    async fn poll_task(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Req {
            task_id: String,
        }
        let r = Self::parse::<Req>("poll_task", "{taskId}", input)?;
        let snap = crate::commands::background_tasks::task_snapshot(&self.db, &r.task_id)
            .await
            .map_err(AxAgentError::Provider)?
            .ok_or_else(|| AxAgentError::Provider(format!("任务 `{}` 不存在", r.task_id)))?;
        Self::to_value("poll_task", &snap)
    }

    /// 写 op：协作式取消宿主长任务（#349）。入参 `{ taskId }`；与宿主
    /// `stop_background_task` 同源（`cancel_task_core`）。
    async fn cancel_task(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Req {
            task_id: String,
        }
        let r = Self::parse::<Req>("cancel_task", "{taskId}", input)?;
        crate::commands::background_tasks::cancel_task_core(&self.db, &r.task_id)
            .await
            .map_err(AxAgentError::Provider)?;
        Ok(Value::Null)
    }

    /// 写 op：个股 HTML 报告渲染落盘（宿主 `generate_stock_report` 迁 T2，#349）。
    async fn generate_stock_report(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Req {
            analysis_id: String,
        }
        let r = Self::parse::<Req>("generate_stock_report", "{analysisId}", input)?;
        let out = crate::commands::stock_analysis::generate_stock_report_task(
            &self.db,
            &self.astock_client,
            r.analysis_id,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("generate_stock_report", &out)
    }

    /// 读 op：时间旅行批次状态（宿主 `get_time_travel_batch` 迁 T2，#349）。
    /// 历史表权威；`ensure_stale_reaped` 不跟随（同 #338/#347 判据）。
    async fn get_time_travel_batch(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Req {
            batch_id: String,
        }
        let r = Self::parse::<Req>("get_time_travel_batch", "{batchId}", input)?;
        let batch =
            crate::commands::time_travel_batch::get_time_travel_batch_core(&self.db, &r.batch_id)
                .await
                .map_err(AxAgentError::Provider)?;
        Self::to_value("get_time_travel_batch", &batch)
    }

    // ── #346 批 B20：11 条（权重/参数台账/估值/工具/What-If）──

    /// 读 op：荐股策略权重（`get_reco_strategy_weights`）。
    async fn get_reco_strategy_weights(&self, _input: Value) -> Result<Value> {
        crate::commands::stock_analysis::get_reco_strategy_weights_core(&self.db)
            .await
            .map_err(AxAgentError::Provider)
    }

    /// 写 op：手动重算策略权重（`manual_recalc_strategy_weights`）。
    async fn manual_recalc_strategy_weights(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct AsOfRequest {
            as_of_date: Option<String>,
        }
        let req =
            Self::parse::<AsOfRequest>("manual_recalc_strategy_weights", "{asOfDate?}", input)?;
        crate::commands::stock_analysis::manual_recalc_strategy_weights_core(
            &self.db,
            req.as_of_date,
        )
        .await
        .map_err(AxAgentError::Provider)
    }

    /// 写 op：重算荐股闭环逐格权重（`recalc_reco_loop_weights`）。
    async fn recalc_reco_loop_weights(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct AsOfRequest {
            as_of_date: Option<String>,
        }
        let req = Self::parse::<AsOfRequest>("recalc_reco_loop_weights", "{asOfDate?}", input)?;
        crate::commands::stock_analysis::recalc_reco_loop_weights_core(&self.db, req.as_of_date)
            .await
            .map_err(AxAgentError::Provider)
    }

    /// 写 op：应用参数调整建议（`apply_param_suggestions`）。
    async fn apply_param_suggestions(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct SuggestionsRequest {
            updates: Vec<Value>,
        }
        let req = Self::parse::<SuggestionsRequest>(
            "apply_param_suggestions",
            "{updates: [...]}",
            input,
        )?;
        crate::commands::stock_analysis::apply_param_suggestions_core(&self.db, req.updates)
            .await
            .map_err(AxAgentError::Provider)
    }

    /// 写 op：回滚参数变更（`rollback_param_change`）。
    async fn rollback_param_change(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct RollbackRequest {
            change_id: String,
        }
        let req = Self::parse::<RollbackRequest>("rollback_param_change", "{changeId}", input)?;
        crate::commands::stock_analysis::rollback_param_change_core(&self.db, req.change_id)
            .await
            .map_err(AxAgentError::Provider)
    }

    /// 写 op：应用荐股策略权重（`apply_reco_weights`）。
    async fn apply_reco_weights(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct ApplyWeightsRequest {
            weights: Option<Vec<Value>>,
        }
        let req = Self::parse::<ApplyWeightsRequest>("apply_reco_weights", "{weights?}", input)?;
        crate::commands::stock_analysis::apply_reco_weights_core(&self.db, req.weights)
            .await
            .map_err(AxAgentError::Provider)
    }

    /// 写 op：校准组合管理参数（`calibrate_portfolio_mgr_params`）。
    async fn calibrate_portfolio_mgr_params(&self, _input: Value) -> Result<Value> {
        crate::commands::stock_analysis::calibrate_portfolio_mgr_params_core(&self.db)
            .await
            .map_err(AxAgentError::Provider)
    }

    /// 写 op：优化评分权重（`optimize_scoring_weights`）。
    async fn optimize_scoring_weights(&self, _input: Value) -> Result<Value> {
        let w = crate::commands::stock_analysis::optimize_scoring_weights_core(
            &self.db,
            &self.astock_client,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("optimize_scoring_weights", &w)
    }

    /// 读 op：估值带（`compute_valuation_band`，读并回填 financial_snapshots）。
    async fn compute_valuation_band(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct BandRequest {
            stock_code: String,
            years: Option<u32>,
        }
        let req =
            Self::parse::<BandRequest>("compute_valuation_band", "{stockCode, years?}", input)?;
        let band = crate::commands::stock_analysis::compute_valuation_band_core(
            &self.db,
            &self.astock_client,
            req.stock_code,
            req.years,
        )
        .await
        .map_err(AxAgentError::Provider)?;
        Self::to_value("compute_valuation_band", &band)
    }

    /// 读 op：股票分析工具名清单（`list_stock_tools`）。
    async fn list_stock_tools(&self, _input: Value) -> Result<Value> {
        let names = crate::commands::stock_analysis::list_stock_tools_core()
            .await
            .map_err(AxAgentError::Provider)?;
        Self::to_value("list_stock_tools", &names)
    }

    /// 读 op：What-If 确定性公式（`compute_what_if`，纯计算）。
    async fn compute_what_if(&self, input: Value) -> Result<Value> {
        let req = Self::parse::<crate::commands::stock_analysis::WhatIfRequest>(
            "compute_what_if",
            "WhatIfRequest（camelCase）",
            input,
        )?;
        let out = crate::commands::stock_analysis::compute_what_if_core(req)
            .map_err(AxAgentError::Provider)?;
        Self::to_value("compute_what_if", &out)
    }

    /// 读 op：参数调整建议列表（宿主 `list_param_suggestions` 的接缝面）。
    ///
    /// 入参 `{stockCode?: string}`；出参 = 建议行数组（形状权威在 commands 层 core 函数）。
    async fn list_param_suggestions(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct ParamSuggestionsRequest {
            stock_code: Option<String>,
        }
        let request = serde_json::from_value::<ParamSuggestionsRequest>(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::list_param_suggestions 参数解析失败: {e}。期望 {{stockCode?: string}}"
            ))
        })?;
        let rows = list_param_suggestions_core(&self.db, request.stock_code.as_deref())
            .await
            .map_err(|e| {
                AxAgentError::Provider(format!(
                    "stock.analysis::list_param_suggestions 查询失败: {e}"
                ))
            })?;
        Ok(Value::Array(rows))
    }

    /// 写 op：失效荐股缓存（宿主 `invalidate_recommendation_cache` 的接缝面）。
    ///
    /// ⚠ 必须落在宿主进程：`recommender::invalidate_cache()` 失效的是**当前进程**的
    /// 进程内缓存；worker 直调只会清掉插件进程副本，而宿主 `recommend_stocks_multi`
    /// 等活消费方用的是宿主副本（§183 表的裁定依据）。入参恒空对象，出参 = `null`。
    async fn invalidate_recommendation_cache(&self, _input: Value) -> Result<Value> {
        axagent_analysis_engine::recommender::invalidate_cache();
        Ok(Value::Null)
    }

    /// 写 op：自改进分析循环（宿主 `run_self_improving_stock_analysis` 的接缝面）。
    ///
    /// 执行体在宿主 agent crate（consumer 层，插件不可链接）：`SelfImprovementExecutor`
    /// + 领域回合 `StockAnalysisRound`。入参 `{task: string, maxRounds?: number}`；
    /// 出参与宿主原命令同形（`partial: true` = 中途出错但带回已完成轮次）。
    async fn run_self_improving_stock_analysis(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct SelfImprovingRequest {
            task: String,
            max_rounds: Option<u32>,
        }
        let request = serde_json::from_value::<SelfImprovingRequest>(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::run_self_improving_stock_analysis 参数解析失败: {e}。期望 \
                 {{task: string, maxRounds?: number}}"
            ))
        })?;
        let round = StockAnalysisRound::new(self.astock_client.clone());
        let config = SelfImprovementConfig::new(
            request.max_rounds.unwrap_or(3),
            0.80, // 收敛阈值：评估分数高于此值直接 Accept
            3,    // 连续无进展多少次后 Escalate
        );
        let mut executor = SelfImprovementExecutor::new(Box::new(round), config);
        match executor.run(&request.task).await {
            Ok(output) => Ok(serde_json::json!({
                "text": output.text,
                "totalRounds": output.total_rounds,
                "finalScore": output.final_evaluation.score,
                "confidence": output.final_evaluation.confidence,
                "strengths": output.final_evaluation.strengths,
                "gaps": output.final_evaluation.gaps,
            })),
            Err(e) => {
                // 即使出错也尝试返回已有的 round_history（宿主原命令同口径）
                let partial = executor.round_history();
                if let Some(last) = partial.last() {
                    Ok(serde_json::json!({
                        "text": last.output.clone(),
                        "totalRounds": partial.len(),
                        "finalScore": last.evaluation.as_ref().map(|e| e.score),
                        "error": e.to_string(),
                        "partial": true,
                    }))
                } else {
                    Err(AxAgentError::Provider(format!("自改进分析失败: {e}")))
                }
            },
        }
    }

    /// 读 op：待审批进化计划列表（宿主 `list_pending_evolution_plans` 的接缝面）。
    ///
    /// 出参 = dao `EvolutionHistoryModel` 数组的 JSON（宿主命令原样返回该模型）。
    /// 入参恒 `{}` —— 该命令无参。
    async fn pending_evolution_plans(&self, input: Value) -> Result<Value> {
        if !input.is_null() && !input.as_object().is_some_and(|o| o.is_empty()) {
            return Err(AxAgentError::Validation(
                "stock.analysis::pending_evolution_plans 不收参数（期望 {} 或缺省）".into(),
            ));
        }
        let rows = evo_hist::list_pending_asc(&self.db).await.map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::pending_evolution_plans 查询失败: {e}"))
        })?;
        serde_json::to_value(&rows).map_err(|e| {
            AxAgentError::Provider(format!(
                "stock.analysis::pending_evolution_plans 序列化失败: {e}"
            ))
        })
    }

    /// 纯计算 op：Walk-Forward + DES 对比仿真（宿主 `wf_des_integration` 的接缝面）。
    ///
    /// ⚠ 实现体内部自建 tokio runtime（`commands/wf_des.rs` 的同步命令形态），**必须**
    /// `spawn_blocking` 执行 —— 在 async executor 上直调构成嵌套 runtime（禁区 9）。
    async fn des_integration(&self, input: Value) -> Result<Value> {
        let request: WfDesRequest = serde_json::from_value(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::des_integration 参数解析失败: {e}。期望 WfDesRequest \
                 （形状权威：commands/wf_des.rs，camelCase）"
            ))
        })?;
        let report = tokio::task::spawn_blocking(move || wf_des_integration(request))
            .await
            .map_err(|e| {
                AxAgentError::Provider(format!("stock.analysis::des_integration 任务失败: {e}"))
            })?
            .map_err(|e| {
                AxAgentError::Provider(format!("stock.analysis::des_integration 仿真失败: {e}"))
            })?;
        serde_json::to_value(&report).map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::des_integration 序列化失败: {e}"))
        })
    }

    // ── 定时任务配置 op（#35 批 1a）──────────────────────────────────────
    //
    // 每个 op 都是「解析入参 → 调命令侧无 State 核心 → 序列化出参」。核心是宿主命令
    // 与接缝的**同一份**实现（校验 / 默认值 / 响应形状只有一处，禁区 12）。

    /// 写 op：创建荐股定时任务。
    async fn create_recommendation_cron(&self, input: Value) -> Result<Value> {
        let request = Self::parse::<RecoCreateRequest>(
            "create_recommendation_cron",
            "{name, cronExpression, periods[], minConfidence, topN}",
            input,
        )?;
        let response = create_recommendation_cron_core(&self.cron_job_store, request)
            .await
            .map_err(|e| Self::cron_err("create_recommendation_cron", e))?;
        Self::to_value("create_recommendation_cron", &response)
    }

    /// 读 op：列出荐股定时任务。
    async fn list_recommendation_crons(&self, input: Value) -> Result<Value> {
        Self::require_no_params("list_recommendation_crons", &input)?;
        let response = list_recommendation_crons_core(&self.cron_job_store)
            .await
            .map_err(|e| Self::cron_err("list_recommendation_crons", e))?;
        Self::to_value("list_recommendation_crons", &response)
    }

    /// 写 op：启停荐股定时任务。
    async fn toggle_recommendation_cron(&self, input: Value) -> Result<Value> {
        let request =
            Self::parse::<CronToggleRequest>("toggle_recommendation_cron", "{id, enabled}", input)?;
        toggle_recommendation_cron_core(&self.cron_job_store, &request.id, request.enabled)
            .await
            .map_err(|e| Self::cron_err("toggle_recommendation_cron", e))?;
        Ok(Value::Null)
    }

    /// 写 op：删除荐股定时任务。
    async fn delete_recommendation_cron(&self, input: Value) -> Result<Value> {
        let request = Self::parse::<CronIdRequest>("delete_recommendation_cron", "{id}", input)?;
        delete_recommendation_cron_core(&self.cron_job_store, &request.id)
            .await
            .map_err(|e| Self::cron_err("delete_recommendation_cron", e))?;
        Ok(Value::Null)
    }

    /// 写 op：创建趋势智选定任务（三参可选）。
    async fn create_trend_screening_cron(&self, input: Value) -> Result<Value> {
        let request = Self::parse::<TrendCreateRequest>(
            "create_trend_screening_cron",
            "{cronExpression?, enabled?, workflowId?}（缺省 = 原链 / 0 16 * * 1-5 / 启用）",
            input,
        )?;
        let response = create_trend_screening_cron_core(&self.cron_job_store, request)
            .await
            .map_err(|e| Self::cron_err("create_trend_screening_cron", e))?;
        Self::to_value("create_trend_screening_cron", &response)
    }

    /// 读 op：列出趋势智选定时任务。
    async fn list_trend_screening_crons(&self, input: Value) -> Result<Value> {
        Self::require_no_params("list_trend_screening_crons", &input)?;
        let response = list_trend_screening_crons_core(&self.cron_job_store)
            .await
            .map_err(|e| Self::cron_err("list_trend_screening_crons", e))?;
        Self::to_value("list_trend_screening_crons", &response)
    }

    /// 写 op：启停趋势智选定时任务。
    async fn toggle_trend_screening_cron(&self, input: Value) -> Result<Value> {
        let request = Self::parse::<CronToggleRequest>(
            "toggle_trend_screening_cron",
            "{id, enabled}",
            input,
        )?;
        toggle_trend_screening_cron_core(&self.cron_job_store, &request.id, request.enabled)
            .await
            .map_err(|e| Self::cron_err("toggle_trend_screening_cron", e))?;
        Ok(Value::Null)
    }

    /// 写 op：删除趋势智选定时任务。
    async fn delete_trend_screening_cron(&self, input: Value) -> Result<Value> {
        let request = Self::parse::<CronIdRequest>("delete_trend_screening_cron", "{id}", input)?;
        delete_trend_screening_cron_core(&self.cron_job_store, &request.id)
            .await
            .map_err(|e| Self::cron_err("delete_trend_screening_cron", e))?;
        Ok(Value::Null)
    }

    /// 写 op：创建候选池扫设定时任务（六参可选，缺省值在核心内）。
    async fn create_pool_scan_cron(&self, input: Value) -> Result<Value> {
        let request = Self::parse::<PoolCreateRequest>(
            "create_pool_scan_cron",
            "{cronExpression?, lookbackDays?, maxStocks?, minConfidence?, enabled?, \
             maxPerRun?, runDeadlineSeconds?}",
            input,
        )?;
        let response = create_pool_scan_cron_core(&self.cron_job_store, request)
            .await
            .map_err(|e| Self::cron_err("create_pool_scan_cron", e))?;
        Self::to_value("create_pool_scan_cron", &response)
    }

    /// 读 op：列出候选池扫设定时任务。
    async fn list_pool_scan_crons(&self, input: Value) -> Result<Value> {
        Self::require_no_params("list_pool_scan_crons", &input)?;
        let response = list_pool_scan_crons_core(&self.cron_job_store)
            .await
            .map_err(|e| Self::cron_err("list_pool_scan_crons", e))?;
        Self::to_value("list_pool_scan_crons", &response)
    }

    /// 写 op：启停候选池扫设定时任务。
    async fn toggle_pool_scan_cron(&self, input: Value) -> Result<Value> {
        let request =
            Self::parse::<CronToggleRequest>("toggle_pool_scan_cron", "{id, enabled}", input)?;
        toggle_pool_scan_cron_core(&self.cron_job_store, &request.id, request.enabled)
            .await
            .map_err(|e| Self::cron_err("toggle_pool_scan_cron", e))?;
        Ok(Value::Null)
    }

    /// 写 op：删除候选池扫设定时任务。
    async fn delete_pool_scan_cron(&self, input: Value) -> Result<Value> {
        let request = Self::parse::<CronIdRequest>("delete_pool_scan_cron", "{id}", input)?;
        delete_pool_scan_cron_core(&self.cron_job_store, &request.id)
            .await
            .map_err(|e| Self::cron_err("delete_pool_scan_cron", e))?;
        Ok(Value::Null)
    }

    /// 读 op：列估值快照原始行（F1 余三条 #35）。
    ///
    /// 与被退役的宿主命令 `list_financial_snapshots` 同一查询形状（同一份 filter / 排序）；
    /// `start` / `end` 是 `snapshot_date` 闭区间（YYYY-MM-DD，缺省不筛）。实体行整块透传，
    /// 不上收 DTO（形状权威在 entities，照 `list_recent_mainlines` 先例）。
    async fn list_financial_snapshots(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        struct Input {
            stock_code: String,
            #[serde(default)]
            start: Option<String>,
            #[serde(default)]
            end: Option<String>,
        }
        let req: Input = serde_json::from_value(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::list_financial_snapshots 参数解析失败: {e}。\
                 期望 {{stock_code, start?, end?}}"
            ))
        })?;
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
        let mut q = financial_snapshots::Entity::find()
            .filter(financial_snapshots::Column::StockCode.eq(req.stock_code));
        if let Some(s) = req.start {
            q = q.filter(financial_snapshots::Column::SnapshotDate.gte(s));
        }
        if let Some(e) = req.end {
            q = q.filter(financial_snapshots::Column::SnapshotDate.lte(e));
        }
        let rows =
            q.order_by_asc(financial_snapshots::Column::SnapshotDate).all(&self.db).await.map_err(
                |err| {
                    AxAgentError::Provider(format!(
                        "stock.analysis::list_financial_snapshots 查询失败: {err}"
                    ))
                },
            )?;
        serde_json::to_value(rows).map_err(|e| {
            AxAgentError::Provider(format!(
                "stock.analysis::list_financial_snapshots 序列化失败: {e}"
            ))
        })
    }

    /// 写 op：触发每日快照采集（F1 余三条 #35）。
    ///
    /// **触发口，不是执行体**：`run_daily_snapshot_sweep` 留宿主（后台定时服务
    /// `start_daily_snapshot_sweep` 复用同一实现体），归属日计算与宿主命令共用
    /// `snapshot_sweep_date`（盘中触发绝不把半日数据当收盘值落盘）。入参 `{}`。
    async fn sweep_daily_snapshots(&self, input: Value) -> Result<Value> {
        Self::require_no_params("sweep_daily_snapshots", &input)?;
        let date = snapshot_sweep_date();
        let summary =
            run_daily_snapshot_sweep(&self.astock_client, &self.db, &date).await.map_err(|e| {
                AxAgentError::Provider(format!("stock.analysis::sweep_daily_snapshots 失败: {e}"))
            })?;
        serde_json::to_value(summary).map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::sweep_daily_snapshots 序列化失败: {e}"))
        })
    }

    // ── F6 别名族（#322）：analyst 进化 / 反馈六条宿主命令的取数面 ──────────────
    // 实现权威：analysis-engine 的 `evolve_node` / `get_node_evolution_status` 与泛化形
    // 命令抽出的无 State 核心（`save_node_feedback_core` 等 —— 前端仍直调的泛化形命令
    // 与本层共用同一份实现，不造第二份查询）。别名形状逐字保留（PLAN §174 五处不等价）；
    // 接缝键名 snake_case（同本文件其它 op），IPC 命令面的 camelCase 不受影响。

    /// 写 op：触发分析师节点自我进化（别名：硬编码 `NodeType::Analyst`，不开放成泛化形）。
    async fn evolve_analyst(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        struct Input {
            analyst_id: String,
        }
        let req: Input = Self::parse("evolve_analyst", "{analyst_id}", input)?;
        let status =
            evolve_node(&self.db, &NodeType::Analyst, &req.analyst_id).await.map_err(|e| {
                AxAgentError::Provider(format!("stock.analysis::evolve_analyst 失败: {e}"))
            })?;
        serde_json::to_value(status).map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::evolve_analyst 序列化失败: {e}"))
        })
    }

    /// 读 op：获取分析师节点进化状态（别名，不触发进化）。
    async fn get_analyst_evolution_status(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        struct Input {
            analyst_id: String,
        }
        let req: Input = Self::parse("get_analyst_evolution_status", "{analyst_id}", input)?;
        let status = get_node_evolution_status(&self.db, &NodeType::Analyst, &req.analyst_id)
            .await
            .map_err(|e| {
                AxAgentError::Provider(format!(
                    "stock.analysis::get_analyst_evolution_status 失败: {e}"
                ))
            })?;
        serde_json::to_value(status).map_err(|e| {
            AxAgentError::Provider(format!(
                "stock.analysis::get_analyst_evolution_status 序列化失败: {e}"
            ))
        })
    }

    /// 读 op：获取节点进化状态（**泛化形本体**：收 `node_type` + `node_id` 双键；
    /// 未知 node_type 照宿主 `parse_node_type` 兜 `Other`，不拒参）。
    async fn get_node_evolution_status(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        struct Input {
            node_type: String,
            node_id: String,
        }
        let req: Input = Self::parse("get_node_evolution_status", "{node_type, node_id}", input)?;
        let node_type = parse_node_type(&req.node_type);
        let status =
            get_node_evolution_status(&self.db, &node_type, &req.node_id).await.map_err(|e| {
                AxAgentError::Provider(format!(
                    "stock.analysis::get_node_evolution_status 失败: {e}"
                ))
            })?;
        serde_json::to_value(status).map_err(|e| {
            AxAgentError::Provider(format!(
                "stock.analysis::get_node_evolution_status 序列化失败: {e}"
            ))
        })
    }

    /// 写 op：保存分析师质量反馈（别名：13 字段无 node_type，宿主按 `Analyst` 补；
    /// bull/bear/confidence 三可空字段拼进 `quality_metrics_json`——与被退役的宿主命令
    /// 同一段转换）。null 键照原样透传（读口全按缺键处理，PLAN §175）。
    async fn save_analyst_feedback(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        struct Input {
            analyst_id: String,
            report_id: String,
            stock_code: String,
            execution_id: String,
            quality_score: i32,
            grade: String,
            issue_count: i32,
            warning_count: i32,
            good_count: i32,
            checks_json: String,
            #[serde(default)]
            bull_score: Option<f64>,
            #[serde(default)]
            bear_score: Option<f64>,
            #[serde(default)]
            confidence: Option<f64>,
            score_consistent: bool,
            direction_consistent: bool,
        }
        let req: Input =
            Self::parse("save_analyst_feedback", "{analyst_id, report_id, … 13 字段}", input)?;
        let quality_metrics = serde_json::json!({
            "bull_score": req.bull_score,
            "bear_score": req.bear_score,
            "confidence": req.confidence,
            "score_consistent": req.score_consistent,
            "direction_consistent": req.direction_consistent,
        });
        let id = save_node_feedback_core(
            &self.db,
            SaveNodeFeedbackRequest {
                node_type: "analyst".to_string(),
                node_id: req.analyst_id,
                report_id: req.report_id,
                stock_code: req.stock_code,
                execution_id: req.execution_id,
                quality_score: req.quality_score,
                grade: req.grade,
                issue_count: req.issue_count,
                warning_count: req.warning_count,
                good_count: req.good_count,
                checks_json: req.checks_json,
                quality_metrics_json: quality_metrics.to_string(),
            },
        )
        .await
        .map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::save_analyst_feedback 落库失败: {e}"))
        })?;
        Ok(Value::String(id))
    }

    /// 读 op：获取分析师反馈历史（别名：单键 `analyst_id` 必填，limit/only_issues 可选）。
    async fn get_analyst_feedbacks(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        struct Input {
            analyst_id: String,
            #[serde(default)]
            limit: Option<u64>,
            #[serde(default)]
            only_issues: Option<bool>,
        }
        let req: Input =
            Self::parse("get_analyst_feedbacks", "{analyst_id, limit?, only_issues?}", input)?;
        let rows = get_node_feedbacks_core(
            &self.db,
            GetNodeFeedbacksRequest {
                node_type: Some("analyst".to_string()),
                node_id: Some(req.analyst_id),
                limit: req.limit,
                only_issues: req.only_issues,
            },
        )
        .await
        .map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::get_analyst_feedbacks 查询失败: {e}"))
        })?;
        serde_json::to_value(rows).map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::get_analyst_feedbacks 序列化失败: {e}"))
        })
    }

    /// 读 op：获取分析师反馈统计（别名：平铺单参 `analyst_id`，node_type 固定 analyst）。
    async fn get_analyst_feedback_stats(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        struct Input {
            analyst_id: String,
        }
        let req: Input = Self::parse("get_analyst_feedback_stats", "{analyst_id}", input)?;
        let stats = get_node_feedback_stats_core(&self.db, "analyst".to_string(), req.analyst_id)
            .await
            .map_err(|e| {
                AxAgentError::Provider(format!(
                    "stock.analysis::get_analyst_feedback_stats 查询失败: {e}"
                ))
            })?;
        serde_json::to_value(stats).map_err(|e| {
            AxAgentError::Provider(format!(
                "stock.analysis::get_analyst_feedback_stats 序列化失败: {e}"
            ))
        })
    }

    // ── #323 T2 五命令批迁（子批②③）：token 写入面 + 决策回测读面 ──────────────

    /// 写 op：保存 NeoData token（宿主 `save_neodata_token` 退役后的接缝面）。
    ///
    /// 三段效应**逐字下沉**（与被退役的宿主命令同序同语义）：① 写 Python 脚本缓存；
    /// ② `astock_client.neodata_token` 共享内存立即生效（无需重启）；③ 持久化到
    /// `stock-analysis` 模板变量 `vendor_neodata_token`（替换或新增，`is_secret`）。
    /// 权限闸在分派层（`required_permission_for` 登记为 `Write`），本层不重复校验。
    async fn save_neodata_token(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        struct Input {
            token: String,
        }
        let req: Input = Self::parse("save_neodata_token", "{token}", input)?;
        if req.token.is_empty() {
            return Err(AxAgentError::Validation(
                "stock.analysis::save_neodata_token token 不能为空".into(),
            ));
        }
        // 1) 写入 Python 脚本缓存
        axagent_astock_data::vendors::neodata::save_token_to_cache(&req.token).await.map_err(
            |e| {
                AxAgentError::Provider(format!(
                    "stock.analysis::save_neodata_token 保存 NeoData token 到脚本缓存失败: {e}"
                ))
            },
        )?;
        // 2) 同时写入共享内存（立即生效，无需重启）
        if let Some(ref nd) = self.astock_client.neodata_token {
            *nd.write().await = req.token.clone();
        }
        // 3) 持久化到数据库（设置页下次加载时自动读取）
        use axagent_entities::workflow_template;
        use sea_orm::{ActiveModelTrait, EntityTrait};
        if let Some(t) = workflow_template::Entity::find_by_id("stock-analysis")
            .one(&self.db)
            .await
            .map_err(|e| {
                AxAgentError::Provider(format!(
                    "stock.analysis::save_neodata_token 查询模板失败: {e}"
                ))
            })?
        {
            let mut vars = t
                .variables
                .as_ref()
                .and_then(|s| serde_json::from_str::<Vec<Value>>(s).ok())
                .unwrap_or_default();
            let token_val = serde_json::json!({
                "name": "vendor_neodata_token",
                "is_secret": true,
                "defaultValue": null,
                "value": req.token,
                "type": "string",
            });
            // 替换或新增
            if let Some(pos) = vars.iter().position(|v| {
                v.get("name").and_then(|n| n.as_str()) == Some("vendor_neodata_token")
            }) {
                vars[pos] = token_val;
            } else {
                vars.push(token_val);
            }
            let json_str = serde_json::to_string(&vars).unwrap_or_default();
            let mut am: workflow_template::ActiveModel = t.into();
            am.variables = sea_orm::ActiveValue::Set(Some(json_str));
            am.update(&self.db).await.map_err(|e| {
                AxAgentError::Provider(format!(
                    "stock.analysis::save_neodata_token 持久化 NeoData token 失败: {e}"
                ))
            })?;
        }
        Ok(Value::Null)
    }

    /// 读 op：查询决策回测分析（宿主 `query_decision_backtest` 退役后的接缝面）。
    ///
    /// 投影与过滤**逐字下沉**（与被退役的宿主命令同一段变换，不造第二份）：
    /// `Outcome` / `DecisionAction` 双 `IS NOT NULL` + `AnalysisDate` 降序；
    /// `limit` 缺省 100 钳 500；`llmAction` 取 `llm_decision_json` 的 `action`/`stance`，
    /// `decisionConfidence` / `userTrustDecision` 取 `decision_json` 对应键。出参 =
    /// 手拼投影数组整块透传（形状权威在被退役的宿主命令，harness 不上收 DTO）。
    async fn query_decision_backtest(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct Input {
            limit: Option<u32>,
        }
        let req = serde_json::from_value::<Input>(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::query_decision_backtest 参数解析失败: {e}。期望 {{limit?: number}}"
            ))
        })?;
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
        let records = stock_analyses::Entity::find()
            .filter(stock_analyses::Column::Outcome.is_not_null())
            .filter(stock_analyses::Column::DecisionAction.is_not_null())
            .order_by(stock_analyses::Column::AnalysisDate, sea_orm::Order::Desc)
            .all(&self.db)
            .await
            .map_err(|e| {
                AxAgentError::Provider(format!(
                    "stock.analysis::query_decision_backtest 查询失败: {e}"
                ))
            })?;
        let limit = req.limit.unwrap_or(100).min(500) as usize;
        let mut results: Vec<Value> = Vec::new();
        for r in records.iter().take(limit) {
            let formula_action = r.decision_action.as_deref().unwrap_or("");
            let outcome_str = r.outcome.as_deref().unwrap_or("");
            let llm_action: Option<String> = r
                .llm_decision_json
                .as_ref()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                .and_then(|v| {
                    v.get("action")
                        .or_else(|| v.get("stance"))
                        .and_then(|a| a.as_str().map(String::from))
                });
            results.push(serde_json::json!({
                "stockCode": r.stock_code,
                "stockName": r.stock_name,
                "analysisDate": r.analysis_date,
                "formulaAction": formula_action,
                "llmAction": llm_action,
                "outcome": outcome_str,
                "decisionConfidence": r.decision_json.as_ref()
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                    .and_then(|v| v.get("confidence").and_then(|c| c.as_f64())),
                "userTrustDecision": r.decision_json.as_ref()
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                    .and_then(|v| v.get("userTrustDecision").and_then(|t| t.as_str().map(String::from))),
            }));
        }
        Ok(Value::Array(results))
    }

    // ── #324 T2 F7 回测验证三条迁（§160 分档表 F7）────────────────────────────

    /// 写 op：跑决策回测（宿主 `run_decision_backtest` 退役后的接缝面）。
    ///
    /// 执行体 `run_decision_backtest_inner` **留宿主**（cron 与 CLI
    /// `axagent-validate-backfill` 共用同一实现，接缝只做触发口，照
    /// `sweep_daily_snapshots` 先例）；本层只做入参解析与出参透传。权限闸在分派层
    /// （`required_permission_for` 登记为 `Write`），本层不重复校验。
    async fn run_decision_backtest(&self, input: Value) -> Result<Value> {
        let req: crate::commands::backtest_validation::RunDecisionBacktestRequest =
            serde_json::from_value(input).map_err(|e| {
                AxAgentError::Validation(format!(
                    "stock.analysis::run_decision_backtest 参数解析失败: {e}。期望 \
                     {{tPlusNList?, excludeSynthetic?, maxPicks?, dryRun?, periodFilter?, \
                     includeReplay?}}（全可选，传空对象即全默认）"
                ))
            })?;
        let resp = crate::commands::backtest_validation::run_decision_backtest_inner(
            &self.db,
            &self.astock_client,
            req,
        )
        .await
        .map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::run_decision_backtest 失败: {e}"))
        })?;
        serde_json::to_value(resp).map_err(|e| {
            AxAgentError::Provider(format!(
                "stock.analysis::run_decision_backtest 出参序列化失败: {e}"
            ))
        })
    }

    /// 读 op：列出决策验证记录（宿主 `list_decision_validations` 退役后的接缝面）。
    ///
    /// 实现体保号留在 `commands/backtest_validation.rs`（三个静态门按「文件+fn 名」
    /// 钉住，抽 State 化不改名不搬文件）；本层解析 camelCase 入参后调它并整块透传。
    async fn list_decision_validations(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct Input {
            stock_code: Option<String>,
            hit_outcome: Option<String>,
            limit: Option<u32>,
            offset: Option<u32>,
            generation: Option<String>,
        }
        let req = serde_json::from_value::<Input>(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::list_decision_validations 参数解析失败: {e}。期望 \
                 {{stockCode?, hitOutcome?, limit?, offset?, generation?}}（全可选）"
            ))
        })?;
        let items = crate::commands::backtest_validation::list_decision_validations(
            &self.db,
            req.stock_code,
            req.hit_outcome,
            req.limit,
            req.offset,
            req.generation,
        )
        .await
        .map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::list_decision_validations 失败: {e}"))
        })?;
        serde_json::to_value(items).map_err(|e| {
            AxAgentError::Provider(format!(
                "stock.analysis::list_decision_validations 出参序列化失败: {e}"
            ))
        })
    }

    /// 读 op：计算验证报告（宿主 `compute_validation_report` 退役后的接缝面）。
    ///
    /// 同上：实现体保号留原文件（窗口权威门 R3 / 代际分母门都钉它），本层透传。
    async fn compute_validation_report(&self, input: Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct Input {
            generation: Option<String>,
        }
        let req = serde_json::from_value::<Input>(input).map_err(|e| {
            AxAgentError::Validation(format!(
                "stock.analysis::compute_validation_report 参数解析失败: {e}。期望 \
                 {{generation?}}（可选，缺省 = 实盘出票一代）"
            ))
        })?;
        let resp = crate::commands::backtest_validation::compute_validation_report(
            &self.db,
            req.generation,
        )
        .await
        .map_err(|e| {
            AxAgentError::Provider(format!("stock.analysis::compute_validation_report 失败: {e}"))
        })?;
        serde_json::to_value(resp).map_err(|e| {
            AxAgentError::Provider(format!(
                "stock.analysis::compute_validation_report 出参序列化失败: {e}"
            ))
        })
    }

    // ── G2 模拟组合九条：宿主 `paper_portfolio_*` Tauri 命令退役后的接缝面 ──────
    // 一律调用 `analysis-engine::paper_portfolio` 的同一份权威函数（原宿主命令也只调它），
    // 本层只做 `Value` ↔ DTO 转换（模块头「转换点唯一」判据）。
    /// 写 op：创建模拟组合（入参 = `CreatePortfolioInput`，出参 = 组合行）。
    async fn paper_portfolio_create(&self, input: Value) -> Result<Value> {
        const OP: &str = "paper_portfolio_create";
        let req = Self::parse::<axagent_analysis_engine::paper_portfolio::CreatePortfolioInput>(
            OP,
            "{name, sourceEvent, sourceNewsId?, sourceScreenshotDiagnosisId?}",
            input,
        )?;
        let model = axagent_analysis_engine::paper_portfolio::create_portfolio(&self.db, req)
            .await
            .map_err(|e| AxAgentError::Provider(format!("stock.analysis::{OP} 失败: {e}")))?;
        Self::to_value(OP, &model)
    }

    /// 读 op：列出组合（入参 `{status?}`，缺省 = 全部）。
    async fn paper_portfolio_list(&self, input: Value) -> Result<Value> {
        const OP: &str = "paper_portfolio_list";
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct ListRequest {
            status: Option<String>,
        }
        let req = Self::parse::<ListRequest>(OP, "{status?}", input)?;
        let rows = axagent_analysis_engine::paper_portfolio::list_portfolios(
            &self.db,
            req.status.as_deref(),
        )
        .await
        .map_err(|e| AxAgentError::Provider(format!("stock.analysis::{OP} 失败: {e}")))?;
        Self::to_value(OP, &rows)
    }

    /// 读 op：组合详情（入参 `{portfolioId}`，含持仓 + 实时盈亏）。
    async fn paper_portfolio_get(&self, input: Value) -> Result<Value> {
        const OP: &str = "paper_portfolio_get";
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct IdRequest {
            portfolio_id: String,
        }
        let req = Self::parse::<IdRequest>(OP, "{portfolioId}", input)?;
        let detail = axagent_analysis_engine::paper_portfolio::get_portfolio_detail(
            &self.db,
            &*self.astock_client,
            &req.portfolio_id,
        )
        .await
        .map_err(|e| AxAgentError::Provider(format!("stock.analysis::{OP} 失败: {e}")))?;
        Self::to_value(OP, &detail)
    }

    /// 写 op：关闭组合（入参 `{portfolioId}`，status=closed + closed_at）。
    async fn paper_portfolio_close(&self, input: Value) -> Result<Value> {
        const OP: &str = "paper_portfolio_close";
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct IdRequest {
            portfolio_id: String,
        }
        let req = Self::parse::<IdRequest>(OP, "{portfolioId}", input)?;
        let model =
            axagent_analysis_engine::paper_portfolio::close_portfolio(&self.db, &req.portfolio_id)
                .await
                .map_err(|e| AxAgentError::Provider(format!("stock.analysis::{OP} 失败: {e}")))?;
        Self::to_value(OP, &model)
    }

    /// 写 op：归档组合（入参 `{portfolioId}`，status=archived，不写 closed_at）。
    async fn paper_portfolio_archive(&self, input: Value) -> Result<Value> {
        const OP: &str = "paper_portfolio_archive";
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct IdRequest {
            portfolio_id: String,
        }
        let req = Self::parse::<IdRequest>(OP, "{portfolioId}", input)?;
        let model = axagent_analysis_engine::paper_portfolio::archive_portfolio(
            &self.db,
            &req.portfolio_id,
        )
        .await
        .map_err(|e| AxAgentError::Provider(format!("stock.analysis::{OP} 失败: {e}")))?;
        Self::to_value(OP, &model)
    }

    /// 写 op：添加虚拟持仓（入参 = `AddPositionInput`，出参 = 持仓行）。
    async fn paper_portfolio_add_position(&self, input: Value) -> Result<Value> {
        const OP: &str = "paper_portfolio_add_position";
        let req = Self::parse::<axagent_analysis_engine::paper_portfolio::AddPositionInput>(
            OP,
            "{portfolioId, symbol, market?, entryPrice, entryDate, quantity, note?}",
            input,
        )?;
        let model = axagent_analysis_engine::paper_portfolio::add_position(&self.db, req)
            .await
            .map_err(|e| AxAgentError::Provider(format!("stock.analysis::{OP} 失败: {e}")))?;
        Self::to_value(OP, &model)
    }

    /// 写 op：平仓单只持仓（入参 = `ClosePositionInput`，出参 = 持仓行）。
    async fn paper_portfolio_close_position(&self, input: Value) -> Result<Value> {
        const OP: &str = "paper_portfolio_close_position";
        let req = Self::parse::<axagent_analysis_engine::paper_portfolio::ClosePositionInput>(
            OP,
            "{positionId, exitPrice, exitDate}",
            input,
        )?;
        let model =
            axagent_analysis_engine::paper_portfolio::close_position(&self.db, req)
                .await
                .map_err(|e| AxAgentError::Provider(format!("stock.analysis::{OP} 失败: {e}")))?;
        Self::to_value(OP, &model)
    }

    /// 写 op：批量平仓（入参 `{portfolioId, exitPrice, exitDate}`，出参 = 受影响行数）。
    async fn paper_portfolio_close_all_positions(&self, input: Value) -> Result<Value> {
        const OP: &str = "paper_portfolio_close_all_positions";
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct CloseAllRequest {
            portfolio_id: String,
            exit_price: f64,
            exit_date: String,
        }
        let req = Self::parse::<CloseAllRequest>(OP, "{portfolioId, exitPrice, exitDate}", input)?;
        let affected = axagent_analysis_engine::paper_portfolio::close_all_positions(
            &self.db,
            &req.portfolio_id,
            req.exit_price,
            &req.exit_date,
        )
        .await
        .map_err(|e| AxAgentError::Provider(format!("stock.analysis::{OP} 失败: {e}")))?;
        Self::to_value(OP, &affected)
    }

    /// 读 op：active 组合详情（无参，Dashboard 用，逐票拉最新价）。
    async fn paper_portfolio_list_active_details(&self, input: Value) -> Result<Value> {
        const OP: &str = "paper_portfolio_list_active_details";
        Self::require_no_params(OP, &input)?;
        let rows = axagent_analysis_engine::paper_portfolio::list_active_portfolios_detail(
            &self.db,
            &*self.astock_client,
        )
        .await
        .map_err(|e| AxAgentError::Provider(format!("stock.analysis::{OP} 失败: {e}")))?;
        Self::to_value(OP, &rows)
    }

    // ── G6 截图诊断八条：退役的 `screenshot_diagnosis_*` Tauri 命令 ──────────────
    // 业务核心在 `commands/screenshot_diagnosis.rs`（无 State），LLM 上下文用本结构同源的
    // `db` + `master_key` 重建；仍复用 analysis-engine 的同一份 CRUD / 风险诊断实现。

    /// 写 op：上传截图 → OCR + 结构化 → 风险诊断 → 持久化（base64 走插件帧透传）。
    #[cfg(not(mobile))]
    async fn screenshot_diagnosis_create_from_image(&self, input: Value) -> Result<Value> {
        const OP: &str = "screenshot_diagnosis_create_from_image";
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct CreateFromImageRequest {
            image_base64: String,
            source_app: Option<String>,
            provider_id: String,
            model_id: String,
        }
        let req = Self::parse::<CreateFromImageRequest>(
            OP,
            "{imageBase64, sourceApp?, providerId, modelId}",
            input,
        )?;
        let model =
            crate::commands::screenshot_diagnosis::screenshot_diagnosis_create_from_image_core(
                &self.db,
                &self.master_key,
                req.image_base64,
                req.source_app,
                req.provider_id,
                req.model_id,
            )
            .await
            .map_err(|e| AxAgentError::Provider(format!("stock.analysis::{OP} 失败: {e}")))?;
        Self::to_value(OP, &model)
    }

    /// 写 op：直接传 positions 创建诊断（前端预填入口，不调 LLM）。
    async fn screenshot_diagnosis_create(&self, input: Value) -> Result<Value> {
        const OP: &str = "screenshot_diagnosis_create";
        let req = Self::parse::<
            axagent_analysis_engine::screenshot_diagnosis::CreateScreenshotDiagnosisInput,
        >(OP, "CreateScreenshotDiagnosisInput", input)?;
        let model =
            crate::commands::screenshot_diagnosis::screenshot_diagnosis_create_core(&self.db, req)
                .await
                .map_err(|e| AxAgentError::Provider(format!("stock.analysis::{OP} 失败: {e}")))?;
        Self::to_value(OP, &model)
    }

    /// 读 op：按 ID 获取诊断。
    async fn screenshot_diagnosis_get(&self, input: Value) -> Result<Value> {
        const OP: &str = "screenshot_diagnosis_get";
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct DiagnosisIdRequest {
            diagnosis_id: String,
        }
        let req = Self::parse::<DiagnosisIdRequest>(OP, "{diagnosisId}", input)?;
        let model = crate::commands::screenshot_diagnosis::screenshot_diagnosis_get_core(
            &self.db,
            req.diagnosis_id,
        )
        .await
        .map_err(|e| AxAgentError::Provider(format!("stock.analysis::{OP} 失败: {e}")))?;
        Self::to_value(OP, &model)
    }

    /// 读 op：最近 N 条诊断（缺省 20）。
    async fn screenshot_diagnosis_list_recent(&self, input: Value) -> Result<Value> {
        const OP: &str = "screenshot_diagnosis_list_recent";
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ListRecentRequest {
            limit: Option<usize>,
        }
        let req = Self::parse::<ListRecentRequest>(OP, "{limit?}", input)?;
        let rows = crate::commands::screenshot_diagnosis::screenshot_diagnosis_list_recent_core(
            &self.db, req.limit,
        )
        .await
        .map_err(|e| AxAgentError::Provider(format!("stock.analysis::{OP} 失败: {e}")))?;
        Self::to_value(OP, &rows)
    }

    /// 读 op：按状态过滤诊断。
    async fn screenshot_diagnosis_list_by_status(&self, input: Value) -> Result<Value> {
        const OP: &str = "screenshot_diagnosis_list_by_status";
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ListByStatusRequest {
            status: String,
        }
        let req = Self::parse::<ListByStatusRequest>(OP, "{status}", input)?;
        let rows = crate::commands::screenshot_diagnosis::screenshot_diagnosis_list_by_status_core(
            &self.db, req.status,
        )
        .await
        .map_err(|e| AxAgentError::Provider(format!("stock.analysis::{OP} 失败: {e}")))?;
        Self::to_value(OP, &rows)
    }

    /// 写 op：归档诊断。
    async fn screenshot_diagnosis_archive(&self, input: Value) -> Result<Value> {
        const OP: &str = "screenshot_diagnosis_archive";
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct DiagnosisIdRequest {
            diagnosis_id: String,
        }
        let req = Self::parse::<DiagnosisIdRequest>(OP, "{diagnosisId}", input)?;
        let model = crate::commands::screenshot_diagnosis::screenshot_diagnosis_archive_core(
            &self.db,
            req.diagnosis_id,
        )
        .await
        .map_err(|e| AxAgentError::Provider(format!("stock.analysis::{OP} 失败: {e}")))?;
        Self::to_value(OP, &model)
    }

    /// 写 op：更新诊断字段（narrative / recommendedActions / status / errorMessage）。
    async fn screenshot_diagnosis_update(&self, input: Value) -> Result<Value> {
        const OP: &str = "screenshot_diagnosis_update";
        let req = Self::parse::<
            axagent_analysis_engine::screenshot_diagnosis::UpdateScreenshotDiagnosisInput,
        >(OP, "UpdateScreenshotDiagnosisInput", input)?;
        let model =
            crate::commands::screenshot_diagnosis::screenshot_diagnosis_update_core(&self.db, req)
                .await
                .map_err(|e| AxAgentError::Provider(format!("stock.analysis::{OP} 失败: {e}")))?;
        Self::to_value(OP, &model)
    }

    /// 写 op：诊断一键转模拟观察组合。
    async fn screenshot_diagnosis_to_paper_portfolio(&self, input: Value) -> Result<Value> {
        const OP: &str = "screenshot_diagnosis_to_paper_portfolio";
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ToPortfolioRequest {
            diagnosis_id: String,
            name: String,
            source_event: String,
        }
        let req = Self::parse::<ToPortfolioRequest>(OP, "{diagnosisId, name, sourceEvent}", input)?;
        let model =
            crate::commands::screenshot_diagnosis::screenshot_diagnosis_to_paper_portfolio_core(
                &self.db,
                req.diagnosis_id,
                req.name,
                req.source_event,
            )
            .await
            .map_err(|e| AxAgentError::Provider(format!("stock.analysis::{OP} 失败: {e}")))?;
        Self::to_value(OP, &model)
    }
}
