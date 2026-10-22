// SPDX-License-Identifier: AGPL-3.0-only

//! 编码段盲扫的唯一入口（手动命令 + 每日 cron 共用同一实现体）。
//!
//! 语义：**只登记代码到 `market_stock_universe`，不产事件**（PLAN C-1 / 妖-3）。
//! 每轮只扫 `1/UNIVERSE_SCAN_SHARDS` 的段（按日轮转），把请求量摊平；
//! 登记行 `origin='code_scan'`、`last_confirmed_at=0`（不算东财全量确认）。
//!
//! 为什么需要：候选宇宙覆盖率 11.5%（681 / 5924）⇒ 「未被发现的妖股」绝大多数连漏检事件
//! 都不会生成。`clist` 与板块枚举两条扩域路径已实测判废，段扫是剩下的唯一正门。
//! **不进交易日闸门**（清单扩充与今天开不开市无关），但**进 live 时间窗**（23:50–02:10 /
//! 17:50–18:10 要留给实时流水线，段扫会抢行情通道）。

use axagent_agent_macro::agent_command;
use axagent_astock_data::AStockClient;
use axagent_dao::DatabaseConnection;
use tauri::State;

use super::error::ErrorResponse;
use super::error_code::mover_recall_analysis as ec;
use super::time_travel_batch::in_live_window;
use crate::app_state::AppState;

/// 任务类型常量（与 `scheduled_task::BUILTIN_TASK_TYPES`、前端 `taskGrouping.ts` 两表逐字对齐）。
pub use crate::commands::constants::UNIVERSE_SCAN_TASK_TYPE;

/// 默认分片数：6 段 / 3 片 ⇒ 每天 2 段（约 4 000 个代码 ≈ 80 次批量请求），三天覆盖全表。
pub const UNIVERSE_SCAN_SHARDS: u32 = 3;

/// 单次批量请求的代码数（腾讯批量接口的常用上限量级）。
pub const UNIVERSE_SCAN_BATCH: usize = 50;

/// 扫描实现体：手动命令与 cron 共用（live 时间窗判据只写这一处）。
pub(crate) async fn run_universe_scan_inner(
    db: &DatabaseConnection,
    client: &AStockClient,
    shard: Option<u32>,
    shards: Option<u32>,
) -> Result<axagent_analysis_engine::universe_scan::ScanStats, String> {
    if in_live_window() {
        return Err(ErrorResponse::err_with_detail(
            ec::LIVE_WINDOW,
            "当前处于 live 流水线时间窗（23:50-02:10 / 17:50-18:10），段扫会与实时取数抢行情通道，请避开后再跑",
        ));
    }
    let shards = shards.unwrap_or(UNIVERSE_SCAN_SHARDS).clamp(1, 16);
    let shard = shard.unwrap_or_else(|| {
        axagent_analysis_engine::universe_scan::shard_for_day(
            chrono::Utc::now().timestamp() / 86_400,
            shards,
        )
    });
    let stats = axagent_analysis_engine::universe_scan::scan_segment_shard(
        db,
        client,
        shard,
        shards,
        UNIVERSE_SCAN_BATCH,
    )
    .await?;
    // 段扫后更新覆盖率读数：否则市场清单已扩到数千只，面板/门仍读到上一次 clist 的旧值。
    if let Err(e) = axagent_analysis_engine::market_close_store::record_universe_stats_for_scan(
        db,
        stats.registered,
        stats.failed_batches,
    )
    .await
    {
        tracing::warn!("[universe-scan] 覆盖率读数落库失败（不影响扫描结果）: {e}");
    }
    Ok(stats)
}

/// 手动发起一次编码段盲扫（`shard` 缺省按日轮转；`shards` 缺省 3）。
#[agent_command(
    domain = "finance",
    safety = Caution,
    call_mode = StateInput,
    description = "编码段盲扫：只登记市场清单，不产事件"
)]
#[tauri::command]
pub async fn run_universe_code_scan(
    state: State<'_, AppState>,
    shard: Option<u32>,
    shards: Option<u32>,
) -> Result<axagent_analysis_engine::universe_scan::ScanStats, String> {
    run_universe_scan_inner(state.harness.db(), &state.astock_client, shard, shards).await
}
