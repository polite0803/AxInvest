// SPDX-License-Identifier: AGPL-3.0-only

//! Bot 委派 Tauri 命令（docs/bot-integration/P2/IPC-P2B §1/§2.2/§2.3/§3）。
//!
//! 命令层只做校验 + 并行门 + 落库 + 后台执行派发；真正的 assignee 轮由
//! `bot_assignment_runner`（T3''）实现，经 [`set_dispatch_scheduler`] 注入，
//! 使命令层单测在无 AppState / 无模型环境下可跑。

use axagent_dao::repo::bot as bot_repo;
use axagent_dao::repo::bot_assignment::{self as asmt_repo, AssignmentFilter};
use axagent_harness::core_error::AxAgentError;
use axagent_harness::event_names::IpcEventName;
use axagent_harness::types::{BotAssignment, CreateBotAssignment};
use tauri::{AppHandle, Emitter, State};

use crate::AppState;
use crate::commands::error::{ErrorCategory, ErrorResponse};
use crate::commands::error_code::bot as bot_err;

/// 单 requester / 全局在途 dispatch 上限（IPC-P2B §3，Q5 冻结值）。
pub const MAX_IN_FLIGHT_PER_REQUESTER: i64 = 4;
pub const MAX_IN_FLIGHT_GLOBAL: i64 = 8;

const MIN_TASK_CHARS: usize = 1;
const MAX_TASK_CHARS: usize = 20_000;

/// 后台执行派发器签名。T3'' 注入 `tokio::spawn` 的真实 runner；测试期可注入记数桩。
type DispatchScheduler = dyn Fn(String) + Send + Sync;

fn default_scheduler(id: String) {
    // 未注入 runner 时留审计日志，不留幽灵轮（T3'' 接上后覆盖）。
    tracing::warn!(assignment_id = %id, "委派执行器尚未接入，assignment 停留在 queued");
}

static DISPATCH_SCHEDULER: std::sync::OnceLock<Box<DispatchScheduler>> = std::sync::OnceLock::new();

/// T3'' 接 runner 时调用（进程启动时一次）。
// T3'' 接 runner 时调用；当前卡（T4''）只落命令层，故此处暂未消费。
#[allow(dead_code)]
pub fn set_dispatch_scheduler(scheduler: impl Fn(String) + Send + Sync + 'static) {
    let _ = DISPATCH_SCHEDULER.set(Box::new(scheduler));
}

fn schedule(id: String) {
    match DISPATCH_SCHEDULER.get() {
        Some(s) => s(id),
        None => default_scheduler(id),
    }
}

/// T3'''：步骤/子链落库后由 runner 决定调度路径（完成钩子要挂到 run），
/// 不在此处直接 `schedule`。无 workflow 的纯链仍走 P2-b 默认路径。
pub fn schedule_step_dispatch(id: String) {
    if let Some(scheduler) = crate::bot_workflow_runner::step_scheduler() {
        scheduler(id);
    } else {
        schedule(id);
    }
}

fn validation_error(code: &str, field: &str, detail: impl Into<String>) -> String {
    ErrorResponse::new(code)
        .with_category(ErrorCategory::Validation)
        .with_param("field", field)
        .with_detail(detail)
        .to_string()
}

fn map_asmt_error(err: AxAgentError) -> String {
    match err {
        AxAgentError::NotFound(detail) => ErrorResponse::new(bot_err::ASSIGNMENT_NOT_FOUND)
            .with_category(ErrorCategory::Validation)
            .with_detail(detail)
            .to_string(),
        AxAgentError::Validation(detail) => ErrorResponse::new("ERR_VALIDATION")
            .with_category(ErrorCategory::Validation)
            .with_detail(detail)
            .to_string(),
        other => ErrorResponse::from_error(other, ErrorCategory::Unrecoverable).to_string(),
    }
}

async fn assignee_name(db: &axagent_dao::DatabaseConnection, id: &str) -> String {
    // 命令层只读已有 Bot 记录；删 Bot 后审计行保留，名字回落到 id。
    bot_repo::get_bot(db, id).await.map(|b| b.name).unwrap_or_else(|_| id.to_string())
}

/// P3 ADR-P3-2 ③④：多层链的环/深度/逐层预算/链总预算校验。
/// 返回的错误串含可审计的 error_code（工具层据此转 LLM 可读文本）。
pub async fn check_chain_constraints(
    db: &axagent_dao::DatabaseConnection,
    input: &CreateBotAssignment,
) -> std::result::Result<(), String> {
    let Some(parent_id) = input.parent_assignment_id.as_deref() else {
        return Ok(());
    };
    let parent = asmt_repo::get_assignment(db, parent_id, |_| String::new())
        .await
        .map_err(map_asmt_error)?;
    let depth = parent.depth + 1;
    if depth > asmt_repo::MAX_DEPTH {
        return Err(validation_error(
            bot_err::DELEGATE_MAX_DEPTH,
            "parentAssignmentId",
            format!("委派链深度上限为 {}", asmt_repo::MAX_DEPTH),
        ));
    }
    let chain = asmt_repo::chain_bot_ids(db, parent_id).await.map_err(map_asmt_error)?;
    if chain.iter().any(|id| id == &input.assignee_bot_id) {
        return Err(validation_error(
            bot_err::DELEGATE_CYCLE,
            "assigneeBotId",
            format!("Bot {} 已在委派链上", input.assignee_bot_id),
        ));
    }
    let in_flight = asmt_repo::count_in_flight_dispatch_at_depth(db, &parent.root_id, depth)
        .await
        .map_err(map_asmt_error)?;
    // P4：部门配额可把每层在途上限收紧到 1..4；无配额（纯委派/实例级）沿用 P3 的 2。
    let depth_cap = crate::bot_quota::AssignmentContext::limits_for_run(
        input.workflow_run_id.as_deref(),
    )
    .max_in_flight_per_depth;
    if in_flight >= depth_cap as i64 {
        return Err(validation_error(
            bot_err::DELEGATE_BUSY,
            "assigneeBotId",
            format!("root {} depth {depth} 在途已达 {depth_cap}", parent.root_id),
        ));
    }
    // 链总预算从根开始计，不逐层重置。
    let root = asmt_repo::get_assignment(db, &parent.root_id, |_| String::new())
        .await
        .map_err(map_asmt_error)?;
    if axagent_harness::util_fns::now_ts().saturating_sub(root.created_at)
        >= crate::bot_assignment_runner::DISPATCH_TIMEOUT_SECS as i64
    {
        return Err(validation_error(
            bot_err::DELEGATE_TIMEOUT,
            "parentAssignmentId",
            "委派链总时长已达 300 秒",
        ));
    }
    Ok(())
}

/// 编排步骤/子链的落库入口：复用 create_inner 的校验与并行门。
/// 按 PLAN-P3 §3 不在此调用 scheduler —— 由 runner 挂完成钩子后显式调度。
pub async fn create_step_assignment(
    db: &axagent_dao::DatabaseConnection,
    input: &CreateBotAssignment,
) -> std::result::Result<BotAssignment, String> {
    check_chain_constraints(db, input).await?;
    create_inner(db, input).await
}
async fn create_inner(
    db: &axagent_dao::DatabaseConnection,
    input: &CreateBotAssignment,
) -> std::result::Result<BotAssignment, String> {
    if input.mode != "dispatch" {
        // IPC-P2B §2.3：consult 是 assignee 轮工具语义，person 不直接发起。
        return Err(validation_error("ERR_VALIDATION", "mode", "consult cannot be created via IPC"));
    }
    let len = input.task_text.chars().count();
    if len < MIN_TASK_CHARS || len > MAX_TASK_CHARS {
        return Err(validation_error(
            "ERR_VALIDATION",
            "taskText",
            format!("taskText must be {MIN_TASK_CHARS}..{MAX_TASK_CHARS} characters"),
        ));
    }

    let assignee = bot_repo::get_bot(db, &input.assignee_bot_id)
        .await
        .map_err(|_| validation_error(bot_err::NOT_FOUND, "assigneeBotId", "Bot 不存在"))?;
    if !assignee.is_enabled {
        return Err(validation_error(bot_err::NOT_FOUND, "assigneeBotId", "Bot 已禁用"));
    }
    if let Some(requester) = &input.requester_bot_id {
        if requester == &input.assignee_bot_id {
            return Err(validation_error(
                bot_err::DELEGATE_SELF,
                "assigneeBotId",
                "Bot 不能委派给自己",
            ));
        }
        let requester_bot = bot_repo::get_bot(db, requester)
            .await
            .map_err(|_| validation_error(bot_err::NOT_FOUND, "requesterBotId", "Bot 不存在"))?;
        if !requester_bot.is_enabled {
            return Err(validation_error(bot_err::NOT_FOUND, "requesterBotId", "Bot 已禁用"));
        }
    }

    // 并行门（IPC-P2B §3）：单 requester 4 / 全局 8。
    let per = asmt_repo::count_in_flight_dispatch(db, input.requester_bot_id.as_deref())
        .await
        .map_err(map_asmt_error)?;
    let global = asmt_repo::count_in_flight_dispatch_global(db).await.map_err(map_asmt_error)?;
    if per >= MAX_IN_FLIGHT_PER_REQUESTER || global >= MAX_IN_FLIGHT_GLOBAL {
        return Err(validation_error(
            bot_err::DELEGATE_BUSY,
            "assigneeBotId",
            format!("in-flight dispatch: requester={per}, global={global}"),
        ));
    }

    let asmt = asmt_repo::create_assignment(db, input).await.map_err(map_asmt_error)?;
    let mut result = asmt;
    result.assignee_name = assignee.name;
    Ok(result)
}

#[tauri::command]
pub async fn create_bot_assignment(
    app: AppHandle,
    state: State<'_, AppState>,
    input: CreateBotAssignment,
) -> Result<BotAssignment, String> {
    let db = state.harness.db();
    let created = create_inner(db, &input).await?;
    let id = created.id.clone();
    // 落库即发 queued 事件；running/终态由 T3'' runner 在状态迁移处发。
    let _ = app.emit(IpcEventName::BotAssignmentUpdated.as_str(), &created);
    // 后台轮由 T3'' 注入的 scheduler 推进状态机。
    schedule(id);
    Ok(created)
}

#[tauri::command]
pub async fn list_bot_assignments(
    state: State<'_, AppState>,
    requester_bot_id: Option<String>,
    assignee_bot_id: Option<String>,
    bot_id: Option<String>,
    status: Option<String>,
    root_id: Option<String>,
) -> Result<Vec<BotAssignment>, String> {
    let db = state.harness.db();
    let filter = AssignmentFilter { requester_bot_id, assignee_bot_id, bot_id, status, root_id, ..Default::default() };
    let rows = asmt_repo::list_assignments(db, &filter, |_| String::new())
        .await
        .map_err(map_asmt_error)?;
    let mut out = Vec::with_capacity(rows.len());
    for mut row in rows {
        row.assignee_name = assignee_name(db, &row.assignee_bot_id).await;
        out.push(row);
    }
    Ok(out)
}

#[tauri::command]
pub async fn cancel_bot_assignment(
    app: AppHandle,
    state: State<'_, AppState>,
    assignment_id: String,
) -> Result<BotAssignment, String> {
    let db = state.harness.db();
    let mut row = asmt_repo::cancel_assignment(db, &assignment_id).await.map_err(map_asmt_error)?;
    row.assignee_name = assignee_name(db, &row.assignee_bot_id).await;
    let _ = app.emit(IpcEventName::BotAssignmentUpdated.as_str(), &row);
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> axagent_dao::db::DbHandle {
        axagent_dao::db::create_test_pool().await.expect("测试库")
    }

    fn input(assignee: &str, requester: Option<&str>, mode: &str) -> CreateBotAssignment {
        CreateBotAssignment {
            assignee_bot_id: assignee.to_string(),
            task_text: "汇总今日盘面".to_string(),
            mode: mode.to_string(),
            requester_conversation_id: None,
            requester_bot_id: requester.map(|s| s.to_string()),
            ..Default::default()
        }
    }

    async fn make_bot(db: &axagent_dao::DatabaseConnection, name: &str) -> String {
        bot_repo::create_bot(
            db,
            &axagent_harness::types::CreateBot { name: name.to_string(), ..Default::default() },
        )
        .await
        .expect("建 Bot")
        .id
    }

    #[tokio::test]
    async fn self_delegation_rejected() {
        let h = pool().await;
        let bot = make_bot(&h.conn, "A").await;
        let err = create_inner(&h.conn, &input(&bot, Some(&bot), "dispatch"))
            .await
            .expect_err("自身委派应拒绝");
        assert!(err.contains(bot_err::DELEGATE_SELF), "{err}");
    }

    #[tokio::test]
    async fn bad_assignee_rejected() {
        let h = pool().await;
        let err = create_inner(&h.conn, &input("missing-bot", None, "dispatch"))
            .await
            .expect_err("不存在 assignee 应拒绝");
        assert!(err.contains(bot_err::NOT_FOUND), "{err}");
    }

    #[tokio::test]
    async fn consult_mode_rejected_with_validation() {
        let h = pool().await;
        let bot = make_bot(&h.conn, "B").await;
        let err = create_inner(&h.conn, &input(&bot, None, "consult"))
            .await
            .expect_err("person 直接发 consult 应拒绝");
        assert!(err.contains("ERR_VALIDATION"), "{err}");
    }

    #[tokio::test]
    async fn created_dto_is_queued_and_named() {
        let h = pool().await;
        let bot = make_bot(&h.conn, "执行者").await;
        let a = create_inner(&h.conn, &input(&bot, None, "dispatch")).await.expect("建单");
        assert_eq!(a.status, "queued");
        assert_eq!(a.assignee_name, "执行者");
        assert_eq!(a.root_id, a.id);
        assert_eq!(a.depth, 1);
    }

    #[tokio::test]
    async fn parallel_gate_enforced() {
        let h = pool().await;
        let db = &h.conn;
        let requester = make_bot(db, "R").await;
        let assignee = make_bot(db, "A").await;
        for _ in 0..MAX_IN_FLIGHT_PER_REQUESTER {
            create_inner(db, &input(&assignee, Some(&requester), "dispatch"))
                .await
                .expect("门内应成功");
        }
        let err = create_inner(db, &input(&assignee, Some(&requester), "dispatch"))
            .await
            .expect_err("第 5 单应被门拒绝");
        assert!(err.contains(bot_err::DELEGATE_BUSY), "{err}");
    }

    #[tokio::test]
    async fn cancel_terminal_rejected() {
        let h = pool().await;
        let db = &h.conn;
        let bot = make_bot(db, "A").await;
        let a = create_inner(db, &input(&bot, None, "dispatch")).await.expect("建单");
        let cancelled = asmt_repo::cancel_assignment(db, &a.id).await.expect("取消 queued");
        assert_eq!(cancelled.status, "cancelled");
        let err = asmt_repo::cancel_assignment(db, &a.id).await;
        assert!(matches!(err, Err(AxAgentError::Validation(_))), "终态再取消应报 Validation");
    }
}
