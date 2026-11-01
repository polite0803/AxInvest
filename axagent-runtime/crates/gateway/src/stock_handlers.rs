// SPDX-License-Identifier: AGPL-3.0-only

//! Gateway HTTP API for Stock Analysis
//!
//! 对外暴露股票数据查询与分析记录接口，供外部脚本调用。
//!
//! 接缝架构（消除 gateway → axagent-entities / astock-data 直接依赖）：
//! - 行情查询（search/quote/kline）：`GatewayAppState.market_data`（harness
//!   `MarketDataProvider` trait，实现方 = astock-data `AStockClient`，wiring 注入）；
//!   未注入返回 503。
//! - 分析记录 / 自选股 CRUD：`GatewayAppState.stock_store`（harness `StockStore`
//!   trait，实现方 = 主 crate `DaoStockStore`，entities 后端）；未注入返回 503。
//!
//! 注：不提供 start_analysis 端点——旧实现写入的 status="submitted" 行在现行
//! 架构（stock_workflow / stock_analysis 命令创建分析）中零消费方，属于造死数据。

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;

use crate::server::GatewayAppState;

// ── Query / Payload types ──

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub keyword: String,
}

#[derive(Debug, Deserialize)]
pub struct QuoteQuery {
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct KlineQuery {
    pub code: String,
    #[serde(default = "default_period")]
    pub period: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_period() -> String {
    "daily".to_string()
}

fn default_limit() -> u32 {
    120
}

#[derive(Debug, Deserialize)]
pub struct AnalysisListQuery {
    #[serde(default = "default_20")]
    pub limit: u64,
    #[serde(default)]
    pub offset: u64,
}

fn default_20() -> u64 {
    20
}

#[derive(Debug, Deserialize)]
pub struct WatchlistAddRequest {
    pub stock_code: String,
    pub stock_name: String,
}

// ── Helpers ──

fn ok_json<T: serde::Serialize>(data: T) -> Response {
    Json(serde_json::json!({ "data": data })).into_response()
}

fn error_json(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

/// 行情接缝未注入时的统一 503 兜底
fn seam_unavailable(seam: &str) -> Response {
    error_json(
        StatusCode::SERVICE_UNAVAILABLE,
        &format!("{seam} 接缝未注入（网关启动时未提供该能力）"),
    )
}

// ── Handlers ──

/// GET /api/stock/search?keyword=茅台
pub async fn search_stock(
    State(state): State<GatewayAppState>,
    Query(q): Query<SearchQuery>,
) -> Response {
    let Some(market_data) = &state.market_data else {
        return seam_unavailable("market_data");
    };
    match market_data.search_stock(&q.keyword).await {
        Ok(results) => ok_json(results),
        Err(e) => error_json(StatusCode::BAD_GATEWAY, &e.to_string()),
    }
}

/// GET /api/stock/quote?code=600519
pub async fn get_quote(
    State(state): State<GatewayAppState>,
    Query(q): Query<QuoteQuery>,
) -> Response {
    let Some(market_data) = &state.market_data else {
        return seam_unavailable("market_data");
    };
    match market_data.get_quote(&q.code).await {
        Ok(quote) => ok_json(quote),
        Err(e) => error_json(StatusCode::BAD_GATEWAY, &e.to_string()),
    }
}

/// GET /api/stock/kline?code=600519&period=daily&limit=120
pub async fn get_kline(
    State(state): State<GatewayAppState>,
    Query(q): Query<KlineQuery>,
) -> Response {
    let Some(market_data) = &state.market_data else {
        return seam_unavailable("market_data");
    };
    match market_data.get_klines(&q.code, &q.period, q.limit, None).await {
        Ok(klines) => ok_json(klines),
        Err(e) => error_json(StatusCode::BAD_GATEWAY, &e.to_string()),
    }
}

/// GET /api/stock/analysis/:id — 获取单个分析详情
pub async fn get_analysis(
    State(state): State<GatewayAppState>,
    Path(analysis_id): Path<String>,
) -> Response {
    let Some(stock_store) = &state.stock_store else {
        return seam_unavailable("stock_store");
    };
    match stock_store.get_analysis(&analysis_id).await {
        Ok(Some(record)) => ok_json(record),
        Ok(None) => error_json(StatusCode::NOT_FOUND, &format!("分析记录不存在: {}", analysis_id)),
        Err(e) => error_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

/// GET /api/stock/analyses?limit=20&offset=0 — 分析记录列表
pub async fn list_analyses(
    State(state): State<GatewayAppState>,
    Query(q): Query<AnalysisListQuery>,
) -> Response {
    let Some(stock_store) = &state.stock_store else {
        return seam_unavailable("stock_store");
    };
    match stock_store.list_analyses(q.limit, q.offset).await {
        Ok(records) => ok_json(records),
        Err(e) => error_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

/// GET /api/stock/watchlist — 自选股列表
pub async fn get_watchlist(State(state): State<GatewayAppState>) -> Response {
    let Some(stock_store) = &state.stock_store else {
        return seam_unavailable("stock_store");
    };
    match stock_store.list_watchlist().await {
        Ok(items) => ok_json(items),
        Err(e) => error_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

/// POST /api/stock/watchlist — 添加自选股
pub async fn add_watchlist(
    State(state): State<GatewayAppState>,
    Json(req): Json<WatchlistAddRequest>,
) -> Response {
    let Some(stock_store) = &state.stock_store else {
        return seam_unavailable("stock_store");
    };
    match stock_store.add_watchlist(&req.stock_code, &req.stock_name).await {
        Ok(record) => {
            (StatusCode::CREATED, Json(serde_json::json!({ "data": record }))).into_response()
        },
        Err(e) => error_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

/// DELETE /api/stock/watchlist/:id — 移除自选股
pub async fn delete_watchlist(
    State(state): State<GatewayAppState>,
    Path(id): Path<String>,
) -> Response {
    let Some(stock_store) = &state.stock_store else {
        return seam_unavailable("stock_store");
    };
    match stock_store.delete_watchlist(&id).await {
        Ok(true) => ok_json(serde_json::json!({ "deleted": true })),
        Ok(false) => error_json(StatusCode::NOT_FOUND, &format!("自选股记录不存在: {}", id)),
        Err(e) => error_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}
