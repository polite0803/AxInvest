//! 工作流驱动的股票分析 — 基于持久化 WorkflowTemplate + WorkEngine DAG 执行。
//!
//! 启动时种子化 stock-analysis 工作流模板到 workflow_templates 表，
//! 每次分析从模板加载 DAG 结构，注入实时行情数据，由 WorkEngine 并行执行。

use crate::AppState;
use axagent_core::entity::stock_analyses;
use axagent_core::workflow_types::{WorkflowEdge, WorkflowNode};
use axagent_rt_workflow::work_engine::{
    ProgressCallback, RunOptions, StepProgressEvent, ToolCallback,
};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde_json::json;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tauri::{Emitter, State};

// ── 算法工具：内联计算（独立于 orchestrator，供 Tool 节点调用）──

fn compute_scoring_inner(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let klines: Vec<axagent_astock_data::KLine> = args
        .get("kline_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    if klines.is_empty() {
        return Ok(json!({"error": "K线数据为空，无法评分"}));
    }
    let price = klines.last().map(|k| k.close).unwrap_or(0.0);
    let sc = args
        .get("stock_code")
        .and_then(|v| v.as_str())
        .unwrap_or("000001");
    let indicators = axagent_astock_data::indicators::compute_indicators(sc, &klines);
    let score = axagent_stock_analysis::scoring::ScoringEngine::score(&indicators, price, None);
    Ok(serde_json::to_value(&score).unwrap_or_default())
}

fn compute_valuation_inner(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let financials: Vec<axagent_astock_data::FinancialReport> = args
        .get("financials_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let quote_price = args
        .get("quote_price")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let metrics = axagent_stock_analysis::value::ValueEngine::assess(quote_price, &financials, 1.0);
    Ok(serde_json::to_value(&metrics).unwrap_or_default())
}

fn compute_portfolio_risk_inner(args: &serde_json::Value) -> serde_json::Value {
    let positions: Vec<serde_json::Value> = args
        .get("positions_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let total_mv: f64 = positions
        .iter()
        .filter_map(|p| p.get("marketValue").and_then(|v| v.as_f64()))
        .sum();
    let max_single = positions
        .iter()
        .filter_map(|p| p.get("marketValue").and_then(|v| v.as_f64()))
        .fold(0.0_f64, f64::max);
    let concentration = if total_mv > 0.0 {
        (max_single / total_mv) * 100.0
    } else {
        0.0
    };
    let risk_level = if concentration > 50.0 {
        "高风险"
    } else if concentration > 30.0 {
        "中高风险"
    } else if concentration > 20.0 {
        "中等风险"
    } else {
        "低风险"
    };
    json!({
        "total_market_value": total_mv,
        "position_count": positions.len(),
        "concentration_pct": (concentration * 10.0).round() / 10.0,
        "max_single_pct": if total_mv > 0.0 { (max_single / total_mv * 1000.0).round() / 10.0 } else { 0.0 },
        "risk_level": risk_level,
    })
}

fn run_quality_gate_inner(args: &serde_json::Value) -> serde_json::Value {
    let reports: HashMap<String, String> = args
        .get("reports_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let check = axagent_stock_analysis::quality::run_quality_gate(&reports);
    json!({
        "grade": format!("{:?}", check.grade),
        "summary": check.summary,
        "warnings": check.warnings,
    })
}

/// 从 DB 加载工作流模板，注入 stock_code 到 Trigger + 替换静态占位符。
/// 运行时变量（如 {{t-market-data}}）由 prompt_template 两阶段渲染处理。
async fn load_and_inject_template(
    db: &sea_orm::DatabaseConnection,
    stock_code: &str,
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

    // 注入占位符（静态部分由 load 阶段处理，{{tool_id}} 运行时由 prompt_template 渲染）
    for node in &mut nodes {
        match node {
            WorkflowNode::Trigger(tn) => {
                // 注入实际股票代码到 trigger config
                if let Some(sc) = tn.config.config.get_mut("stock_code") {
                    *sc = serde_json::Value::String(stock_code.to_string());
                }
            },
            WorkflowNode::Agent(an) => {
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
                    .replace("{{goal}}", &an.base.title)
                    .replace("{{data_ctx}}", data_ctx)
                    .replace(&format!("{{{{expert_prompt_{expert_id}}}}}"), &expert_prompt);
            },
            _ => {},
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
        config_id: Set(None),
        created_at: Set(now_ms),
        updated_at: Set(now_ms),
    }
    .insert(&state.sea_db)
    .await
    .map_err(|e| format!("DB 写入失败: {e}"))?;

    // ── 2. 多源行情上下文（报价 + K线摘要 + 财务 + 新闻 + 资金流向）──
    let sc = stock_code.clone();
    let (klines, financials, news, money_flow) = tokio::join!(
        state.astock_client.get_klines(&sc, "daily", 60),
        state.astock_client.get_financials(&sc),
        state.astock_client.get_news(&sc, 10),
        state.astock_client.get_money_flow(&sc),
    );

    let kline_summary = match &klines {
        Ok(k) if !k.is_empty() => {
            let _last = k.last().unwrap();
            let ma5: f64 = k.iter().rev().take(5).map(|x| x.close).sum::<f64>() / 5.0;
            let ma20: f64 = k.iter().rev().take(20).map(|x| x.close).sum::<f64>() / 20.0;
            let max60 = k.iter().map(|x| x.high).fold(f64::MIN, f64::max);
            let min60 = k.iter().map(|x| x.low).fold(f64::MAX, f64::min);
            format!(
                "最近60日K线：最高¥{:.2} 最低¥{:.2} MA5=¥{:.2} MA20=¥{:.2}",
                max60, min60, ma5, ma20
            )
        },
        _ => "K线数据暂不可用".into(),
    };

    let fin_summary = match &financials {
        Ok(f) if !f.is_empty() => {
            let last = &f[0];
            format!(
                "最新财报：营收{} 净利润{} EPS={} ROE={}% 毛利率={}%",
                last.revenue
                    .map_or("N/A".into(), |v| format!("{:.1}亿", v / 1e8)),
                last.net_profit
                    .map_or("N/A".into(), |v| format!("{:.1}亿", v / 1e8)),
                last.eps.map_or("N/A".into(), |v| format!("{:.2}", v)),
                last.roe.map_or("N/A".into(), |v| format!("{:.1}", v)),
                last.gross_margin
                    .map_or("N/A".into(), |v| format!("{:.1}", v)),
            )
        },
        _ => "财务数据暂不可用".into(),
    };

    let news_summary = match &news {
        Ok(n) if !n.is_empty() => {
            let titles: Vec<&str> = n.iter().take(5).map(|x| x.title.as_str()).collect();
            format!("最近{}条新闻：{}", n.len(), titles.join("；"))
        },
        _ => "新闻数据暂不可用".into(),
    };

    let mf_summary = match &money_flow {
        Ok(Some(mf)) => format!(
            "资金流向：主力净流入{:.1}亿 超大单{:.1}亿 大单{:.1}亿 中单{:.1}亿 小单{:.1}亿",
            mf.main_net_inflow / 1e8,
            mf.super_large_net / 1e8,
            mf.large_net / 1e8,
            mf.medium_net / 1e8,
            mf.small_net / 1e8,
        ),
        _ => "资金流向数据暂不可用".into(),
    };

    let data_ctx = format!(
        "{name} ({code})\n现价:¥{price:.2} 涨跌:{pct:.2}% PE:{pe} PB:{pb} 市值:{mv}\n\n{kline_summary}\n\n{fin_summary}\n\n{news_summary}\n\n{mf_summary}",
        name = quote.name,
        code = stock_code,
        price = quote.price,
        pct = quote.change_pct,
        pe = quote.pe.map_or("N/A".into(), |v| format!("{:.1}", v)),
        pb = quote.pb.map_or("N/A".into(), |v| format!("{:.1}", v)),
        mv = quote
            .total_mv
            .map_or("N/A".into(), |v| format!("{:.0}亿", v / 1e8)),
    );

    // ── 3. 从模板加载 DAG 并注入数据 ──
    let (nodes, edges) = load_and_inject_template(&state.sea_db, &stock_code, &data_ctx).await?;

    // ── 4. 注入 ToolCallback：将 MCP 工具调用桥接到 AStockClient ──
    let engine = Arc::clone(&state.work_engine);
    let tool_client = Arc::clone(&state.astock_client);
    let sc_tool = stock_code.clone();
    let tool_cb: ToolCallback = Arc::new(
        move |tool_name: String,
              args: serde_json::Value|
              -> Pin<
            Box<dyn std::future::Future<Output = Result<serde_json::Value, String>> + Send>,
        > {
            let client = Arc::clone(&tool_client);
            let code = sc_tool.clone();
            Box::pin(async move {
                let result: serde_json::Value = match tool_name.as_str() {
                    "search_stock" => {
                        let kw = args["keyword"].as_str().unwrap_or(&code);
                        match client.search_stock(kw).await {
                            Ok(v) => serde_json::to_value(v).unwrap_or_default(),
                            Err(e) => json!({"error": e.to_string()}),
                        }
                    },
                    "get_stock_quote" => {
                        let c = args["stock_code"].as_str().unwrap_or(&code);
                        match client.get_quote(c).await {
                            Ok(v) => serde_json::to_value(v).unwrap_or_default(),
                            Err(e) => json!({"error": e.to_string()}),
                        }
                    },
                    "get_stock_kline" => {
                        let c = args["stock_code"].as_str().unwrap_or(&code);
                        let period = args["period"].as_str().unwrap_or("daily");
                        let limit = args["limit"].as_u64().unwrap_or(120) as u32;
                        match client.get_klines(c, period, limit).await {
                            Ok(v) => serde_json::to_value(v).unwrap_or_default(),
                            Err(e) => json!({"error": e.to_string()}),
                        }
                    },
                    "get_stock_financials" => {
                        let c = args["stock_code"].as_str().unwrap_or(&code);
                        match client.get_financials(c).await {
                            Ok(v) => serde_json::to_value(v).unwrap_or_default(),
                            Err(e) => json!({"error": e.to_string()}),
                        }
                    },
                    "get_stock_news" => {
                        let c = args["stock_code"].as_str().unwrap_or(&code);
                        let limit = args["limit"].as_u64().unwrap_or(30) as u32;
                        match client.get_news(c, limit).await {
                            Ok(v) => serde_json::to_value(v).unwrap_or_default(),
                            Err(e) => json!({"error": e.to_string()}),
                        }
                    },
                    "get_stock_money_flow" => {
                        let c = args["stock_code"].as_str().unwrap_or(&code);
                        match client.get_money_flow(c).await {
                            Ok(v) => serde_json::to_value(v).unwrap_or_default(),
                            Err(e) => json!({"error": e.to_string()}),
                        }
                    },
                    // ── 算法工具 ──
                    "compute_scoring" => {
                        compute_scoring_inner(&args).unwrap_or_else(|e| json!({"error": e}))
                    },
                    "compute_valuation" => {
                        compute_valuation_inner(&args).unwrap_or_else(|e| json!({"error": e}))
                    },
                    "compute_portfolio_risk" => compute_portfolio_risk_inner(&args),
                    "run_quality_gate" => run_quality_gate_inner(&args),
                    _ => json!({"error": format!("未知工具: {tool_name}")}),
                };
                Ok(result)
            })
        },
    );
    engine.set_tool_callback(tool_cb).await;

    // ── 5. 创建并执行工作流 ──
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
