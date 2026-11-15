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
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

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
}

impl HostStockAnalysis {
    pub fn new(
        db: DatabaseConnection,
        cron_job_store: Arc<CronJobStore>,
        astock_client: Arc<AStockClient>,
    ) -> Self {
        Self { db, cron_job_store, astock_client }
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
}
