// SPDX-License-Identifier: AGPL-3.0-only

//! P4-b 待答请求 Tauri 命令（docs/bot-integration/P4/IPC-P4 §1/§4）。
//!
//! 三命令：list（四过滤全可选，status 非法 → `ERR_VALIDATION`）、answer（只接受
//! `pending` + 有活 sender，投递 oneshot → 转 answered + 事件）、cancel（只接受
//! `pending`，丢弃 sender 唤醒等待方为「已取消」→ 转 cancelled + 事件）。
//!
//! 行状态迁移全部经 `crate::bot_input_request::{settle_by_id}`，与交互桥同一条
//! 路径（条件更新 + 事件），不在这里另写一套。

use std::collections::HashMap;
use std::sync::Arc;

use axagent_dao::DatabaseConnection;
use axagent_dao::repo::bot_input_request as input_repo;
use axagent_harness::types::bot::input_status;
use axagent_harness::types::{BotInputRequest, BotInputRequestFilter};
use tauri::{AppHandle, State};
use tokio::sync::{Mutex, oneshot};

use crate::AppState;
use crate::commands::error::{ErrorCategory, ErrorResponse};
use crate::commands::error_code::bot as bot_err;

/// 交互桥的 oneshot 表（`AppState::agent_ask_senders` 同型）。
pub type AskSenders = Arc<Mutex<HashMap<String, oneshot::Sender<String>>>>;

fn bot_error(code: &str, detail: impl Into<String>) -> String {
    ErrorResponse::new(code)
        .with_category(ErrorCategory::Validation)
        .with_detail(detail)
        .to_string()
}

fn not_found(id: &str) -> String {
    bot_error(bot_err::INPUT_REQUEST_NOT_FOUND, format!("待答请求 {id} 不存在"))
}

fn not_pending(id: &str) -> String {
    bot_error(bot_err::INPUT_REQUEST_NOT_PENDING, format!("待答请求 {id} 已回答或已失效"))
}

fn validate_status(status: Option<&str>) -> Result<(), String> {
    let Some(status) = status else { return Ok(()) };
    let valid = [
        input_status::PENDING,
        input_status::ANSWERED,
        input_status::EXPIRED,
        input_status::CANCELLED,
    ];
    if valid.contains(&status) {
        Ok(())
    } else {
        Err(ErrorResponse::new("ERR_VALIDATION")
            .with_category(ErrorCategory::Validation)
            .with_param("field", "status")
            .with_detail(format!("status 只能是 pending/answered/expired/cancelled，实得 {status}"))
            .to_string())
    }
}

/// 列表（`created_at DESC`）。四过滤全可选。
pub async fn list_inner(
    db: &DatabaseConnection,
    filter: BotInputRequestFilter,
) -> Result<Vec<BotInputRequest>, String> {
    validate_status(filter.status.as_deref())?;
    input_repo::list(db, &filter)
        .await
        .map_err(|e| ErrorResponse::from_error(e, ErrorCategory::Unrecoverable).to_string())
}

/// 答复：只接受 pending 且有活 sender。次序 = 取 sender → 落账 → 投递，
/// 保证模型被唤醒时 DB 已是 answered（IPC-P4 §1）。
pub async fn answer_inner(
    db: &DatabaseConnection,
    senders: &AskSenders,
    app: Option<&AppHandle>,
    request_id: &str,
    answer: &str,
) -> Result<BotInputRequest, String> {
    let trimmed = answer.trim();
    if trimmed.is_empty() {
        return Err(bot_error(bot_err::INPUT_ANSWER_EMPTY, "答复去空白后为空"));
    }
    let row = input_repo::get_by_id(db, request_id)
        .await
        .map_err(|e| ErrorResponse::from_error(e, ErrorCategory::Unrecoverable).to_string())?
        .ok_or_else(|| not_found(request_id))?;
    if row.status != input_status::PENDING {
        return Err(not_pending(request_id));
    }
    let sender = { senders.lock().await.remove(&row.ask_id) };
    let Some(sender) = sender else {
        // 行还 pending 但已无等待方（例如桥所在进程路径已被清扫）：
        // 不允许把它标成 answered，否则 UI 会显示一条没人收到过的答复。
        return Err(not_pending(request_id));
    };
    let updated = crate::bot_input_request::settle_by_id(
        db,
        app,
        request_id,
        input_status::ANSWERED,
        Some(trimmed),
    )
    .await
    .ok_or_else(|| not_pending(request_id))?;
    let _ = sender.send(trimmed.to_string());
    Ok(updated)
}

/// 取消：只接受 pending；丢弃 sender（等待方收到通道关闭 → 桥侧按 cancelled 处理）。
pub async fn cancel_inner(
    db: &DatabaseConnection,
    senders: &AskSenders,
    app: Option<&AppHandle>,
    request_id: &str,
) -> Result<BotInputRequest, String> {
    let row = input_repo::get_by_id(db, request_id)
        .await
        .map_err(|e| ErrorResponse::from_error(e, ErrorCategory::Unrecoverable).to_string())?
        .ok_or_else(|| not_found(request_id))?;
    if row.status != input_status::PENDING {
        return Err(not_pending(request_id));
    }
    let dropped = { senders.lock().await.remove(&row.ask_id) };
    let updated =
        crate::bot_input_request::settle_by_id(db, app, request_id, input_status::CANCELLED, None)
            .await
            .ok_or_else(|| not_pending(request_id))?;
    drop(dropped);
    Ok(updated)
}

/// 列出待答请求（`IPC-P4 §1`）。
#[tauri::command]
pub async fn list_bot_input_requests(
    state: State<'_, AppState>,
    filter: BotInputRequestFilter,
) -> Result<Vec<BotInputRequest>, String> {
    list_inner(state.harness.db(), filter).await
}

/// 答复一条待答请求：投递 oneshot → answered + 事件。
#[tauri::command]
pub async fn answer_bot_input_request(
    app: AppHandle,
    state: State<'_, AppState>,
    request_id: String,
    answer: String,
) -> Result<BotInputRequest, String> {
    answer_inner(state.harness.db(), &state.agent_ask_senders, Some(&app), &request_id, &answer)
        .await
}

/// 取消一条待答请求：唤醒等待方为「已取消」→ cancelled + 事件。
#[tauri::command]
pub async fn cancel_bot_input_request(
    app: AppHandle,
    state: State<'_, AppState>,
    request_id: String,
) -> Result<BotInputRequest, String> {
    cancel_inner(state.harness.db(), &state.agent_ask_senders, Some(&app), &request_id).await
}
