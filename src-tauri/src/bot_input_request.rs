// SPDX-License-Identifier: AGPL-3.0-only

//! P4-b 持久化待答请求的生命周期（PLAN-P4 §4 / MODEL-P4 §3 / IPC-P4 §1·§3）。
//!
//! 交互桥（`AppAskUserBridge`）在提问前落一行 `pending`，在答复/超时/通道关闭
//! 时转终态并 emit `bot-input-request-updated`；应用启动时清扫遗留 `pending`
//! （D7=A：只投活轮 + 重启清扫，不恢复被中断的模型轮）。
//!
//! 边界：**只有 Bot 线程落行**（D4=A）。`bot_id = None`（通用聊天）时
//! [`persist_pending`] 直接返回 `None` 且不碰数据库，`settle_*` 也查不到行、
//! 自然成为 no-op —— 通用聊天的行为与 P3 逐字节一致。
//!
//! 事件发射失败只记日志：待答行是审计/刷新恢复的主账，UI 通知是投影，
//! 不能因为窗口已关就回滚状态迁移。

use axagent_dao::DatabaseConnection;
use axagent_dao::repo::bot_input_request as input_repo;
use axagent_harness::IpcEventName;
use axagent_harness::types::bot::{input_origin, input_status};
use axagent_harness::types::{BotInputRequest, BotInputRequestFilter};
use axagent_harness::util_fns::now_ms;
use tauri::{AppHandle, Emitter};
use tracing::warn;

/// 过期窗口：300s（ADR-P4-4 / D5=A，与交互桥等待超时同源）。
pub const EXPIRY_MS: i64 = input_repo::INPUT_REQUEST_TTL_MS;

/// 广播一条待答请求的最新快照（落库/答复/超时/取消各一次）。
pub fn emit_updated(app: Option<&AppHandle>, req: &BotInputRequest) {
    if let Some(app) = app {
        let _ = app.emit(IpcEventName::BotInputRequestUpdated.as_str(), req);
    }
}

/// 落一行 `pending`。
///
/// `bot_id = None`（通用聊天）⇒ 立即返回 `None`，零数据库写入。
/// 其他失败（唯一键冲突、写库错误）只记日志：提问本身不能因为审计失败而卡住。
#[allow(clippy::too_many_arguments)]
pub async fn persist_pending(
    db: &DatabaseConnection,
    app: Option<&AppHandle>,
    conversation_id: &str,
    bot_id: Option<&str>,
    ask_id: &str,
    question: &str,
    options: Vec<String>,
) -> Option<BotInputRequest> {
    let bot_id = bot_id?;
    let now = now_ms();
    let draft = BotInputRequest {
        id: String::new(),
        ask_id: ask_id.to_string(),
        conversation_id: conversation_id.to_string(),
        bot_id: Some(bot_id.to_string()),
        workflow_run_id: None,
        assignment_id: None,
        origin: input_origin::PERSON.to_string(),
        question: question.to_string(),
        options,
        status: input_status::PENDING.to_string(),
        answer: None,
        answered_at: None,
        expires_at: now + EXPIRY_MS,
        created_at: now,
        updated_at: now,
    };
    match input_repo::create(db, &draft).await {
        Ok(row) => {
            emit_updated(app, &row);
            Some(row)
        },
        Err(e) => {
            warn!("[bot_input_request] 待答行落库失败 askId={ask_id}: {e}");
            None
        },
    }
}

/// 交互桥失败码 → 终态（IPC-P4 §1：超时 = expired，通道关闭 = cancelled）。
pub fn terminal_status_for(err_code: &str) -> &'static str {
    use crate::commands::error_code::agent_input as agent_input_err;
    if err_code == agent_input_err::WAIT_REPLY_TIMEOUT {
        input_status::EXPIRED
    } else {
        input_status::CANCELLED
    }
}

/// 按 `ask_id` 把 `pending` 迁移到终态；返回迁移后的最新行。
///
/// 返回 `None` 的含义有两种：没有对应行（通用聊天 / 已清扫）、或已被并发
/// 答复/取消（条件更新未命中）—— 对交互桥都是「无需再处理」。
pub async fn settle_by_ask_id(
    db: &DatabaseConnection,
    app: Option<&AppHandle>,
    ask_id: &str,
    to_status: &str,
    answer: Option<&str>,
) -> Option<BotInputRequest> {
    let row = match input_repo::get_by_ask_id(db, ask_id).await {
        Ok(row) => row?,
        Err(e) => {
            warn!("[bot_input_request] 读待答行失败 askId={ask_id}: {e}");
            return None;
        },
    };
    settle_row(db, app, row, to_status, answer).await
}

/// 按主键迁移（`answer_bot_input_request` / `cancel_bot_input_request`）。
pub async fn settle_by_id(
    db: &DatabaseConnection,
    app: Option<&AppHandle>,
    id: &str,
    to_status: &str,
    answer: Option<&str>,
) -> Option<BotInputRequest> {
    let row = match input_repo::get_by_id(db, id).await {
        Ok(row) => row?,
        Err(e) => {
            warn!("[bot_input_request] 读待答行失败 id={id}: {e}");
            return None;
        },
    };
    settle_row(db, app, row, to_status, answer).await
}

async fn settle_row(
    db: &DatabaseConnection,
    app: Option<&AppHandle>,
    row: BotInputRequest,
    to_status: &str,
    answer: Option<&str>,
) -> Option<BotInputRequest> {
    let now = now_ms();
    match input_repo::transition(db, &row.id, input_status::PENDING, to_status, answer, now).await {
        Ok(true) => {
            let fresh = match input_repo::get_by_id(db, &row.id).await {
                Ok(fresh) => fresh,
                Err(e) => {
                    warn!("[bot_input_request] 迁移后回读失败 id={}: {e}", row.id);
                    None
                },
            };
            if let Some(fresh) = fresh.as_ref() {
                emit_updated(app, fresh);
            }
            fresh
        },
        Ok(false) => None,
        Err(e) => {
            warn!("[bot_input_request] 状态迁移失败 id={}: {e}", row.id);
            None
        },
    }
}

/// 启动清扫（D7=A）：遗留 `pending` 全部置 `expired` 并逐行广播。
/// 返回清扫行数。任何错误只记日志 —— 启动流程不能被历史残留卡住。
pub async fn startup_sweep(db: &DatabaseConnection, app: Option<&AppHandle>) -> usize {
    let filter = BotInputRequestFilter {
        status: Some(input_status::PENDING.to_string()),
        ..Default::default()
    };
    let pending = match input_repo::list(db, &filter).await {
        Ok(rows) => rows,
        Err(e) => {
            warn!("[bot_input_request] 启动清扫读列表失败: {e}");
            return 0;
        },
    };
    if pending.is_empty() {
        return 0;
    }
    match input_repo::expire_stale(db, now_ms()).await {
        Ok(n) => {
            for row in pending {
                if let Ok(Some(fresh)) = input_repo::get_by_id(db, &row.id).await {
                    emit_updated(app, &fresh);
                }
            }
            n
        },
        Err(e) => {
            warn!("[bot_input_request] 启动清扫失败: {e}");
            0
        },
    }
}
