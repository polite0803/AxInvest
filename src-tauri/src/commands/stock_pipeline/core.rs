//! 股票管道核心编排逻辑
//!
//! 编排流程：
//! 编排由 seed_stock_pipeline.rs 的工作流模板经 WorkEngine 执行（run_stock_pipeline_inner
//! 加载模板），旧的 Rust 手写编排层（discover_candidates / analyze_stocks_batch /
//! build_summary）已于 2026-09-03 删除（被模板机制取代，见 output/backup-2026-09-03）。
//!
//! 反思阶段由现有 6h cron 接力（hindsight_date = analysis_date + expected_holding_days）。

#![allow(clippy::type_complexity)]

use axagent_agent_macro::agent_command;
use std::sync::Arc;

use axagent_entities::stock_pipeline_runs;
use axagent_harness::workflow_types::{Variable, WorkflowEdge, WorkflowNode};
use axagent_rt_workflow::work_engine::{ProgressCallback, RunOptions, StepProgressEvent};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, State};

use crate::AppState;
use crate::commands::error::ErrorResponse;
use crate::commands::error_code::stock_workflow as wf_err;

/// 管道执行结果
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineResult {
    pub run_id: String,
    pub run_date: String,
    pub status: String,
    pub candidates: Vec<String>,
    pub new_analyses: Vec<AnalysisSummary>,
    pub reassessed: Vec<AnalysisSummary>,
    pub summary: serde_json::Value,
    pub error: Option<String>,
}

/// 单只股票分析摘要
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisSummary {
    pub stock_code: String,
    pub stock_name: String,
    pub status: String,
    pub analysis_id: Option<String>,
    pub action: Option<String>,
    pub confidence: Option<f64>,
    pub error: Option<String>,
}

/// 管道配置参数
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// 候选股最大数量
    pub max_candidates: usize,
    /// 新候选股分析并发数
    pub new_analysis_concurrency: usize,
    /// 持仓再评估并发数
    pub holdings_reassess_concurrency: usize,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self { max_candidates: 5, new_analysis_concurrency: 2, holdings_reassess_concurrency: 2 }
    }
}

impl PipelineConfig {
    /// P3-D12: 根据 vendor 健康度动态调节并发数。
    ///
    /// 算法（基于 `astock-data` 的 VendorHealthTracker 状态）：
    /// - healthy_ratio = healthy_count / (healthy + degraded)（Disabled 不计入分母）
    /// - healthy_ratio < 0.3（数据源大面积降级）→ 降到 1（保命模式）
    /// - healthy_ratio < 0.6（部分降级）→ 降到 `max(1, base / 2)`
    /// - 否则 → 保持 `base`
    ///
    /// 这样在批量股票分析时：
    /// - 上游数据源健康 → 保持配置并发（默认 2）
    /// - 部分降级 → 自动降速避免雪崩（如 200 只股票 × 8 并发 × 全部 vendor 重试）
    /// - 大面积降级 → 强制串行（避免拖垮上游）
    ///
    /// 返回 `(actual_new_concurrency, actual_reassess_concurrency, healthy_ratio)`。
    /// 调用方可记录日志反映调节情况。
    ///
    /// 当前唯一调用方在 mod tests（8 场景测试套件）；生产管线走 WorkEngine 模板执行，
    /// 待批量分析接入 vendor 健康度自适应后移除豁免。
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn resolve_concurrency_with_vendor_health(
        &self,
        health_snapshot: &[axagent_astock_data::vendor_health::VendorHealth],
    ) -> (usize, usize, f64) {
        use axagent_astock_data::vendor_health::VendorStatus;

        // 过滤 Disabled（手动禁用的 vendor 不参与健康度计算）
        let active: Vec<_> =
            health_snapshot.iter().filter(|h| h.status != VendorStatus::Disabled).collect();
        if active.is_empty() {
            // 无 vendor 健康数据（首次运行 / 未探测）→ 保持配置值，不阻断业务
            return (self.new_analysis_concurrency, self.holdings_reassess_concurrency, 1.0);
        }
        let healthy_count = active.iter().filter(|h| h.status == VendorStatus::Healthy).count();
        let healthy_ratio = healthy_count as f64 / active.len() as f64;

        let (new_c, reassess_c) = if healthy_ratio < 0.3 {
            (1usize, 1usize)
        } else if healthy_ratio < 0.6 {
            (
                (self.new_analysis_concurrency / 2).max(1),
                (self.holdings_reassess_concurrency / 2).max(1),
            )
        } else {
            (self.new_analysis_concurrency, self.holdings_reassess_concurrency)
        };

        (new_c, reassess_c, healthy_ratio)
    }
}

/// 管道执行内部函数（供 Tauri 命令和 cron 调用）
///
/// 使用 WorkEngine 加载工作流模板并执行，与股票分析工作流保持一致。
/// 进度通过可选的回调推送。
pub async fn run_stock_pipeline_inner(
    db: &sea_orm::DatabaseConnection,
    client: &Arc<axagent_astock_data::AStockClient>,
    engine: &Arc<axagent_rt_workflow::work_engine::WorkEngine>,
    config: &PipelineConfig,
    as_of_date: Option<&str>,
    progress_callback: Option<Arc<dyn Fn(&str, &str) + Send + Sync>>,
) -> Result<PipelineResult, String> {
    let run_id = uuid::Uuid::new_v4().to_string();
    let run_date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let started_at = chrono::Utc::now().timestamp_millis();

    // 创建 pipeline_run 记录（status=running）
    let _ = stock_pipeline_runs::ActiveModel {
        id: Set(run_id.clone()),
        run_date: Set(run_date.clone()),
        as_of_date: Set(as_of_date.map(String::from)),
        status: Set("running".to_string()),
        candidates_json: Set(None),
        new_analyses_json: Set(None),
        reassessed_json: Set(None),
        summary_json: Set(None),
        error_message: Set(None),
        started_at: Set(started_at),
        completed_at: Set(None),
        created_at: Set(started_at),
    }
    .insert(db)
    .await;

    // 加载工作流模板
    let loaded = load_pipeline_template(db).await?;

    // 构建进度回调
    let progress_cb: ProgressCallback = Arc::new(move |event: StepProgressEvent| {
        if let Some(cb) = progress_callback.as_ref() {
            let step = match event.status.as_str() {
                "running" => format!("{}: 执行中", event.node_id),
                "completed" => format!("{}: 完成", event.node_id),
                s if s == "failed" || s == "timeout" => format!("{}: {}", event.node_id, s),
                _ => event.node_id.clone(),
            };
            cb("pipeline_step", &step);
        }
        Box::pin(async move {})
    });

    // 注入变量
    let variables = build_pipeline_variables(config, as_of_date, &run_date, &run_id);

    // 创建工作流
    let wf_name = format!("stock-pipeline-{run_id}");
    let workflow =
        engine.create_workflow(&wf_name, loaded.nodes, loaded.edges).await.map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("创建工作流失败: {e}"))
        })?;
    let wf_id = workflow.id.clone();

    // 组装运行选项
    let mut opts = RunOptions::default().with_progress_callback(progress_cb);
    opts.input = Some(json!({
        "max_candidates": config.max_candidates,
        "new_analysis_concurrency": config.new_analysis_concurrency,
        "holdings_reassess_concurrency": config.holdings_reassess_concurrency,
    }));
    opts.variables = Some(variables);
    opts.input_schema = loaded.input_schema;
    opts.output_schema = loaded.output_schema;

    // 执行工作流
    let result = engine.run_workflow(&wf_id, opts).await;

    // 更新 pipeline_run 记录（成功/失败都更新）
    let completed_at = chrono::Utc::now().timestamp_millis();
    let pipeline_result = match result {
        Ok(ref wf) if wf.status == axagent_rt_workflow::workflow_engine::WorkflowStatus::Failed => {
            let _ = stock_pipeline_runs::Entity::update_many()
                .col_expr(
                    stock_pipeline_runs::Column::Status,
                    sea_orm::sea_query::Expr::value("failed"),
                )
                .col_expr(
                    stock_pipeline_runs::Column::ErrorMessage,
                    sea_orm::sea_query::Expr::value("工作流执行失败"),
                )
                .col_expr(
                    stock_pipeline_runs::Column::CompletedAt,
                    sea_orm::sea_query::Expr::value(completed_at),
                )
                .filter(stock_pipeline_runs::Column::Id.eq(run_id.clone()))
                .exec(db)
                .await;
            Err("工作流执行失败".to_string())
        },
        Ok(_wf) => {
            // 从工作流结果构建 PipelineResult
            let pr = build_pipeline_result_from_workflow(&_wf, &run_id, &run_date);
            update_pipeline_run_success(db, &run_id, &pr, completed_at).await;
            Ok(pr)
        },
        Err(e) => {
            let _ = stock_pipeline_runs::Entity::update_many()
                .col_expr(
                    stock_pipeline_runs::Column::Status,
                    sea_orm::sea_query::Expr::value("failed"),
                )
                .col_expr(
                    stock_pipeline_runs::Column::ErrorMessage,
                    sea_orm::sea_query::Expr::value(e.to_string()),
                )
                .col_expr(
                    stock_pipeline_runs::Column::CompletedAt,
                    sea_orm::sea_query::Expr::value(completed_at),
                )
                .filter(stock_pipeline_runs::Column::Id.eq(run_id.clone()))
                .exec(db)
                .await;
            Err(e.to_string())
        },
    };

    let _ = client; // client 保留供未来扩展
    pipeline_result
}

/// 加载股票管道工作流模板
async fn load_pipeline_template(
    db: &sea_orm::DatabaseConnection,
) -> Result<crate::commands::stock_workflow::decision::LoadedTemplate, String> {
    use axagent_entities::workflow_template;

    let template = workflow_template::Entity::find_by_id("stock-pipeline")
        .one(db)
        .await
        .map_err(|e| format!("查询工作流模板失败: {e}"))?
        .ok_or("工作流模板 stock-pipeline 未种子化，请重启应用")?;

    let nodes: Vec<WorkflowNode> =
        serde_json::from_str(&template.nodes).map_err(|e| format!("解析模板节点失败: {e}"))?;
    let edges: Vec<WorkflowEdge> =
        serde_json::from_str(&template.edges).map_err(|e| format!("解析模板边失败: {e}"))?;

    let input_schema = template.input_schema.as_ref().and_then(|s| serde_json::from_str(s).ok());
    let output_schema = template.output_schema.as_ref().and_then(|s| serde_json::from_str(s).ok());
    let variables = template.variables.as_ref().and_then(|v| serde_json::from_str(v).ok());

    Ok(crate::commands::stock_workflow::decision::LoadedTemplate {
        nodes,
        edges,
        input_schema,
        output_schema,
        variables,
    })
}

/// 构建管道工作流变量
fn build_pipeline_variables(
    config: &PipelineConfig,
    as_of_date: Option<&str>,
    run_date: &str,
    run_id: &str,
) -> Vec<Variable> {
    vec![
        Variable {
            name: "max_candidates".into(),
            var_type: "integer".into(),
            value: serde_json::json!(config.max_candidates),
            description: Some("候选股最大数量".into()),
            is_secret: false,
        },
        Variable {
            name: "new_analysis_concurrency".into(),
            var_type: "integer".into(),
            value: serde_json::json!(config.new_analysis_concurrency),
            description: Some("新候选股分析并发数".into()),
            is_secret: false,
        },
        Variable {
            name: "holdings_reassess_concurrency".into(),
            var_type: "integer".into(),
            value: serde_json::json!(config.holdings_reassess_concurrency),
            description: Some("持仓再评估并发数".into()),
            is_secret: false,
        },
        Variable {
            name: "as_of_date".into(),
            var_type: "string".into(),
            value: as_of_date
                .map(|d| serde_json::Value::String(d.to_string()))
                .unwrap_or(serde_json::Value::Null),
            description: Some("指定分析日期".into()),
            is_secret: false,
        },
        Variable {
            name: "run_date".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(run_date.to_string()),
            description: Some("管道执行日期".into()),
            is_secret: false,
        },
        Variable {
            name: "run_id".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(run_id.to_string()),
            description: Some("管道运行ID".into()),
            is_secret: false,
        },
    ]
}

/// 从工作流结果构建 PipelineResult
fn build_pipeline_result_from_workflow(
    wf: &axagent_rt_workflow::workflow_engine::Workflow,
    run_id: &str,
    run_date: &str,
) -> PipelineResult {
    // 从工作流结果中提取数据
    let candidates = extract_var_from_results(&wf.results, "candidates")
        .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
        .unwrap_or_default();

    let new_analyses = extract_var_from_results(&wf.results, "new_analyses")
        .and_then(|v| serde_json::from_value::<Vec<AnalysisSummary>>(v).ok())
        .unwrap_or_default();

    let reassessed = extract_var_from_results(&wf.results, "reassessed")
        .and_then(|v| serde_json::from_value::<Vec<AnalysisSummary>>(v).ok())
        .unwrap_or_default();

    let summary = extract_var_from_results(&wf.results, "summary").unwrap_or_else(|| json!({}));

    PipelineResult {
        run_id: run_id.to_string(),
        run_date: run_date.to_string(),
        status: "completed".to_string(),
        candidates,
        new_analyses,
        reassessed,
        summary,
        error: None,
    }
}

/// 从工作流 results 中提取变量值
fn extract_var_from_results(
    results: &std::collections::HashMap<String, serde_json::Value>,
    var_name: &str,
) -> Option<serde_json::Value> {
    // 尝试从 results 中查找变量
    for (key, value) in results {
        if key.contains(var_name) {
            return Some(value.clone());
        }
    }
    None
}

/// 更新 pipeline_run 为成功状态
async fn update_pipeline_run_success(
    db: &sea_orm::DatabaseConnection,
    run_id: &str,
    pr: &PipelineResult,
    completed_at: i64,
) {
    let _ = stock_pipeline_runs::Entity::update_many()
        .col_expr(stock_pipeline_runs::Column::Status, sea_orm::sea_query::Expr::value("completed"))
        .col_expr(
            stock_pipeline_runs::Column::CandidatesJson,
            sea_orm::sea_query::Expr::value(
                serde_json::to_string(&pr.candidates).unwrap_or_default(),
            ),
        )
        .col_expr(
            stock_pipeline_runs::Column::NewAnalysesJson,
            sea_orm::sea_query::Expr::value(
                serde_json::to_string(&pr.new_analyses).unwrap_or_default(),
            ),
        )
        .col_expr(
            stock_pipeline_runs::Column::ReassessedJson,
            sea_orm::sea_query::Expr::value(
                serde_json::to_string(&pr.reassessed).unwrap_or_default(),
            ),
        )
        .col_expr(
            stock_pipeline_runs::Column::SummaryJson,
            sea_orm::sea_query::Expr::value(serde_json::to_string(&pr.summary).unwrap_or_default()),
        )
        .col_expr(
            stock_pipeline_runs::Column::CompletedAt,
            sea_orm::sea_query::Expr::value(completed_at),
        )
        .filter(stock_pipeline_runs::Column::Id.eq(run_id.to_string()))
        .exec(db)
        .await;
}

// ── Tauri 命令 ──

/// 手动触发股票管道
#[agent_command(domain = invest, safety = Caution, call_mode = StateInput, description = "手动触发股票管道")]
#[tauri::command]
pub async fn run_stock_pipeline(
    app: AppHandle,
    state: State<'_, AppState>,
    as_of_date: Option<String>,
) -> Result<PipelineResult, String> {
    let db = state.harness.db().clone();
    let client = state.astock_client.clone();
    let engine = state.work_engine.clone();
    let config = PipelineConfig::default();

    let app_handle = app.clone();
    let progress_callback = Arc::new(move |step: &str, detail: &str| {
        let _ = app_handle.emit(
            "pipeline-step",
            json!({
                "step": step,
                "detail": detail,
                "timestamp": chrono::Utc::now().timestamp_millis()
            }),
        );
    });

    run_stock_pipeline_inner(
        &db,
        &client,
        &engine,
        &config,
        as_of_date.as_deref(),
        Some(progress_callback),
    )
    .await
}

/// 查询管道执行历史
#[agent_command(domain = invest, safety = Safe, call_mode = StateInput, description = "查询管道执行历史")]
#[tauri::command]
pub async fn get_pipeline_history(
    state: State<'_, AppState>,
    limit: Option<u64>,
) -> Result<Vec<serde_json::Value>, String> {
    let db = state.harness.db();
    let limit = limit.unwrap_or(20);

    let runs = stock_pipeline_runs::Entity::find()
        .order_by_desc(stock_pipeline_runs::Column::CreatedAt)
        .limit(limit)
        .all(db)
        .await
        .map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("查询管道运行记录失败: {e}"))
        })?;

    Ok(runs
        .into_iter()
        .map(|r| {
            json!({
                "id": r.id,
                "runDate": r.run_date,
                "asOfDate": r.as_of_date,
                "status": r.status,
                "startedAt": r.started_at,
                "completedAt": r.completed_at,
                "errorMessage": r.error_message,
                "summary": r.summary_json.and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()),
            })
        })
        .collect())
}

/// 查询单次管道执行详情
#[agent_command(domain = invest, safety = Safe, call_mode = StateInput, description = "查询单次管道执行详情")]
#[tauri::command]
pub async fn get_pipeline_run_detail(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<serde_json::Value, String> {
    let db = state.harness.db();

    let run = stock_pipeline_runs::Entity::find_by_id(&run_id)
        .one(db)
        .await
        .map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("查询管道运行记录失败: {e}"))
        })?
        .ok_or_else(|| {
            ErrorResponse::new(wf_err::INTERNAL)
                .with_detail(format!("管道运行记录不存在: {run_id}"))
                .to_string()
        })?;

    Ok(json!({
        "id": run.id,
        "runDate": run.run_date,
        "asOfDate": run.as_of_date,
        "status": run.status,
        "startedAt": run.started_at,
        "completedAt": run.completed_at,
        "errorMessage": run.error_message,
        "candidates": run.candidates_json.and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()),
        "newAnalyses": run.new_analyses_json.and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()),
        "reassessed": run.reassessed_json.and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()),
        "summary": run.summary_json.and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_astock_data::vendor_health::{VendorHealth, VendorStatus};

    fn make_health(name: &str, status: VendorStatus) -> VendorHealth {
        let mut h = VendorHealth::new(name);
        h.status = status;
        h
    }

    fn default_config() -> PipelineConfig {
        PipelineConfig {
            max_candidates: 5,
            new_analysis_concurrency: 4,
            holdings_reassess_concurrency: 4,
        }
    }

    #[test]
    fn resolve_concurrency_all_healthy_keeps_config() {
        let cfg = default_config();
        let health = vec![
            make_health("tencent", VendorStatus::Healthy),
            make_health("eastmoney", VendorStatus::Healthy),
            make_health("sina", VendorStatus::Healthy),
        ];
        let (new_c, reassess_c, ratio) = cfg.resolve_concurrency_with_vendor_health(&health);
        assert_eq!(new_c, 4);
        assert_eq!(reassess_c, 4);
        assert!((ratio - 1.0).abs() < 1e-10);
    }

    #[test]
    fn resolve_concurrency_partial_degradation_halves_concurrency() {
        // 5 个 active vendor, 2 个 Degraded → ratio = 3/5 = 0.6
        // 0.6 不 < 0.6 → 保持配置值
        let cfg = default_config();
        let health = vec![
            make_health("tencent", VendorStatus::Healthy),
            make_health("eastmoney", VendorStatus::Healthy),
            make_health("sina", VendorStatus::Healthy),
            make_health("ths", VendorStatus::Degraded),
            make_health("cninfo", VendorStatus::Degraded),
        ];
        let (new_c, reassess_c, ratio) = cfg.resolve_concurrency_with_vendor_health(&health);
        assert_eq!(new_c, 4, "ratio=0.6 应保持配置值");
        assert_eq!(reassess_c, 4);
        assert!((ratio - 0.6).abs() < 1e-10);
    }

    #[test]
    fn resolve_concurrency_below_60_pct_halves_concurrency() {
        // 4 个 active vendor, 2 个 Degraded → ratio = 2/4 = 0.5 < 0.6
        // → max(1, 4/2) = 2
        let cfg = default_config();
        let health = vec![
            make_health("tencent", VendorStatus::Healthy),
            make_health("eastmoney", VendorStatus::Healthy),
            make_health("sina", VendorStatus::Degraded),
            make_health("ths", VendorStatus::Degraded),
        ];
        let (new_c, reassess_c, ratio) = cfg.resolve_concurrency_with_vendor_health(&health);
        assert_eq!(new_c, 2, "ratio=0.5 < 0.6 应降到 base/2");
        assert_eq!(reassess_c, 2);
        assert!((ratio - 0.5).abs() < 1e-10);
    }

    #[test]
    fn resolve_concurrency_severe_degradation_forces_serial() {
        // 5 个 active, 4 个 Degraded → ratio = 1/5 = 0.2 < 0.3 → 降到 1
        let cfg = default_config();
        let health = vec![
            make_health("tencent", VendorStatus::Healthy),
            make_health("eastmoney", VendorStatus::Degraded),
            make_health("sina", VendorStatus::Degraded),
            make_health("ths", VendorStatus::Degraded),
            make_health("cninfo", VendorStatus::Degraded),
        ];
        let (new_c, reassess_c, ratio) = cfg.resolve_concurrency_with_vendor_health(&health);
        assert_eq!(new_c, 1, "ratio<0.3 应强制串行");
        assert_eq!(reassess_c, 1);
        assert!(ratio < 0.3);
    }

    #[test]
    fn resolve_concurrency_excludes_disabled_vendors() {
        // 5 个 vendor: 2 Healthy + 1 Degraded + 2 Disabled
        // active = 3 (Healthy+Degraded), ratio = 2/3 ≈ 0.667 → 保持配置
        let cfg = default_config();
        let health = vec![
            make_health("tencent", VendorStatus::Healthy),
            make_health("eastmoney", VendorStatus::Healthy),
            make_health("sina", VendorStatus::Degraded),
            make_health("ths", VendorStatus::Disabled),
            make_health("cninfo", VendorStatus::Disabled),
        ];
        let (new_c, reassess_c, ratio) = cfg.resolve_concurrency_with_vendor_health(&health);
        assert_eq!(new_c, 4, "Disabled 不计入分母, ratio=2/3>0.6 应保持");
        assert_eq!(reassess_c, 4);
        assert!((ratio - 2.0 / 3.0).abs() < 1e-10);
    }

    #[test]
    fn resolve_concurrency_empty_health_keeps_config() {
        // 无 vendor 健康数据（首次运行）→ 保持配置值
        let cfg = default_config();
        let (new_c, reassess_c, ratio) = cfg.resolve_concurrency_with_vendor_health(&[]);
        assert_eq!(new_c, 4);
        assert_eq!(reassess_c, 4);
        assert!((ratio - 1.0).abs() < 1e-10);
    }

    #[test]
    fn resolve_concurrency_all_disabled_keeps_config() {
        // 全部 Disabled（极端场景）→ active 为空 → 保持配置值
        let cfg = default_config();
        let health = vec![
            make_health("tencent", VendorStatus::Disabled),
            make_health("eastmoney", VendorStatus::Disabled),
        ];
        let (new_c, reassess_c, ratio) = cfg.resolve_concurrency_with_vendor_health(&health);
        assert_eq!(new_c, 4);
        assert_eq!(reassess_c, 4);
        assert!((ratio - 1.0).abs() < 1e-10);
    }

    #[test]
    fn resolve_concurrency_min_one_floor() {
        // base=1, 部分降级 → max(1, 1/2) = 1（不应降到 0）
        let cfg = PipelineConfig {
            max_candidates: 5,
            new_analysis_concurrency: 1,
            holdings_reassess_concurrency: 1,
        };
        let health = vec![
            make_health("tencent", VendorStatus::Healthy),
            make_health("eastmoney", VendorStatus::Degraded),
            make_health("sina", VendorStatus::Degraded),
            make_health("ths", VendorStatus::Degraded),
        ];
        let (new_c, reassess_c, _) = cfg.resolve_concurrency_with_vendor_health(&health);
        assert_eq!(new_c, 1, "base=1, ratio=0.25 < 0.3 应强制 1");
        assert_eq!(reassess_c, 1);
    }
}
