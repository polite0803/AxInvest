// SPDX-License-Identifier: AGPL-3.0-only

//! M3（PLAN-stock-decision-hitrate-validation）：命中率统计只读命令 `reflection_stats`。
//!
//! 取数 + 展开 + 聚合的唯一实现住在
//! [`axagent_analysis_engine::reflection_stats::build_hitrate_stats`] ——
//! `stock_workflow/hooks.rs` 的先验注入与 `stock_analysis.rs` 的荐股先验共用它。
//! 放命令层会导致后两处跨模块调命令（分层门禁规则 2 `commands-no-sibling-call`），
//! 放 engine 后三处都只是调库函数，依赖方向保持 wiring → implementor。

use tauri::State;

use axagent_analysis_engine::reflection_stats::{HitrateStats, build_hitrate_stats};

use crate::AppState;

/// 命中率统计（只读命令）：取数与聚合的唯一实现见
/// [`axagent_analysis_engine::reflection_stats::build_hitrate_stats`]。
#[tauri::command]
pub async fn reflection_stats(state: State<'_, AppState>) -> Result<HitrateStats, String> {
    build_hitrate_stats(state.harness.db()).await
}
