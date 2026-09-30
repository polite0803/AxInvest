//! 全市场收盘快照与代码清单采集 —— 命令层薄封装
//! （`PLAN-mover-recall-attribution.md` Phase 1）。
//!
//! DB 落库全在 `axagent_analysis_engine::market_close_store`（分层护栏
//! `commands-no-direct-db` 禁止本目录直连库）；这里只做「把库里的事实喂进去、
//! 把结果按 Tauri 命令报出来」，不写任何查询与 upsert。

use serde::Serialize;

use axagent_agent_macro::agent_command;
pub use axagent_analysis_engine::market_close_store::{
    CloseSweepReport, DEFAULT_BATCH_INTERVAL_MS, DEFAULT_BATCH_SIZE, UniverseReport,
};

/// 手动触发一轮收盘采集（cron 走同一 engine 实现）。
#[agent_command(
    domain = "finance",
    safety = Caution,
    call_mode = StateInput,
    description = "采集全市场日收盘快照"
)]
#[tauri::command]
pub async fn sweep_market_close(
    state: tauri::State<'_, crate::AppState>,
    trade_date: Option<String>,
) -> Result<CloseSweepReport, String> {
    let date = trade_date.unwrap_or_else(|| {
        axagent_astock_data::calendar::previous_trading_day(chrono::Local::now().date_naive())
            .format("%Y-%m-%d")
            .to_string()
    });
    let db = state.harness.db();
    axagent_analysis_engine::market_close_store::run_close_sweep(
        db,
        &state.astock_client,
        &date,
        DEFAULT_BATCH_SIZE,
        DEFAULT_BATCH_INTERVAL_MS,
    )
    .await
}

/// 分 tick 扩股票清单（每次若干页，避免连爬触发封禁）。
#[agent_command(
    domain = "finance",
    safety = Caution,
    call_mode = StateInput,
    description = "扩全市场股票清单"
)]
#[tauri::command]
pub async fn refresh_stock_universe(
    state: tauri::State<'_, crate::AppState>,
    pages: Vec<u32>,
) -> Result<UniverseReport, String> {
    let db = state.harness.db();
    axagent_analysis_engine::market_close_store::refresh_universe_pages(
        db,
        &state.astock_client,
        &pages,
    )
    .await
}

/// 清单与快照的覆盖状况（面板据此显式声明枚举域是否完整，禁止把「没采到」演成「没涨」）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageView {
    pub universe_size: i64,
    /// 清单里被 clist 全量确认过的票数（低于 universe_size ⇒ 枚举域由本地票拼出，不完整）
    pub confirmed_by_clist: i64,
    pub close_dates: Vec<String>,
    pub rows_per_date: Vec<i64>,
}

#[agent_command(domain = "finance", safety = Safe, call_mode = StateInput, description = "市场快照覆盖状况")]
#[tauri::command]
pub async fn get_market_coverage(
    state: tauri::State<'_, crate::AppState>,
) -> Result<CoverageView, String> {
    let data =
        axagent_analysis_engine::market_close_store::load_market_coverage(state.harness.db())
            .await?;
    Ok(CoverageView {
        universe_size: data.universe_size,
        confirmed_by_clist: data.confirmed_by_clist,
        close_dates: data.close_dates,
        rows_per_date: data.rows_per_date,
    })
}
