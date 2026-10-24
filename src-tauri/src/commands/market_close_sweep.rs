//! 全市场收盘快照与代码清单采集 —— 命令层薄封装
//! （`PLAN-mover-recall-attribution.md` Phase 1）。
//!
//! DB 落库全在 `axagent_analysis_engine::market_close_store`（分层护栏
//! `commands-no-direct-db` 禁止本目录直连库）；这里只做「把库里的事实喂进去、
//! 把结果按 Tauri 命令报出来」，不写任何查询与 upsert。

use serde::Serialize;

use axagent_agent_macro::agent_command;
pub use axagent_analysis_engine::market_close_store::{
    DEFAULT_BATCH_INTERVAL_MS, DEFAULT_BATCH_SIZE, UniverseReport,
};

use super::error::ErrorResponse;
use super::error_code::common as common_err;

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
    app_handle: tauri::AppHandle,
    trade_date: Option<String>,
) -> Result<crate::commands::background_tasks::TaskAccepted, String> {
    // 只允许最近一个**已收盘**交易日：盘中点一次会把半日价写成收盘价
    // （2026-10-23 12:19 实测：10-23 盘中价被写成 5,215 行"收盘"）。
    let now_local = chrono::Local::now();
    let target = axagent_astock_data::daily_snapshot::snapshot_target_date(&now_local);
    let date = match (trade_date, target) {
        (Some(d), Some(t)) => {
            let t = t.format("%Y-%m-%d").to_string();
            if d != t {
                return Err(ErrorResponse::new(common_err::INVALID_INPUT)
                    .with_detail(format!(
                        "收盘采集只支持最近一个已收盘交易日（{t}）；历史日期请用「补窗口历史」"
                    ))
                    .to_string());
            }
            d
        },
        (None, Some(t)) => t.format("%Y-%m-%d").to_string(),
        (Some(_), None) | (None, None) => {
            return Err(ErrorResponse::new(common_err::INVALID_INPUT)
                .with_detail(
                    "盘中不采集收盘：请 15:00 收盘后再点「补最新收盘」；历史日期请用「补窗口历史」"
                        .to_string(),
                )
                .to_string());
        },
    };
    // 命令本身只做登记与起任务；执行体在 spawn 里跑（130s 不冻 IPC，
    // 报告终态后落在 result_json，重载可回取）
    let db = state.harness.db().clone();
    let client = state.astock_client.clone();
    let title = format!("采集收盘快照 {date}");
    crate::commands::background_tasks::spawn_tracked(
        &app_handle,
        state.harness.db(),
        &title,
        "market-close-sweep",
        "手动触发全市场日收盘快照采集（cron 走同一 engine 实现）",
        async move {
            axagent_analysis_engine::market_close_store::run_close_sweep(
                &db,
                &client,
                &date,
                DEFAULT_BATCH_SIZE,
                DEFAULT_BATCH_INTERVAL_MS,
            )
            .await
        },
        |r| {
            format!(
                "落库 {} 行 / 停牌跳过 {} / 缺报价 {} / 失败批次 {}",
                r.written, r.skipped_suspended, r.missing_quotes, r.failed_batches
            )
        },
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
    /// 行情源申报的全市场总数（0 = 还没拿到）
    pub reported_total: i64,
    /// 最新收盘日的代码数（全 A 股样本的实际收盘覆盖）
    pub latest_close_codes: i64,
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
        reported_total: data.reported_total,
        latest_close_codes: data.latest_close_codes,
        close_dates: data.close_dates,
        rows_per_date: data.rows_per_date,
    })
}

/// 逐票历史补齐报告（阶段 0.3：全 A 股窗口数据）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryBackfillReport {
    pub requested: usize,
    pub codes: u32,
    pub rows: u32,
    /// 取到了日线但没有新增行的票数（可用日线不足 `target_days`，重复点也不会再涨）
    pub no_new: u32,
    /// 整条 vendor 链都回答「无此代码行情数据」的票数（未上市/已退市，重复点也一样）
    pub no_data: u32,
    pub failed: u32,
    pub target_days: u32,
    /// 首条失败原因（全失败时给面板成句；成功时为 None）
    pub error: Option<String>,
    /// 用户中途取消 ⇒ 剩余票没取；`codes`/`rows` 只统计已执行部分
    pub cancelled: bool,
}

/// 分批补齐清单里历史行数不足的票（默认 90 个交易日）。
///
/// 每次最多 `limit` 只（默认 500，上限 6000）；全市场 5,925 只约 12 批。
/// 幂等：已够 `target_days` 的票由 `codes_needing_history` 排除，重复调用只补缺口。
#[agent_command(
    domain = "finance",
    safety = Caution,
    call_mode = StateInput,
    description = "分批补齐全市场收盘历史"
)]
#[tauri::command]
pub async fn backfill_market_close_history(
    state: tauri::State<'_, crate::AppState>,
    app_handle: tauri::AppHandle,
    limit: Option<usize>,
) -> Result<crate::commands::background_tasks::TaskAccepted, String> {
    const TARGET_DAYS: u32 = 90;
    let limit = limit.unwrap_or(500).clamp(1, 6000);
    // 命令本身只做登记与起任务；12 批 × 限速间隔的 2 分钟执行体在 spawn 里跑
    let db = state.harness.db().clone();
    let client = state.astock_client.clone();
    crate::commands::background_tasks::spawn_tracked(
        &app_handle,
        state.harness.db(),
        "补齐全市场收盘历史",
        "market-close-backfill",
        "分批补齐清单中历史行数不足的票（幂等，重复点只补缺口）",
        async move {
            let entries = axagent_analysis_engine::market_close_store::codes_needing_history(
                &db,
                TARGET_DAYS,
                limit,
            )
            .await?;
            if entries.is_empty() {
                return Ok(HistoryBackfillReport {
                    requested: 0,
                    codes: 0,
                    rows: 0,
                    no_new: 0,
                    no_data: 0,
                    failed: 0,
                    target_days: TARGET_DAYS,
                    error: None,
                    cancelled: false,
                });
            }
            let requested = entries.len();
            let stats =
                axagent_analysis_engine::market_close_store::backfill_close_history_for_codes(
                    &db,
                    &client,
                    &entries,
                    TARGET_DAYS,
                    120,
                )
                .await?;
            Ok(HistoryBackfillReport {
                requested,
                codes: stats.codes,
                rows: stats.rows,
                no_new: stats.no_new,
                no_data: stats.no_data,
                failed: stats.failed,
                target_days: TARGET_DAYS,
                error: stats.first_error,
                cancelled: stats.cancelled,
            })
        },
        |r| {
            if r.requested == 0 {
                "无需补齐（全部票已达目标天数）".to_string()
            } else {
                format!(
                    "请求 {} 只 / 取到 {} 只 / 落 {} 行 / 无新增 {} / 无行情 {} / 失败 {}",
                    r.requested, r.codes, r.rows, r.no_new, r.no_data, r.failed
                )
            }
        },
    )
    .await
}
