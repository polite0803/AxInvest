// SPDX-License-Identifier: AGPL-3.0-only

//! `bot_workflows` 仓库层（docs/bot-integration/P3/MODEL-P3 §2.1 / §4）。
//! `steps_json` 对外只以 `Vec<BotWorkflowStep>` 形态出现，JSON 序列化在此收口。

use sea_orm::*;

use axagent_entities::bot_workflows;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::types::{BotWorkflow, BotWorkflowStep};
use axagent_harness::util_fns::{gen_id, now_ts};

fn to_dto(m: bot_workflows::Model) -> Result<BotWorkflow> {
    let steps: Vec<BotWorkflowStep> = serde_json::from_str(&m.steps_json)
        .map_err(|e| AxAgentError::Validation(format!("编排步骤 JSON 损坏: {e}")))?;
    Ok(BotWorkflow {
        id: m.id,
        name: m.name,
        description: m.description,
        owner_scope: m.owner_scope,
        department_id: m.department_id,
        steps,
        max_concurrency: m.max_concurrency,
        total_timeout_secs: m.total_timeout_secs,
        created_at: m.created_at,
        updated_at: m.updated_at,
    })
}

async fn load(db: &DatabaseConnection, id: &str) -> Result<bot_workflows::Model> {
    bot_workflows::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("BotWorkflow {id}")))
}

pub async fn get_workflow(db: &DatabaseConnection, id: &str) -> Result<BotWorkflow> {
    to_dto(load(db, id).await?)
}

/// `department_id=None` = 全部；`Some(dept)` = 实例级 + 该部门。
pub async fn list_workflows(
    db: &DatabaseConnection,
    department_id: Option<&str>,
) -> Result<Vec<BotWorkflow>> {
    let mut query = bot_workflows::Entity::find();
    if let Some(dept) = department_id {
        let mut cond = Condition::any().add(bot_workflows::Column::DepartmentId.is_null());
        if !dept.is_empty() {
            cond = cond.add(bot_workflows::Column::DepartmentId.eq(dept));
        }
        query = query.filter(cond);
    }
    let rows = query.order_by_desc(bot_workflows::Column::CreatedAt).all(db).await?;
    rows.into_iter().map(to_dto).collect()
}

pub async fn create_workflow(
    db: &DatabaseConnection,
    input: &axagent_harness::types::CreateBotWorkflow,
) -> Result<BotWorkflow> {
    let id = gen_id();
    let now = now_ts();
    let steps = serde_json::to_string(&input.steps)
        .map_err(|e| AxAgentError::Validation(format!("编排步骤序列化失败: {e}")))?;
    let am = bot_workflows::ActiveModel {
        id: Set(id.clone()),
        name: Set(input.name.clone()),
        description: Set(input.description.clone()),
        owner_scope: Set(input.owner_scope.clone()),
        department_id: Set(input.department_id.as_ref().and_then(|v| v.clone())),
        steps_json: Set(steps),
        max_concurrency: Set(input.max_concurrency),
        total_timeout_secs: Set(input.total_timeout_secs),
        created_at: Set(now),
        updated_at: Set(now),
    };
    am.insert(db).await?;
    get_workflow(db, &id).await
}

pub async fn update_workflow(
    db: &DatabaseConnection,
    id: &str,
    patch: &axagent_harness::types::UpdateBotWorkflowPatch,
) -> Result<BotWorkflow> {
    let mut am: bot_workflows::ActiveModel = load(db, id).await?.into();
    if let Some(v) = &patch.name {
        am.name = Set(v.clone());
    }
    if let Some(v) = &patch.description {
        am.description = Set(v.clone().unwrap_or_default());
    }
    if let Some(v) = &patch.owner_scope {
        am.owner_scope = Set(v.clone());
    }
    if let Some(v) = &patch.department_id {
        am.department_id = Set(v.clone());
    }
    if let Some(v) = &patch.steps {
        am.steps_json = Set(serde_json::to_string(v)
            .map_err(|e| AxAgentError::Validation(format!("编排步骤序列化失败: {e}")))?);
    }
    if let Some(v) = patch.max_concurrency {
        am.max_concurrency = Set(v);
    }
    if let Some(v) = patch.total_timeout_secs {
        am.total_timeout_secs = Set(v);
    }
    am.updated_at = Set(now_ts());
    am.update(db).await?;
    get_workflow(db, id).await
}

pub async fn delete_workflow(db: &DatabaseConnection, id: &str) -> Result<()> {
    bot_workflows::Entity::delete_by_id(id).exec(db).await?;
    Ok(())
}

/// 删部门：该部门的编排转实例归属保留（ADR-P3-3），定义不删。
pub async fn transfer_department_to_instance(
    db: &DatabaseConnection,
    department_id: &str,
) -> Result<u64> {
    let am = bot_workflows::ActiveModel {
        owner_scope: Set("instance".to_string()),
        department_id: Set(None),
        updated_at: Set(now_ts()),
        ..Default::default()
    };
    let res = bot_workflows::Entity::update_many()
        .set(am)
        .filter(bot_workflows::Column::DepartmentId.eq(department_id))
        .exec(db)
        .await?;
    Ok(res.rows_affected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn steps_json_roundtrip_and_instance_transfer() {
        let h = crate::db::create_test_pool().await.expect("测试库");
        let db = &h.conn;
        let steps = vec![BotWorkflowStep {
            step_key: "research".into(),
            bot_id: "bot-a".into(),
            prompt_template: "汇总 {{steps.other.output}}".into(),
            depends_on: vec!["other".into()],
            timeout_secs: 30,
            on_failure: "abort".into(),
        }];
        let wf = create_workflow(
            db,
            &axagent_harness::types::CreateBotWorkflow {
                name: "盘前汇总".into(),
                owner_scope: "department".into(),
                department_id: Some(Some("dept-1".into())),
                steps: steps.clone(),
                ..Default::default()
            },
        )
        .await
        .expect("建编排");
        assert_eq!(wf.steps, steps, "steps_json round-trip");
        let got = get_workflow(db, &wf.id).await.expect("读回");
        assert_eq!(got.steps[0].step_key, "research");
        let all = list_workflows(db, Some("dept-1")).await.expect("筛选");
        assert_eq!(all.len(), 1);

        let n = transfer_department_to_instance(db, "dept-1").await.expect("转实例");
        assert_eq!(n, 1);
        let after = get_workflow(db, &wf.id).await.expect("读回");
        assert_eq!(after.owner_scope, "instance");
        assert_eq!(after.department_id, None);
        assert_eq!(after.steps, steps, "转归属不动定义");
    }
}
