// SPDX-License-Identifier: AGPL-3.0-only
//! G4 市场主线自动提炼 Tauri 命令层
//!
//! 委托 `axagent_analysis_engine::market_mainline`（crate 层实现，sea-orm 读写）。
//! 本层**不重复实现**任何业务逻辑：只做「参数解包 → 调 crate → 结果序列化」。
//!
//! # 历史订正（2026-09-20）
//!
//! 本文件此前是 **10 个 `Err("功能已移除")` 存根**，头注释称 crate 层模块
//! 「已删除」。该说法与事实不符：`crates/analysis-engine/src/market_mainline.rs`
//! 及其 `lib.rs` 的 `pub mod market_mainline;` 一直完好，`batch_upsert_mainlines` /
//! `list_mainlines_by_date` 等实现齐全 —— 是上游合并时**命令层存根没跟着更新**。
//!
//! 代价是**全链不可达**：命令恒返 Err ⇒ 前端与 chat 侧的
//! `execute_tauri_command` 均拿不到数据；而三处声明（`seed_daily_market_events.rs`
//! 的模板提示词与工具表、`market-synthesizer.md`、`skills/market-mainline/SKILL.md`）
//! 仍在指示模型去用这套工具 ⇒ 声明 ≠ 实际。
//!
//! # ⚠ 两条通道，别只修一条
//!
//! 工作流 Agent 节点的工具解析**不走**本模块。`init/services.rs` 注入的
//! `ToolResolver` 的判据空间是 `axagent_tools::tools::register_all` 注册的工具集
//! ∪ MCP 工具表 —— `#[agent_command]` 元数据**不在其中**（它只喂
//! `command_bridge` 的 `execute_tauri_command` 索引）。
//! 因此 daily-market-events 模板声明的 `market_mainline_batch_upsert` 由
//! `crates/tools/src/tools/market_mainline.rs` 提供；两条通道共享同一 crate 实现。

use axagent_agent_macro::agent_command;
use axagent_analysis_engine::market_mainline::{
    BatchUpsertInput, CreateMainlineInput, UpdateMainlineInput, archive_mainline,
    batch_upsert_mainlines, create_mainline, delete_mainlines_by_date, get_mainline,
    list_mainlines_by_category, list_mainlines_by_date, list_mainlines_by_status,
    list_recent_mainlines, update_mainline,
};
use serde_json::Value;
use tauri::State;

use crate::AppState;
use crate::commands::error::{CommandError, ErrorCategory};

/// `market_mainline_list_recent` 未传 `days` 时的默认窗口
const DEFAULT_RECENT_DAYS: usize = 7;
/// `list_recent_mainlines` 的入参上界（防单次查询拖全表）
const MAX_RECENT_DAYS: usize = 90;

fn parse_input<T: serde::de::DeserializeOwned>(input: Value) -> Result<T, String> {
    serde_json::from_value(input).map_err(|e| format!("参数解析失败: {e}"))
}

fn to_value<T: serde::Serialize>(v: &T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| format!("序列化返回结果失败: {e}"))
}

// ── 写 ────────────────────────────────────────────────────────────────────

/// 创建单条市场主线
#[agent_command(domain = "finance", safety = Caution, call_mode = StateInput, description = "创建市场主线")]
#[tauri::command]
pub async fn market_mainline_create(
    state: State<'_, AppState>,
    input: Value,
) -> Result<Value, String> {
    let input: CreateMainlineInput = parse_input(input)?;
    let row = create_mainline(state.harness.db(), input)
        .await
        .map_err(|e| CommandError::from_error(e, ErrorCategory::Unrecoverable))?;
    to_value(&row)
}

/// 批量 upsert 主线（工作流 persist_to_db 节点用）
#[agent_command(domain = "finance", safety = Caution, call_mode = StateInput, description = "批量写入市场主线（同日同主题更新，archiveMissing=true 时归档当日未提及主线）")]
#[tauri::command]
pub async fn market_mainline_batch_upsert(
    state: State<'_, AppState>,
    input: Value,
) -> Result<Value, String> {
    let input: BatchUpsertInput = parse_input(input)?;
    let result = batch_upsert_mainlines(state.harness.db(), input)
        .await
        .map_err(|e| CommandError::from_error(e, ErrorCategory::Unrecoverable))?;
    to_value(&result)
}

/// 更新主线（部分字段）
#[agent_command(domain = "finance", safety = Caution, call_mode = StateInput, description = "更新市场主线（仅传入的字段被更新）")]
#[tauri::command]
pub async fn market_mainline_update(
    state: State<'_, AppState>,
    input: Value,
) -> Result<Value, String> {
    let input: UpdateMainlineInput = parse_input(input)?;
    let row = update_mainline(state.harness.db(), input)
        .await
        .map_err(|e| CommandError::from_error(e, ErrorCategory::Unrecoverable))?;
    to_value(&row)
}

/// 归档主线（status=archived）
#[agent_command(domain = "finance", safety = Caution, call_mode = StateInput, description = "归档市场主线（status=archived）")]
#[tauri::command]
pub async fn market_mainline_archive(
    state: State<'_, AppState>,
    mainline_id: String,
) -> Result<Value, String> {
    let row = archive_mainline(state.harness.db(), &mainline_id)
        .await
        .map_err(|e| CommandError::from_error(e, ErrorCategory::Unrecoverable))?;
    to_value(&row)
}

/// 清除某日所有主线（管理用，慎调）
#[agent_command(domain = "finance", safety = Caution, call_mode = StateInput, description = "删除指定日期的全部市场主线（不可恢复）")]
#[tauri::command]
pub async fn market_mainline_delete_by_date(
    state: State<'_, AppState>,
    mainline_date: String,
) -> Result<u64, String> {
    Ok(delete_mainlines_by_date(state.harness.db(), &mainline_date)
        .await
        .map_err(|e| CommandError::from_error(e, ErrorCategory::Unrecoverable))?)
}

// ── 读 ────────────────────────────────────────────────────────────────────

/// 按 ID 获取主线
#[agent_command(domain = "finance", safety = Safe, call_mode = StateInput, description = "获取市场主线详情")]
#[tauri::command]
pub async fn market_mainline_get(
    state: State<'_, AppState>,
    mainline_id: String,
) -> Result<Value, String> {
    let row = get_mainline(state.harness.db(), &mainline_id)
        .await
        .map_err(|e| CommandError::from_error(e, ErrorCategory::Unrecoverable))?;
    // 未命中返回显式 null（而非 Err）：调用方据 `null` 区分「不存在」与「查库失败」
    to_value(&row)
}

/// 列出某日所有主线（按强度降序）
#[agent_command(domain = "finance", safety = Safe, call_mode = StateInput, description = "按日期列出市场主线（强度降序）")]
#[tauri::command]
pub async fn market_mainline_list_by_date(
    state: State<'_, AppState>,
    mainline_date: String,
) -> Result<Value, String> {
    let rows = list_mainlines_by_date(state.harness.db(), &mainline_date)
        .await
        .map_err(|e| CommandError::from_error(e, ErrorCategory::Unrecoverable))?;
    to_value(&rows)
}

/// 列出最近 N 天的主线
#[agent_command(domain = "finance", safety = Safe, call_mode = StateInput, description = "列出最近 N 天的市场主线（默认 7 天）")]
#[tauri::command]
pub async fn market_mainline_list_recent(
    state: State<'_, AppState>,
    days: Option<usize>,
) -> Result<Value, String> {
    let days = days.unwrap_or(DEFAULT_RECENT_DAYS).clamp(1, MAX_RECENT_DAYS);
    let rows = list_recent_mainlines(state.harness.db(), days)
        .await
        .map_err(|e| CommandError::from_error(e, ErrorCategory::Unrecoverable))?;
    to_value(&rows)
}

/// 按状态过滤主线
#[agent_command(domain = "finance", safety = Safe, call_mode = StateInput, description = "按状态列出市场主线（active / fading / archived）")]
#[tauri::command]
pub async fn market_mainline_list_by_status(
    state: State<'_, AppState>,
    status: String,
) -> Result<Value, String> {
    let rows = list_mainlines_by_status(state.harness.db(), &status)
        .await
        .map_err(|e| CommandError::from_error(e, ErrorCategory::Unrecoverable))?;
    to_value(&rows)
}

/// 按主题大类过滤主线
#[agent_command(domain = "finance", safety = Safe, call_mode = StateInput, description = "按主题大类列出市场主线（科技/消费/周期/金融/医药/政策/其他）")]
#[tauri::command]
pub async fn market_mainline_list_by_category(
    state: State<'_, AppState>,
    theme_category: String,
) -> Result<Value, String> {
    let rows = list_mainlines_by_category(state.harness.db(), &theme_category)
        .await
        .map_err(|e| CommandError::from_error(e, ErrorCategory::Unrecoverable))?;
    to_value(&rows)
}
