//! 工作流驱动的股票分析 — 基于统一 WorkEngine 的 DAG 执行。
//!
//! 节点类型统一为 WorkflowNode::Agent(AgentNode)，
//! 通过 WorkflowEdge 定义依赖关系，WorkEngine 自动拓扑排序 + 并行执行 + DB 持久化。

use crate::AppState;
use axagent_core::entity::stock_analyses;
use axagent_core::workflow_types::{
    AgentNode, AgentNodeConfig, EdgeType, OutputMode, Position, RetryConfig, WorkflowEdge,
    WorkflowNode, WorkflowNodeBase,
};
use axagent_rt_workflow::work_engine::{ProgressCallback, RunOptions, StepProgressEvent};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use std::sync::Arc;
use tauri::{Emitter, State};

/// 构建 Agent 节点 — 关联种子化的 AgentProfile，嵌入行情上下文
fn agent_node(
    id: &str,
    title: &str,
    expert_id: &str,
    system_prompt: &str,
    data_ctx: &str,
    model: Option<String>,
) -> WorkflowNode {
    WorkflowNode::Agent(AgentNode {
        base: WorkflowNodeBase {
            id: id.into(),
            title: title.into(),
            description: Some(format!("股票分析专家: {expert_id}")),
            position: Position { x: 0.0, y: 0.0 },
            retry: RetryConfig {
                enabled: true,
                max_retries: 2,
                ..Default::default()
            },
            timeout: Some(300),
            enabled: true,
        },
        config: AgentNodeConfig {
            role: None,
            system_prompt: format!("{system_prompt}\n\n行情数据:\n{data_ctx}"),
            context_sources: vec![],
            output_var: id.into(),
            model,
            temperature: Some(0.3),
            max_tokens: Some(4096),
            tools: vec![],
            output_mode: OutputMode::Text,
            agent_profile_id: Some(format!("stock-{expert_id}")),
            agent_role_override: None,
        },
    })
}

/// 构建依赖边
fn edge(id: &str, source: &str, target: &str) -> WorkflowEdge {
    WorkflowEdge {
        id: id.into(),
        source: source.into(),
        source_handle: None,
        target: target.into(),
        target_handle: None,
        edge_type: EdgeType::Direct,
        label: None,
    }
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
    let prompts = super::stock_analysis::load_stock_analysis_prompts(&state.sea_db).await;
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

    let prompt = |id: &str| -> String {
        prompts
            .get(id)
            .cloned()
            .unwrap_or_else(|| format!("你是{id}，基于数据分析给出专业判断。"))
    };

    // ── 3. 构建 DAG ──
    let model = None; // AgentExecutor 从系统默认 provider 自动解析

    let mut nodes: Vec<WorkflowNode> = Vec::new();
    let mut edges: Vec<WorkflowEdge> = Vec::new();

    // 9 个分析师（并行，无依赖）
    let analysts = [
        ("a-market-analyst", "市场技术分析师", "market-analyst"),
        ("a-sentiment", "情绪面分析师", "sentiment-analyst"),
        ("a-news", "消息面分析师", "news-analyst"),
        ("a-fundamentals", "基本面分析师", "fundamentals-analyst"),
        ("a-policy", "政策面分析师", "policy-analyst"),
        ("a-hot-money", "资金面分析师", "hot-money-tracker"),
        ("a-lockup", "解禁监控分析师", "lockup-watcher"),
        ("a-research", "研报分析师", "research-analyst"),
        ("a-sector", "行业板块分析师", "sector-analyst"),
    ];
    let a_ids: Vec<String> = analysts.iter().map(|(id, _, _)| id.to_string()).collect();

    for (id, title, expert) in analysts {
        nodes.push(agent_node(id, title, expert, &prompt(expert), &data_ctx, model.clone()));
    }

    // 辩论（串行，6 轮）
    nodes.push(agent_node(
        "bull-r1",
        "多方第1轮",
        "bull-researcher",
        &prompt("bull-researcher"),
        &data_ctx,
        model.clone(),
    ));
    edges.append(
        &mut a_ids
            .iter()
            .map(|a| edge(&format!("e-{a}-bull-r1"), a, "bull-r1"))
            .collect(),
    );

    nodes.push(agent_node(
        "bear-r1",
        "空方第1轮",
        "bear-researcher",
        &prompt("bear-researcher"),
        &data_ctx,
        model.clone(),
    ));
    edges.push(edge("e-bull-r1-bear-r1", "bull-r1", "bear-r1"));

    nodes.push(agent_node(
        "bull-r2",
        "多方第2轮",
        "bull-researcher",
        &prompt("bull-researcher"),
        &data_ctx,
        model.clone(),
    ));
    edges.push(edge("e-bear-r1-bull-r2", "bear-r1", "bull-r2"));

    nodes.push(agent_node(
        "bear-r2",
        "空方第2轮",
        "bear-researcher",
        &prompt("bear-researcher"),
        &data_ctx,
        model.clone(),
    ));
    edges.push(edge("e-bull-r2-bear-r2", "bull-r2", "bear-r2"));

    nodes.push(agent_node(
        "bull-r3",
        "多方第3轮",
        "bull-researcher",
        &prompt("bull-researcher"),
        &data_ctx,
        model.clone(),
    ));
    edges.push(edge("e-bear-r2-bull-r3", "bear-r2", "bull-r3"));

    nodes.push(agent_node(
        "bear-r3",
        "空方第3轮",
        "bear-researcher",
        &prompt("bear-researcher"),
        &data_ctx,
        model.clone(),
    ));
    edges.push(edge("e-bull-r3-bear-r3", "bull-r3", "bear-r3"));

    // 风险评估（并行，均依赖 bear-r3）
    nodes.push(agent_node(
        "risk-agg",
        "激进风险评估",
        "aggressive-debator",
        &prompt("aggressive-debator"),
        &data_ctx,
        model.clone(),
    ));
    edges.push(edge("e-bear-r3-risk-agg", "bear-r3", "risk-agg"));

    nodes.push(agent_node(
        "risk-con",
        "保守风险评估",
        "conservative-debator",
        &prompt("conservative-debator"),
        &data_ctx,
        model.clone(),
    ));
    edges.push(edge("e-bear-r3-risk-con", "bear-r3", "risk-con"));

    nodes.push(agent_node(
        "risk-neu",
        "中性风险评估",
        "neutral-debator",
        &prompt("neutral-debator"),
        &data_ctx,
        model.clone(),
    ));
    edges.push(edge("e-bear-r3-risk-neu", "bear-r3", "risk-neu"));

    // 综合风险总评（依赖 3 个风险评估）
    nodes.push(agent_node(
        "research-mgr",
        "综合风险总评",
        "research-manager",
        &prompt("research-manager"),
        &data_ctx,
        model.clone(),
    ));
    for risk_id in &["risk-agg", "risk-con", "risk-neu"] {
        edges.push(edge(&format!("e-{risk_id}-research-mgr"), risk_id, "research-mgr"));
    }

    // 交易方案
    nodes.push(agent_node(
        "trader",
        "A股交易方案",
        "trader",
        &prompt("trader"),
        &data_ctx,
        model.clone(),
    ));
    edges.push(edge("e-research-mgr-trader", "research-mgr", "trader"));

    // 最终决策
    nodes.push(agent_node(
        "portfolio-mgr",
        "最终投资决策",
        "portfolio-manager",
        &prompt("portfolio-manager"),
        &data_ctx,
        model,
    ));
    edges.push(edge("e-trader-portfolio-mgr", "trader", "portfolio-mgr"));

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

    // 构建进度回调，向前端推送中间步骤事件
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
            .with_max_concurrent(9) // 9 个分析师并行
            .with_step_timeout(std::time::Duration::from_secs(300))
            .with_progress_callback(progress_cb);
        let engine_clone = engine.clone();
        match engine_clone.run_workflow(&wf_id, opts).await {
            Ok(result) => {
                match result.status {
                    axagent_rt_workflow::workflow_engine::WorkflowStatus::Cancelled => {
                        let _ = app_h.emit(
                            "workflow-error",
                            serde_json::json!({
                                "workflowId": wf_id,
                                "error": "分析已被取消",
                            }),
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
                            serde_json::json!({
                                "workflowId": wf_id,
                                "error": "部分分析步骤失败，请重试",
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
                        // Completed / PartiallyCompleted
                        let _ = app_h.emit(
                            "workflow-completed",
                            serde_json::json!({
                                "workflowId": wf_id,
                                "results": result.results,
                            }),
                        );

                        // 仅提取最终决策节点的输出落库
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
                    serde_json::json!({
                        "workflowId": wf_id,
                        "error": e.to_string(),
                    }),
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
    let engine = &*state.work_engine;
    engine
        .cancel_workflow(&workflow_id)
        .await
        .map(|_| ())
        .map_err(|e| format!("取消工作流失败: {e}"))
}
