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
use axagent_analysis_engine::industry_constituents::{
    IndustryConstituentsInput, fetch_industry_constituents,
};
use axagent_analysis_engine::market_mainline::{BatchUpsertInput, batch_upsert_mainlines};
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
use crate::commands::stock_pipeline::core::{pipeline_history_rows, pipeline_run_detail_row};
use crate::commands::wf_des::{WfDesRequest, wf_des_integration};
use axagent_dao::repo::stock_evolution_history as evo_hist;
use axagent_harness::DatabaseConnection;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::stock_analysis::StockAnalysisProvider;
use axagent_runtime_core::CronJobStore;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

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
}

impl HostStockAnalysis {
    pub fn new(db: DatabaseConnection, cron_job_store: Arc<CronJobStore>) -> Self {
        Self { db, cron_job_store }
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
}
