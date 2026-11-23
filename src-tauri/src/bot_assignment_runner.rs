// SPDX-License-Identifier: AGPL-3.0-only

//! Bot 委派执行器（docs/bot-integration/P2/ADR-P2B-2/3/4/5）。
//!
//! dispatch：`create_bot_assignment` 落 queued 后由调度器 `tokio::spawn` 本模块，
//! 状态机 queued→running→completed/failed（或 cancelled），每次迁移发
//! `bot-assignment-updated`；completed 的正文 append 进 `requester_conversation_id`
//! 线程（普通 assistant 消息，无新事件）。
//!
//! consult：requester 轮内 `delegate_to_bot` 工具同步调用，直接落终态审计行、
//! 不落回执线程（ADR-P2B-4 / Q3）。
//!
//! assignee 轮 = P2-a 例程同构路径（共享 `run_bot_non_interactive_turn`）：
//! 只取 assignee 自己的 Bot 档，prompter=None，ask 类写操作自动 Deny；
//! 工具表**不含** `delegate_to_bot`（一层委派/防环，ADR-P2B-5）。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axagent_harness::event_names::IpcEventName;
use axagent_harness::types::{BotAssignment, MessageRole};
use axagent_tools::tools::bot_delegate::{BotDelegateInput, BotDelegateOutcome, BotDelegateRunner};
use tauri::{AppHandle, Emitter};

use axagent_dao::repo::bot_assignment as asmt_repo;
use axagent_dao::DatabaseConnection;

/// dispatch 超时预算（IPC-P2B §3，Q5 冻结值）。常量集中在本文件头部。
pub const DISPATCH_TIMEOUT_SECS: u64 = 300;

/// 事件出口：生产接 AppHandle::emit；测试传闭包收状态序列，避免 Tauri 测试壳。
pub type EventSink = dyn Fn(&BotAssignment) + Send + Sync;

/// 一次 assignee 轮的注入式执行接口。真实实现复用 P2-a 例程同构路径；
/// 测试用 Fake 在无模型环境锁死全链状态机。
#[async_trait]
pub trait AssignmentTurnExecutor: Send + Sync {
    async fn execute(&self, bot_id: &str, task_text: &str) -> Result<String, String>;
}

/// 真实执行器：与例程共用 `run_bot_non_interactive_turn`。
struct RoutineTurnExecutor {
    ctx: Arc<crate::bot_routine_runner::BotRoutineContext>,
}

#[async_trait]
impl AssignmentTurnExecutor for RoutineTurnExecutor {
    async fn execute(&self, bot_id: &str, task_text: &str) -> Result<String, String> {
        let bot = axagent_dao::repo::bot::get_bot(self.ctx.harness.db(), bot_id)
            .await
            .map_err(|e| format!("assignee Bot 不存在: {e}"))?;
        if !bot.is_enabled {
            return Err(format!("assignee Bot {} 已禁用", bot.id));
        }
        let boundary = "本轮来自 Bot 委派：只使用你自己的 SOUL/MEMORY 与知识作答，\
                        不要引用发起方身份或上下文。结果将回传发起方；\
                        本轮不允许再委派其他 Bot（一层委派边界，ADR-P2B-5）。";
        crate::bot_routine_runner::run_bot_non_interactive_turn(
            &self.ctx,
            &bot.id,
            &bot.approval_mode,
            bot.provider_id.as_deref(),
            bot.model_id.as_deref(),
            task_text,
            Some(boundary),
        )
        .await
    }
}

/// 回执落回执线程（dispatch 专属；consult 不落，Q3）。
async fn append_receipt(db: &DatabaseConnection, conversation_id: &str, text: &str) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    if let Err(e) = axagent_dao::repo::message::create_message(
        db,
        conversation_id,
        MessageRole::Assistant,
        trimmed,
        &[],
        None,
        0,
    )
    .await
    {
        tracing::warn!(
            conversation_id,
            error = %e,
            "委派回执 append 失败（Assignment 已完成，回执可手补）"
        );
    }
}

/// 跑一单 dispatch。状态迁移与事件在每一步落库后发；超时按 `timeout` 注入
/// （测试传短时钟；生产走 `run_assignment` 的固定 300s）。
pub async fn run_assignment_with_timeout(
    db: &DatabaseConnection,
    sink: &EventSink,
    assignment_id: &str,
    executor: &dyn AssignmentTurnExecutor,
    timeout: Duration,
) {
    // queued → running
    let running = match asmt_repo::transition(
        db,
        assignment_id,
        asmt_repo::status::RUNNING,
        asmt_repo::TransitionPatch::default(),
    )
    .await
    {
        Ok(row) => row,
        Err(e) => {
            tracing::warn!(assignment_id, error = %e, "委派单无法进入 running（可能已取消/非 queued）");
            return;
        }
    };
    sink(&running);

    let assignee = running.assignee_bot_id.clone();
    let task_card = format!(
        "发起方：{}\n任务：{}",
        running
            .requester_bot_id
            .clone()
            .unwrap_or_else(|| "用户(person)".to_string()),
        running.task_text
    );

    let outcome = match tokio::time::timeout(timeout, executor.execute(&assignee, &task_card)).await
    {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(e)) => Err((None, e)),
        Err(_) => Err((
            Some(crate::commands::error_code::bot::DELEGATE_TIMEOUT),
            format!("委派超过 {} 秒", timeout.as_secs()),
        )),
    };

    match outcome {
        Ok(text) => {
            if let Ok(done) = asmt_repo::transition(
                db,
                assignment_id,
                asmt_repo::status::COMPLETED,
                asmt_repo::TransitionPatch {
                    result_text: Some(text.clone()),
                    finished_at: Some(axagent_harness::util_fns::now_ts()),
                    ..Default::default()
                },
            )
            .await
            {
                if let Some(conv) = &done.requester_conversation_id {
                    append_receipt(db, conv, &text).await;
                }
                sink(&done);
            }
        }
        Err((code, detail)) => {
            if let Ok(failed) = asmt_repo::transition(
                db,
                assignment_id,
                asmt_repo::status::FAILED,
                asmt_repo::TransitionPatch {
                    error_code: code.map(str::to_string),
                    error_detail: Some(detail),
                    finished_at: Some(axagent_harness::util_fns::now_ts()),
                    ..Default::default()
                },
            )
            .await
            {
                sink(&failed);
            }
        }
    }
}

/// 生产入口：300s 超时（IPC-P2B §3）。
pub async fn run_assignment(
    db: &DatabaseConnection,
    app: Option<AppHandle>,
    assignment_id: &str,
    executor: &dyn AssignmentTurnExecutor,
) {
    let sink = move |a: &BotAssignment| {
        if let Some(app) = &app {
            let _ = app.emit(IpcEventName::BotAssignmentUpdated.as_str(), a);
        }
    };
    run_assignment_with_timeout(
        db,
        &sink,
        assignment_id,
        executor,
        Duration::from_secs(DISPATCH_TIMEOUT_SECS),
    )
    .await;
}

/// 启动时把 dispatch 调度器接入命令层（`set_dispatch_scheduler` 只设一次）。
pub fn install_dispatch_scheduler(
    ctx: Arc<crate::bot_routine_runner::BotRoutineContext>,
    app: Option<AppHandle>,
) {
    crate::commands::bot_assignment::set_dispatch_scheduler(move |id| {
        let ctx = ctx.clone();
        let app = app.clone();
        tokio::spawn(async move {
            let db = ctx.harness.db().clone();
            let executor = RoutineTurnExecutor { ctx };
            run_assignment(&db, app, &id, &executor).await;
        });
    });
}

/// consult 执行器：同步轮内调用，落终态审计行并发一次事件，不落回执线程。
struct ConsultRunner {
    ctx: Arc<crate::bot_routine_runner::BotRoutineContext>,
    app: Option<AppHandle>,
}

#[async_trait]
impl BotDelegateRunner for ConsultRunner {
    async fn consult(&self, input: BotDelegateInput) -> Result<BotDelegateOutcome, String> {
        let db = self.ctx.harness.db();
        let assignee = axagent_dao::repo::bot::get_bot(db, &input.bot_id)
            .await
            .map_err(|e| format!("BOT_NOT_FOUND: {e}"))?;
        if !assignee.is_enabled {
            return Err("BOT_NOT_FOUND: assignee 已禁用".to_string());
        }
        if input.requester_bot_id.as_deref() == Some(&input.bot_id) {
            return Err("BOT_DELEGATE_SELF: Bot 不能委派给自己".to_string());
        }
        if let Some(requester) = &input.requester_bot_id {
            let bot = axagent_dao::repo::bot::get_bot(db, requester)
                .await
                .map_err(|e| format!("BOT_NOT_FOUND: {e}"))?;
            if !bot.is_enabled {
                return Err("BOT_NOT_FOUND: requester 已禁用".to_string());
            }
        }

        let prompt = format!(
            "发起方：{}\n征询：{}",
            input.requester_bot_id.as_deref().unwrap_or("用户(person)"),
            input.question
        );
        let executor = RoutineTurnExecutor { ctx: self.ctx.clone() };
        let outcome = executor.execute(&assignee.id, &prompt).await;
        let row = asmt_repo::record_consult_assignment(
            db,
            &input.origin,
            input.requester_bot_id.as_deref().unwrap_or(""),
            &assignee.id,
            &input.question,
            outcome.clone(),
        )
        .await
        .map_err(|e| e.to_string())?;

        if let Some(app) = &self.app {
            let _ = app.emit(IpcEventName::BotAssignmentUpdated.as_str(), &row);
        }
        Ok(BotDelegateOutcome {
            assignment_id: row.id,
            result_text: outcome.unwrap_or_default(),
            status: row.status,
        })
    }
}

/// 启动时接入 consult runner（与 dispatch 同一 Bot 例程执行路径）。
pub fn install_consult_runner(
    ctx: Arc<crate::bot_routine_runner::BotRoutineContext>,
    app: Option<AppHandle>,
) {
    axagent_tools::tools::bot_delegate::set_bot_delegate_runner(Arc::new(ConsultRunner { ctx, app }));
}
