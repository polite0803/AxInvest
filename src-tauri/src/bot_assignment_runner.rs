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
use axagent_tools::tools::bot_delegate::{
    BotDelegateInput, BotDelegateOutcome, BotDelegateRunner, DelegateContext, MAX_DELEGATE_DEPTH,
    MODE_CONSULT, MODE_DISPATCH,
};
use tauri::{AppHandle, Emitter};

use axagent_dao::DatabaseConnection;
use axagent_dao::repo::bot_assignment as asmt_repo;

/// dispatch 超时预算（IPC-P2B §3，Q5 冻结值）。常量集中在本文件头部。
/// P4 起同时是「无部门配额」时的默认值：生效值由 `bot_quota::QuotaLimits` 经
/// `AssignmentContext` 注入（`run_assignment_inner` 按 run 封顶）。
pub const DISPATCH_TIMEOUT_SECS: u64 = 300;
/// ADR-P3-2 ④：同一 root 链每层在途 dispatch 上限（编排并发预算另算）。
/// P4 起同时是默认值：部门配额可把它收紧，见 `bot_quota::AssignmentContext`。
pub const MAX_IN_FLIGHT_PER_DEPTH: i64 = 2;

/// 事件出口：生产接 AppHandle::emit；测试传闭包收状态序列，避免 Tauri 测试壳。
pub type EventSink = dyn Fn(&BotAssignment) + Send + Sync;

/// 一次 assignee 轮的注入式执行接口。真实实现复用 P2-a 例程同构路径；
/// 测试用 Fake 在无模型环境锁死全链状态机。
#[async_trait]
pub trait AssignmentTurnExecutor: Send + Sync {
    async fn execute(&self, bot_id: &str, task_text: &str) -> Result<String, String>;

    /// P3：真实执行器用链上下文放行子委派；P2-b 的 Fake 走默认实现，签名不变。
    async fn execute_with_context(
        &self,
        assignment: &BotAssignment,
        task_text: &str,
    ) -> Result<String, String> {
        self.execute(&assignment.assignee_bot_id, task_text).await
    }

    /// P5-b B2（ADR-P5-8）：带用量回传的委派执行。默认实现保持旧行为（usage=None，
    /// 账本不写行），P2-b Fake/无模型实现零改动；真实执行器覆写以回传 TokenUsage。
    async fn execute_with_usage(
        &self,
        assignment: &BotAssignment,
        task_text: &str,
    ) -> Result<crate::bot_routine_runner::BotTurnOutcome, String> {
        let text = self.execute_with_context(assignment, task_text).await?;
        Ok(crate::bot_routine_runner::BotTurnOutcome { text, usage: None })
    }
}

/// 真实执行器：与例程共用 `run_bot_non_interactive_turn`。
struct RoutineTurnExecutor {
    ctx: Arc<crate::bot_routine_runner::BotRoutineContext>,
}

impl RoutineTurnExecutor {
    async fn execute_with_context_impl(
        &self,
        assignment: &BotAssignment,
        task_text: &str,
    ) -> Result<crate::bot_routine_runner::BotTurnOutcome, String> {
        let bot =
            axagent_dao::repo::bot::get_bot(self.ctx.harness.db(), &assignment.assignee_bot_id)
                .await
                .map_err(|e| format!("assignee Bot 不存在: {e}"))?;
        if !bot.is_enabled {
            return Err(format!("assignee Bot {} 已禁用", bot.id));
        }
        let boundary = if assignment.depth < MAX_DELEGATE_DEPTH as i32 {
            "本轮来自 Bot 委派：只使用你自己的 SOUL/MEMORY 与知识作答，\
             不要引用发起方身份或上下文。结果将回传发起方；\
             可继续委派（depth<3，ADR-P3-2）。"
        } else {
            "本轮来自 Bot 委派：只使用你自己的 SOUL/MEMORY 与知识作答，\
             不要引用发起方身份或上下文。结果将回传发起方；\
             本轮不得再委派（depth=3，ADR-P3-2）。"
        };
        let chain = DelegateContext {
            parent_assignment_id: Some(assignment.id.clone()),
            root_assignment_id: Some(assignment.root_id.clone()),
            depth: assignment.depth as u8,
            workflow_run_id: assignment.workflow_run_id.clone(),
            step_key: assignment.step_key.clone(),
            requester_conversation_id: assignment.requester_conversation_id.clone(),
        };
        crate::bot_routine_runner::run_bot_non_interactive_turn_with_usage(
            &self.ctx,
            &bot.id,
            &bot.approval_mode,
            bot.provider_id.as_deref(),
            bot.model_id.as_deref(),
            task_text,
            Some(boundary),
            chain,
        )
        .await
    }
}

#[async_trait]
impl AssignmentTurnExecutor for RoutineTurnExecutor {
    async fn execute(&self, bot_id: &str, task_text: &str) -> Result<String, String> {
        self.execute_with_context_impl(
            &BotAssignment { assignee_bot_id: bot_id.to_string(), ..Default::default() },
            task_text,
        )
        .await
        .map(|o| o.text)
    }

    async fn execute_with_context(
        &self,
        assignment: &BotAssignment,
        task_text: &str,
    ) -> Result<String, String> {
        self.execute_with_context_impl(assignment, task_text).await.map(|o| o.text)
    }

    async fn execute_with_usage(
        &self,
        assignment: &BotAssignment,
        task_text: &str,
    ) -> Result<crate::bot_routine_runner::BotTurnOutcome, String> {
        self.execute_with_context_impl(assignment, task_text).await
    }
}

/// 回执落回执线程（dispatch 专属；consult 不落，Q3）。
/// P3：正文前缀 `[L{depth} · {stepKey} · {botName}]`，parent 仍指根 anchor。
async fn append_receipt(
    db: &DatabaseConnection,
    conversation_id: &str,
    parent_message_id: Option<&str>,
    prefix: &str,
    text: &str,
) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    let content = format!("{prefix}{trimmed}");
    if let Err(e) = axagent_dao::repo::message::create_message(
        db,
        conversation_id,
        MessageRole::Assistant,
        &content,
        &[],
        parent_message_id,
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
pub async fn run_assignment_inner(
    db: &DatabaseConnection,
    sink: &EventSink,
    assignment_id: &str,
    executor: &dyn AssignmentTurnExecutor,
    timeout: Duration,
    completion: &(dyn Fn(&BotAssignment) + Sync),
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
        },
    };
    sink(&running);
    completion(&running);

    // P4：workflow 子树内按 run 配额封顶单轮超时（无登记的纯委派链原样不变）。
    let timeout = crate::bot_quota::cap_turn_timeout(running.workflow_run_id.as_deref(), timeout);

    let bot_name = axagent_dao::repo::bot::get_bot(db, &running.assignee_bot_id)
        .await
        .map(|b| b.name)
        .unwrap_or_else(|_| running.assignee_bot_id.clone());
    let step_tag = running.step_key.as_deref().unwrap_or("-");
    let receipt_prefix = if running.depth <= 1 {
        format!("[步骤 {step_tag} · {bot_name}] ")
    } else {
        format!("[L{} · {step_tag} · {bot_name}] ", running.depth)
    };
    // run 内步骤/子孙回执 parent=根 anchor；纯委派链无 anchor 时 parent=None。
    let anchor_id = if let Some(run_id) = &running.workflow_run_id {
        axagent_dao::repo::bot_workflow_run::get_run(db, run_id)
            .await
            .ok()
            .and_then(|r| r.anchor_message_id)
    } else {
        None
    };
    let task_card = format!(
        "发起方：{}\n任务：{}",
        running.requester_bot_id.clone().unwrap_or_else(|| "用户(person)".to_string()),
        running.task_text
    );

    // P5-b B3：事后熔断（ADR-P5-3）—— 仅拦新建委派轮，已启动的跑完。
    if let Err(cost_err) =
        crate::bot_cost_quota::ensure_bot_cost_quota_ok(db, &running.assignee_bot_id).await
    {
        let code = if cost_err.contains(crate::commands::error_code::bot::COST_QUOTA_EXCEEDED) {
            Some(crate::commands::error_code::bot::COST_QUOTA_EXCEEDED)
        } else {
            Some("ERR_PERMISSION")
        };
        if let Ok(failed) = asmt_repo::transition(
            db,
            assignment_id,
            asmt_repo::status::FAILED,
            asmt_repo::TransitionPatch {
                error_code: code.map(str::to_string),
                error_detail: Some(cost_err),
                finished_at: Some(axagent_harness::util_fns::now_ts()),
                ..Default::default()
            },
        )
        .await
        {
            sink(&failed);
            completion(&failed);
        }
        return;
    }

    let outcome = match tokio::time::timeout(
        timeout,
        executor.execute_with_usage(&running, &task_card),
    )
    .await
    {
        Ok(Ok(turn_outcome)) => Ok(turn_outcome),
        Ok(Err(e)) => Err((None, e)),
        Err(_) => Err((
            Some(crate::commands::error_code::bot::DELEGATE_TIMEOUT),
            format!("委派超过 {} 秒", timeout.as_secs()),
        )),
    };

    match outcome {
        Ok(turn_outcome) => {
            // P5-b B2（ADR-P5-3/8）：用量写入失败只 warn，不阻塞终态与回执。
            if let Some(u) = turn_outcome.usage.as_ref() {
                if let Err(e) = crate::bot_cost_quota::record_usage(
                    db,
                    &running.assignee_bot_id,
                    None,
                    Some(&u.execution_id),
                    running.workflow_run_id.as_deref(),
                    Some(&running.id),
                    axagent_harness::types::BotUsageOrigin::Assignment,
                    &u.model_id,
                    &u.usage,
                )
                .await
                {
                    tracing::warn!(
                        assignment_id = %running.id,
                        bot_id = %running.assignee_bot_id,
                        error = %e,
                        "委派轮用量记账失败（不阻塞）"
                    );
                }
            }
            let text = turn_outcome.text;
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
                    append_receipt(db, conv, anchor_id.as_deref(), &receipt_prefix, &text).await;
                }
                sink(&done);
                completion(&done);
            }
        },
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
                completion(&failed);
            }
        },
    }
}

/// P3 步骤/子链入口：挂 run 完成钩子；执行器仍是 P2-b 的 RoutineTurnExecutor。
pub async fn run_step_assignment(
    db: &DatabaseConnection,
    ctx: Arc<crate::bot_routine_runner::BotRoutineContext>,
    assignment_id: &str,
    timeout: Duration,
    sink: &EventSink,
    completion: &(dyn Fn(&BotAssignment) + Sync),
) {
    let executor = RoutineTurnExecutor { ctx };
    run_assignment_inner(db, sink, assignment_id, &executor, timeout, completion).await;
}

/// P2-b 兼容入口：保留原签名，完成钩子为空（行为不变）。
pub async fn run_assignment_with_timeout(
    db: &DatabaseConnection,
    sink: &EventSink,
    assignment_id: &str,
    executor: &dyn AssignmentTurnExecutor,
    timeout: Duration,
) {
    let noop = |_: &BotAssignment| {};
    run_assignment_inner(db, sink, assignment_id, executor, timeout, &noop).await;
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
    let noop = |_: &BotAssignment| {};
    run_assignment_inner(
        db,
        &sink,
        assignment_id,
        executor,
        Duration::from_secs(DISPATCH_TIMEOUT_SECS),
        &noop,
    )
    .await;
}

/// 启动时把 dispatch 调度器接入命令层（`set_dispatch_scheduler` 只设一次）。
pub fn install_dispatch_scheduler(
    ctx: Arc<crate::bot_routine_runner::BotRoutineContext>,
    app: Option<AppHandle>,
) {
    crate::bot_workflow_runner::install_runner(ctx.harness.db().clone(), ctx.clone(), app.clone());
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

/// ErrorResponse 是 JSON 串，抽 `code` 字段供审计行落码。
fn extract_error_code(raw: &str) -> &'static str {
    if raw.contains(crate::commands::error_code::bot::DELEGATE_CYCLE) {
        crate::commands::error_code::bot::DELEGATE_CYCLE
    } else if raw.contains(crate::commands::error_code::bot::DELEGATE_MAX_DEPTH) {
        crate::commands::error_code::bot::DELEGATE_MAX_DEPTH
    } else if raw.contains(crate::commands::error_code::bot::DELEGATE_TIMEOUT) {
        crate::commands::error_code::bot::DELEGATE_TIMEOUT
    } else if raw.contains(crate::commands::error_code::bot::DELEGATE_BUSY) {
        crate::commands::error_code::bot::DELEGATE_BUSY
    } else {
        "ERR_PERMISSION"
    }
}

/// consult/dispatch 工具执行器：同步征询落终态审计行；dispatch 落 queued 子单。
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
        let chain_input = axagent_harness::types::CreateBotAssignment {
            assignee_bot_id: input.bot_id.clone(),
            task_text: input.question.clone(),
            mode: MODE_CONSULT.to_string(),
            parent_assignment_id: input.parent_assignment_id.clone(),
            root_assignment_id: input.root_assignment_id.clone(),
            workflow_run_id: input.workflow_run_id.clone(),
            step_key: input.step_key.clone(),
            requester_bot_id: input.requester_bot_id.clone(),
            ..Default::default()
        };
        if let Err(e) =
            crate::commands::bot_assignment::check_chain_constraints(db, &chain_input).await
        {
            let code = extract_error_code(&e);
            let row = asmt_repo::record_consult_assignment_with_code(
                db,
                &input.origin,
                input.requester_bot_id.as_deref().unwrap_or(""),
                &assignee.id,
                &input.question,
                Err(e.clone()),
                Some(code),
            )
            .await
            .map_err(|e| e.to_string())?;
            if let Some(app) = &self.app {
                let _ = app.emit(IpcEventName::BotAssignmentUpdated.as_str(), &row);
            }
            return Err(e);
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
        // P5-b B3：consult 也是新建委派轮前熔断（ADR-P5-3）。
        crate::bot_cost_quota::ensure_bot_cost_quota_ok(db, &assignee.id).await?;

        let turn_outcome = executor
            .execute_with_usage(
                &BotAssignment { assignee_bot_id: assignee.id.clone(), ..Default::default() },
                &prompt,
            )
            .await;
        // P5-b B2：记账失败只 warn，不阻塞 consult 结果。
        if let Ok(ref o) = turn_outcome {
            if let Some(u) = o.usage.as_ref() {
                if let Err(e) = crate::bot_cost_quota::record_usage(
                    db,
                    &assignee.id,
                    None,
                    Some(&u.execution_id),
                    input.workflow_run_id.as_deref(),
                    None, // consult 审计行 id 在 record_consult_assignment 后才有；先不绑 assignment_id
                    axagent_harness::types::BotUsageOrigin::Assignment,
                    &u.model_id,
                    &u.usage,
                )
                .await
                {
                    tracing::warn!(
                        bot_id = %assignee.id,
                        error = %e,
                        "consult 轮用量记账失败（不阻塞）"
                    );
                }
            }
        }
        let outcome = turn_outcome.map(|o| o.text);
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

    /// P3 dispatch：创建子 assignment 后立即返回 queued，回执异步落同一线程。
    async fn dispatch(&self, input: BotDelegateInput) -> Result<BotDelegateOutcome, String> {
        let db = self.ctx.harness.db();
        let assignee = axagent_dao::repo::bot::get_bot(db, &input.bot_id)
            .await
            .map_err(|e| format!("BOT_NOT_FOUND: {e}"))?;
        if !assignee.is_enabled {
            return Err("BOT_NOT_FOUND: assignee 已禁用".to_string());
        }
        let requester = input
            .requester_bot_id
            .clone()
            .ok_or_else(|| "ERR_VALIDATION: dispatch 需要 requesterBotId".to_string())?;
        let child = crate::commands::bot_assignment::create_step_assignment(
            db,
            &axagent_harness::types::CreateBotAssignment {
                assignee_bot_id: input.bot_id.clone(),
                task_text: input.question.clone(),
                mode: MODE_DISPATCH.to_string(),
                requester_bot_id: Some(requester),
                requester_conversation_id: input.requester_conversation_id.clone(),
                parent_assignment_id: input.parent_assignment_id.clone(),
                root_assignment_id: input.root_assignment_id.clone(),
                workflow_run_id: input.workflow_run_id.clone(),
                step_key: input.step_key.clone(),
                origin: Some("bot".to_string()),
                ..Default::default()
            },
        )
        .await?;
        if let Some(app) = &self.app {
            let _ = app.emit(IpcEventName::BotAssignmentUpdated.as_str(), &child);
        }
        let child_id = child.id.clone();
        crate::commands::bot_assignment::schedule_step_dispatch(child_id);
        tracing::info!(
            error_code = "BOT_DELEGATE_DISPATCH_OK",
            assignment_id = %child.id,
            depth = child.depth,
            "多层委派 dispatch 已落库"
        );
        Ok(BotDelegateOutcome {
            assignment_id: child.id.clone(),
            result_text: format!("{{\"assignmentId\":\"{}\",\"status\":\"queued\"}}", child.id),
            status: child.status,
        })
    }
}

/// 启动时接入 consult runner（与 dispatch 同一 Bot 例程执行路径）。
pub fn install_consult_runner(
    ctx: Arc<crate::bot_routine_runner::BotRoutineContext>,
    app: Option<AppHandle>,
) {
    axagent_tools::tools::bot_delegate::set_bot_delegate_runner(Arc::new(ConsultRunner {
        ctx,
        app,
    }));
}
