use crate::AppState;
use axagent_agent::shared_blackboard::SharedBlackboard;
use axagent_astock_data::AStockClient;
use axagent_core::entity::stock_analyses;
use axagent_core::types::ProviderProxyConfig;
use axagent_providers::{resolve_base_url_for_type, ProviderAdapter, ProviderRequestContext};
use axagent_stock_analysis::decision::{AgentRunner, AnalysisConfig, AnalysisEvent};
use axagent_stock_analysis::orchestrator::StockAnalysisOrchestrator;
use axagent_stock_analysis::runner::SessionManagerRunner;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use std::sync::atomic::{AtomicBool, Ordering};
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

    // 4. 创建取消令牌并存入 AppState
    let cancel_token = Arc::new(AtomicBool::new(false));
    {
        let mut tokens = state.agent_cancel_tokens.lock().await;
        tokens.insert(analysis_id.clone(), cancel_token.clone());
    }

    // 5. 尝试构建 AgentRunner（在 spawn 之前，因为需要访问 state）
    let master_key = state.master_key;
    let db_for_runner = state.sea_db.clone();
    let provider_id_for_runner = provider_id.clone();

    let runner: Option<Arc<dyn AgentRunner>> =
        match build_stock_analysis_runner(&db_for_runner, &master_key, &provider_id_for_runner)
            .await
        {
            Ok(r) => {
                tracing::info!(
                    "[stock_analysis] AgentRunner 已注入 (provider={})",
                    provider_id_for_runner
                );
                Some(Arc::new(r))
            },
            Err(e) => {
                tracing::warn!(
                    "[stock_analysis] 无法构建 AgentRunner，使用占位报告: {}",
                    e
                );
                None
            },
        };

    // 6. spawn 异步分析任务
    let app_handle = app.clone();
    let analysis_id_clone = analysis_id.clone();
    let db = state.sea_db.clone();
    let stock_code_for_spawn = stock_code.clone();
    let stock_name_for_spawn = stock_name.clone();
    let cancel_tokens = state.agent_cancel_tokens.clone();

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

        let result = StockAnalysisOrchestrator::run(
            &data_client,
            blackboard,
            stock_code_for_spawn,
            stock_name_for_spawn,
            date,
            config,
            event_tx,
            runner,
            Some(cancel_token),
        )
        .await;

        // 清理取消令牌
        {
            let mut tokens = cancel_tokens.lock().await;
            tokens.remove(&analysis_id_clone);
        }

        // 更新 DB 状态
        match result {
            Ok(decision) => {
                let decision_json = serde_json::to_string(&decision).unwrap_or_default();
                let now = chrono::Utc::now().timestamp_millis();
                let _ = stock_analyses::Entity::update_many()
                    .col_expr(stock_analyses::Column::Status, Expr::value("completed"))
                    .col_expr(stock_analyses::Column::DecisionAction, Expr::value(&decision.action))
                    .col_expr(
                        stock_analyses::Column::DecisionPositionPct,
                        Expr::value(decision.position_pct),
                    )
                    .col_expr(
                        stock_analyses::Column::DecisionReasoning,
                        Expr::value(&decision.reasoning),
                    )
                    .col_expr(stock_analyses::Column::DecisionJson, Expr::value(&decision_json))
                    .col_expr(stock_analyses::Column::UpdatedAt, Expr::value(now))
                    .filter(stock_analyses::Column::Id.eq(&analysis_id_clone))
                    .exec(&db)
                    .await;
            },
            Err(e) => {
                let now = chrono::Utc::now().timestamp_millis();
                let status = format!("failed: {}", e);
                let _ = stock_analyses::Entity::update_many()
                    .col_expr(stock_analyses::Column::Status, Expr::value(&status))
                    .col_expr(stock_analyses::Column::UpdatedAt, Expr::value(now))
                    .filter(stock_analyses::Column::Id.eq(&analysis_id_clone))
                    .exec(&db)
                    .await;
            },
        }
    });

    Ok(serde_json::json!({
        "analysis_id": analysis_id,
        "stock_code": stock_code,
        "stock_name": stock_name,
        "status": "running",
    }))
}

/// 取消分析 — 设置取消令牌让后台任务停止
#[tauri::command]
pub async fn cancel_stock_analysis(
    state: State<'_, AppState>,
    analysis_id: String,
) -> Result<(), String> {
    let tokens = state.agent_cancel_tokens.lock().await;
    if let Some(token) = tokens.get(&analysis_id) {
        token.store(true, Ordering::Relaxed);
        tracing::info!("cancel_stock_analysis: 已设置取消令牌 {}", analysis_id);
        Ok(())
    } else {
        Err(format!("分析任务不存在或已完成: {}", analysis_id))
    }
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

// ── Helper: 构建 SessionManagerRunner ──

/// 从数据库中的 provider 配置构建 SessionManagerRunner。
///
/// 流程: DB 查 provider → 取激活 key → 解密 → 构建 ProviderRequestContext →
/// 构建 ProviderAdapter → 选第一个启用模型 → 构建 SessionManagerRunner。
///
/// 任何步骤失败都会返回 `Err`，调用方可回退到占位报告模式。
async fn build_stock_analysis_runner(
    db: &sea_orm::DatabaseConnection,
    master_key: &[u8; 32],
    provider_id: &str,
) -> Result<SessionManagerRunner, String> {
    // 1. 查询 provider 配置
    let prov = axagent_core::repo::provider::get_provider(db, provider_id)
        .await
        .map_err(|e| format!("Provider 查询失败: {}", e))?;

    if !prov.enabled {
        return Err("Provider 已禁用".into());
    }

    // 2. 取激活的 API key
    let key = prov
        .keys
        .iter()
        .find(|k| k.enabled)
        .ok_or_else(|| "没有启用的 API key".to_string())?;

    // 3. 解密 key
    let api_key = axagent_core::crypto::decrypt_key(&key.key_encrypted, master_key)
        .map_err(|e| format!("密钥解密失败: {}", e))?;

    // 4. 获取全局设置（用于 proxy 回退）
    let settings = axagent_core::repo::settings::get_settings(db)
        .await
        .unwrap_or_default();

    // 5. 构建 ProviderRequestContext
    let custom_headers: Option<std::collections::HashMap<String, String>> = prov
        .custom_headers
        .as_ref()
        .and_then(|s| serde_json::from_str(s).ok());

    let ctx = ProviderRequestContext {
        api_key,
        key_id: key.id.clone(),
        provider_id: prov.id.clone(),
        base_url: Some(resolve_base_url_for_type(
            &prov.api_host,
            &prov.provider_type,
        )),
        api_path: prov.api_path.clone(),
        proxy_config: ProviderProxyConfig::resolve(&prov.proxy_config, &settings),
        custom_headers,
        api_mode: None,
        conversation: None,
        previous_response_id: None,
        store_response: None,
    };

    // 6. 根据 provider 类型构建对应的 adapter
    let adapter: Arc<dyn ProviderAdapter> = match prov.provider_type {
        axagent_core::types::ProviderType::OpenAI => {
            Arc::new(axagent_providers::openai::OpenAIAdapter::new())
        },
        axagent_core::types::ProviderType::OpenAIResponses => {
            Arc::new(axagent_providers::openai_responses::OpenAIResponsesAdapter::new())
        },
        axagent_core::types::ProviderType::Anthropic => {
            Arc::new(axagent_providers::anthropic::AnthropicAdapter::new())
        },
        axagent_core::types::ProviderType::Gemini => {
            Arc::new(axagent_providers::gemini::GeminiAdapter::new())
        },
        axagent_core::types::ProviderType::OpenClaw => {
            Arc::new(axagent_providers::openclaw::OpenClawAdapter::new())
        },
        axagent_core::types::ProviderType::Hermes => {
            Arc::new(axagent_providers::hermes::HermesAdapter::new())
        },
        axagent_core::types::ProviderType::Ollama => {
            Arc::new(axagent_providers::ollama::OllamaAdapter::new())
        },
    };

    // 7. 选第一个启用的模型
    let model_id = prov
        .models
        .iter()
        .find(|m| m.enabled)
        .map(|m| m.model_id.clone())
        .ok_or_else(|| "没有可用的模型".to_string())?;

    // 8. 构建 runner（股票分析偏确定性，temperature=0.3）
    Ok(
        SessionManagerRunner::new(adapter, ctx, model_id)
            .with_temperature(Some(0.3))
            .with_max_tokens(Some(4096)),
    )
}
