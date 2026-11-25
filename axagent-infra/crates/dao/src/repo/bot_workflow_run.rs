// SPDX-License-Identifier: AGPL-3.0-only

//! `bot_workflow_runs` 仓库层（docs/bot-integration/P3/MODEL-P3 §2.2 / §4）。

use sea_orm::*;

use axagent_entities::{bot_workflow_runs, bot_workflows};
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::types::{
    BotWorkflowRun, BotWorkflowRunContext, BotWorkflowRunPatch, CreateBotWorkflowRun,
};
use axagent_harness::util_fns::{gen_id, now_ts};

pub mod status {
    pub const QUEUED: &str = "queued";
    pub const RUNNING: &str = "running";
    pub const WAITING: &str = "waiting";
    pub const COMPLETED: &str = "completed";
    pub const FAILED: &str = "failed";
    pub const CANCELLED: &str = "cancelled";
    pub const TERMINAL: &[&str] = &[COMPLETED, FAILED, CANCELLED];

    pub fn is_terminal(s: &str) -> bool {
        TERMINAL.contains(&s)
    }
    pub fn is_valid(s: &str) -> bool {
        matches!(s, QUEUED | RUNNING | WAITING | COMPLETED | FAILED | CANCELLED)
    }
}

fn parse_context(raw: &str) -> Result<BotWorkflowRunContext> {
    if raw.trim().is_empty() {
        return Ok(BotWorkflowRunContext::default());
    }
    serde_json::from_str(raw)
        .map_err(|e| AxAgentError::Validation(format!("run context_json 损坏: {e}")))
}

fn to_dto(m: bot_workflow_runs::Model, workflow_name: String) -> Result<BotWorkflowRun> {
    Ok(BotWorkflowRun {
        id: m.id,
        workflow_id: m.workflow_id,
        workflow_name,
        status: m.status,
        origin: m.origin,
        requester_bot_id: m.requester_bot_id,
        requester_conversation_id: m.requester_conversation_id,
        anchor_message_id: m.anchor_message_id,
        context: parse_context(&m.context_json)?,
        completed_steps: m.completed_steps,
        total_steps: m.total_steps,
        error_code: m.error_code,
        error_detail: m.error_detail,
        started_at: m.started_at,
        finished_at: m.finished_at,
        created_at: m.created_at,
        updated_at: m.updated_at,
    })
}

async fn load(db: &DatabaseConnection, id: &str) -> Result<bot_workflow_runs::Model> {
    bot_workflow_runs::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("BotWorkflowRun {id}")))
}

async fn workflow_name(db: &DatabaseConnection, id: &str) -> String {
    axagent_entities::bot_workflows::Entity::find_by_id(id)
        .one(db)
        .await
        .ok()
        .flatten()
        .map(|m| m.name)
        .unwrap_or_else(|| id.to_string())
}

pub async fn get_run(db: &DatabaseConnection, id: &str) -> Result<BotWorkflowRun> {
    let row = load(db, id).await?;
    let name = workflow_name(db, &row.workflow_id).await;
    to_dto(row, name)
}

pub async fn list_runs(
    db: &DatabaseConnection,
    workflow_id: Option<&str>,
    status_filter: Option<&str>,
) -> Result<Vec<BotWorkflowRun>> {
    let mut query = bot_workflow_runs::Entity::find();
    if let Some(v) = workflow_id {
        query = query.filter(bot_workflow_runs::Column::WorkflowId.eq(v));
    }
    if let Some(v) = status_filter {
        query = query.filter(bot_workflow_runs::Column::Status.eq(v));
    }
    let rows = query.order_by_desc(bot_workflow_runs::Column::CreatedAt).all(db).await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let name = workflow_name(db, &row.workflow_id).await;
        out.push(to_dto(row, name)?);
    }
    Ok(out)
}

/// 检查定义是否存在未结束的 run（改/删定义前置校验）。
/// 部门同时在跑的编排数（P4 配额 `max_concurrent_runs`）：该部门名下所有编排的
/// 非终态 run。`exclude_run_id` 供 start_run 复核时排除本 run 自身。
pub async fn count_active_runs_by_department(
    db: &DatabaseConnection,
    department_id: &str,
    exclude_run_id: Option<&str>,
) -> Result<u64> {
    let workflow_ids: Vec<String> = bot_workflows::Entity::find()
        .filter(bot_workflows::Column::DepartmentId.eq(department_id))
        .all(db)
        .await?
        .into_iter()
        .map(|w| w.id)
        .collect();
    if workflow_ids.is_empty() {
        return Ok(0);
    }
    let mut query = bot_workflow_runs::Entity::find()
        .filter(bot_workflow_runs::Column::WorkflowId.is_in(workflow_ids))
        .filter(bot_workflow_runs::Column::Status.is_not_in(status::TERMINAL.iter().copied()));
    if let Some(run_id) = exclude_run_id {
        query = query.filter(bot_workflow_runs::Column::Id.ne(run_id));
    }
    Ok(query.count(db).await?)
}

pub async fn has_active_runs(db: &DatabaseConnection, workflow_id: &str) -> Result<bool> {
    let n = bot_workflow_runs::Entity::find()
        .filter(bot_workflow_runs::Column::WorkflowId.eq(workflow_id))
        .filter(bot_workflow_runs::Column::Status.is_not_in(status::TERMINAL.iter().copied()))
        .count(db)
        .await?;
    Ok(n > 0)
}

pub async fn create_run(
    db: &DatabaseConnection,
    input: &CreateBotWorkflowRun,
) -> Result<BotWorkflowRun> {
    let id = gen_id();
    let now = now_ts();
    let ctx = BotWorkflowRunContext { inputs: input.inputs.clone(), ..Default::default() };
    let am = bot_workflow_runs::ActiveModel {
        id: Set(id.clone()),
        workflow_id: Set(input.workflow_id.clone()),
        status: Set(status::QUEUED.to_string()),
        origin: Set(input.origin.clone()),
        requester_bot_id: Set(input.requester_bot_id.clone()),
        requester_conversation_id: Set(input.requester_conversation_id.clone()),
        anchor_message_id: Set(None),
        context_json: Set(serde_json::to_string(&ctx)
            .map_err(|e| AxAgentError::Validation(format!("run context 序列化失败: {e}")))?),
        completed_steps: Set(0),
        total_steps: Set(input.total_steps),
        error_code: Set(None),
        error_detail: Set(None),
        started_at: Set(None),
        finished_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };
    am.insert(db).await?;
    get_run(db, &id).await
}

/// run 状态/上下文单入口。状态变迁合法性由调用方（runner）把关。
pub async fn patch_run(
    db: &DatabaseConnection,
    id: &str,
    patch: BotWorkflowRunPatch,
) -> Result<BotWorkflowRun> {
    let mut am: bot_workflow_runs::ActiveModel = load(db, id).await?.into();
    if let Some(v) = patch.status {
        am.status = Set(v);
    }
    if let Some(v) = patch.anchor_message_id {
        am.anchor_message_id = Set(Some(v));
    }
    if let Some(v) = patch.context {
        am.context_json = Set(serde_json::to_string(&v)
            .map_err(|e| AxAgentError::Validation(format!("run context 序列化失败: {e}")))?);
    }
    if let Some(v) = patch.completed_steps {
        am.completed_steps = Set(v);
    }
    if let Some(v) = patch.error_code {
        am.error_code = Set(Some(v));
    }
    if let Some(v) = patch.error_detail {
        am.error_detail = Set(Some(v));
    }
    if let Some(v) = patch.started_at {
        am.started_at = Set(Some(v));
    }
    if let Some(v) = patch.finished_at {
        am.finished_at = Set(Some(v));
    }
    am.updated_at = Set(now_ts());
    am.update(db).await?;
    get_run(db, id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_roundtrip_context_and_active_query() {
        let h = crate::db::create_test_pool().await.expect("测试库");
        let db = &h.conn;
        let wf_id = "wf-lifecycle";
        axagent_entities::bot_workflows::ActiveModel {
            id: Set(wf_id.into()),
            name: Set("测试编排".into()),
            created_at: Set(now_ts()),
            updated_at: Set(now_ts()),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("建定义");
        let run = create_run(
            db,
            &CreateBotWorkflowRun {
                workflow_id: wf_id.into(),
                workflow_name: "测试编排".into(),
                origin: "person".into(),
                requester_bot_id: None,
                requester_conversation_id: "conv-1".into(),
                total_steps: 2,
                inputs: [("topic".into(), "盘面".into())].into_iter().collect(),
            },
        )
        .await
        .expect("建 run");
        assert_eq!(run.status, status::QUEUED);
        assert_eq!(run.workflow_name, "测试编排");
        assert_eq!(run.context.inputs["topic"], "盘面");
        assert!(has_active_runs(db, wf_id).await.expect("在途查询"));

        let mut ctx = run.context.clone();
        ctx.outputs.insert(
            "a".into(),
            axagent_harness::types::BotWorkflowStepOutput {
                assignment_id: "as-1".into(),
                result_text: "结论".into(),
            },
        );
        let updated = patch_run(
            db,
            &run.id,
            BotWorkflowRunPatch {
                status: Some(status::WAITING.into()),
                context: Some(ctx),
                anchor_message_id: Some("msg-anchor".into()),
                ..Default::default()
            },
        )
        .await
        .expect("patch");
        assert_eq!(updated.status, status::WAITING);
        assert_eq!(updated.context.outputs["a"].result_text, "结论");
        assert_eq!(updated.anchor_message_id.as_deref(), Some("msg-anchor"));
        patch_run(
            db,
            &run.id,
            BotWorkflowRunPatch { status: Some(status::COMPLETED.into()), ..Default::default() },
        )
        .await
        .expect("终态");
        assert!(!has_active_runs(db, wf_id).await.expect("在途查询"));
    }
}
