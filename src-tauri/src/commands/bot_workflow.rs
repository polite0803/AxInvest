// SPDX-License-Identifier: AGPL-3.0-only

//! Bot 编排定义 Tauri 命令（docs/bot-integration/P3/IPC-P3 §1/§2.1/§2.2/§2.4）。
//! 定义期校验在 `bot_workflow_steps`（纯函数），命令层只做 Bot/部门存在性与在途 run 检查。

use axagent_dao::repo::bot as bot_repo;
use axagent_dao::repo::bot_department as dept_repo;
use axagent_dao::repo::bot_workflow as workflow_repo;
use axagent_dao::repo::bot_workflow_run as run_repo;
use axagent_harness::event_names::IpcEventName;
use axagent_harness::types::{
    BotWorkflow, BotWorkflowRun, BotWorkflowStep, CreateBotWorkflow, CreateBotWorkflowRun,
    UpdateBotWorkflowPatch,
};
use tauri::{AppHandle, Emitter, State};

use crate::AppState;
use crate::bot_workflow_steps::{StepError, validate_definition};
use crate::commands::error::{ErrorCategory, ErrorResponse};
use crate::commands::error_code::bot as bot_err;

fn response(code: &'static str, field: &str, detail: impl Into<String>) -> String {
    ErrorResponse::new(code)
        .with_category(ErrorCategory::Validation)
        .with_param("field", field)
        .with_detail(detail)
        .to_string()
}

fn map_error(err: axagent_harness::core_error::AxAgentError, code: &'static str) -> String {
    response(code, "id", err.to_string())
}

/// runner / 命令共用的 run 级错误出口（无需 AppState）。
pub fn run_error(code: &'static str, detail: impl Into<String>) -> String {
    response(code, "id", detail)
}

/// Bot 存在 / enabled / 部门成员检查（定义期 + 运行前共用的 DB 半段）。
pub async fn validate_bot_refs(
    db: &axagent_dao::DatabaseConnection,
    steps: &[BotWorkflowStep],
    department_id: Option<&str>,
) -> std::result::Result<(), String> {
    for step in steps {
        let bot = bot_repo::get_bot(db, &step.bot_id).await.map_err(|_| {
            response(
                bot_err::WORKFLOW_STEP_BOT_MISSING,
                "botId",
                format!("Bot {} 不存在", step.bot_id),
            )
        })?;
        if !bot.is_enabled {
            return Err(response(
                bot_err::WORKFLOW_STEP_BOT_MISSING,
                "botId",
                format!("Bot {} 已禁用", step.bot_id),
            ));
        }
        if let Some(dept) = department_id {
            let member = dept_repo::department_of_bot(db, &step.bot_id)
                .await
                .map_err(|e| map_error(e, bot_err::WORKFLOW_DEPARTMENT_REQUIRED))?;
            if member.as_deref() != Some(dept) {
                return Err(response(
                    bot_err::WORKFLOW_MEMBER_CHANGED,
                    "botId",
                    format!("Bot {} 不属于部门 {dept}", step.bot_id),
                ));
            }
        }
    }
    Ok(())
}

fn validate_definition_or_response(
    name: &str,
    description: &str,
    owner_scope: &str,
    department_id: Option<&str>,
    steps: &[BotWorkflowStep],
    max_concurrency: i32,
    total_timeout_secs: i32,
) -> std::result::Result<(), String> {
    validate_definition(
        name,
        description,
        owner_scope,
        department_id,
        steps,
        max_concurrency,
        total_timeout_secs,
    )
    .map(|_| ())
    .map_err(StepError::into_response)
}

pub async fn create_inner(
    db: &axagent_dao::DatabaseConnection,
    input: &CreateBotWorkflow,
) -> Result<BotWorkflow, String> {
    if input.owner_scope == "department" {
        let Some(dept_id) = input.department_id.as_ref().and_then(|v| v.as_deref()) else {
            return Err(response(
                bot_err::WORKFLOW_DEPARTMENT_REQUIRED,
                "departmentId",
                "部门编排必须指定部门",
            ));
        };
        dept_repo::get_department(db, dept_id)
            .await
            .map_err(|e| map_error(e, bot_err::DEPARTMENT_NOT_FOUND))?;
    }
    validate_definition_or_response(
        &input.name,
        &input.description,
        &input.owner_scope,
        input.department_id.as_ref().and_then(|v| v.as_deref()),
        &input.steps,
        input.max_concurrency,
        input.total_timeout_secs,
    )?;
    let dept = if input.owner_scope == "department" {
        input.department_id.as_ref().and_then(|v| v.as_deref())
    } else {
        None
    };
    // P4 配额（ADR-P4-2）：部门作用域且存在配额行时，定义值超限直接拒绝。
    if let Some(dept_id) = dept {
        crate::bot_quota::check_definition_allowed(
            db,
            dept_id,
            input.max_concurrency,
            input.total_timeout_secs,
            &input.steps,
        )
        .await?;
    }
    validate_bot_refs(db, &input.steps, dept).await?;
    workflow_repo::create_workflow(db, input)
        .await
        .map_err(|e| map_error(e, bot_err::WORKFLOW_NOT_FOUND))
}

#[tauri::command]
pub async fn create_bot_workflow(
    state: State<'_, AppState>,
    input: CreateBotWorkflow,
) -> Result<BotWorkflow, String> {
    create_inner(state.harness.db(), &input).await
}

pub async fn update_inner(
    db: &axagent_dao::DatabaseConnection,
    id: &str,
    patch: &UpdateBotWorkflowPatch,
) -> Result<BotWorkflow, String> {
    let current = workflow_repo::get_workflow(db, &id)
        .await
        .map_err(|_| response(bot_err::WORKFLOW_NOT_FOUND, "id", format!("编排 {id} 不存在")))?;
    if run_repo::has_active_runs(db, &id)
        .await
        .map_err(|e| map_error(e, bot_err::WORKFLOW_RUN_BUSY))?
    {
        return Err(response(bot_err::WORKFLOW_RUN_BUSY, "id", "编排存在未结束的运行"));
    }

    let name = patch.name.clone().unwrap_or(current.name);
    let description = patch.description.clone().flatten().unwrap_or(current.description);
    let owner_scope = patch.owner_scope.clone().unwrap_or(current.owner_scope);
    let department_id = if owner_scope == "instance" {
        None
    } else {
        patch.department_id.clone().unwrap_or(current.department_id)
    };
    let steps = patch.steps.clone().unwrap_or(current.steps);
    let max_concurrency = patch.max_concurrency.unwrap_or(current.max_concurrency);
    let total_timeout_secs = patch.total_timeout_secs.unwrap_or(current.total_timeout_secs);

    if owner_scope == "department" {
        let Some(dept_id) = department_id.as_deref() else {
            return Err(response(
                bot_err::WORKFLOW_DEPARTMENT_REQUIRED,
                "departmentId",
                "部门编排必须指定部门",
            ));
        };
        dept_repo::get_department(db, dept_id)
            .await
            .map_err(|e| map_error(e, bot_err::DEPARTMENT_NOT_FOUND))?;
    }
    validate_definition_or_response(
        &name,
        &description,
        &owner_scope,
        department_id.as_deref(),
        &steps,
        max_concurrency,
        total_timeout_secs,
    )?;
    if owner_scope == "department" {
        if let Some(dept_id) = department_id.as_deref() {
            crate::bot_quota::check_definition_allowed(
                db,
                dept_id,
                max_concurrency,
                total_timeout_secs,
                &steps,
            )
            .await?;
        }
    }
    validate_bot_refs(db, &steps, department_id.as_deref()).await?;

    workflow_repo::update_workflow(
        db,
        id,
        &UpdateBotWorkflowPatch {
            name: Some(name),
            description: Some(Some(description)),
            owner_scope: Some(owner_scope),
            department_id: Some(department_id),
            steps: Some(steps),
            max_concurrency: Some(max_concurrency),
            total_timeout_secs: Some(total_timeout_secs),
        },
    )
    .await
    .map_err(|e| map_error(e, bot_err::WORKFLOW_NOT_FOUND))
}

/// 由命令层的扁平入参组装 patch；外层 None = 不改，内层 None = 显式清除。
fn update_patch_from_args(
    name: Option<String>,
    description: Option<Option<String>>,
    owner_scope: Option<String>,
    department_id: Option<Option<String>>,
    steps: Option<Vec<BotWorkflowStep>>,
    max_concurrency: Option<i32>,
    total_timeout_secs: Option<i32>,
) -> UpdateBotWorkflowPatch {
    UpdateBotWorkflowPatch {
        name,
        description,
        owner_scope,
        department_id,
        steps,
        max_concurrency,
        total_timeout_secs,
    }
}

/// IPC-P3 §2.4：冻结入参 = 显式 `id` + 扁平 patch，无 `{ workflowId, input }` 兼容分支。
#[tauri::command]
pub async fn update_bot_workflow(
    state: State<'_, AppState>,
    id: String,
    name: Option<String>,
    description: Option<Option<String>>,
    owner_scope: Option<String>,
    department_id: Option<Option<String>>,
    steps: Option<Vec<BotWorkflowStep>>,
    max_concurrency: Option<i32>,
    total_timeout_secs: Option<i32>,
) -> Result<BotWorkflow, String> {
    let patch = update_patch_from_args(
        name,
        description,
        owner_scope,
        department_id,
        steps,
        max_concurrency,
        total_timeout_secs,
    );
    update_inner(state.harness.db(), &id, &patch).await
}

/// `None` = 全部；`Some(None)` = 仅 instance；`Some(Some(id))` = instance + 该部门。
pub async fn list_inner(
    db: &axagent_dao::DatabaseConnection,
    department_id: Option<Option<String>>,
) -> Result<Vec<BotWorkflow>, String> {
    let mut rows =
        workflow_repo::list_workflows(db, department_id.as_ref().and_then(|v| v.as_deref()))
            .await
            .map_err(|e| map_error(e, bot_err::WORKFLOW_NOT_FOUND))?;
    if matches!(department_id, Some(None)) {
        rows.retain(|w| w.owner_scope == "instance");
    }
    Ok(rows)
}

#[tauri::command]
pub async fn list_bot_workflows(
    state: State<'_, AppState>,
    department_id: Option<Option<String>>,
) -> Result<Vec<BotWorkflow>, String> {
    list_inner(state.harness.db(), department_id).await
}

pub async fn delete_inner(db: &axagent_dao::DatabaseConnection, id: &str) -> Result<(), String> {
    workflow_repo::get_workflow(db, id)
        .await
        .map_err(|_| response(bot_err::WORKFLOW_NOT_FOUND, "id", format!("编排 {id} 不存在")))?;
    if run_repo::has_active_runs(db, &id)
        .await
        .map_err(|e| map_error(e, bot_err::WORKFLOW_RUN_BUSY))?
    {
        return Err(response(bot_err::WORKFLOW_RUN_BUSY, "id", "编排存在未结束的运行"));
    }
    workflow_repo::delete_workflow(db, id)
        .await
        .map_err(|e| map_error(e, bot_err::WORKFLOW_NOT_FOUND))
}

#[tauri::command]
pub async fn delete_bot_workflow(state: State<'_, AppState>, id: String) -> Result<(), String> {
    delete_inner(state.harness.db(), &id).await
}

#[tauri::command]
pub async fn run_bot_workflow(
    app: AppHandle,
    state: State<'_, AppState>,
    workflow_id: String,
    requester_conversation_id: Option<String>,
    inputs: Option<std::collections::HashMap<String, String>>,
) -> Result<BotWorkflowRun, String> {
    let db = state.harness.db();
    let workflow = workflow_repo::get_workflow(db, &workflow_id)
        .await
        .map_err(|e| map_error(e, bot_err::WORKFLOW_NOT_FOUND))?;
    let Some(conversation_id) = requester_conversation_id.filter(|v| !v.trim().is_empty()) else {
        return Err(response("ERR_VALIDATION", "requesterConversationId", "手工触发必须指定线程"));
    };
    // P4 配额（ADR-P4-2）：定义不超配额 + 部门同时在跑编排数未达上限。
    crate::bot_quota::check_run_allowed(db, &workflow, None).await?;
    let queued = run_repo::create_run(
        db,
        &CreateBotWorkflowRun {
            workflow_id: workflow.id.clone(),
            workflow_name: workflow.name.clone(),
            origin: "person".to_string(),
            requester_bot_id: None,
            requester_conversation_id: conversation_id,
            total_steps: workflow.steps.len() as i32,
            inputs: inputs.unwrap_or_default(),
        },
    )
    .await
    .map_err(|e| map_error(e, bot_err::WORKFLOW_RUN_NOT_FOUND))?;
    let _ = app.emit(IpcEventName::BotWorkflowRunUpdated.as_str(), &queued);
    crate::bot_workflow_runner::start_run(db, Some(app), &queued.id).await
}

#[tauri::command]
pub async fn list_bot_workflow_runs(
    state: State<'_, AppState>,
    workflow_id: Option<String>,
    status: Option<String>,
) -> Result<Vec<BotWorkflowRun>, String> {
    if let Some(value) = &status {
        if !run_repo::status::is_valid(value) {
            return Err(response("ERR_VALIDATION", "status", format!("非法 run 状态 {value}")));
        }
    }
    run_repo::list_runs(state.harness.db(), workflow_id.as_deref(), status.as_deref())
        .await
        .map_err(|e| map_error(e, bot_err::WORKFLOW_RUN_NOT_FOUND))
}

#[tauri::command]
pub async fn cancel_bot_workflow_run(
    app: AppHandle,
    state: State<'_, AppState>,
    run_id: String,
) -> Result<BotWorkflowRun, String> {
    crate::bot_workflow_runner::cancel_run(state.harness.db(), Some(&app), &run_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_workflow_args_keep_missing_and_clear_distinct() {
        let patch = update_patch_from_args(
            Some("新名".into()),
            Some(None),
            None,
            Some(None),
            None,
            None,
            None,
        );
        assert_eq!(patch.name.as_deref(), Some("新名"));
        assert!(matches!(patch.description, Some(None)), "描述显式 null = 清除");
        assert!(patch.owner_scope.is_none(), "缺键 = 不改");
        assert!(matches!(patch.department_id, Some(None)), "部门显式 null = 清除");
        assert!(patch.steps.is_none());
    }
    use axagent_harness::types::BotWorkflowStep;

    async fn pool() -> axagent_dao::db::DbHandle {
        axagent_dao::db::create_test_pool().await.expect("测试库")
    }

    async fn make_bot(db: &axagent_dao::DatabaseConnection, name: &str) -> String {
        bot_repo::create_bot(
            db,
            &axagent_harness::types::CreateBot { name: name.into(), ..Default::default() },
        )
        .await
        .expect("建 Bot")
        .id
    }

    fn input(bot_id: &str) -> CreateBotWorkflow {
        CreateBotWorkflow {
            name: "盘前汇总".into(),
            owner_scope: "instance".into(),
            steps: vec![BotWorkflowStep {
                step_key: "a".into(),
                bot_id: bot_id.into(),
                prompt_template: "分析 {{inputs.topic}}".into(),
                timeout_secs: 300,
                on_failure: "abort".into(),
                ..Default::default()
            }],
            max_concurrency: 2,
            total_timeout_secs: 900,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn create_update_list_delete_roundtrip() {
        let h = pool().await;
        let db = &h.conn;
        let bot = make_bot(db, "步骤 Bot").await;
        let created = create_inner(db, &input(&bot)).await.expect("建编排");
        assert_eq!(created.name, "盘前汇总");
        assert_eq!(created.steps.len(), 1);
        let listed = list_inner(db, None).await.expect("列表");
        assert_eq!(listed.len(), 1);
        let instance_only = list_inner(db, Some(None)).await.expect("实例筛选");
        assert_eq!(instance_only.len(), 1);

        let updated = update_inner(
            db,
            &created.id,
            &UpdateBotWorkflowPatch {
                description: Some(Some("新描述".into())),
                ..Default::default()
            },
        )
        .await
        .expect("更新");
        assert_eq!(updated.description, "新描述");

        delete_inner(db, &created.id).await.expect("删除");
        let missing = delete_inner(db, &created.id).await;
        assert!(missing.is_err(), "重复删除应报错");
    }

    #[tokio::test]
    async fn definition_validation_errors() {
        let h = pool().await;
        let db = &h.conn;
        let bot = make_bot(db, "步骤 Bot2").await;
        let mut bad = input(&bot);
        bad.steps[0].step_key = "bad key".into();
        let err = create_inner(db, &bad).await.unwrap_err();
        assert!(err.contains(bot_err::WORKFLOW_STEP_INVALID), "{err}");

        let mut dept = input(&bot);
        dept.owner_scope = "department".into();
        let err = create_inner(db, &dept).await.unwrap_err();
        assert!(err.contains(bot_err::WORKFLOW_DEPARTMENT_REQUIRED), "{err}");

        let mut missing_bot = input(&"not-a-bot".to_string());
        missing_bot.steps[0].bot_id = "not-a-bot".into();
        let err = create_inner(db, &missing_bot).await.unwrap_err();
        assert!(err.contains(bot_err::WORKFLOW_STEP_BOT_MISSING), "{err}");
    }
}
