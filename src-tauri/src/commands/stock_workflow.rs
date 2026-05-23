//! 工作流驱动的股票分析 — 基于持久化 WorkflowTemplate + WorkEngine DAG 执行。
//!
//! 启动时种子化 stock-analysis 工作流模板到 workflow_templates 表，
//! 每次分析从模板加载 DAG 结构，注入实时行情数据，由 WorkEngine 并行执行。

use crate::AppState;
use axagent_core::entity::stock_analyses;
use axagent_core::workflow_types::{WorkflowEdge, WorkflowNode};
use axagent_rt_workflow::work_engine::{ProgressCallback, RunOptions, StepProgressEvent};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use std::sync::Arc;
use tauri::{Emitter, State};

/// 从 DB 加载工作流模板，将占位符替换为实际行情数据及专家提示词
async fn load_and_inject_template(
    db: &sea_orm::DatabaseConnection,
    data_ctx: &str,
) -> Result<(Vec<WorkflowNode>, Vec<WorkflowEdge>), String> {
    use axagent_core::entity::workflow_template;

    let template = workflow_template::Entity::find_by_id("stock-analysis")
        .one(db)
        .await
        .map_err(|e| format!("查询工作流模板失败: {e}"))?
        .ok_or("股票分析工作流模板未种子化，请重启应用")?;

    let mut nodes: Vec<WorkflowNode> =
        serde_json::from_str(&template.nodes).map_err(|e| format!("解析模板节点失败: {e}"))?;
    let edges: Vec<WorkflowEdge> =
        serde_json::from_str(&template.edges).map_err(|e| format!("解析模板边失败: {e}"))?;

    let prompts = super::stock_analysis::load_stock_analysis_prompts(db).await;

    // 替换占位符
    for node in &mut nodes {
        if let WorkflowNode::Agent(an) = node {
            // 提取 expert_id：从 agent_profile_id "stock-xxx" 中取 "xxx"
            let expert_id = an
                .config
                .agent_profile_id
                .as_deref()
                .and_then(|s| s.strip_prefix("stock-"))
                .unwrap_or("unknown");

            let expert_prompt = prompts
                .get(expert_id)
                .cloned()
                .unwrap_or_else(|| format!("你是{expert_id}，基于数据分析给出专业判断。"));

            an.config.system_prompt = an
                .config
                .system_prompt
                .replace("{{data_ctx}}", data_ctx)
                .replace(&format!("{{{{expert_prompt_{expert_id}}}}}"), &expert_prompt);
        }
    }

    Ok((nodes, edges))
}

#[tauri::command]
pub async fn run_stock_workflow(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    stock_code: String,
) -> Result<serde_json::Value, String> {
    // ── 1. 行情数据 ──
    let quote = state
        .astock_client
        .get_quote(&stock_code)
        .await
        .map_err(|e| format!("行情获取失败: {e}"))?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let analysis_id = uuid::Uuid::new_v4().to_string();

    // 写入 stock_analyses 表
    stock_analyses::ActiveModel {
        id: Set(analysis_id.clone()),
        stock_code: Set(stock_code.clone()),
        stock_name: Set(quote.name.clone()),
        analysis_date: Set(chrono::Utc::now().format("%Y-%m-%d").to_string()),
        provider_id: Set("workflow".into()),
        conversation_id: Set(uuid::Uuid::new_v4().to_string()),
        status: Set("running".into()),
        decision_action: Set(None),
        decision_position_pct: Set(None),
        decision_reasoning: Set(None),
        decision_json: Set(None),
        blackboard_snapshot: Set(None),
        created_at: Set(now_ms),
        updated_at: Set(now_ms),
    }
    .insert(&state.sea_db)
    .await
    .map_err(|e| format!("DB 写入失败: {e}"))?;

    // ── 2. 行情上下文 ──
    let data_ctx = format!(
        "{} ({})\n现价:¥{:.2} 涨跌:{:.2}% PE:{} PB:{} 市值:{}",
        quote.name,
        stock_code,
        quote.price,
        quote.change_pct,
        quote.pe.map_or("N/A".into(), |v| format!("{:.1}", v)),
        quote.pb.map_or("N/A".into(), |v| format!("{:.1}", v)),
        quote
            .total_mv
            .map_or("N/A".into(), |v| format!("{:.0}亿", v / 1e8)),
    );

    // ── 3. 从模板加载 DAG 并注入数据 ──
    let (nodes, edges) = load_and_inject_template(&state.sea_db, &data_ctx).await?;

    // ── 4. 创建并执行工作流 ──
    let engine = Arc::clone(&state.work_engine);
    let wf_name = format!("stock-analysis-{stock_code}");
    let workflow = engine
        .create_workflow(&wf_name, nodes, edges)
        .await
        .map_err(|e| format!("创建工作流失败: {e}"))?;
    let wf_id = workflow.id.clone();
    let wf_id_ret = wf_id.clone();
    let app_h = app.clone();
    let db = state.sea_db.clone();
    let aid = analysis_id.clone();

    // 进度回调
    let progress_app = app.clone();
    let progress_wf_id = wf_id.clone();
    let progress_cb: ProgressCallback = Arc::new(move |event: StepProgressEvent| {
        let app = progress_app.clone();
        let wf_id = progress_wf_id.clone();
        Box::pin(async move {
            let _ = app.emit(
                "workflow-step-done",
                serde_json::json!({
                    "workflowId": wf_id,
                    "nodeId": event.node_id,
                    "status": event.status,
                    "totalNodes": event.total_nodes,
                    "completedNodes": event.completed_nodes,
                }),
            );
        })
    });

    tokio::spawn(async move {
        let opts = RunOptions::default()
            .with_max_concurrent(9)
            .with_step_timeout(std::time::Duration::from_secs(300))
            .with_progress_callback(progress_cb);

        match engine.run_workflow(&wf_id, opts).await {
            Ok(result) => {
                let wf_status = result.status;
                match wf_status {
                    axagent_rt_workflow::workflow_engine::WorkflowStatus::Cancelled => {
                        let _ = app_h.emit(
                            "workflow-error",
                            serde_json::json!({ "workflowId": wf_id, "error": "分析已被取消" }),
                        );
                        let _ = stock_analyses::Entity::update_many()
                            .col_expr(stock_analyses::Column::Status, Expr::value("cancelled"))
                            .col_expr(
                                stock_analyses::Column::UpdatedAt,
                                Expr::value(chrono::Utc::now().timestamp_millis()),
                            )
                            .filter(stock_analyses::Column::Id.eq(&aid))
                            .exec(&db)
                            .await;
                    },
                    axagent_rt_workflow::workflow_engine::WorkflowStatus::Failed => {
                        let _ = app_h.emit(
                            "workflow-error",
                            serde_json::json!({ "workflowId": wf_id, "error": "部分分析步骤失败" }),
                        );
                        let _ = stock_analyses::Entity::update_many()
                            .col_expr(stock_analyses::Column::Status, Expr::value("failed"))
                            .col_expr(
                                stock_analyses::Column::UpdatedAt,
                                Expr::value(chrono::Utc::now().timestamp_millis()),
                            )
                            .filter(stock_analyses::Column::Id.eq(&aid))
                            .exec(&db)
                            .await;
                    },
                    _ => {
                        let _ = app_h.emit(
                            "workflow-completed",
                            serde_json::json!({ "workflowId": wf_id, "results": result.results }),
                        );
                        let decision_json = result
                            .results
                            .get("portfolio-mgr")
                            .and_then(|v| serde_json::to_string(v).ok());
                        let _ = stock_analyses::Entity::update_many()
                            .col_expr(stock_analyses::Column::Status, Expr::value("completed"))
                            .col_expr(
                                stock_analyses::Column::DecisionJson,
                                Expr::value(decision_json),
                            )
                            .col_expr(
                                stock_analyses::Column::UpdatedAt,
                                Expr::value(chrono::Utc::now().timestamp_millis()),
                            )
                            .filter(stock_analyses::Column::Id.eq(&aid))
                            .exec(&db)
                            .await;
                    },
                }
            },
            Err(e) => {
                let _ = app_h.emit(
                    "workflow-error",
                    serde_json::json!({ "workflowId": wf_id, "error": e.to_string() }),
                );
                let _ = stock_analyses::Entity::update_many()
                    .col_expr(stock_analyses::Column::Status, Expr::value(format!("failed: {e}")))
                    .col_expr(
                        stock_analyses::Column::UpdatedAt,
                        Expr::value(chrono::Utc::now().timestamp_millis()),
                    )
                    .filter(stock_analyses::Column::Id.eq(&aid))
                    .exec(&db)
                    .await;
            },
        }
    });

    Ok(serde_json::json!({
        "analysisId": analysis_id,
        "workflowId": wf_id_ret,
        "stockCode": stock_code,
        "stockName": quote.name,
    }))
}

/// 取消正在运行的股票分析工作流
#[tauri::command]
pub async fn cancel_stock_workflow(
    state: State<'_, AppState>,
    workflow_id: String,
) -> Result<(), String> {
    state
        .work_engine
        .cancel_workflow(&workflow_id)
        .await
        .map(|_| ())
        .map_err(|e| format!("取消工作流失败: {e}"))
}
