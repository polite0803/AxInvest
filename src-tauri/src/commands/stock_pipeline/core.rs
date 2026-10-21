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
use axagent_harness::IpcEventName;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};

use axagent_entities::stock_pipeline_runs;
use axagent_harness::workflow_types::{Variable, WorkflowEdge, WorkflowNode};
use axagent_rt_workflow::work_engine::{ProgressCallback, RunOptions, StepProgressEvent};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DbErr, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
    sea_query::Expr,
};
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

use crate::AppState;
use crate::commands::error::ErrorResponse;
use crate::commands::error_code::stock_workflow as wf_err;

/// `stock_pipeline_runs.status` 的取值。
const STATUS_RUNNING: &str = "running";
const STATUS_COMPLETED: &str = "completed";
const STATUS_FAILED: &str = "failed";
const STATUS_INTERRUPTED: &str = "interrupted";

/// 遗留 running 行的收敛说明：写进 `error_message`，列表行与详情抽屉都会显示。
const INTERRUPTED_MESSAGE: &str =
    "应用重启：管道运行中进程退出，本次运行无法跨进程续跑（已按中断收敛，结果未写入）";

/// 本进程内正在执行的管道 run id。
///
/// 收敛判据是「库里 running 且**不在**这里」，不是时间阈值 —— 管道跑几小时也正常，
/// 按时间判会把活着的运行误杀；进程内登记才是精确真值。
static ACTIVE_RUNS: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// 遗留 running 行每进程只收敛一次。失败不置位（下次读取再试）。
static STALE_REAP_DONE: AtomicBool = AtomicBool::new(false);

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

/// 管道节点进度载荷（结构化）。
///
/// **为什么是结构体而不是拼好的字符串**：历史实现用
/// `format!("{}: 执行中", event.node_id)` 拼出中文塞进 `detail`，经 `pipeline-step`
/// 事件由 `PipelinePage` **直接渲染** ⇒ 中文硬编码漏到界面（同一处还混着英文
/// `"failed"`/`"timeout"`，三语混杂），且非中文用户看到中文。
/// 现在**只在产出端发机器可读值**（`node_id` + `status`），**文案由前端按 i18n 组装**
/// （`pipeline.stepRunning` 等）—— 与 `error`/`errorCode` 的分工同一原则：
/// 结构化值负责判定与本地化，产出端不产出人类可读文本。
#[derive(Debug, Clone)]
pub struct PipelineStepInfo {
    /// 工作流节点 ID（如 `p1-discovery`）
    pub node_id: String,
    /// 节点状态：`running` / `completed` / `failed` / `timeout`（原样透传，不加工）
    pub status: String,
}

/// 把「库里 running 且不在 `active` 中」的行收敛为 interrupted。
/// 返回实际收敛行数（幂等：再跑一次命中 0）。
async fn reap_running_runs(
    db: &sea_orm::DatabaseConnection,
    active: &HashSet<String>,
) -> Result<usize, DbErr> {
    let rows = stock_pipeline_runs::Entity::find()
        .filter(stock_pipeline_runs::Column::Status.eq(STATUS_RUNNING))
        .all(db)
        .await?;
    let mut reaped = 0usize;
    for row in rows {
        if active.contains(&row.id) {
            continue;
        }
        let res = stock_pipeline_runs::Entity::update_many()
            .col_expr(stock_pipeline_runs::Column::Status, Expr::value(STATUS_INTERRUPTED))
            .col_expr(stock_pipeline_runs::Column::ErrorMessage, Expr::value(INTERRUPTED_MESSAGE))
            // 不写 completed_at：真实结束时刻未知，写收敛时刻会伪造一个「耗时数周」的读数
            .filter(stock_pipeline_runs::Column::Id.eq(row.id.clone()))
            .filter(stock_pipeline_runs::Column::Status.eq(STATUS_RUNNING))
            .exec(db)
            .await?;
        if res.rows_affected > 0 {
            reaped += 1;
        }
    }
    Ok(reaped)
}

/// 每进程一次：收敛上次进程退出遗留的 running 行。
async fn ensure_stale_reaped(db: &sea_orm::DatabaseConnection) {
    if STALE_REAP_DONE.load(Ordering::SeqCst) {
        return;
    }
    let active = ACTIVE_RUNS.lock().await.clone();
    match reap_running_runs(db, &active).await {
        Ok(0) => {},
        Ok(n) => {
            tracing::info!("[stock_pipeline] 启动收敛：{n} 个遗留 running 运行标记为 interrupted")
        },
        Err(e) => {
            // 不置位：下次读列表/发起运行再试（一次性标志位不能把失败也吃掉）
            tracing::warn!("[stock_pipeline] 遗留 running 行收敛失败: {e}");
            return;
        },
    }
    STALE_REAP_DONE.store(true, Ordering::SeqCst);
}

/// 管道执行内部函数（供 Tauri 命令调用）
///
/// 使用 WorkEngine 加载工作流模板并执行，与股票分析工作流保持一致。
/// 进度通过可选的回调推送（载荷为 [`PipelineStepInfo`]，不含人类可读文案）。
pub async fn run_stock_pipeline_inner(
    db: &sea_orm::DatabaseConnection,
    client: &Arc<axagent_astock_data::AStockClient>,
    engine: &Arc<axagent_rt_workflow::work_engine::WorkEngine>,
    config: &PipelineConfig,
    as_of_date: Option<&str>,
    progress_callback: Option<Arc<dyn Fn(&str, PipelineStepInfo) + Send + Sync>>,
) -> Result<PipelineResult, String> {
    let run_id = uuid::Uuid::new_v4().to_string();
    let run_date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let started_at = chrono::Utc::now().timestamp_millis();
    // 执行体暂不直接用 client（模板里的取数走节点自己的通道），保留参数以稳定调用契约。
    let _ = client;

    // 上次进程退出遗留的 running 行先收敛（本进程存活的 run 由 ACTIVE_RUNS 保护）。
    ensure_stale_reaped(db).await;

    // 先登记活跃再落库：反过来的话，落库与登记之间若正好有列表查询触发收敛，
    // 这条刚插入的 running 行会被当成遗留行误杀。
    ACTIVE_RUNS.lock().await.insert(run_id.clone());

    // 创建 pipeline_run 记录（status=running）
    let insert_result = stock_pipeline_runs::ActiveModel {
        id: Set(run_id.clone()),
        run_date: Set(run_date.clone()),
        as_of_date: Set(as_of_date.map(String::from)),
        status: Set(STATUS_RUNNING.to_string()),
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
    if let Err(e) = insert_result {
        // 不阻断分析本身，但必须留痕：落库失败时这次运行不进历史列表，
        // 静默吞掉会让「刚跑完却看不到」无从排查。
        tracing::warn!("[stock_pipeline] 运行记录 {run_id} 落库失败（本次运行将不进历史）: {e}");
    }

    // 模板加载 / 建图 / 执行：任何一步提前返回（`?`）也必须在下面写终态，
    // 否则该行会永远停在 running（2026-08-26 两条僵尸行的成因）。
    let outcome =
        execute_pipeline(db, engine, config, as_of_date, &run_id, &run_date, progress_callback)
            .await;

    let completed_at = chrono::Utc::now().timestamp_millis();
    match &outcome {
        Ok(pr) => update_pipeline_run_success(db, &run_id, pr, completed_at).await,
        Err(e) => {
            if let Err(db_err) = stock_pipeline_runs::Entity::update_many()
                .col_expr(stock_pipeline_runs::Column::Status, Expr::value(STATUS_FAILED))
                .col_expr(stock_pipeline_runs::Column::ErrorMessage, Expr::value(e.clone()))
                .col_expr(stock_pipeline_runs::Column::CompletedAt, Expr::value(completed_at))
                .filter(stock_pipeline_runs::Column::Id.eq(run_id.clone()))
                .exec(db)
                .await
            {
                tracing::warn!("[stock_pipeline] 运行记录 {run_id} 失败态回写失败: {db_err}");
            }
        },
    }
    ACTIVE_RUNS.lock().await.remove(&run_id);

    outcome
}

/// 执行体：模板加载 → 建图 → 跑工作流 → 从 results 取摘要。
/// **不写运行记录**：起点与终态都由 [`run_stock_pipeline_inner`] 统一收口，
/// 这样 `?` 提前返回也不会漏掉终态。
async fn execute_pipeline(
    db: &sea_orm::DatabaseConnection,
    engine: &Arc<axagent_rt_workflow::work_engine::WorkEngine>,
    config: &PipelineConfig,
    as_of_date: Option<&str>,
    run_id: &str,
    run_date: &str,
    progress_callback: Option<Arc<dyn Fn(&str, PipelineStepInfo) + Send + Sync>>,
) -> Result<PipelineResult, String> {
    // 加载工作流模板
    let loaded = load_pipeline_template(db).await?;

    // 构建进度回调
    let progress_cb: ProgressCallback = Arc::new(move |event: StepProgressEvent| {
        if let Some(cb) = progress_callback.as_ref() {
            // 只透传结构化值：不在这里拼 `"<nodeId>: 执行中"` 这类人类可读文案
            // （文案由前端 i18n 组装，见 `PipelineStepInfo` 的说明）。
            cb("pipeline_step", PipelineStepInfo { node_id: event.node_id, status: event.status });
        }
        Box::pin(async move {})
    });

    // 注入变量（D8 修复：**模板变量作基座，管道计算值覆盖同名项**）
    //
    // 原实现只传 6 个管道自带变量 ⇒ 模板里声明的其余参数在下游 tool/agent 节点的
    // `input_mapping` 中全部解析不到，静默回退模块常量，与工作区路径取值不一致
    // （D8 同族：同一参数在不同路径上取不同来源）。详见
    // `AUDIT-300642-run-variance-2026-09-22.md` §13。
    let mut variables = loaded.variables.clone().unwrap_or_default();
    for pv in build_pipeline_variables(config, as_of_date, run_date, run_id) {
        if let Some(pos) = variables.iter().position(|v| v.name == pv.name) {
            variables[pos] = pv;
        } else {
            variables.push(pv);
        }
    }

    // 创建工作流
    let wf_name = format!("stock-pipeline-{run_id}");
    // 统一 with_hooks 模式：模板未来声明钩子时不会静默丢失
    let workflow = engine
        .create_workflow_with_hooks(&wf_name, loaded.nodes, loaded.edges, loaded.hooks_config)
        .await
        .map_err(|e| {
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
    match engine.run_workflow(&wf_id, opts).await {
        Ok(wf) if wf.status == axagent_rt_workflow::workflow_engine::WorkflowStatus::Failed => {
            Err("工作流执行失败".to_string())
        },
        Ok(wf) => Ok(build_pipeline_result_from_workflow(&wf, run_id, run_date)),
        Err(e) => Err(e.to_string()),
    }
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
    // 解析失败**必须留痕**：`.ok()` 会静默降级成 None，而 None 的后果是下游所有
    // `input_mapping`（`stock_code` 除外）解析不到并回退默认值 —— 这类"配置看起来配了
    // 但没生效"的缺陷极难从行为上察觉（D8 的成因之一）。与同文件 hooks_config 同语义：
    // 降级但不静默。
    let variables: Option<Vec<Variable>> =
        template.variables.as_ref().and_then(|v| match serde_json::from_str(v) {
            Ok(vars) => Some(vars),
            Err(e) => {
                tracing::warn!(
                    "[stock_pipeline] 模板 stock-pipeline 的 variables 解析失败，按无变量处理\
                     （下游 input_mapping 将回退默认值）: {e}"
                );
                None
            },
        });
    // NULL → None；解析失败降级 None（与 rt-workflow parse_hooks_config 同语义）
    let hooks_config: Option<axagent_harness::WorkflowHooksConfig> =
        template.hooks_config.as_ref().and_then(|s| serde_json::from_str(s).ok());

    Ok(crate::commands::stock_workflow::decision::LoadedTemplate {
        nodes,
        edges,
        input_schema,
        output_schema,
        variables,
        // 决策落库时写进 stock_analyses.template_version，供离线复算判定公式版本
        version: template.version,
        hooks_config,
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
        .col_expr(stock_pipeline_runs::Column::Status, Expr::value(STATUS_COMPLETED))
        .col_expr(
            stock_pipeline_runs::Column::CandidatesJson,
            Expr::value(serde_json::to_string(&pr.candidates).unwrap_or_default()),
        )
        .col_expr(
            stock_pipeline_runs::Column::NewAnalysesJson,
            Expr::value(serde_json::to_string(&pr.new_analyses).unwrap_or_default()),
        )
        .col_expr(
            stock_pipeline_runs::Column::ReassessedJson,
            Expr::value(serde_json::to_string(&pr.reassessed).unwrap_or_default()),
        )
        .col_expr(
            stock_pipeline_runs::Column::SummaryJson,
            Expr::value(serde_json::to_string(&pr.summary).unwrap_or_default()),
        )
        // 成功态必须清掉错误列：否则被误判过一次中断的 run 会带着旧说明停在 completed
        .col_expr(stock_pipeline_runs::Column::ErrorMessage, Expr::value(None::<String>))
        .col_expr(stock_pipeline_runs::Column::CompletedAt, Expr::value(completed_at))
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
    let progress_callback = Arc::new(move |step: &str, info: PipelineStepInfo| {
        let _ = app_handle.emit(
            IpcEventName::PipelineStep.as_str(),
            // 结构化载荷：`nodeId` + `status` 由前端按 i18n 组装文案。
            // 不再发拼接好的中文 `detail`（前端 `PipelineStepEvent` 里该字段已标 `@deprecated`，
            // 仅作旧载荷的展示兜底 —— 后端从这里起不再产出它）。
            json!({
                "step": step,
                "nodeId": info.node_id,
                "status": info.status,
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

    // 读列表前先收敛遗留 running 行：否则进程退出过一次，任务中心就永久显示
    // 「运行中」（2026-08-26 起四条僵尸行的实际形态）。
    ensure_stale_reaped(db).await;

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

    ensure_stale_reaped(db).await;

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

    // 明细 / 候选列在批次落库时为 None（运行中/失败/旧批次从未回写），
    // 或 JSON 解析失败 —— 这两种情况都按**空数组**返回（而非 null）：
    // 前端 Drawer 直接读 `newAnalyses.length`，null 会整页崩
    // （Cannot read properties of null，2026-10-12 实测）。
    let json_array_or_empty = |raw: Option<String>| -> serde_json::Value {
        raw.and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .unwrap_or_else(|| serde_json::json!([]))
    };

    Ok(json!({
        "id": run.id,
        "runDate": run.run_date,
        "asOfDate": run.as_of_date,
        "status": run.status,
        "startedAt": run.started_at,
        "completedAt": run.completed_at,
        "errorMessage": run.error_message,
        "candidates": json_array_or_empty(run.candidates_json),
        "newAnalyses": json_array_or_empty(run.new_analyses_json),
        "reassessed": json_array_or_empty(run.reassessed_json),
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

    /// 遗留 running 行收敛（2026-08-26 四条僵尸行的回归）：
    /// 只有「库里 running 且不在本进程活跃集」的行被改成 interrupted；
    /// 终态行与活跃行不动；不伪造完成时刻；幂等。
    #[tokio::test]
    async fn stale_running_rows_converge_to_interrupted() {
        let db = axagent_dao::db::create_test_pool().await.expect("测试库应创建成功").conn;
        for (id, status) in
            [("zombie", STATUS_RUNNING), ("alive", STATUS_RUNNING), ("done", STATUS_COMPLETED)]
        {
            stock_pipeline_runs::ActiveModel {
                id: Set(id.to_string()),
                run_date: Set("2026-08-26".to_string()),
                as_of_date: Set(None),
                status: Set(status.to_string()),
                candidates_json: Set(None),
                new_analyses_json: Set(None),
                reassessed_json: Set(None),
                summary_json: Set(None),
                error_message: Set(None),
                started_at: Set(1_000),
                completed_at: Set(None),
                created_at: Set(1_000),
            }
            .insert(&db)
            .await
            .expect("插入运行记录应成功");
        }

        let active: HashSet<String> = ["alive".to_string()].into_iter().collect();
        assert_eq!(reap_running_runs(&db, &active).await.unwrap(), 1, "只收敛非活跃 running 行");

        let zombie =
            stock_pipeline_runs::Entity::find_by_id("zombie").one(&db).await.unwrap().unwrap();
        assert_eq!(zombie.status, STATUS_INTERRUPTED);
        assert!(zombie.error_message.as_deref().unwrap_or_default().contains("重启"));
        assert!(zombie.completed_at.is_none(), "结束时刻未知时不得伪造耗时");

        let alive =
            stock_pipeline_runs::Entity::find_by_id("alive").one(&db).await.unwrap().unwrap();
        assert_eq!(alive.status, STATUS_RUNNING, "本进程活跃行不得被误杀");
        let done = stock_pipeline_runs::Entity::find_by_id("done").one(&db).await.unwrap().unwrap();
        assert_eq!(done.status, STATUS_COMPLETED, "终态行不受影响");

        assert_eq!(reap_running_runs(&db, &active).await.unwrap(), 0, "幂等");
    }
}
