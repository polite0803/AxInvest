//! 工作流驱动的股票分析 — 基于持久化 WorkflowTemplate + WorkEngine DAG 执行。
//!
//! 启动时种子化 stock-analysis 工作流模板到 workflow_templates 表，
//! 每次分析从模板加载 DAG 结构，注入实时行情数据，由 WorkEngine 并行执行。

use crate::AppState;
use axagent_core::entity::stock_analyses;
use axagent_core::workflow_types::{JsonSchema, Variable, WorkflowEdge, WorkflowNode};
use axagent_rt_workflow::work_engine::{ProgressCallback, RunOptions, StepProgressEvent};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde_json::json;
use std::sync::Arc;
use tauri::{Emitter, State};

struct LoadedTemplate {
    nodes: Vec<WorkflowNode>,
    edges: Vec<WorkflowEdge>,
    input_schema: Option<JsonSchema>,
    output_schema: Option<JsonSchema>,
    variables: Option<Vec<Variable>>,
}

async fn load_and_inject_template(
    db: &sea_orm::DatabaseConnection,
    stock_code: &str,
) -> Result<LoadedTemplate, String> {
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

    if nodes.is_empty() {
        tracing::warn!("[stock_workflow] 模板节点为空，自动重新种子化");
        crate::commands::stock_analysis_setup::ensure_stock_analysis_experts_seeded(db).await?;
        let template = workflow_template::Entity::find_by_id("stock-analysis")
            .one(db)
            .await
            .map_err(|e| format!("重查模板失败: {e}"))?
            .ok_or("模板种子化后仍不存在")?;
        nodes =
            serde_json::from_str(&template.nodes).map_err(|e| format!("解析模板节点失败: {e}"))?;
    }

    for node in &mut nodes {
        if let WorkflowNode::Trigger(tn) = node {
            if let Some(sc) = tn.config.config.get_mut("stock_code") {
                *sc = serde_json::Value::String(stock_code.to_string());
            }
        }
    }

    let input_schema: Option<JsonSchema> = template
        .input_schema
        .as_ref()
        .and_then(|s| serde_json::from_str(s).ok());
    let output_schema: Option<JsonSchema> = template
        .output_schema
        .as_ref()
        .and_then(|s| serde_json::from_str(s).ok());
    let variables: Option<Vec<Variable>> = template
        .variables
        .as_ref()
        .and_then(|v| serde_json::from_str(v).ok());

    Ok(LoadedTemplate {
        nodes,
        edges,
        input_schema,
        output_schema,
        variables,
    })
}

fn extract_decision_fields(
    decision_json: &Option<String>,
) -> (Option<String>, Option<f64>, Option<String>) {
    let raw = match decision_json {
        Some(s) if !s.is_empty() => s,
        _ => return (None, None, None),
    };
    let parsed: serde_json::Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => return (None, None, None),
    };
    let action = parsed
        .get("action")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let position_pct = parsed
        .get("positionPct")
        .or_else(|| parsed.get("position_pct"))
        .and_then(|v| v.as_f64());
    let reasoning = parsed
        .get("reasoning")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    (action, position_pct, reasoning)
}

#[tauri::command]
pub async fn run_stock_workflow(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    stock_code: String,
    dry_run: Option<bool>,
) -> Result<serde_json::Value, String> {
    let quote = state
        .astock_client
        .get_quote(&stock_code)
        .await
        .map_err(|e| format!("行情获取失败: {e}"))?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let analysis_id = uuid::Uuid::new_v4().to_string();

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
        config_id: Set(None),
        created_at: Set(now_ms),
        updated_at: Set(now_ms),
    }
    .insert(&state.sea_db)
    .await
    .map_err(|e| format!("DB 写入失败: {e}"))?;

    let loaded = load_and_inject_template(&state.sea_db, &stock_code).await?;

    if let Some(ref vars) = loaded.variables {
        for v in vars {
            if v.name == "vendor_iwencai_key" {
                if let serde_json::Value::String(ref key) = v.value {
                    if !key.is_empty() {
                        *state.astock_client.iwencai_key.write().await = key.clone();
                    }
                }
            }
        }
    }

    let engine = Arc::clone(&state.work_engine);

    let wf_name = format!("stock-analysis-{stock_code}");
    let workflow = engine
        .create_workflow(&wf_name, loaded.nodes, loaded.edges)
        .await
        .map_err(|e| format!("创建工作流失败: {e}"))?;
    let wf_id = workflow.id.clone();
    let wf_id_ret = wf_id.clone();
    let app_h = app.clone();
    let db = state.sea_db.clone();
    let aid = analysis_id.clone();

    let progress_app = app.clone();
    let progress_wf_id = wf_id.clone();
    let progress_cb: ProgressCallback = Arc::new(move |event: StepProgressEvent| {
        let app = progress_app.clone();
        let wf_id = progress_wf_id.clone();
        Box::pin(async move {
            let mut payload = serde_json::json!({
                "workflowId": wf_id,
                "nodeId": event.node_id,
                "status": event.status,
                "totalNodes": event.total_nodes,
                "completedNodes": event.completed_nodes,
            });
            if let Some(output) = event.output {
                payload["output"] = output;
            }
            let _ = app.emit("workflow-step-done", payload);
        })
    });

    let input_schema = loaded.input_schema;
    let output_schema = loaded.output_schema;
    let template_vars = loaded.variables;

    let sc_for_ret = stock_code.clone();
    let sc_name = quote.name.clone();
    let sc_name_for_spawn = sc_name.clone();
    tokio::spawn(async move {
        let mut opts = RunOptions::default()
            .with_max_concurrent(9)
            .with_step_timeout(std::time::Duration::from_secs(300))
            .with_progress_callback(progress_cb)
            .with_input(json!({"stock_code": &stock_code}));
        if let Some(s) = input_schema {
            opts = opts.with_input_schema(s);
        }
        if let Some(s) = output_schema {
            opts = opts.with_output_schema(s);
        }
        if dry_run.unwrap_or(false) {
            opts.dry_run = true;
        }
        let mut merged_vars: Vec<axagent_core::workflow_types::Variable> = vec![
            axagent_core::workflow_types::Variable {
                name: "stock_code".into(),
                var_type: "string".into(),
                value: serde_json::Value::String(stock_code.clone()),
                description: Some("当前分析的股票代码".into()),
                is_secret: false,
            },
            axagent_core::workflow_types::Variable {
                name: "stock_name".into(),
                var_type: "string".into(),
                value: serde_json::Value::String(sc_name_for_spawn.clone()),
                description: Some("当前分析的股票名称".into()),
                is_secret: false,
            },
        ];
        if let Some(v) = template_vars {
            for tv in v {
                if !merged_vars.iter().any(|mv| mv.name == tv.name) {
                    merged_vars.push(tv);
                }
            }
        }
        opts = opts.with_variables(merged_vars);

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
                        tracing::warn!(%wf_id, status=?wf_status, "工作流以 Failed 状态结束");
                        let _ = app_h.emit(
                            "workflow-error",
                            serde_json::json!({
                                "workflowId": wf_id,
                                "error": "部分分析步骤失败",
                                "results": result.results,
                                "output": result.output,
                            }),
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
                            serde_json::json!({
                                "workflowId": wf_id,
                                "results": result.results,
                                "output": result.output,
                            }),
                        );
                        let decision_json = result
                            .output
                            .and_then(|v| serde_json::to_string(&v).ok())
                            .or_else(|| {
                                result
                                    .results
                                    .get("portfolio-mgr")
                                    .and_then(|v| serde_json::to_string(v).ok())
                            });
                        let (action, position_pct, reasoning) =
                            extract_decision_fields(&decision_json);
                        let _ = stock_analyses::Entity::update_many()
                            .col_expr(stock_analyses::Column::Status, Expr::value("completed"))
                            .col_expr(stock_analyses::Column::DecisionAction, Expr::value(action))
                            .col_expr(
                                stock_analyses::Column::DecisionPositionPct,
                                Expr::value(position_pct),
                            )
                            .col_expr(
                                stock_analyses::Column::DecisionReasoning,
                                Expr::value(reasoning),
                            )
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
        "stockCode": sc_for_ret,
        "stockName": sc_name,
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
