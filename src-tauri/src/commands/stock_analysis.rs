use crate::AppState;
use axagent_agent::shared_blackboard::SharedBlackboard;
use axagent_core::entity::{
    portfolio_holdings, price_alerts, stock_analyses, trades, watchlist_items,
};
use axagent_core::types::ProviderProxyConfig;
use axagent_providers::{ProviderAdapter, ProviderRequestContext, resolve_base_url_for_type};
use axagent_stock_analysis::backtest::{
    BacktestEngine, BacktestResult, BacktestStats, HistoricalAnalysis,
};
use axagent_stock_analysis::decision::{AgentRunner, AnalysisEvent, StockAnalysisFullConfig};
use axagent_stock_analysis::key_levels::{KeyLevelBacktestStats, KeyLevelTracker};
use axagent_stock_analysis::orchestrator::StockAnalysisOrchestrator;
use axagent_stock_analysis::plugin::AnalystPluginManager;
use axagent_stock_analysis::portfolio_risk::{PortfolioRiskManager, PortfolioRiskMetrics};
use axagent_stock_analysis::position_limits::PositionLimits;
use axagent_stock_analysis::review::{DailyReview, PostCloseReview};
use axagent_stock_analysis::screener::{ScreenCriteria, ScreenResult, StockScreener};
use axagent_stock_analysis::trading::{PositionSummary, TradePredictionComparison};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, Manager, State};
use tokio::sync::RwLock;
use zeroize::Zeroizing;

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
