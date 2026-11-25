// SPDX-License-Identifier: AGPL-3.0-only

//! Bot 编排执行器（docs/bot-integration/P3/PLAN-P3 §3、IPC-P3 §3）。
//!
//! 静态 DAG（`steps_json`）由 `bot_workflow_steps::validate_dag` 在定义期收口；
//! 本模块只做运行期拓扑推进：就绪集 + 并发门 + 总超时 + 完成钩子 + 子树终态等待。
//! 不新建调度器/会话池/模型调用路径：assignee 轮一律复用 P2-b 的
//! `run_assignment_inner` + `RoutineTurnExecutor`。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axagent_dao::DatabaseConnection;
use axagent_dao::repo::bot_assignment as asmt_repo;
use axagent_dao::repo::bot_workflow as workflow_repo;
use axagent_dao::repo::bot_workflow_run as run_repo;
use axagent_harness::event_names::IpcEventName;
use axagent_harness::types::{
    BotAssignment, BotWorkflow, BotWorkflowRun, BotWorkflowRunContext, BotWorkflowRunPatch,
};
use axagent_harness::util_fns::now_ts;
use tauri::{AppHandle, Emitter};
use tokio::sync::Notify;

use crate::commands::error_code::bot as bot_err;

/// Q7 冻结预算（PLAN-P3 §9）：编排并发 2 / 总超时 900s / ≤12 步。
/// 链预算（逐层 2 / 单步与链总 300s）在 `bot_assignment_runner.rs` 头部，两套独立叠加。
#[allow(dead_code)]
pub const MAX_CONCURRENCY: i32 = 2;
#[allow(dead_code)]
pub const TOTAL_TIMEOUT_SECS: i32 = 900;
#[allow(dead_code)]
pub const MAX_STEPS: usize = 12;

type StepScheduler = dyn Fn(String) + Send + Sync;

static STEP_SCHEDULER: OnceLock<Box<StepScheduler>> = OnceLock::new();
static CONTEXT_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
static NOTIFIERS: OnceLock<Mutex<HashMap<String, Arc<Notify>>>> = OnceLock::new();

fn context_lock() -> &'static tokio::sync::Mutex<()> {
    CONTEXT_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// 供 `commands::bot_assignment::schedule_step_dispatch` 取用；未安装时走 P2-b 默认调度。
pub fn step_scheduler() -> Option<&'static StepScheduler> {
    STEP_SCHEDULER.get().map(|s| s.as_ref())
}

fn notifier(run_id: &str) -> Arc<Notify> {
    let map = NOTIFIERS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().expect("notifier lock poisoned");
    guard.entry(run_id.to_string()).or_insert_with(|| Arc::new(Notify::new())).clone()
}

fn emit_run(app: Option<&AppHandle>, run: &BotWorkflowRun) {
    if let Some(app) = app {
        let _ = app.emit(IpcEventName::BotWorkflowRunUpdated.as_str(), run);
    }
}

fn code_in(raw: &str) -> Option<&'static str> {
    const CODES: &[&str] = &[
        bot_err::QUOTA_EXCEEDED,
        bot_err::WORKFLOW_STEP_BOT_MISSING,
        bot_err::WORKFLOW_MEMBER_CHANGED,
        bot_err::WORKFLOW_TIMEOUT,
        bot_err::DELEGATE_CYCLE,
        bot_err::DELEGATE_MAX_DEPTH,
        bot_err::DELEGATE_BUSY,
        bot_err::DELEGATE_TIMEOUT,
        bot_err::DELEGATE_SELF,
        bot_err::NOT_FOUND,
        "ERR_VALIDATION",
    ];
    CODES.iter().copied().find(|code| raw.contains(code))
}

/// 进程启动时由 `install_dispatch_scheduler` 一并安装：步骤单由本模块调度并挂完成钩子；
/// 普通 `create_bot_assignment` 仍走原有 P2-b scheduler，行为不变。
pub fn install_runner(
    db: DatabaseConnection,
    ctx: Arc<crate::bot_routine_runner::BotRoutineContext>,
    app: Option<AppHandle>,
) {
    let _ = STEP_SCHEDULER.set(Box::new(move |assignment_id: String| {
        let db = db.clone();
        let ctx = ctx.clone();
        let app = app.clone();
        tokio::spawn(async move {
            let row = match asmt_repo::get_assignment(&db, &assignment_id, |_| String::new()).await
            {
                Ok(row) => row,
                Err(e) => {
                    tracing::warn!(assignment_id, error = %e, "步骤单读取失败");
                    return;
                },
            };
            if asmt_repo::status::is_terminal(&row.status) {
                return;
            }
            let timeout = step_timeout_secs(&db, &row).await;
            let sink_app = app.clone();
            let sink = move |a: &BotAssignment| {
                if let Some(app) = &sink_app {
                    let _ = app.emit(IpcEventName::BotAssignmentUpdated.as_str(), a);
                }
            };
            let hook_app = app.clone();
            let hook_db = db.clone();
            let completion = move |a: &BotAssignment| {
                let app = hook_app.clone();
                let db = hook_db.clone();
                let row = a.clone();
                tokio::spawn(async move {
                    on_assignment_completed(&db, app.as_ref(), &row).await;
                });
            };
            crate::bot_assignment_runner::run_step_assignment(
                &db,
                ctx.clone(),
                &assignment_id,
                Duration::from_secs(timeout as u64),
                &sink,
                &completion,
            )
            .await;
        });
    }));
}

async fn step_timeout_secs(db: &DatabaseConnection, row: &BotAssignment) -> i32 {
    let default = crate::bot_assignment_runner::DISPATCH_TIMEOUT_SECS as i32;
    let Some(run_id) = row.workflow_run_id.as_deref() else {
        return default;
    };
    let Ok(run) = run_repo::get_run(db, run_id).await else {
        return default;
    };
    let Ok(workflow) = workflow_repo::get_workflow(db, &run.workflow_id).await else {
        return default;
    };
    workflow
        .steps
        .iter()
        .find(|s| Some(&s.step_key) == row.step_key.as_ref())
        .map(|s| s.timeout_secs)
        .unwrap_or(default)
}

/// 运行前 Bot 存在/enabled/部门成员复查（IPC-P3 §2.4；定义期校验后成员可能已变更）。
pub async fn validate_runtime_bots(
    db: &DatabaseConnection,
    workflow: &BotWorkflow,
) -> std::result::Result<(), String> {
    let dept = if workflow.owner_scope == "department" {
        workflow.department_id.as_deref()
    } else {
        None
    };
    crate::commands::bot_workflow::validate_bot_refs(db, &workflow.steps, dept).await
}

/// 手工/定时触发共用的 run 启动入口：queued→running + anchor System 消息 + 后台编排。
pub async fn start_run(
    db: &DatabaseConnection,
    app: Option<AppHandle>,
    run_id: &str,
) -> std::result::Result<BotWorkflowRun, String> {
    let run = run_repo::get_run(db, run_id).await.map_err(|e| {
        crate::commands::bot_workflow::run_error(bot_err::WORKFLOW_RUN_NOT_FOUND, e.to_string())
    })?;
    if !matches!(run.status.as_str(), run_repo::status::QUEUED | run_repo::status::WAITING) {
        return Err(crate::commands::bot_workflow::run_error(
            bot_err::WORKFLOW_CANCEL_INVALID,
            format!("run {} 当前状态 {} 不可启动", run.id, run.status),
        ));
    }
    let workflow = workflow_repo::get_workflow(db, &run.workflow_id).await.map_err(|e| {
        crate::commands::bot_workflow::run_error(bot_err::WORKFLOW_NOT_FOUND, e.to_string())
    })?;
    if let Err(e) = validate_runtime_bots(db, &workflow).await {
        fail_run(
            db,
            app.as_ref(),
            &run,
            code_in(&e).unwrap_or(bot_err::WORKFLOW_STEP_BOT_MISSING),
            &e,
        )
        .await?;
        return Ok(run_repo::get_run(db, run_id).await.map_err(|e| e.to_string())?);
    }
    // P4 配额（ADR-P4-8）：run 创建时解析一次，登记到 AssignmentContext；
    // 排除本 run 自身后复核部门同时在跑数（手工触发已在命令层查过一次，
    // 这里覆盖 cron/其他入口）。实例级编排/纯委派零 DB 读。
    let limits = match crate::bot_quota::check_run_allowed(db, &workflow, Some(&run.id)).await {
        Ok(limits) => limits,
        Err(e) => {
            fail_run(db, app.as_ref(), &run, code_in(&e).unwrap_or(bot_err::QUOTA_EXCEEDED), &e)
                .await?;
            return Ok(run_repo::get_run(db, run_id).await.map_err(|e| e.to_string())?);
        },
    };
    crate::bot_quota::AssignmentContext::install(&run.id, limits);

    let anchor = axagent_dao::repo::message::create_message(
        db,
        &run.requester_conversation_id,
        axagent_harness::types::MessageRole::System,
        &format!("Bot 编排 {} 已启动（run {}）", workflow.name, run.id),
        &[],
        None,
        0,
    )
    .await
    .map_err(|e| e.to_string())?;

    let started = run_repo::patch_run(
        db,
        run_id,
        BotWorkflowRunPatch {
            status: Some(run_repo::status::RUNNING.to_string()),
            anchor_message_id: Some(anchor.id),
            started_at: Some(now_ts()),
            ..Default::default()
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    emit_run(app.as_ref(), &started);

    let db_bg = db.clone();
    let app_bg = app.clone();
    let run_bg = started.clone();
    tokio::spawn(async move {
        orchestrate(db_bg, app_bg, run_bg, workflow).await;
    });
    Ok(started)
}

/// 完成钩子：任一 assignment（含子孙）终态都更新 run 上下文 / 失败传播 / 终态判定。
pub async fn on_assignment_completed(
    db: &DatabaseConnection,
    app: Option<&AppHandle>,
    row: &BotAssignment,
) {
    let Some(run_id) = row.workflow_run_id.as_deref() else {
        return;
    };
    let _guard = context_lock().lock().await;
    let Ok(mut run) = run_repo::get_run(db, run_id).await else {
        return;
    };
    if run_repo::status::is_terminal(&run.status) {
        return;
    }
    if row.status == asmt_repo::status::RUNNING {
        if run.status != run_repo::status::RUNNING {
            match run_repo::patch_run(
                db,
                run_id,
                BotWorkflowRunPatch {
                    status: Some(run_repo::status::RUNNING.to_string()),
                    ..Default::default()
                },
            )
            .await
            {
                Ok(updated) => run = updated,
                Err(e) => tracing::warn!(run_id, error = %e, "run 切 running 失败"),
            }
            emit_run(app, &run);
        }
        notifier(run_id).notify_one();
        return;
    }
    if !asmt_repo::status::is_terminal(&row.status) {
        return;
    }

    let Ok(workflow) = workflow_repo::get_workflow(db, &run.workflow_id).await else {
        return;
    };
    let step =
        row.step_key.as_deref().and_then(|key| workflow.steps.iter().find(|s| s.step_key == key));
    let mut ctx = run.context.clone();

    if row.status == asmt_repo::status::COMPLETED {
        if let Some(key) = row.step_key.as_deref() {
            if row.depth == 1 {
                ctx.outputs.insert(
                    key.to_string(),
                    axagent_harness::types::BotWorkflowStepOutput {
                        assignment_id: row.id.clone(),
                        result_text: row.result_text.clone().unwrap_or_default(),
                    },
                );
            }
        }
    } else {
        let code = row.error_code.as_deref().unwrap_or("ERR_UNKNOWN").to_string();
        let detail = row.error_detail.clone().unwrap_or_else(|| format!("步骤 {} 失败", row.id));
        run.error_code = Some(code.clone());
        run.error_detail = Some(detail.clone());
        let failed_key = row.step_key.as_deref().unwrap_or("");
        // 失败步骤本身也计入 settled，否则 continue 分支会永远等待已失败的步骤。
        ctx.skipped.push(failed_key.to_string());
        let descendants = downstream_steps(&workflow, failed_key);
        ctx.skipped.extend(descendants);
        let abort = step.map(|s| s.on_failure != "continue").unwrap_or(true);
        if abort {
            for s in &workflow.steps {
                if !ctx.outputs.contains_key(&s.step_key) {
                    ctx.skipped.push(s.step_key.clone());
                }
            }
            cancel_in_flight(db, app, run_id).await;
            // 终态失败也要带走本轮 skipped 清单，便于运行面板显示跳过步骤。
            if let Err(e) = run_repo::patch_run(
                db,
                run_id,
                BotWorkflowRunPatch {
                    context: Some(ctx.clone()),
                    error_code: Some(code.clone()),
                    error_detail: Some(detail.clone()),
                    ..Default::default()
                },
            )
            .await
            {
                tracing::warn!(run_id, error = %e, "abort 落 skipped/错误码失败");
            }
            let code_ref = code_in(&code).unwrap_or(bot_err::WORKFLOW_STEP_BOT_MISSING);
            if let Err(e) = fail_run(db, app, &run, code_ref, &detail).await {
                tracing::warn!(run_id, error = %e, "abort 落 run.failed 失败");
            }
            #[cfg(not(mobile))]
            if let Err(e) = crate::commands::desktop::send_desktop_notification_opt(
                app.cloned(),
                "Bot 编排步骤失败".to_string(),
                format!("{}：{}（{}）", failed_key, row.assignee_name, code),
            )
            .await
            {
                tracing::warn!(run_id, error = %e, "编排失败通知发送失败");
            }
            notifier(run_id).notify_one();
            return;
        }
    }
    ctx.skipped.sort();
    ctx.skipped.dedup();

    let assignments =
        asmt_repo::list_by_workflow_run(db, run_id, |_| String::new()).await.unwrap_or_default();
    let all_terminal = assignments.iter().all(|a| asmt_repo::status::is_terminal(&a.status));
    let all_steps_settled = workflow
        .steps
        .iter()
        .all(|s| ctx.outputs.contains_key(&s.step_key) || ctx.skipped.contains(&s.step_key));
    let next_status = if all_terminal && all_steps_settled {
        if run.error_code.is_some() {
            run_repo::status::FAILED
        } else {
            run_repo::status::COMPLETED
        }
    } else {
        run_repo::status::WAITING
    };
    let now = if run_repo::status::is_terminal(next_status) {
        Some(now_ts())
    } else {
        None
    };
    let completed_steps = ctx.outputs.len() as i32;
    match run_repo::patch_run(
        db,
        run_id,
        BotWorkflowRunPatch {
            status: Some(next_status.to_string()),
            context: Some(ctx),
            completed_steps: Some(completed_steps),
            error_code: run.error_code.clone(),
            error_detail: run.error_detail.clone(),
            finished_at: now,
            ..Default::default()
        },
    )
    .await
    {
        Ok(updated) => {
            emit_run(app, &updated);
            if run_repo::status::is_terminal(next_status) {
                crate::bot_quota::AssignmentContext::remove(run_id);
            }
        },
        Err(e) => tracing::warn!(run_id, error = %e, "run 上下文更新失败"),
    }
    // 通知复用 P2-a 桌面管线；run 级不另发通知。
    #[cfg(not(mobile))]
    if let Err(e) = crate::commands::desktop::send_desktop_notification_opt(
        app.cloned(),
        "Bot 编排步骤完成".to_string(),
        format!(
            "{}：{}（{}）",
            row.step_key.as_deref().unwrap_or("-"),
            row.assignee_name,
            row.status
        ),
    )
    .await
    {
        tracing::warn!(run_id, error = %e, "编排完成通知发送失败");
    }
    notifier(run_id).notify_one();
}

fn downstream_steps(workflow: &BotWorkflow, failed_key: &str) -> Vec<String> {
    let mut changed: HashSet<String> = [failed_key.to_string()].into_iter().collect();
    loop {
        let mut added = Vec::new();
        for step in &workflow.steps {
            if !changed.contains(&step.step_key)
                && step.depends_on.iter().any(|dep| changed.contains(dep))
            {
                added.push(step.step_key.clone());
            }
        }
        if added.is_empty() {
            break;
        }
        changed.extend(added);
    }
    changed.remove(failed_key);
    let mut out: Vec<String> = changed.into_iter().collect();
    out.sort();
    out
}

async fn cancel_in_flight(db: &DatabaseConnection, app: Option<&AppHandle>, run_id: &str) {
    let rows =
        asmt_repo::list_by_workflow_run(db, run_id, |_| String::new()).await.unwrap_or_default();
    for row in rows {
        if asmt_repo::status::is_terminal(&row.status) {
            continue;
        }
        if let Ok(mut cancelled) = asmt_repo::cancel_assignment(db, &row.id).await {
            cancelled.assignee_name = row.assignee_name.clone();
            if let Some(app) = app {
                let _ = app.emit(IpcEventName::BotAssignmentUpdated.as_str(), &cancelled);
            }
        }
    }
}

/// 内部失败终态（不取消在途单；abort 路径另行调用 `cancel_in_flight`）。
pub async fn fail_run(
    db: &DatabaseConnection,
    app: Option<&AppHandle>,
    run: &BotWorkflowRun,
    code: &'static str,
    detail: &str,
) -> std::result::Result<BotWorkflowRun, String> {
    let patched = run_repo::patch_run(
        db,
        &run.id,
        BotWorkflowRunPatch {
            status: Some(run_repo::status::FAILED.to_string()),
            error_code: Some(code.to_string()),
            error_detail: Some(detail.to_string()),
            finished_at: Some(now_ts()),
            ..Default::default()
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    emit_run(app, &patched);
    notifier(&run.id).notify_one();
    crate::bot_quota::AssignmentContext::remove(&run.id);
    Ok(patched)
}

/// 取消整棵 run（含步骤 assignment 及其所有子孙；PLAN-P3 §3 测试⑧ / ADR-P3-2 ⑤）。
pub async fn cancel_run(
    db: &DatabaseConnection,
    app: Option<&AppHandle>,
    run_id: &str,
) -> std::result::Result<BotWorkflowRun, String> {
    let run = run_repo::get_run(db, run_id).await.map_err(|e| {
        crate::commands::bot_workflow::run_error(bot_err::WORKFLOW_RUN_NOT_FOUND, e.to_string())
    })?;
    if run_repo::status::is_terminal(&run.status) {
        return Err(crate::commands::bot_workflow::run_error(
            bot_err::WORKFLOW_CANCEL_INVALID,
            format!("run {} 已终态 {}", run.id, run.status),
        ));
    }
    let roots = asmt_repo::list_by_workflow_run(db, run_id, |_| String::new())
        .await
        .map_err(|e| e.to_string())?;
    let mut root_ids: HashSet<String> = roots.iter().map(|a| a.root_id.clone()).collect();
    let mut seen = HashSet::new();
    while let Some(root_id) = root_ids.iter().find(|id| seen.insert((*id).clone())).cloned() {
        for row in
            asmt_repo::list_by_root(db, &root_id, |_| String::new()).await.unwrap_or_default()
        {
            if !asmt_repo::status::is_terminal(&row.status) {
                if let Ok(mut cancelled) = asmt_repo::cancel_assignment(db, &row.id).await {
                    cancelled.assignee_name = row.assignee_name.clone();
                    if let Some(app) = app {
                        let _ = app.emit(IpcEventName::BotAssignmentUpdated.as_str(), &cancelled);
                    }
                }
            }
            root_ids.insert(row.id);
        }
    }
    let patched = run_repo::patch_run(
        db,
        run_id,
        BotWorkflowRunPatch {
            status: Some(run_repo::status::CANCELLED.to_string()),
            finished_at: Some(now_ts()),
            ..Default::default()
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    emit_run(app, &patched);
    notifier(run_id).notify_one();
    crate::bot_quota::AssignmentContext::remove(run_id);
    Ok(patched)
}

/// 后台编排主循环：就绪集 → 并发门 → 挂钩子调度；等待完成通知或总超时。
async fn orchestrate(
    db: DatabaseConnection,
    app: Option<AppHandle>,
    run: BotWorkflowRun,
    workflow: BotWorkflow,
) {
    let run_id = run.id.clone();
    let notify = notifier(&run_id);
    let limits = crate::bot_quota::AssignmentContext::limits_for_run(Some(&run_id));
    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(limits.total_timeout_secs.max(1) as u64);
    let mut launched: HashSet<String> = HashSet::new();

    loop {
        let Ok(current) = run_repo::get_run(&db, &run_id).await else {
            return;
        };
        if run_repo::status::is_terminal(&current.status) {
            crate::bot_quota::AssignmentContext::remove(&run_id);
            return;
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            cancel_in_flight(&db, app.as_ref(), &run_id).await;
            if let Err(e) = fail_run(
                &db,
                app.as_ref(),
                &current,
                bot_err::WORKFLOW_TIMEOUT,
                &format!("编排总时长超过 {} 秒", limits.total_timeout_secs),
            )
            .await
            {
                tracing::warn!(run_id, error = %e, "总超时落 run.failed 失败");
            }
            return;
        }

        let assignments = asmt_repo::list_by_workflow_run(&db, &run_id, |_| String::new())
            .await
            .unwrap_or_default();
        let in_flight =
            assignments.iter().filter(|a| !asmt_repo::status::is_terminal(&a.status)).count();
        let ctx: BotWorkflowRunContext = current.context.clone();
        let settled =
            |key: &str| ctx.outputs.contains_key(key) || ctx.skipped.contains(&key.to_string());

        let ready: Vec<&axagent_harness::types::BotWorkflowStep> = workflow
            .steps
            .iter()
            .filter(|s| !settled(&s.step_key) && !launched.contains(&s.step_key))
            .filter(|s| s.depends_on.iter().all(|dep| settled(dep)))
            .collect();
        let capacity = (limits.max_concurrency as usize).saturating_sub(in_flight);
        for step in ready.into_iter().take(capacity) {
            let prompt = crate::bot_workflow_steps::substitute_prompt(
                &step.prompt_template,
                &ctx.inputs,
                &ctx.outputs,
            );
            let input = axagent_harness::types::CreateBotAssignment {
                assignee_bot_id: step.bot_id.clone(),
                task_text: prompt,
                mode: axagent_tools::tools::bot_delegate::MODE_DISPATCH.to_string(),
                requester_conversation_id: Some(current.requester_conversation_id.clone()),
                origin: Some("workflow".to_string()),
                workflow_run_id: Some(run_id.clone()),
                step_key: Some(step.step_key.clone()),
                ..Default::default()
            };
            match crate::commands::bot_assignment::create_step_assignment(&db, &input).await {
                Ok(created) => {
                    launched.insert(step.step_key.clone());
                    crate::commands::bot_assignment::schedule_step_dispatch(created.id.clone());
                },
                Err(e) => {
                    if let Err(e) = fail_run(
                        &db,
                        app.as_ref(),
                        &current,
                        code_in(&e).unwrap_or(bot_err::WORKFLOW_STEP_INVALID),
                        &e,
                    )
                    .await
                    {
                        tracing::warn!(run_id, error = %e, "步骤建单失败落 run.failed 失败");
                    }
                    return;
                },
            }
        }

        let all_settled = workflow.steps.iter().all(|s| settled(&s.step_key));
        let any_in_flight = assignments.iter().any(|a| !asmt_repo::status::is_terminal(&a.status))
            || launched.iter().any(|key| !settled(key));
        if all_settled && !any_in_flight {
            let status = if current.error_code.is_some() {
                run_repo::status::FAILED
            } else {
                run_repo::status::COMPLETED
            };
            if let Err(e) = run_repo::patch_run(
                &db,
                &run_id,
                BotWorkflowRunPatch {
                    status: Some(status.to_string()),
                    finished_at: Some(now_ts()),
                    ..Default::default()
                },
            )
            .await
            {
                tracing::warn!(run_id, error = %e, "编排终态落库失败");
            } else if let Ok(final_run) = run_repo::get_run(&db, &run_id).await {
                emit_run(app.as_ref(), &final_run);
            }
            crate::bot_quota::AssignmentContext::remove(&run_id);
            return;
        }

        // 未到下轮事件就睡眠；任何 assignment 终态都会 notify_one 唤醒。
        tokio::select! {
            _ = notify.notified() => {}
            _ = tokio::time::sleep(remaining.min(Duration::from_secs(5))) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downstream_skips_transitive_dependents() {
        let steps: Vec<axagent_harness::types::BotWorkflowStep> = ["a", "b", "c", "d"]
            .iter()
            .map(|key| axagent_harness::types::BotWorkflowStep {
                step_key: key.to_string(),
                bot_id: "bot".to_string(),
                prompt_template: String::new(),
                depends_on: match *key {
                    "b" => vec!["a".to_string()],
                    "c" => vec!["b".to_string()],
                    _ => vec![],
                },
                timeout_secs: 30,
                on_failure: "abort".to_string(),
            })
            .collect();
        let wf = BotWorkflow {
            id: "wf".into(),
            name: "wf".into(),
            description: String::new(),
            owner_scope: "instance".into(),
            department_id: None,
            steps,
            max_concurrency: 2,
            total_timeout_secs: 900,
            created_at: 0,
            updated_at: 0,
        };
        assert_eq!(downstream_steps(&wf, "a"), vec!["b".to_string(), "c".to_string()]);
        assert_eq!(downstream_steps(&wf, "d"), Vec::<String>::new());
    }
}
