use crate::AppState;
use axagent_core::entity::{
    portfolio_holdings, price_alerts, stock_analyses, trades, watchlist_items,
};
use axagent_stock_analysis::backtest::{
    BacktestEngine, BacktestResult, BacktestStats, HistoricalAnalysis,
};
use axagent_stock_analysis::key_levels::{KeyLevelBacktestStats, KeyLevelTracker};
use axagent_stock_analysis::plugin::AnalystPluginManager;
use axagent_stock_analysis::portfolio_risk::{PortfolioRiskManager, PortfolioRiskMetrics};
use axagent_stock_analysis::position_limits::PositionLimits;
use axagent_stock_analysis::recommender::{self, RecoResponse};
use axagent_stock_analysis::review::{DailyReview, PostCloseReview};
use axagent_stock_analysis::screener::{ScreenCriteria, ScreenResult, StockScreener};
use axagent_stock_analysis::trading::{PositionSummary, TradePredictionComparison};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tauri::State;

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
        .all(state.harness.db())
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
        .one(state.harness.db())
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
    model
        .insert(state.harness.db())
        .await
        .map_err(|e| e.to_string())
}

/// 移除自选股
#[tauri::command]
pub async fn remove_from_watchlist(state: State<'_, AppState>, id: String) -> Result<(), String> {
    watchlist_items::Entity::delete_by_id(id)
        .exec(state.harness.db())
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
        .all(state.harness.db())
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
    model
        .insert(state.harness.db())
        .await
        .map_err(|e| e.to_string())
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
        .exec(state.harness.db())
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
        .exec(state.harness.db())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 持仓列表（含实时盈亏）
#[tauri::command]
pub async fn list_portfolio(state: State<'_, AppState>) -> Result<Vec<serde_json::Value>, String> {
    let holdings = portfolio_holdings::Entity::find()
        .all(state.harness.db())
        .await
        .map_err(|e| e.to_string())?;

    let client = state.astock_client.clone();
    let codes: Vec<String> = holdings.iter().map(|h| h.stock_code.clone()).collect();
    let mut quote_tasks = tokio::task::JoinSet::new();
    for code in codes {
        let c = client.clone();
        quote_tasks.spawn(async move {
            let quote = c.get_quote(&code).await.ok();
            (code, quote)
        });
    }
    let mut quotes = std::collections::HashMap::new();
    while let Some(result) = quote_tasks.join_next().await {
        if let Ok((code, quote)) = result {
            quotes.insert(code, quote);
        }
    }

    let enriched: Vec<serde_json::Value> = holdings
        .into_iter()
        .map(|h| {
            let quote = quotes.get(&h.stock_code).and_then(|q| q.as_ref());
            let current_price = quote.map(|q| q.price).unwrap_or(h.avg_cost);
            let market_value = current_price * h.shares;
            let cost_basis = h.avg_cost * h.shares;
            let pnl = market_value - cost_basis;
            let pnl_pct = if cost_basis != 0.0 {
                (pnl / cost_basis) * 100.0
            } else {
                0.0
            };

            serde_json::json!({
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
            })
        })
        .collect();
    Ok(enriched)
}

/// 从 settings 表加载估值参数（ValueConfig），仅提取需要的部分
async fn load_value_config(
    db: &sea_orm::DatabaseConnection,
) -> axagent_stock_analysis::decision::ValueConfig {
    if let Ok(Some(v)) =
        axagent_core::repo::settings::get_setting(db, "stock_analysis_config").await
    {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&v) {
            if let Some(value_section) = parsed.get("value") {
                if let Ok(cfg) = serde_json::from_value::<
                    axagent_stock_analysis::decision::ValueConfig,
                >(value_section.clone())
                {
                    return cfg;
                }
            }
        }
    }
    axagent_stock_analysis::decision::ValueConfig::default()
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
        .all(state.harness.db())
        .await
        .map_err(|e| e.to_string())?;

    let historical: Vec<HistoricalAnalysis> = analyses
        .iter()
        .map(|a| {
            let confidence = a
                .decision_json
                .as_ref()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                .and_then(|v| v.get("confidence").and_then(|c| c.as_f64()))
                .unwrap_or(0.5);
            HistoricalAnalysis {
                stock_code: a.stock_code.clone(),
                analysis_date: a.analysis_date.clone(),
                decision_action: a
                    .decision_action
                    .clone()
                    .unwrap_or_else(|| "持有".to_string()),
                decision_confidence: confidence,
            }
        })
        .collect();

    let results =
        BacktestEngine::backtest_history(&state.astock_client, historical, holding_days).await?;
    let stats = BacktestEngine::compute_stats(&results);
    Ok(stats)
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
    model
        .insert(state.harness.db())
        .await
        .map_err(|e| e.to_string())
}

/// 查询价格告警列表
#[tauri::command]
pub async fn list_price_alerts(
    state: State<'_, AppState>,
) -> Result<Vec<price_alerts::Model>, String> {
    price_alerts::Entity::find()
        .order_by_desc(price_alerts::Column::CreatedAt)
        .all(state.harness.db())
        .await
        .map_err(|e| e.to_string())
}

/// 删除价格告警
#[tauri::command]
pub async fn delete_price_alert(state: State<'_, AppState>, id: String) -> Result<(), String> {
    price_alerts::Entity::delete_by_id(id)
        .exec(state.harness.db())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ── 自定义分析师插件 ──

/// 列出所有自定义分析师插件
#[tauri::command]
pub async fn list_custom_analysts()
-> Result<Vec<axagent_stock_analysis::plugin::CustomAnalyst>, String> {
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
        .one(state.harness.db())
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "分析记录不存在".to_string())?;

    // 生成报告路径
    let reports_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("AxInvest")
        .join("reports");
    std::fs::create_dir_all(&reports_dir).map_err(|e| e.to_string())?;

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
    let mut score =
        axagent_stock_analysis::scoring::ScoringEngine::score(&indicators, quote.price, None);
    let pe = quote.pe;
    let pb = quote.pb;
    let roe = state
        .astock_client
        .get_financials(&record.stock_code)
        .await
        .ok()
        .and_then(|f| f.first().and_then(|r| r.roe));
    axagent_stock_analysis::scoring::ScoringEngine::apply_fundamental_adjustment(
        &mut score, pe, pb, roe,
    );
    axagent_stock_analysis::scoring::ScoringEngine::apply_industry_adjustment(
        &mut score, pe, None, pb, None,
    );

    let quote_json = serde_json::to_string(&quote).unwrap_or_default();
    let score_json = serde_json::to_string(&score).unwrap_or_default();
    let decision_json = record.decision_json.clone().unwrap_or_default();

    // 从 blackboard_snapshot 恢复分析师报告（仅提取 report.* 条目）
    // 注：snapshot 是 JSON 对象，value 可能是字符串（来自工作流结果）或嵌套对象
    // （来自 key_levels API 追加），用 Value 解析兼容两种情况。
    let bb_value: serde_json::Value = record
        .blackboard_snapshot
        .as_ref()
        .and_then(|snap| serde_json::from_str(snap).ok())
        .unwrap_or(serde_json::Value::Object(Default::default()));

    // 辅助：从 Value 中取字符串（空值视为缺失）
    let bb_str = |k: &str| -> String {
        bb_value
            .get(k)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };

    // 分析师报告：所有 report.* 前缀的键
    let analyst_reports: std::collections::HashMap<String, String> = bb_value
        .as_object()
        .map(|obj| {
            obj.iter()
                .filter(|(k, _)| k.starts_with("report."))
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    let value_assessment_json = bb_str("value.assessment");

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
        &value_assessment_json,
        &bb_str("raw.block_trades"),
        &bb_str("raw.institutional_visits"),
        &bb_str("market.index_quotes"),
        &bb_str("raw.peers"),
        &bb_str("raw.option_pcr"),
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
    let engine = state.trading_engine.read().await;
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
    let engine = state.trading_engine.read().await;
    engine
        .get_trades(stock_code.as_deref(), limit.unwrap_or(50))
        .await
}

/// 获取持仓汇总（交易日志驱动的成本跟踪）
#[tauri::command]
pub async fn get_trade_positions(
    state: State<'_, AppState>,
) -> Result<Vec<PositionSummary>, String> {
    let engine = state.trading_engine.read().await;
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
        state.harness.db(),
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
    let engine = state.trading_engine.read().await;
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
        .one(state.harness.db())
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "交易记录不存在".to_string())?;

    let engine = state.trading_engine.read().await;
    engine.compare_trade_vs_prediction(&trade).await
}

// ── Key Levels Commands ──

/// 回测关键价位命中率
#[tauri::command]
pub async fn backtest_key_levels(
    state: State<'_, AppState>,
    lookback_days: u32,
) -> Result<KeyLevelBacktestStats, String> {
    let tracker =
        KeyLevelTracker::new(Arc::new(state.harness.db().clone()), state.astock_client.clone());
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
        .all(state.harness.db())
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
        .all(state.harness.db())
        .await
        .map_err(|e| e.to_string())?
        .iter()
        .map(|w| (w.stock_code.clone(), w.stock_name.clone()))
        .collect();

    // 查询当日已触发的价格告警
    let triggered_alerts_result = price_alerts::Entity::find()
        .filter(price_alerts::Column::IsTriggered.eq(true))
        .all(state.harness.db())
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
    axagent_stock_analysis::backtest::optimize_weights(&state.astock_client, state.harness.db())
        .await
}

// ── Portfolio Risk ──

/// 获取组合风险指标
#[tauri::command]
pub async fn get_portfolio_risk(
    state: State<'_, AppState>,
) -> Result<PortfolioRiskMetrics, String> {
    let engine = state.trading_engine.read().await;
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
    let shares = quote.total_mv.and_then(|mv| {
        if quote.price > 0.0 {
            Some(mv / quote.price / 1_0000_0000.0)
        } else {
            None
        }
    });
    let value_config = load_value_config(state.harness.db()).await;
    Ok(match shares {
        Some(s) if s > 0.0 => axagent_stock_analysis::value::ValueEngine::assess(
            quote.price,
            &financials,
            s,
            Some(&value_config),
        ),
        _ => axagent_stock_analysis::value::ValueEngine::assess_no_shares(
            quote.price,
            &financials,
            Some(&value_config),
        ),
    })
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
    let total_shares = quote.total_mv.and_then(|mv| {
        if quote.price > 0.0 {
            Some(mv / quote.price / 1_0000_0000.0)
        } else {
            None
        }
    });
    let value_config = load_value_config(state.harness.db()).await;
    Ok(axagent_stock_analysis::value_investing::ValueInvestingEngine::compute(
        &stock_code,
        quote.price,
        total_shares,
        &financials,
        quote.pe,
        quote.pb,
        Some(&value_config),
    ))
}

// ── Position Limits ──

/// 获取全局仓位限制配置
#[tauri::command]
pub async fn get_position_limits() -> Result<PositionLimits, String> {
    Ok(PositionLimits::default())
}

// ── 新增数据源命令 ──

#[tauri::command]
pub async fn get_stock_research_reports(
    state: State<'_, AppState>,
    stock_code: String,
) -> Result<Vec<axagent_astock_data::ResearchReport>, String> {
    state
        .astock_client
        .get_research_reports(&stock_code)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_stock_consensus_eps(
    state: State<'_, AppState>,
    stock_code: String,
) -> Result<Option<axagent_astock_data::ConsensusEPS>, String> {
    state
        .astock_client
        .get_consensus_eps(&stock_code)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_stock_concept_blocks(
    state: State<'_, AppState>,
    stock_code: String,
) -> Result<Option<axagent_astock_data::ConceptBlocks>, String> {
    state
        .astock_client
        .get_concept_blocks(&stock_code)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_stock_announcements(
    state: State<'_, AppState>,
    stock_code: String,
) -> Result<Vec<axagent_astock_data::Announcement>, String> {
    state
        .astock_client
        .get_announcements(&stock_code)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_hot_stocks(
    state: State<'_, AppState>,
) -> Result<Vec<axagent_astock_data::HotStock>, String> {
    state
        .astock_client
        .get_hot_stocks()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_industry_ranking(
    state: State<'_, AppState>,
) -> Result<Vec<axagent_astock_data::IndustryRank>, String> {
    state
        .astock_client
        .get_industry_ranking()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_cls_flash(
    state: State<'_, AppState>,
) -> Result<Vec<axagent_astock_data::ClsFlashItem>, String> {
    state
        .astock_client
        .get_cls_flash()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_market_dragon_tiger(
    state: State<'_, AppState>,
) -> Result<Vec<axagent_astock_data::MarketDragonTiger>, String> {
    state
        .astock_client
        .get_market_dragon_tiger()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_north_bound_flow(
    state: State<'_, AppState>,
) -> Result<Option<axagent_astock_data::NorthBoundFlow>, String> {
    state
        .astock_client
        .get_north_bound_flow()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_index_quotes(
    state: State<'_, AppState>,
) -> Result<Vec<axagent_astock_data::IndexQuote>, String> {
    state
        .astock_client
        .get_index_quotes()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_stock_peers(
    state: State<'_, AppState>,
    stock_code: String,
) -> Result<Vec<axagent_astock_data::PeerComparison>, String> {
    state
        .astock_client
        .get_peers(&stock_code)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_stock_option_pcr(
    state: State<'_, AppState>,
    stock_code: String,
) -> Result<Option<axagent_astock_data::OptionPCR>, String> {
    state
        .astock_client
        .get_option_pcr(&stock_code)
        .await
        .map_err(|e| e.to_string())
}

// ── CronJob 定时任务（基于上游 CronJobStore + 持久化）──

use axagent_runtime_core::{CronJob, CronJobStatus};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CronJobResponse {
    id: String,
    name: String,
    description: String,
    schedule: String,
    status: String,
    recurring: bool,
    run_count: u32,
    last_run_at: Option<i64>,
    next_run_at: Option<i64>,
}

impl From<&CronJob> for CronJobResponse {
    fn from(j: &CronJob) -> Self {
        Self {
            id: j.id.clone(),
            name: j.name.clone(),
            description: j.description.clone(),
            schedule: j.schedule.clone(),
            status: format!("{:?}", j.status).to_lowercase(),
            recurring: j.recurring,
            run_count: j.run_count,
            last_run_at: j.last_run_at,
            next_run_at: j.next_run_at,
        }
    }
}

/// 创建股票定时分析任务
#[tauri::command]
pub async fn create_stock_cron(
    state: State<'_, AppState>,
    stock_code: String,
    stock_name: String,
    cron_expression: String,
) -> Result<CronJobResponse, String> {
    let id = format!(
        "stock-{}-{}",
        stock_code,
        uuid::Uuid::new_v4()
            .to_string()
            .split('-')
            .next()
            .unwrap_or("x")
    );
    let prompt = format!("对 {} ({}) 执行完整股票分析", stock_code, stock_name);
    let desc = format!("定时分析 {}", stock_code);
    let job = CronJob::new(&id, &cron_expression, &prompt, &desc)
        .with_workflow_id("stock-analysis".to_string())
        .with_task_type("stock-analysis");
    state.cron_job_store.add(job.clone()).await;
    Ok(CronJobResponse::from(&job))
}

/// 列出所有股票定时分析任务
#[tauri::command]
pub async fn list_stock_crons(state: State<'_, AppState>) -> Result<Vec<CronJobResponse>, String> {
    let jobs = state.cron_job_store.list().await;
    Ok(jobs
        .iter()
        .filter(|j| j.task_type.as_deref() == Some("stock-analysis"))
        .map(CronJobResponse::from)
        .collect())
}

/// 启停定时任务
#[tauri::command]
pub async fn toggle_stock_cron(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    state
        .cron_job_store
        .set_status(
            &id,
            if enabled {
                CronJobStatus::Active
            } else {
                CronJobStatus::Paused
            },
        )
        .await;
    Ok(())
}

/// 删除定时任务
#[tauri::command]
pub async fn delete_stock_cron(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.cron_job_store.remove(&id).await;
    Ok(())
}

/// 检查指定数据源的连接可用性
#[tauri::command]
pub async fn check_vendor_health(state: State<'_, AppState>, vendor: String) -> Result<(), String> {
    state
        .astock_client
        .check_vendor_health(&vendor)
        .await
        .map_err(|e| e.to_string())
}

/// 拉取智能荐股结果（按周期）
///
/// 前端传 period 序列化为 [Period] 枚举（"short" | "mid" | "long"）
/// 响应见 [RecoResponse]
#[tauri::command]
pub async fn recommend_stocks(
    state: State<'_, AppState>,
    period: axagent_stock_analysis::recommender::Period,
) -> Result<RecoResponse, String> {
    // 读取 workflow template 变量用于 vendor 启用检测
    let template = axagent_core::entity::workflow_template::Entity::find_by_id("stock-analysis")
        .one(state.harness.db())
        .await
        .map_err(|e| e.to_string())?;

    let vars: Vec<(String, serde_json::Value)> = match template {
        Some(t) => extract_template_vars(&t),
        None => Vec::new(),
    };

    // state.astock_client 已是 Arc<AStockClient>，直接 clone Arc 即可
    let client: std::sync::Arc<_> = state.astock_client.clone();
    recommender::recommend_stocks(client, period, &vars).await
}

/// 失效荐股缓存（设置页保存 vendor 后由前端调用）
#[tauri::command]
pub fn invalidate_recommendation_cache() {
    recommender::invalidate_cache();
}

/// 从 workflow_template 实体提取 (name, value) 列表
fn extract_template_vars(
    t: &axagent_core::entity::workflow_template::Model,
) -> Vec<(String, serde_json::Value)> {
    use axagent_harness::workflow_types::Variable;
    let raw = match t.variables.as_ref() {
        Some(s) => s,
        None => return Vec::new(),
    };
    match serde_json::from_str::<Vec<Variable>>(raw) {
        Ok(vs) => vs.into_iter().map(|v| (v.name, v.value)).collect(),
        Err(_) => Vec::new(),
    }
}
