// SPDX-License-Identifier: AGPL-3.0-only

//! `bot_assignments` 表的仓库层（P2-B MODEL-P2B §2.3 + P3 MODEL-P3 §2.3）。
//!
//! 状态机迁移（queued→running→终态 + cancelled）的合法性校验在 runner/命令层，
//! 本层提供 [`transition`] 一个受控入口按 IPC-P2B 的五态表逐态把关。
//! 无 `parent_assignment_id` 时仍是 P2-b 一层根单（`root_id = id`、`depth = 1`）；
//! 多层委派由 `parent_assignment_id` 显式搭树，depth 上限 3（ADR-P3-2）。

use sea_orm::*;

use axagent_entities::bot_assignments;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::types::{BotAssignment, CreateBotAssignment};
use axagent_harness::util_fns::{gen_id, now_ts};

pub mod status {
    pub const QUEUED: &str = "queued";
    pub const RUNNING: &str = "running";
    pub const COMPLETED: &str = "completed";
    pub const FAILED: &str = "failed";
    pub const CANCELLED: &str = "cancelled";

    /// 终态集合。
    pub const TERMINAL: &[&str] = &[COMPLETED, FAILED, CANCELLED];

    pub fn is_terminal(s: &str) -> bool {
        TERMINAL.contains(&s)
    }
}

fn assignment_from_entity(m: bot_assignments::Model, assignee_name: String) -> BotAssignment {
    BotAssignment {
        id: m.id,
        root_id: m.root_id,
        parent_id: m.parent_id,
        depth: m.depth,
        origin: m.origin,
        requester_bot_id: m.requester_bot_id,
        requester_conversation_id: m.requester_conversation_id,
        assignee_bot_id: m.assignee_bot_id,
        assignee_name,
        mode: m.mode,
        task_text: m.task_text,
        workflow_run_id: m.workflow_run_id,
        step_key: m.step_key,
        status: m.status,
        result_text: m.result_text,
        error_code: m.error_code,
        error_detail: m.error_detail,
        started_at: m.started_at,
        finished_at: m.finished_at,
        created_at: m.created_at,
        updated_at: m.updated_at,
    }
}

async fn load(db: &DatabaseConnection, id: &str) -> Result<bot_assignments::Model> {
    bot_assignments::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("BotAssignment {}", id)))
}

/// 读取单条（assignee_name 由调用方回填或传入空串由命令层补）。
pub async fn get_assignment(
    db: &DatabaseConnection,
    id: &str,
    assignee_name: impl FnOnce(&str) -> String,
) -> Result<BotAssignment> {
    let row = load(db, id).await?;
    let name = assignee_name(&row.assignee_bot_id);
    Ok(assignment_from_entity(row, name))
}

#[derive(Debug, Clone, Default)]
pub struct AssignmentFilter {
    pub requester_bot_id: Option<String>,
    pub assignee_bot_id: Option<String>,
    pub bot_id: Option<String>,
    pub status: Option<String>,
    /// P3 运行期树边筛选。
    pub root_id: Option<String>,
    pub parent_id: Option<String>,
    pub workflow_run_id: Option<String>,
    pub step_key: Option<String>,
}

pub async fn list_assignments(
    db: &DatabaseConnection,
    filter: &AssignmentFilter,
    assignee_name: impl Fn(&str) -> String,
) -> Result<Vec<BotAssignment>> {
    let mut query = bot_assignments::Entity::find();
    if let Some(v) = &filter.requester_bot_id {
        query = query.filter(bot_assignments::Column::RequesterBotId.eq(v.clone()));
    }
    if let Some(v) = &filter.assignee_bot_id {
        query = query.filter(bot_assignments::Column::AssigneeBotId.eq(v.clone()));
    }
    if let Some(v) = &filter.bot_id {
        // requester 或 assignee 任一命中。
        query = query.filter(
            Condition::any()
                .add(bot_assignments::Column::RequesterBotId.eq(v.clone()))
                .add(bot_assignments::Column::AssigneeBotId.eq(v.clone())),
        );
    }
    if let Some(v) = &filter.status {
        query = query.filter(bot_assignments::Column::Status.eq(v.clone()));
    }
    if let Some(v) = &filter.root_id {
        query = query.filter(bot_assignments::Column::RootId.eq(v.clone()));
    }
    if let Some(v) = &filter.parent_id {
        query = query.filter(bot_assignments::Column::ParentId.eq(v.clone()));
    }
    if let Some(v) = &filter.workflow_run_id {
        query = query.filter(bot_assignments::Column::WorkflowRunId.eq(v.clone()));
    }
    if let Some(v) = &filter.step_key {
        query = query.filter(bot_assignments::Column::StepKey.eq(v.clone()));
    }
    let rows = query.order_by_desc(bot_assignments::Column::CreatedAt).all(db).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let name = assignee_name(&r.assignee_bot_id);
            assignment_from_entity(r, name)
        })
        .collect())
}

/// 新建委派合同。无 parent 时保持 P2-b 一层根单；有 parent 时挂到其链上。
/// 状态机校验与业务校验（Bot 存在 / 非自身 / 并行门）在命令层。
pub const MAX_DEPTH: i32 = 3;

pub async fn create_assignment(
    db: &DatabaseConnection,
    input: &CreateBotAssignment,
) -> Result<BotAssignment> {
    let id = gen_id();
    let now = now_ts();

    let (root_id, parent_id, depth, inherited);
    if let Some(parent_id_value) = &input.parent_assignment_id {
        let parent = load(db, parent_id_value).await?;
        let child_depth = parent.depth + 1;
        if child_depth > MAX_DEPTH {
            return Err(AxAgentError::Validation(format!(
                "委派链深度超过 {MAX_DEPTH}: parent={} depth={}",
                parent.id, parent.depth
            )));
        }
        // 环防护：父链上已出现同一 Bot 时拒绝（T3 的动态环检测在工具层，这里是落库兜底）。
        let mut cursor = Some(parent.clone());
        let mut seen = std::collections::HashSet::new();
        while let Some(row) = cursor {
            if !seen.insert(row.id.clone()) {
                return Err(AxAgentError::Validation("委派链存在环".into()));
            }
            if row.assignee_bot_id == input.assignee_bot_id {
                return Err(AxAgentError::Validation(format!(
                    "Bot {} 已在委派链上",
                    input.assignee_bot_id
                )));
            }
            cursor = match &row.parent_id {
                Some(pid) => bot_assignments::Entity::find_by_id(pid).one(db).await?,
                None => None,
            };
        }
        root_id = if let Some(r) = &input.root_assignment_id {
            r.clone()
        } else {
            parent.root_id.clone()
        };
        parent_id = Some(parent.id.clone());
        depth = child_depth;
        inherited = (
            input.origin.clone().unwrap_or_else(|| "bot".to_string()),
            input.requester_bot_id.clone().or_else(|| Some(parent.assignee_bot_id.clone())),
            input
                .requester_conversation_id_override
                .clone()
                .or_else(|| input.requester_conversation_id.clone())
                .or_else(|| parent.requester_conversation_id.clone()),
            input.workflow_run_id.clone().or(parent.workflow_run_id.clone()),
            input.step_key.clone().or(parent.step_key.clone()),
        );
    } else {
        root_id = input.root_assignment_id.clone().unwrap_or_else(|| id.clone());
        parent_id = None;
        depth = 1;
        inherited = (
            input.origin.clone().unwrap_or_else(|| {
                input.requester_bot_id.as_ref().map(|_| "bot").unwrap_or("person").to_string()
            }),
            input.requester_bot_id.clone(),
            input
                .requester_conversation_id_override
                .clone()
                .or_else(|| input.requester_conversation_id.clone()),
            input.workflow_run_id.clone(),
            input.step_key.clone(),
        );
    }
    let (origin, requester_bot_id, requester_conversation_id, workflow_run_id, step_key) =
        inherited;

    let am = bot_assignments::ActiveModel {
        id: Set(id.clone()),
        root_id: Set(root_id),
        parent_id: Set(parent_id),
        depth: Set(depth),
        origin: Set(origin),
        requester_bot_id: Set(requester_bot_id),
        requester_conversation_id: Set(requester_conversation_id),
        assignee_bot_id: Set(input.assignee_bot_id.clone()),
        mode: Set(input.mode.clone()),
        task_text: Set(input.task_text.clone()),
        workflow_run_id: Set(workflow_run_id),
        step_key: Set(step_key),
        status: Set(status::QUEUED.to_string()),
        result_text: Set(None),
        error_code: Set(None),
        error_detail: Set(None),
        started_at: Set(None),
        finished_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };
    am.insert(db).await?;
    let row = load(db, &id).await?;
    Ok(assignment_from_entity(row, String::new()))
}

/// 受控状态迁移。合法迁移表（IPC-P2B §5）：
/// queued→running/cancelled；running→completed/failed/cancelled；终态→不可再迁。
#[derive(Debug, Clone, Default)]
pub struct TransitionPatch {
    pub result_text: Option<String>,
    pub error_code: Option<String>,
    pub error_detail: Option<String>,
    pub finished_at: Option<i64>,
}

pub async fn transition(
    db: &DatabaseConnection,
    id: &str,
    to_status: &str,
    patch: TransitionPatch,
) -> Result<BotAssignment> {
    let row = load(db, id).await?;
    let from = row.status.as_str();
    let allowed = match from {
        "queued" => matches!(to_status, "running" | "cancelled"),
        "running" => matches!(to_status, "completed" | "failed" | "cancelled"),
        _ => false,
    };
    if !allowed {
        return Err(AxAgentError::Validation(format!(
            "illegal assignment transition {from} -> {to_status}"
        )));
    }

    let mut am: bot_assignments::ActiveModel = row.into();
    am.status = Set(to_status.to_string());
    if let Some(v) = patch.result_text {
        am.result_text = Set(Some(v));
    }
    if let Some(v) = patch.error_code {
        am.error_code = Set(Some(v));
    }
    if let Some(v) = patch.error_detail {
        am.error_detail = Set(Some(v));
    }
    if let Some(v) = patch.finished_at {
        am.finished_at = Set(Some(v));
    }
    if to_status == status::RUNNING {
        am.started_at = Set(Some(now_ts()));
    }
    am.updated_at = Set(now_ts());
    am.update(db).await?;
    let fresh = load(db, id).await?;
    Ok(assignment_from_entity(fresh, String::new()))
}

/// 取消在途单（queued/running → cancelled）。终态返回 Validation。
pub async fn cancel_assignment(db: &DatabaseConnection, id: &str) -> Result<BotAssignment> {
    let row = load(db, id).await?;
    if status::is_terminal(&row.status) {
        return Err(AxAgentError::Validation(format!(
            "assignment {} already in terminal state {}",
            id, row.status
        )));
    }
    transition(
        db,
        id,
        status::CANCELLED,
        TransitionPatch { finished_at: Some(now_ts()), ..Default::default() },
    )
    .await
}

/// 删 Bot 级联（ADR-P2B-6 / IPC-P2B §1 delete_bot 行）：该 Bot 在途的委派单置
/// cancelled，其余单保留审计。返回 (在途单被取消的条数)。
pub async fn cancel_in_flight_of_bot(db: &DatabaseConnection, bot_id: &str) -> Result<u64> {
    let rows = bot_assignments::Entity::find()
        .filter(
            Condition::any()
                .add(bot_assignments::Column::AssigneeBotId.eq(bot_id))
                .add(bot_assignments::Column::RequesterBotId.eq(bot_id)),
        )
        .filter(bot_assignments::Column::Status.is_in([status::QUEUED, status::RUNNING]))
        .all(db)
        .await?;
    let mut n = 0u64;
    for row in rows {
        let id = row.id.clone();
        if transition(
            db,
            &id,
            status::CANCELLED,
            TransitionPatch {
                error_detail: Some("assignee/requester bot deleted".to_string()),
                finished_at: Some(now_ts()),
                ..Default::default()
            },
        )
        .await
        .is_ok()
        {
            n += 1;
        }
    }
    Ok(n)
}

/// 单 requester 在途 dispatch 计数（并行门，IPC-P2B §3）。
pub async fn count_in_flight_dispatch(
    db: &DatabaseConnection,
    requester_bot_id: Option<&str>,
) -> Result<i64> {
    let mut query = bot_assignments::Entity::find()
        .filter(bot_assignments::Column::Mode.eq("dispatch"))
        .filter(bot_assignments::Column::Status.is_in([status::QUEUED, status::RUNNING]));
    if let Some(r) = requester_bot_id {
        query = query.filter(bot_assignments::Column::RequesterBotId.eq(r));
    } else {
        query = query.filter(bot_assignments::Column::RequesterBotId.is_null());
    }
    Ok(query.count(db).await? as i64)
}

/// 全局在途 dispatch 计数（并行门，IPC-P2B §3）。
pub async fn count_in_flight_dispatch_global(db: &DatabaseConnection) -> Result<i64> {
    let n = bot_assignments::Entity::find()
        .filter(bot_assignments::Column::Mode.eq("dispatch"))
        .filter(bot_assignments::Column::Status.is_in([status::QUEUED, status::RUNNING]))
        .count(db)
        .await?;
    Ok(n as i64)
}

/// consult 工具的审计行：同步语义，status 直接终态（ADR-P2B-4），无 queued/running 窗口。
/// 恒一层赋值与 dispatch 相同（origin 由调用方给出：bot / routine）。
pub async fn record_consult_assignment(
    db: &DatabaseConnection,
    origin: &str,
    requester_bot_id: &str,
    assignee_bot_id: &str,
    task_text: &str,
    outcome: std::result::Result<String, String>,
) -> Result<BotAssignment> {
    record_consult_assignment_with_code(
        db,
        origin,
        requester_bot_id,
        assignee_bot_id,
        task_text,
        outcome,
        None,
    )
    .await
}

/// P3：带 error_code 的 consult 审计行（环/深度门拒绝时落 BOT_DELEGATE_* 码）。
pub async fn record_consult_assignment_with_code(
    db: &DatabaseConnection,
    origin: &str,
    requester_bot_id: &str,
    assignee_bot_id: &str,
    task_text: &str,
    outcome: std::result::Result<String, String>,
    error_code_override: Option<&str>,
) -> Result<BotAssignment> {
    let id = gen_id();
    let now = now_ts();
    let (status_v, result_text, error_code, error_detail, finished_at) = match &outcome {
        Ok(text) => (status::COMPLETED, Some(text.clone()), None, None, Some(now)),
        Err(e) => (
            status::FAILED,
            None,
            Some(error_code_override.unwrap_or("ERR_PERMISSION").to_string()),
            Some(e.clone()),
            Some(now),
        ),
    };
    let am = bot_assignments::ActiveModel {
        id: Set(id.clone()),
        root_id: Set(id.clone()),
        parent_id: Set(None),
        depth: Set(1),
        origin: Set(origin.to_string()),
        requester_bot_id: Set(Some(requester_bot_id.to_string())),
        requester_conversation_id: Set(None),
        assignee_bot_id: Set(assignee_bot_id.to_string()),
        mode: Set("consult".to_string()),
        task_text: Set(task_text.to_string()),
        workflow_run_id: Set(None),
        step_key: Set(None),
        status: Set(status_v.to_string()),
        result_text: Set(result_text),
        error_code: Set(error_code),
        error_detail: Set(error_detail),
        started_at: Set(Some(now)),
        finished_at: Set(finished_at),
        created_at: Set(now),
        updated_at: Set(now),
    };
    am.insert(db).await?;
    let row = load(db, &id).await?;
    Ok(assignment_from_entity(row, String::new()))
}

/// 按编排 run 列出全部步骤 assignment（含继承链上的子孙单）。
pub async fn list_by_workflow_run(
    db: &DatabaseConnection,
    run_id: &str,
    assignee_name: impl Fn(&str) -> String,
) -> Result<Vec<BotAssignment>> {
    list_assignments(
        db,
        &AssignmentFilter { workflow_run_id: Some(run_id.to_string()), ..Default::default() },
        assignee_name,
    )
    .await
}

/// 按链根列出全部子孙 assignment（按 depth / 创建时间正序，便于终态等待）。
pub async fn list_by_root(
    db: &DatabaseConnection,
    root_id: &str,
    assignee_name: impl Fn(&str) -> String,
) -> Result<Vec<BotAssignment>> {
    let mut rows = list_assignments(
        db,
        &AssignmentFilter { root_id: Some(root_id.to_string()), ..Default::default() },
        assignee_name,
    )
    .await?;
    rows.sort_by_key(|a| (a.depth, a.created_at, a.id.clone()));
    Ok(rows)
}

/// 从某单沿 parent 向上收集自身 + 祖先的 Bot id（动态环检测 / 同 Bot 抑制）。
pub async fn chain_bot_ids(db: &DatabaseConnection, assignment_id: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut cursor = Some(load(db, assignment_id).await?);
    let mut seen = std::collections::HashSet::new();
    while let Some(row) = cursor {
        if !seen.insert(row.id.clone()) {
            return Err(AxAgentError::Validation(format!("委派链存在环: {}", row.id)));
        }
        out.push(row.assignee_bot_id.clone());
        cursor = match &row.parent_id {
            Some(pid) => bot_assignments::Entity::find_by_id(pid).one(db).await?,
            None => None,
        };
    }
    Ok(out)
}

/// 同一根链上、指定层号的在途 dispatch 数（每层在途 <= 2 的落库计数口径）。
pub async fn count_in_flight_dispatch_at_depth(
    db: &DatabaseConnection,
    root_id: &str,
    depth: i32,
) -> Result<i64> {
    let n = bot_assignments::Entity::find()
        .filter(bot_assignments::Column::RootId.eq(root_id))
        .filter(bot_assignments::Column::Depth.eq(depth))
        .filter(bot_assignments::Column::Mode.eq("dispatch"))
        .filter(bot_assignments::Column::Status.is_in([status::QUEUED, status::RUNNING]))
        .count(db)
        .await?;
    Ok(n as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> crate::db::DbHandle {
        crate::db::create_test_pool().await.expect("测试库应可建立")
    }

    fn input(assignee: &str, requester: Option<&str>) -> CreateBotAssignment {
        CreateBotAssignment {
            assignee_bot_id: assignee.to_string(),
            task_text: "帮我汇总今天的盘面".to_string(),
            mode: "dispatch".to_string(),
            requester_conversation_id: None,
            requester_bot_id: requester.map(|s| s.to_string()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn create_roundtrip_is_one_layer() {
        let h = pool().await;
        let db = &h.conn;
        let a = create_assignment(db, &input("bot-b", None)).await.expect("建单");
        assert_eq!(a.root_id, a.id, "一层委派 root=自链");
        assert_eq!(a.parent_id, None);
        assert_eq!(a.depth, 1);
        assert_eq!(a.origin, "person", "person 发起 requester=NULL");
        assert_eq!(a.requester_bot_id, None);
        assert_eq!(a.status, status::QUEUED);

        let b = create_assignment(db, &input("bot-c", Some("bot-b"))).await.expect("建单2");
        assert_eq!(b.origin, "bot");
        assert_eq!(b.requester_bot_id.as_deref(), Some("bot-b"));
        assert_eq!(b.depth, 1, "无 parent 的 Bot 发起仍是根单（depth=1）");
    }

    #[tokio::test]
    async fn multilayer_chain_fields_and_workflow_inheritance() {
        let h = pool().await;
        let db = &h.conn;
        let root = create_assignment(db, &input("bot-a", None)).await.expect("根单");
        let child = create_assignment(
            db,
            &CreateBotAssignment {
                parent_assignment_id: Some(root.id.clone()),
                ..input("bot-b", Some("bot-a"))
            },
        )
        .await
        .expect("第二层");
        assert_eq!(child.root_id, root.id);
        assert_eq!(child.parent_id.as_deref(), Some(root.id.as_str()));
        assert_eq!(child.depth, 2);
        assert_eq!(child.origin, "bot");

        let grand = create_assignment(
            db,
            &CreateBotAssignment {
                parent_assignment_id: Some(child.id.clone()),
                workflow_run_id: Some("run-1".into()),
                step_key: Some("step-1".into()),
                ..input("bot-c", Some("bot-b"))
            },
        )
        .await
        .expect("第三层");
        assert_eq!(grand.depth, 3);
        assert_eq!(grand.root_id, root.id);
        assert_eq!(grand.workflow_run_id.as_deref(), Some("run-1"));
        assert_eq!(grand.step_key.as_deref(), Some("step-1"));

        let too_deep = create_assignment(
            db,
            &CreateBotAssignment {
                parent_assignment_id: Some(grand.id.clone()),
                ..input("bot-d", Some("bot-c"))
            },
        )
        .await;
        assert!(matches!(too_deep, Err(AxAgentError::Validation(_))), "第四层应拒绝: {too_deep:?}");

        let repeat = create_assignment(
            db,
            &CreateBotAssignment {
                parent_assignment_id: Some(root.id.clone()),
                ..input("bot-a", Some("bot-a"))
            },
        )
        .await;
        assert!(
            matches!(repeat, Err(AxAgentError::Validation(_))),
            "同链重复 Bot 应拒绝: {repeat:?}"
        );

        let chain = chain_bot_ids(db, &grand.id).await.expect("祖先链");
        assert_eq!(chain, vec!["bot-c", "bot-b", "bot-a"]);
        let by_root = list_by_root(db, &root.id, |id| id.to_string()).await.expect("按根列");
        assert_eq!(by_root.len(), 3);
        assert_eq!(by_root.iter().map(|a| a.depth).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(count_in_flight_dispatch_at_depth(db, &root.id, 3).await.expect("层计数"), 1);

        // 显式 run 字段的子孙继承。
        let inherited = create_assignment(
            db,
            &CreateBotAssignment {
                parent_assignment_id: Some(grand.id.clone()),
                ..input("bot-d", Some("bot-c"))
            },
        )
        .await;
        assert!(inherited.is_err(), "超深拦截优先于继承");
        let step_root = create_assignment(
            db,
            &CreateBotAssignment {
                workflow_run_id: Some("run-2".into()),
                step_key: Some("s2".into()),
                ..input("bot-x", None)
            },
        )
        .await
        .expect("编排根单");
        assert_eq!(step_root.workflow_run_id.as_deref(), Some("run-2"));
        let inherited_child = create_assignment(
            db,
            &CreateBotAssignment {
                parent_assignment_id: Some(step_root.id.clone()),
                ..input("bot-y", Some("bot-x"))
            },
        )
        .await
        .expect("继承子单");
        assert_eq!(inherited_child.workflow_run_id.as_deref(), Some("run-2"));
        assert_eq!(inherited_child.step_key.as_deref(), Some("s2"));
        let by_run =
            list_by_workflow_run(db, "run-2", |id| id.to_string()).await.expect("按 run 列");
        assert_eq!(by_run.len(), 2);
    }

    #[tokio::test]
    async fn status_machine_happy_and_terminal() {
        let h = pool().await;
        let db = &h.conn;
        let a = create_assignment(db, &input("bot-b", None)).await.expect("建单");

        let running = transition(db, &a.id, status::RUNNING, TransitionPatch::default())
            .await
            .expect("queued→running");
        assert!(running.started_at.is_some(), "进入 running 记 started_at");

        let done = transition(
            db,
            &a.id,
            status::COMPLETED,
            TransitionPatch {
                result_text: Some("结论：震荡".to_string()),
                finished_at: Some(now_ts()),
                ..Default::default()
            },
        )
        .await
        .expect("running→completed");
        assert_eq!(done.status, status::COMPLETED);
        assert_eq!(done.result_text.as_deref(), Some("结论：震荡"));

        // 终态不可再迁。
        let bad = transition(db, &a.id, status::FAILED, TransitionPatch::default()).await;
        assert!(matches!(bad, Err(AxAgentError::Validation(_))), "终态再迁应拒绝: {bad:?}");

        // 非法跳迁：queued 直接 completed。
        let b = create_assignment(db, &input("bot-b", None)).await.expect("建单");
        let skip = transition(db, &b.id, status::COMPLETED, TransitionPatch::default()).await;
        assert!(matches!(skip, Err(AxAgentError::Validation(_))), "queued 跳迁应拒绝: {skip:?}");
    }

    #[tokio::test]
    async fn cancel_rules() {
        let h = pool().await;
        let db = &h.conn;
        let queued = create_assignment(db, &input("bot-b", None)).await.expect("建单");
        let c = cancel_assignment(db, &queued.id).await.expect("取消 queued");
        assert_eq!(c.status, status::CANCELLED);

        // 终态取消报错。
        let err = cancel_assignment(db, &queued.id).await;
        assert!(matches!(err, Err(AxAgentError::Validation(_))), "终态取消应拒绝: {err:?}");

        // running 也可取消。
        let running = create_assignment(db, &input("bot-b", None)).await.expect("建单");
        transition(db, &running.id, status::RUNNING, TransitionPatch::default())
            .await
            .expect("转 running");
        let c2 = cancel_assignment(db, &running.id).await.expect("取消 running");
        assert_eq!(c2.status, status::CANCELLED);
    }

    #[tokio::test]
    async fn in_flight_gate_counting() {
        let h = pool().await;
        let db = &h.conn;
        let r = "bot-a".to_string();
        for _ in 0..2 {
            create_assignment(db, &input("bot-b", Some(&r))).await.expect("建单");
        }
        // consult 不进 dispatch 门。
        record_consult_assignment(db, "bot", &r, "bot-b", "问问看", Ok("答案".into()))
            .await
            .expect("consult 审计行");
        // 其他 requester 的单不计入单 requester 门。
        create_assignment(db, &input("bot-b", Some("bot-z"))).await.expect("建单");

        assert_eq!(
            count_in_flight_dispatch(db, Some(&r)).await.expect("计数"),
            2,
            "单 requester 门只数自己的在途 dispatch"
        );
        assert_eq!(
            count_in_flight_dispatch_global(db).await.expect("全局计数"),
            3,
            "全局门数所有在途 dispatch"
        );

        // 终态后退出计数。
        let list = list_assignments(
            db,
            &AssignmentFilter {
                requester_bot_id: Some(r.clone()),
                status: Some(status::QUEUED.to_string()),
                ..Default::default()
            },
            |id| format!("name:{id}"),
        )
        .await
        .expect("列表");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].assignee_name, "name:bot-b", "assignee_name 回填生效");
        // 先转 running 再转终态（状态机不允许 queued 直迁终态）。
        transition(db, &list[0].id, status::RUNNING, TransitionPatch::default())
            .await
            .expect("转 running");
        transition(
            db,
            &list[0].id,
            status::FAILED,
            TransitionPatch {
                error_code: Some("BOT_DELEGATE_TIMEOUT".into()),
                finished_at: Some(now_ts()),
                ..Default::default()
            },
        )
        .await
        .expect("失败");
        assert_eq!(count_in_flight_dispatch(db, Some(&r)).await.expect("计数"), 1);
    }

    #[tokio::test]
    async fn cancel_in_flight_of_bot_cascade() {
        let h = pool().await;
        let db = &h.conn;
        let in_flight = create_assignment(db, &input("bot-b", None)).await.expect("建单");
        let finished = create_assignment(db, &input("bot-b", None)).await.expect("建单");
        transition(db, &finished.id, status::RUNNING, TransitionPatch::default())
            .await
            .expect("转 running");
        transition(
            db,
            &finished.id,
            status::COMPLETED,
            TransitionPatch {
                result_text: Some("完".into()),
                finished_at: Some(now_ts()),
                ..Default::default()
            },
        )
        .await
        .expect("完成");

        let n = cancel_in_flight_of_bot(db, "bot-b").await.expect("级联取消");
        assert_eq!(n, 1, "仅 queued 的那条在途单被取消（completed 的不动）");
        let after = get_assignment(db, &in_flight.id, |_| "x".into()).await.expect("读回");
        assert_eq!(after.status, status::CANCELLED);
        // 终态单保留审计，不动。
        let kept = get_assignment(db, &finished.id, |_| "x".into()).await.expect("读回");
        assert_eq!(kept.status, status::COMPLETED);
    }
}
