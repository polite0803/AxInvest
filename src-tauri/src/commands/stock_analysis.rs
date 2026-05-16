use crate::AppState;
use axagent_agent::shared_blackboard::SharedBlackboard;
use axagent_core::entity::{
    analysis_schedules, portfolio_holdings, price_alerts, stock_analyses, trades, watchlist_items,
};
use axagent_core::types::ProviderProxyConfig;
use axagent_providers::{resolve_base_url_for_type, ProviderAdapter, ProviderRequestContext};
use axagent_stock_analysis::backtest::{
    BacktestEngine, BacktestResult, BacktestStats, HistoricalAnalysis,
};
use axagent_stock_analysis::decision::{AgentRunner, AnalysisConfig, AnalysisEvent};
use axagent_stock_analysis::key_levels::{KeyLevelBacktestStats, KeyLevelTracker};
use axagent_stock_analysis::monitor::MonitorConfig;
use axagent_stock_analysis::orchestrator::StockAnalysisOrchestrator;
use axagent_stock_analysis::plugin::AnalystPluginManager;
use axagent_stock_analysis::portfolio_risk::{PortfolioRiskManager, PortfolioRiskMetrics};
use axagent_stock_analysis::position_limits::PositionLimits;
use axagent_stock_analysis::review::{DailyReview, PostCloseReview};
use axagent_stock_analysis::screener::{ScreenCriteria, ScreenResult, StockScreener};
use axagent_stock_analysis::trading::{PositionSummary, TradePredictionComparison, TradingEngine};
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
    state: State<'_, AppState>,
    keyword: String,
) -> Result<Vec<axagent_astock_data::StockSearchResult>, String> {
    state
        .astock_client
        .search_stock(&keyword)
        .await
        .map_err(|e| e.to_string())
}

/// 获取实时行情
#[tauri::command]
pub async fn get_stock_quote(
    state: State<'_, AppState>,
    stock_code: String,
) -> Result<axagent_astock_data::StockQuote, String> {
    state
        .astock_client
        .get_quote(&stock_code)
        .await
        .map_err(|e| e.to_string())
}

/// 获取K线数据
#[tauri::command]
pub async fn get_stock_kline(
    state: State<'_, AppState>,
    stock_code: String,
    period: String,
    limit: u32,
) -> Result<Vec<axagent_astock_data::KLine>, String> {
    state
        .astock_client
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

    // 1. 获取股票名称（复用 AppState 单例，享受缓存）
    let quote = state
        .astock_client
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

    // 4. 创建取消令牌
    let cancel_token = Arc::new(AtomicBool::new(false));

    // 5. 构建带取消令牌的 AgentRunner
    let master_key = state.master_key;
    let db_for_runner = state.sea_db.clone();
    let provider_id_for_runner = provider_id.clone();

    let runner: Option<Arc<dyn AgentRunner>> = match build_cancel_aware_runner(
        &db_for_runner,
        &master_key,
        &provider_id_for_runner,
        cancel_token.clone(),
    )
    .await
    {
        Ok(r) => {
            tracing::info!(
                "[stock_analysis] CancelAwareRunner 已构建 (provider={})",
                provider_id_for_runner
            );
            Some(Arc::new(r))
        },
        Err(e) => {
            tracing::warn!("[stock_analysis] 无法构建 runner，使用占位报告: {}", e);
            None
        },
    };

    // 6. 从 DB 加载专家提示词（种子化后已有）
    let prompts = load_stock_analysis_prompts(&state.sea_db).await;

    // 6b. 注册取消令牌
    {
        let mut tokens = state.agent_cancel_tokens.lock().await;
        tokens.insert(analysis_id.clone(), cancel_token.clone());
    }

    // 7. spawn 异步分析任务
    let app_handle = app.clone();
    let analysis_id_clone = analysis_id.clone();
    let db = state.sea_db.clone();
    let stock_code_for_spawn = stock_code.clone();
    let stock_name_for_spawn = stock_name.clone();
    let cancel_tokens = state.agent_cancel_tokens.clone();
    let data_client = state.astock_client.clone(); // Arc 克隆，共享单例缓存

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
        let blackboard = Arc::new(RwLock::new(SharedBlackboard::new(
            &analysis_id_clone,
            format!("分析 {} ({})", stock_code_for_spawn, stock_name_for_spawn),
        )));

        let config = AnalysisConfig::default();

        let blackboard_for_run = blackboard.clone();
        let result = StockAnalysisOrchestrator::run(
            &data_client,
            blackboard_for_run,
            stock_code_for_spawn,
            stock_name_for_spawn,
            date,
            config,
            event_tx,
            runner,
            prompts,
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
                // 导出完整黑板快照供历史回看
                let snapshot =
                    axagent_stock_analysis::pipeline::export_blackboard_snapshot(&blackboard).await;
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
                    .col_expr(stock_analyses::Column::BlackboardSnapshot, Expr::value(&snapshot))
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

// ── Watchlist ──

/// 添加自选股
#[tauri::command]
pub async fn add_to_watchlist(
    state: State<'_, AppState>,
    stock_code: String,
    stock_name: String,
) -> Result<watchlist_items::Model, String> {
    let now = chrono::Utc::now().timestamp_millis();
    let model = watchlist_items::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        stock_code: Set(stock_code),
        stock_name: Set(stock_name),
        notes: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };
    model.insert(&state.sea_db).await.map_err(|e| e.to_string())
}

/// 移除自选股
#[tauri::command]
pub async fn remove_from_watchlist(state: State<'_, AppState>, id: String) -> Result<(), String> {
    watchlist_items::Entity::delete_by_id(id)
        .exec(&state.sea_db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 自选股列表
#[tauri::command]
pub async fn list_watchlist(
    state: State<'_, AppState>,
) -> Result<Vec<watchlist_items::Model>, String> {
    watchlist_items::Entity::find()
        .order_by_desc(watchlist_items::Column::CreatedAt)
        .all(&state.sea_db)
        .await
        .map_err(|e| e.to_string())
}

// ── Portfolio ──

/// 添加持仓
#[tauri::command]
pub async fn add_portfolio_holding(
    state: State<'_, AppState>,
    stock_code: String,
    stock_name: String,
    shares: f64,
    avg_cost: f64,
) -> Result<portfolio_holdings::Model, String> {
    let now = chrono::Utc::now().timestamp_millis();
    let model = portfolio_holdings::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        stock_code: Set(stock_code),
        stock_name: Set(stock_name),
        shares: Set(shares),
        avg_cost: Set(avg_cost),
        notes: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };
    model.insert(&state.sea_db).await.map_err(|e| e.to_string())
}

/// 更新持仓
#[tauri::command]
pub async fn update_portfolio_holding(
    state: State<'_, AppState>,
    id: String,
    shares: f64,
    avg_cost: f64,
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp_millis();
    portfolio_holdings::Entity::update_many()
        .col_expr(portfolio_holdings::Column::Shares, Expr::value(shares))
        .col_expr(portfolio_holdings::Column::AvgCost, Expr::value(avg_cost))
        .col_expr(portfolio_holdings::Column::UpdatedAt, Expr::value(now))
        .filter(portfolio_holdings::Column::Id.eq(id))
        .exec(&state.sea_db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 移除持仓
#[tauri::command]
pub async fn remove_portfolio_holding(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    portfolio_holdings::Entity::delete_by_id(id)
        .exec(&state.sea_db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 持仓列表（含实时盈亏）
#[tauri::command]
pub async fn list_portfolio(state: State<'_, AppState>) -> Result<Vec<serde_json::Value>, String> {
    let holdings = portfolio_holdings::Entity::find()
        .all(&state.sea_db)
        .await
        .map_err(|e| e.to_string())?;

    // 附加实时行情计算盈亏
    let mut enriched = Vec::new();
    for h in holdings {
        let quote = state.astock_client.get_quote(&h.stock_code).await.ok();
        let current_price = quote.as_ref().map(|q| q.price).unwrap_or(h.avg_cost);
        let market_value = current_price * h.shares;
        let cost_basis = h.avg_cost * h.shares;
        let pnl = market_value - cost_basis;
        let pnl_pct = if cost_basis != 0.0 {
            (pnl / cost_basis) * 100.0
        } else {
            0.0
        };

        enriched.push(serde_json::json!({
            "id": h.id,
            "stockCode": h.stock_code,
            "stockName": h.stock_name,
            "shares": h.shares,
            "avgCost": h.avg_cost,
            "currentPrice": current_price,
            "marketValue": market_value,
            "pnl": pnl,
            "pnlPct": pnl_pct,
            "notes": h.notes,
            "createdAt": h.created_at,
        }));
    }
    Ok(enriched)
}

// ── Helper: 构建 SessionManagerRunner ──

/// 构建带取消令牌的 AgentRunner（封装 SessionManagerRunner）。
/// 任何步骤失败都会返回 `Err`，调用方可回退到占位报告模式。
async fn build_cancel_aware_runner(
    db: &sea_orm::DatabaseConnection,
    master_key: &[u8; 32],
    provider_id: &str,
    cancel_token: Arc<AtomicBool>,
) -> Result<impl AgentRunner, String> {
    let prov = axagent_core::repo::provider::get_provider(db, provider_id)
        .await
        .map_err(|e| format!("Provider 查询失败: {}", e))?;
    if !prov.enabled {
        return Err("Provider 已禁用".into());
    }
    let key = prov
        .keys
        .iter()
        .find(|k| k.enabled)
        .ok_or_else(|| "没有启用的 API key".to_string())?;
    let api_key = axagent_core::crypto::decrypt_key(&key.key_encrypted, master_key)
        .map_err(|e| format!("密钥解密失败: {}", e))?;
    let settings = axagent_core::repo::settings::get_settings(db)
        .await
        .unwrap_or_default();
    let custom_headers: Option<std::collections::HashMap<String, String>> = prov
        .custom_headers
        .as_ref()
        .and_then(|s| serde_json::from_str(s).ok());
    let ctx = ProviderRequestContext {
        api_key,
        key_id: key.id.clone(),
        provider_id: prov.id.clone(),
        base_url: Some(resolve_base_url_for_type(&prov.api_host, &prov.provider_type)),
        api_path: prov.api_path.clone(),
        proxy_config: ProviderProxyConfig::resolve(&prov.proxy_config, &settings),
        custom_headers,
        api_mode: None,
        conversation: None,
        previous_response_id: None,
        store_response: None,
    };
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
    let model_id = prov
        .models
        .iter()
        .find(|m| m.enabled)
        .map(|m| m.model_id.clone())
        .ok_or_else(|| "没有可用的模型".to_string())?;
    let inner = axagent_stock_analysis::runner::SessionManagerRunner::new(adapter, ctx, model_id)
        .with_temperature(Some(0.3))
        .with_max_tokens(Some(4096));
    Ok(CancelAwareRunner {
        inner,
        token: cancel_token,
    })
}

/// 带取消令牌检查的 AgentRunner 包装
struct CancelAwareRunner {
    inner: axagent_stock_analysis::runner::SessionManagerRunner,
    token: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl AgentRunner for CancelAwareRunner {
    async fn run_agent(
        &self,
        expert_id: &str,
        sys_prompt: &str,
        user_prompt: &str,
    ) -> Result<String, String> {
        if self.token.load(Ordering::Relaxed) {
            return Err("已取消".into());
        }
        self.inner
            .run_agent(expert_id, sys_prompt, user_prompt)
            .await
    }
}

/// 从 DB 的 agency_experts 表加载股票分析专家系统提示词
async fn load_stock_analysis_prompts(
    db: &sea_orm::DatabaseConnection,
) -> std::collections::HashMap<String, String> {
    use axagent_core::entity::agency_experts;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    let mut prompts = std::collections::HashMap::new();
    let rows = match agency_experts::Entity::find()
        .filter(agency_experts::Column::SourceDir.eq("stock-analysis"))
        .all(db)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("[stock_analysis] DB 加载专家提示词失败: {e}");
            return prompts;
        },
    };
    for row in rows {
        let expert_id = row
            .id
            .strip_prefix("agency-stock-analysis-")
            .unwrap_or(&row.id);
        prompts.insert(expert_id.to_string(), row.system_prompt);
    }
    tracing::info!("[stock_analysis] 从 DB 加载了 {} 个专家提示词", prompts.len());
    prompts
}

// ── MCP Stock Data Tools ──

/// 返回 stock data MCP 工具定义列表（供前端 MCP 管理页面注册）
#[tauri::command]
pub async fn get_stock_mcp_tools() -> Result<Vec<serde_json::Value>, String> {
    Ok(axagent_astock_data::mcp_tools::stock_mcp_tools())
}

/// 执行 stock data MCP 工具调用
#[tauri::command]
pub async fn execute_stock_mcp_tool(
    state: State<'_, AppState>,
    tool_name: String,
    arguments: serde_json::Value,
) -> Result<String, String> {
    axagent_astock_data::mcp_tools::execute_mcp_tool(&state.astock_client, &tool_name, &arguments)
        .await
}

// ── Backtesting ──

/// 回测单个分析决策
#[tauri::command]
pub async fn backtest_analysis(
    state: State<'_, AppState>,
    stock_code: String,
    analysis_date: String,
    decision_action: String,
    decision_confidence: f64,
    holding_days: u32,
) -> Result<BacktestResult, String> {
    BacktestEngine::backtest_decision(
        &state.astock_client,
        &stock_code,
        &analysis_date,
        &decision_action,
        decision_confidence,
        holding_days,
    )
    .await
}

/// 批量回测历史分析（已完成的分析）
#[tauri::command]
pub async fn backtest_all_history(
    state: State<'_, AppState>,
    holding_days: u32,
) -> Result<BacktestStats, String> {
    let analyses = stock_analyses::Entity::find()
        .filter(stock_analyses::Column::Status.eq("completed"))
        .all(&state.sea_db)
        .await
        .map_err(|e| e.to_string())?;

    let historical: Vec<HistoricalAnalysis> = analyses
        .iter()
        .map(|a| HistoricalAnalysis {
            stock_code: a.stock_code.clone(),
            analysis_date: a.analysis_date.clone(),
            decision_action: a
                .decision_action
                .clone()
                .unwrap_or_else(|| "持有".to_string()),
            decision_confidence: a.decision_position_pct.map(|p| p / 100.0).unwrap_or(0.5),
        })
        .collect();

    let results =
        BacktestEngine::backtest_history(&state.astock_client, historical, holding_days).await?;
    let stats = BacktestEngine::compute_stats(&results);
    Ok(stats)
}

// ── Analysis Schedules ──

/// 创建定时分析计划
#[tauri::command]
pub async fn create_analysis_schedule(
    state: State<'_, AppState>,
    stock_code: String,
    stock_name: String,
    cron_expression: String,
    provider_id: String,
) -> Result<analysis_schedules::Model, String> {
    let now = chrono::Utc::now().timestamp_millis();
    let model = analysis_schedules::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        stock_code: Set(stock_code),
        stock_name: Set(stock_name),
        cron_expression: Set(cron_expression),
        provider_id: Set(provider_id),
        is_enabled: Set(true),
        last_run_at: Set(None),
        next_run_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };
    model.insert(&state.sea_db).await.map_err(|e| e.to_string())
}

/// 查询定时分析计划列表
#[tauri::command]
pub async fn list_analysis_schedules(
    state: State<'_, AppState>,
) -> Result<Vec<analysis_schedules::Model>, String> {
    analysis_schedules::Entity::find()
        .order_by_desc(analysis_schedules::Column::CreatedAt)
        .all(&state.sea_db)
        .await
        .map_err(|e| e.to_string())
}

/// 切换定时分析计划启用/禁用
#[tauri::command]
pub async fn toggle_analysis_schedule(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp_millis();
    analysis_schedules::Entity::update_many()
        .col_expr(analysis_schedules::Column::IsEnabled, Expr::value(enabled))
        .col_expr(analysis_schedules::Column::UpdatedAt, Expr::value(now))
        .filter(analysis_schedules::Column::Id.eq(id))
        .exec(&state.sea_db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 删除定时分析计划
#[tauri::command]
pub async fn delete_analysis_schedule(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    analysis_schedules::Entity::delete_by_id(id)
        .exec(&state.sea_db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ── Price Alerts ──

/// 创建价格告警
#[tauri::command]
pub async fn create_price_alert(
    state: State<'_, AppState>,
    stock_code: String,
    stock_name: String,
    condition: String,
    target_price: f64,
) -> Result<price_alerts::Model, String> {
    let now = chrono::Utc::now().timestamp_millis();
    let model = price_alerts::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        stock_code: Set(stock_code),
        stock_name: Set(stock_name),
        condition: Set(condition),
        target_price: Set(target_price),
        is_triggered: Set(false),
        triggered_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };
    model.insert(&state.sea_db).await.map_err(|e| e.to_string())
}

/// 查询价格告警列表
#[tauri::command]
pub async fn list_price_alerts(
    state: State<'_, AppState>,
) -> Result<Vec<price_alerts::Model>, String> {
    price_alerts::Entity::find()
        .order_by_desc(price_alerts::Column::CreatedAt)
        .all(&state.sea_db)
        .await
        .map_err(|e| e.to_string())
}

/// 删除价格告警
#[tauri::command]
pub async fn delete_price_alert(state: State<'_, AppState>, id: String) -> Result<(), String> {
    price_alerts::Entity::delete_by_id(id)
        .exec(&state.sea_db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ── 自定义分析师插件 ──

/// 列出所有自定义分析师插件
#[tauri::command]
pub async fn list_custom_analysts(
) -> Result<Vec<axagent_stock_analysis::plugin::CustomAnalyst>, String> {
    let mgr = AnalystPluginManager::new("agency_experts/stock-analysis");
    Ok(mgr.discover_custom_analysts())
}

/// 生成股票分析 HTML 报告
#[tauri::command]
pub async fn generate_stock_report(
    state: State<'_, AppState>,
    analysis_id: String,
) -> Result<String, String> {
    let record = stock_analyses::Entity::find_by_id(&analysis_id)
        .one(&state.sea_db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "分析记录不存在".to_string())?;

    // 生成报告路径
    let reports_dir = std::path::Path::new("reports");
    std::fs::create_dir_all(reports_dir).map_err(|e| e.to_string())?;

    let filename = format!("{}_{}.html", record.stock_code, record.analysis_date.replace('-', ""));
    let filepath = reports_dir.join(&filename);

    // 获取行情和K线数据
    let quote = state
        .astock_client
        .get_quote(&record.stock_code)
        .await
        .map_err(|e| format!("获取行情失败: {}", e))?;

    let klines = state
        .astock_client
        .get_klines(&record.stock_code, "daily", 120)
        .await
        .map_err(|e| format!("获取K线失败: {}", e))?;

    // 计算技术指标和客观评分
    let indicators =
        axagent_astock_data::indicators::compute_indicators(&record.stock_code, &klines);
    let score =
        axagent_stock_analysis::scoring::ScoringEngine::score(&indicators, quote.price, None);

    let quote_json = serde_json::to_string(&quote).unwrap_or_default();
    let score_json = serde_json::to_string(&score).unwrap_or_default();
    let decision_json = record.decision_json.clone().unwrap_or_default();

    // 从 blackboard_snapshot 尝试恢复分析师报告
    let analyst_reports: std::collections::HashMap<String, String> = record
        .blackboard_snapshot
        .as_ref()
        .and_then(|snap| serde_json::from_str(snap).ok())
        .unwrap_or_default();

    let html = axagent_stock_analysis::report::generate_html_report(
        &record.stock_code,
        &record.stock_name,
        &record.analysis_date,
        &quote_json,
        &indicators,
        &score_json,
        &analyst_reports,
        &decision_json,
        "",
        "",
    );

    std::fs::write(&filepath, &html).map_err(|e| e.to_string())?;

    Ok(filepath.to_string_lossy().to_string())
}

// ── 手动交易日志 ──

/// 记录一笔交易
#[tauri::command]
pub async fn record_trade(
    state: State<'_, AppState>,
    stock_code: String,
    stock_name: String,
    direction: String,
    price: f64,
    quantity: i32,
    trade_date: String,
    trade_time: String,
    notes: Option<String>,
) -> Result<trades::Model, String> {
    let engine = TradingEngine::new(Arc::new(state.sea_db.clone()), state.astock_client.clone());
    engine
        .execute_trade(
            &stock_code,
            &stock_name,
            &direction,
            price,
            quantity,
            &trade_date,
            &trade_time,
            notes.as_deref(),
        )
        .await
}

/// 获取交易历史
#[tauri::command]
pub async fn list_trades(
    state: State<'_, AppState>,
    stock_code: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<trades::Model>, String> {
    let engine = TradingEngine::new(Arc::new(state.sea_db.clone()), state.astock_client.clone());
    engine
        .get_trades(stock_code.as_deref(), limit.unwrap_or(50))
        .await
}

/// 获取持仓汇总（交易日志驱动的成本跟踪）
#[tauri::command]
pub async fn get_trade_positions(
    state: State<'_, AppState>,
) -> Result<Vec<PositionSummary>, String> {
    let engine = TradingEngine::new(Arc::new(state.sea_db.clone()), state.astock_client.clone());
    engine.get_positions().await
}

/// 开启 / 关闭交易功能
#[tauri::command]
pub async fn toggle_trading_enabled(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    tracing::info!("Trading system {}abled", if enabled { "en" } else { "dis" });
    axagent_core::repo::settings::set_setting(
        &state.sea_db,
        "trading_enabled",
        &enabled.to_string(),
    )
    .await
    .map_err(|e| e.to_string())
}

/// 校验交易（提交前预览）
#[tauri::command]
pub async fn validate_trade(
    state: State<'_, AppState>,
    stock_code: String,
    direction: String,
    quantity: i32,
    price: f64,
) -> Result<serde_json::Value, String> {
    let engine = TradingEngine::new(Arc::new(state.sea_db.clone()), state.astock_client.clone());
    let result = engine
        .validate_trade(&stock_code, &direction, quantity, price)
        .await;
    Ok(serde_json::json!({
        "valid": result.valid,
        "errors": result.errors,
        "warnings": result.warnings,
    }))
}

/// 对比实际交易出场价与最近分析预测价位
#[tauri::command]
pub async fn compare_trade_with_analysis(
    state: State<'_, AppState>,
    trade_id: String,
) -> Result<TradePredictionComparison, String> {
    let trade = trades::Entity::find_by_id(&trade_id)
        .one(&state.sea_db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "交易记录不存在".to_string())?;

    let engine = TradingEngine::new(Arc::new(state.sea_db.clone()), state.astock_client.clone());
    engine.compare_trade_vs_prediction(&trade).await
}

// ── Monitor Commands ──

/// 启动实时监控引擎
#[tauri::command]
pub async fn start_monitor(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(ref monitor) = state.stock_monitor {
        monitor.start().await;
        Ok(())
    } else {
        Err("监控引擎未初始化".to_string())
    }
}

/// 停止实时监控引擎
#[tauri::command]
pub async fn stop_monitor(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(ref monitor) = state.stock_monitor {
        monitor.stop().await;
        Ok(())
    } else {
        Err("监控引擎未初始化".to_string())
    }
}

/// 添加监控配置
#[tauri::command]
pub async fn add_monitor_config(
    state: State<'_, AppState>,
    config: MonitorConfig,
) -> Result<(), String> {
    if let Some(ref monitor) = state.stock_monitor {
        monitor.add_config(config).await;
        Ok(())
    } else {
        Err("监控引擎未初始化".to_string())
    }
}

/// 获取所有监控配置
#[tauri::command]
pub async fn list_monitor_configs(
    state: State<'_, AppState>,
) -> Result<Vec<MonitorConfig>, String> {
    if let Some(ref monitor) = state.stock_monitor {
        Ok(monitor.list_configs().await)
    } else {
        Err("监控引擎未初始化".to_string())
    }
}

// ── Key Levels Commands ──

/// 回测关键价位命中率
#[tauri::command]
pub async fn backtest_key_levels(
    state: State<'_, AppState>,
    lookback_days: u32,
) -> Result<KeyLevelBacktestStats, String> {
    let tracker = KeyLevelTracker::new(Arc::new(state.sea_db.clone()), state.astock_client.clone());
    tracker.backtest_key_levels(lookback_days).await
}

// ── Screen Commands ──

/// 从自选股中筛选
#[tauri::command]
pub async fn screen_stocks(
    state: State<'_, AppState>,
    criteria: ScreenCriteria,
) -> Result<Vec<ScreenResult>, String> {
    let watchlist: Vec<(String, String)> = axagent_core::entity::watchlist_items::Entity::find()
        .all(&state.sea_db)
        .await
        .map_err(|e| e.to_string())?
        .iter()
        .map(|w| (w.stock_code.clone(), w.stock_name.clone()))
        .collect();

    StockScreener::screen_watchlist(&state.astock_client, &watchlist, &criteria).await
}

/// 从全市场发现热门候选标的
#[tauri::command]
pub async fn discover_stock_candidates(
    state: State<'_, AppState>,
) -> Result<Vec<ScreenResult>, String> {
    StockScreener::discover_candidates(&state.astock_client).await
}

// ── Calendar Commands ──

/// 获取市场状态
#[tauri::command]
pub async fn get_market_status() -> Result<serde_json::Value, String> {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let date = chrono::NaiveDate::parse_from_str(&today, "%Y-%m-%d")
        .unwrap_or_else(|_| chrono::Utc::now().date_naive());
    Ok(serde_json::json!({
        "isTradingDay": axagent_astock_data::calendar::is_trading_day(&date),
        "isTradingTime": axagent_astock_data::calendar::is_trading_time(),
        "status": axagent_astock_data::calendar::next_trading_time_desc(),
    }))
}

/// 从东方财富 API 刷新交易日历
#[tauri::command]
pub async fn refresh_trading_calendar() -> Result<Vec<String>, String> {
    axagent_astock_data::calendar::fetch_holiday_calendar().await
}

// ── Review Commands ──

/// 生成每日收盘复盘报告
#[tauri::command]
pub async fn generate_daily_review(state: State<'_, AppState>) -> Result<DailyReview, String> {
    let watchlist: Vec<(String, String)> = axagent_core::entity::watchlist_items::Entity::find()
        .all(&state.sea_db)
        .await
        .map_err(|e| e.to_string())?
        .iter()
        .map(|w| (w.stock_code.clone(), w.stock_name.clone()))
        .collect();

    // 查询当日已触发的价格告警
    let triggered_alerts_result = price_alerts::Entity::find()
        .filter(price_alerts::Column::IsTriggered.eq(true))
        .all(&state.sea_db)
        .await;

    let mut triggered_alerts: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    if let Ok(alerts) = triggered_alerts_result {
        for alert in alerts {
            let desc = format!(
                "{}触发: 价格{}{:.2}(目标{:.2})",
                alert.condition,
                if alert.condition == "above" {
                    "≥"
                } else {
                    "≤"
                },
                state
                    .astock_client
                    .get_quote(&alert.stock_code)
                    .await
                    .map(|q| q.price)
                    .unwrap_or(0.0),
                alert.target_price
            );
            triggered_alerts
                .entry(alert.stock_code)
                .or_default()
                .push(desc);
        }
    }

    PostCloseReview::generate(&state.astock_client, &watchlist, &triggered_alerts).await
}

// ── Scoring Weights Optimization ──

/// 基于回测结果优化评分权重
#[tauri::command]
pub async fn optimize_scoring_weights(
    state: State<'_, AppState>,
) -> Result<axagent_stock_analysis::decision::ScoringWeights, String> {
    axagent_stock_analysis::backtest::optimize_weights(&state.astock_client, &state.sea_db).await
}

// ── Portfolio Risk ──

/// 获取组合风险指标
#[tauri::command]
pub async fn get_portfolio_risk(
    state: State<'_, AppState>,
) -> Result<PortfolioRiskMetrics, String> {
    let engine = TradingEngine::new(Arc::new(state.sea_db.clone()), state.astock_client.clone());
    let positions = engine.get_positions().await?;
    Ok(PortfolioRiskManager::compute_from_positions(&positions))
}

// ── Value Investing ──

/// 获取巴菲特式价值投资评估
#[tauri::command]
pub async fn get_value_assessment(
    state: State<'_, AppState>,
    stock_code: String,
) -> Result<axagent_stock_analysis::value::ValueAssessment, String> {
    let client = &state.astock_client;
    let quote = client
        .get_quote(&stock_code)
        .await
        .map_err(|e| e.to_string())?;
    let financials = client
        .get_financials(&stock_code)
        .await
        .map_err(|e| e.to_string())?;
    Ok(axagent_stock_analysis::value::ValueEngine::assess(
        quote.price,
        &financials,
        1_000_000_000.0,
    ))
}

/// 计算巴菲特式价值投资综合指标（DCF + F-Score + 护城河量化 + 安全边际 + 所有者收益）
#[tauri::command]
pub async fn compute_value_metrics(
    state: State<'_, AppState>,
    stock_code: String,
) -> Result<axagent_stock_analysis::value_investing::ValueMetrics, String> {
    let quote = state
        .astock_client
        .get_quote(&stock_code)
        .await
        .map_err(|e| e.to_string())?;
    let financials = state
        .astock_client
        .get_financials(&stock_code)
        .await
        .map_err(|e| e.to_string())?;
    let total_shares = quote.total_mv.map(|mv| {
        if quote.price > 0.0 {
            mv / quote.price / 1_0000_0000.0
        } else {
            1.0
        }
    });
    Ok(axagent_stock_analysis::value_investing::ValueInvestingEngine::compute(
        &stock_code,
        quote.price,
        total_shares,
        &financials,
        quote.pe,
        quote.pb,
    ))
}

// ── Position Limits ──

/// 获取全局仓位限制配置
#[tauri::command]
pub async fn get_position_limits() -> Result<PositionLimits, String> {
    Ok(PositionLimits::default())
}
