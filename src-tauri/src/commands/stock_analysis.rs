use crate::AppState;
use axagent_agent::session_manager::SessionManager;
use axagent_agent::shared_blackboard::SharedBlackboard;
use axagent_astock_data::AStockClient;
use axagent_core::entity::stock_analyses;
use axagent_stock_analysis::decision::{AnalysisConfig, AnalysisEvent};
use axagent_stock_analysis::orchestrator::StockAnalysisOrchestrator;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set};
use sea_orm::sea_query::Expr;
use std::sync::Arc;
use tauri::{Emitter, State};
use tokio::sync::RwLock;

/// 搜索股票
#[tauri::command]
pub async fn search_stock(
    keyword: String,
) -> Result<Vec<axagent_astock_data::StockSearchResult>, String> {
    let client = AStockClient::new();
    client
        .search_stock(&keyword)
        .await
        .map_err(|e| e.to_string())
}

/// 获取实时行情
#[tauri::command]
pub async fn get_stock_quote(
    stock_code: String,
) -> Result<axagent_astock_data::StockQuote, String> {
    let client = AStockClient::new();
    client
        .get_quote(&stock_code)
        .await
        .map_err(|e| e.to_string())
}

/// 获取K线数据
#[tauri::command]
pub async fn get_stock_kline(
    stock_code: String,
    period: String,
    limit: u32,
) -> Result<Vec<axagent_astock_data::KLine>, String> {
    let client = AStockClient::new();
    client
        .get_klines(&stock_code, &period, limit)
        .await
        .map_err(|e| e.to_string())
}

/// 启动股票分析（异步后台执行，通过 Tauri events 推送进度）
#[tauri::command]
pub async fn start_stock_analysis(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    stock_code: String,
    date: String,
    provider_id: String,
) -> Result<serde_json::Value, String> {
    let analysis_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis();

    // 1. 获取股票名称
    let client = AStockClient::new();
    let quote = client
        .get_quote(&stock_code)
        .await
        .map_err(|e| format!("获取行情失败: {}", e))?;
    let stock_name = quote.name.clone();

    // 2. 创建 conversation_id
    let conversation_id = uuid::Uuid::new_v4().to_string();

    // 3. 写入 stock_analyses 记录
    let model = stock_analyses::ActiveModel {
        id: Set(analysis_id.clone()),
        stock_code: Set(stock_code.clone()),
        stock_name: Set(stock_name.clone()),
        analysis_date: Set(date.clone()),
        provider_id: Set(provider_id.clone()),
        conversation_id: Set(conversation_id.clone()),
        status: Set("running".to_string()),
        decision_action: Set(None),
        decision_position_pct: Set(None),
        decision_reasoning: Set(None),
        decision_json: Set(None),
        blackboard_snapshot: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };

    model
        .insert(&state.sea_db)
        .await
        .map_err(|e| format!("写入分析记录失败: {}", e))?;

    // 4. spawn 异步分析任务
    let app_handle = app.clone();
    let analysis_id_clone = analysis_id.clone();
    let db = state.sea_db.clone();
    let stock_code_for_spawn = stock_code.clone();
    let stock_name_for_spawn = stock_name.clone();

    tokio::spawn(async move {
        let (event_tx, _) = tokio::sync::broadcast::channel::<AnalysisEvent>(64);

        // 转发事件到 Tauri 前端
        let mut event_rx = event_tx.subscribe();
        let app_for_events = app_handle.clone();
        tokio::spawn(async move {
            while let Ok(event) = event_rx.recv().await {
                let _ = app_for_events.emit("stock-analysis-event", &event);
            }
        });

        let data_client = AStockClient::new();
        let blackboard = Arc::new(RwLock::new(SharedBlackboard::new(
            &analysis_id_clone,
            &format!("分析 {} ({})", stock_code_for_spawn, stock_name_for_spawn),
        )));

        let config = AnalysisConfig::default();

        let session_manager = SessionManager::new(db.clone());

        let result = StockAnalysisOrchestrator::run(
            &session_manager,
            &data_client,
            blackboard,
            stock_code_for_spawn,
            stock_name_for_spawn,
            date,
            config,
            provider_id,
            conversation_id,
            event_tx,
        )
        .await;

        // 更新 DB 状态
        match result {
            Ok(decision) => {
                let decision_json =
                    serde_json::to_string(&decision).unwrap_or_default();
                let now = chrono::Utc::now().timestamp_millis();
                let _ = stock_analyses::Entity::update_many()
                    .col_expr(
                        stock_analyses::Column::Status,
                        Expr::value("completed"),
                    )
                    .col_expr(
                        stock_analyses::Column::DecisionAction,
                        Expr::value(&decision.action),
                    )
                    .col_expr(
                        stock_analyses::Column::DecisionPositionPct,
                        Expr::value(decision.position_pct),
                    )
                    .col_expr(
                        stock_analyses::Column::DecisionReasoning,
                        Expr::value(&decision.reasoning),
                    )
                    .col_expr(
                        stock_analyses::Column::DecisionJson,
                        Expr::value(&decision_json),
                    )
                    .col_expr(
                        stock_analyses::Column::UpdatedAt,
                        Expr::value(now),
                    )
                    .filter(stock_analyses::Column::Id.eq(&analysis_id_clone))
                    .exec(&db)
                    .await;
            }
            Err(e) => {
                let now = chrono::Utc::now().timestamp_millis();
                let status = format!("failed: {}", e);
                let _ = stock_analyses::Entity::update_many()
                    .col_expr(
                        stock_analyses::Column::Status,
                        Expr::value(&status),
                    )
                    .col_expr(
                        stock_analyses::Column::UpdatedAt,
                        Expr::value(now),
                    )
                    .filter(stock_analyses::Column::Id.eq(&analysis_id_clone))
                    .exec(&db)
                    .await;
            }
        }
    });

    Ok(serde_json::json!({
        "analysis_id": analysis_id,
        "stock_code": stock_code,
        "stock_name": stock_name,
        "status": "running",
    }))
}

/// 取消分析
#[tauri::command]
pub async fn cancel_stock_analysis(analysis_id: String) -> Result<(), String> {
    tracing::info!("cancel_stock_analysis: {}", analysis_id);
    Ok(())
}

/// 历史分析列表
#[tauri::command]
pub async fn list_stock_analyses(
    state: State<'_, AppState>,
    limit: u32,
    offset: u32,
) -> Result<Vec<stock_analyses::Model>, String> {
    stock_analyses::Entity::find()
        .order_by_desc(stock_analyses::Column::CreatedAt)
        .limit(Some(limit as u64))
        .offset(Some(offset as u64))
        .all(&state.sea_db)
        .await
        .map_err(|e| e.to_string())
}

/// 获取单个分析详情
#[tauri::command]
pub async fn get_stock_analysis(
    state: State<'_, AppState>,
    analysis_id: String,
) -> Result<stock_analyses::Model, String> {
    stock_analyses::Entity::find_by_id(&analysis_id)
        .one(&state.sea_db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("分析记录不存在: {}", analysis_id))
}
