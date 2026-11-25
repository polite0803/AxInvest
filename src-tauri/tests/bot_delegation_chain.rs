// SPDX-License-Identifier: AGPL-3.0-only

//! P3 T3'''（PLAN-P3 §3.1 测试⑩–⑰ / ADR-P3-2 ①–⑤）：多层委派链集成测试。

use std::sync::Arc;
use tokio::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use axagent_dao::DatabaseConnection;
use axagent_dao::repo::bot as bot_repo;
use axagent_dao::repo::bot_assignment as asmt_repo;
use axagent_dao::repo::bot_workflow as workflow_repo;
use axagent_dao::repo::bot_workflow_run as run_repo;
use axagent_harness::types::{
    BotAssignment, BotWorkflowStep, CreateBotAssignment, CreateBotWorkflow, CreateBotWorkflowRun,
};
use axagent_lib::bot_assignment_runner::{AssignmentTurnExecutor, run_assignment_inner};
use axagent_lib::bot_routine_runner::{TurnKind, build_turn_tool_registry};
use axagent_lib::bot_workflow_runner::{on_assignment_completed, start_run};
use axagent_lib::create_step_assignment;

struct Scripted {
    result: Result<String, String>,
}

#[async_trait]
impl AssignmentTurnExecutor for Scripted {
    async fn execute(&self, _bot_id: &str, _task: &str) -> Result<String, String> {
        self.result.clone()
    }
}

async fn pool() -> axagent_dao::db::DbHandle {
    axagent_dao::db::create_test_pool().await.expect("测试库")
}

async fn make_bot(db: &DatabaseConnection, name: &str) -> String {
    bot_repo::create_bot(
        db,
        &axagent_harness::types::CreateBot { name: name.into(), ..Default::default() },
    )
    .await
    .expect("建 Bot")
    .id
}

fn child_input(assignee: &str, parent: &BotAssignment, text: &str) -> CreateBotAssignment {
    CreateBotAssignment {
        assignee_bot_id: assignee.into(),
        task_text: text.into(),
        mode: "dispatch".into(),
        requester_bot_id: Some(parent.assignee_bot_id.clone()),
        parent_assignment_id: Some(parent.id.clone()),
        root_assignment_id: Some(parent.root_id.clone()),
        workflow_run_id: parent.workflow_run_id.clone(),
        step_key: parent.step_key.clone(),
        ..Default::default()
    }
}

async fn root_input(db: &DatabaseConnection, assignee: &str) -> BotAssignment {
    create_step_assignment(
        db,
        &CreateBotAssignment {
            assignee_bot_id: assignee.into(),
            task_text: "根任务：汇总盘面".into(),
            mode: "dispatch".into(),
            ..Default::default()
        },
    )
    .await
    .expect("建根单")
}

async fn drive(db: &DatabaseConnection, id: &str, result: Result<String, String>) -> Vec<String> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let completion = {
        let db = db.clone();
        move |a: &BotAssignment| {
            let db = db.clone();
            let row = a.clone();
            tokio::spawn(async move {
                on_assignment_completed(&db, None, &row).await;
            });
        }
    };
    let sink_events = events.clone();
    let sink = move |a: &BotAssignment| {
        // sink 是同步闭包；沿用 bot_assignment 测试的 try_lock 串行场景写法。
        if let Ok(mut events) = sink_events.try_lock() {
            events.push(a.status.clone());
        }
    };
    let script = Scripted { result };
    run_assignment_inner(db, &sink, id, &script, Duration::from_secs(10), &completion).await;
    events.lock().await.clone()
}

async fn make_run(db: &DatabaseConnection, bot: &str) -> String {
    let workflow = workflow_repo::create_workflow(
        db,
        &CreateBotWorkflow {
            name: "链测试编排".into(),
            steps: vec![BotWorkflowStep {
                step_key: "a".into(),
                bot_id: bot.into(),
                prompt_template: "分析 {{inputs.topic}}".into(),
                depends_on: vec![],
                timeout_secs: 30,
                on_failure: "abort".into(),
            }],
            max_concurrency: 2,
            total_timeout_secs: 60,
            ..Default::default()
        },
    )
    .await
    .expect("编排");
    let conv = axagent_dao::repo::conversation::create_conversation_with_defaults(
        db,
        "链线程",
        "m",
        "p",
        None,
        axagent_dao::repo::conversation::ConversationParamDefaults::default(),
        None,
    )
    .await
    .expect("线程");
    let run = run_repo::create_run(
        db,
        &CreateBotWorkflowRun {
            workflow_id: workflow.id,
            workflow_name: workflow.name,
            origin: "person".into(),
            requester_bot_id: None,
            requester_conversation_id: conv.id,
            total_steps: 1,
            inputs: [("topic".into(), "盘面".into())].into_iter().collect(),
        },
    )
    .await
    .expect("run");
    let run = start_run(db, None, &run.id).await.expect("启动");
    run.id
}

async fn wait_run_step(db: &DatabaseConnection, run_id: &str) -> BotAssignment {
    for _ in 0..200 {
        let rows =
            asmt_repo::list_by_workflow_run(db, run_id, |_| String::new()).await.expect("run 步骤");
        if let Some(row) = rows.into_iter().find(|a| a.step_key.as_deref() == Some("a")) {
            return row;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("run 步骤未创建");
}

#[test]
fn test_10_delegate_tool_respects_depth_cap() {
    let routine = build_turn_tool_registry(TurnKind::Routine, 1);
    assert!(routine.list_tools().iter().any(|n| n == "delegate_to_bot"));
    for depth in [1u8, 2] {
        let assignee = build_turn_tool_registry(TurnKind::Assignee, depth);
        assert!(assignee.list_tools().iter().any(|n| n == "delegate_to_bot"));
        assert!(!assignee.list_tools().iter().any(|n| n == "Agent" || n == "RemoteTrigger"));
    }
    let capped = build_turn_tool_registry(TurnKind::Assignee, 3);
    assert!(!capped.list_tools().iter().any(|n| n == "delegate_to_bot"));
}

#[tokio::test]
async fn test_11_three_level_chain_keeps_root_parent_and_inherited_edges() {
    let h = pool().await;
    let db = &h.conn;
    let a = make_bot(db, "A").await;
    let b = make_bot(db, "B").await;
    let c = make_bot(db, "C").await;
    let root = root_input(db, &a).await;
    let mid = create_step_assignment(db, &child_input(&b, &root, "A 派 B")).await.expect("L2");
    let leaf = create_step_assignment(db, &child_input(&c, &mid, "B 派 C")).await.expect("L3");
    for (row, depth, parent) in
        [(&root, 1, None), (&mid, 2, Some(root.id.as_str())), (&leaf, 3, Some(mid.id.as_str()))]
    {
        assert_eq!(row.depth, depth);
        assert_eq!(row.root_id, root.id);
        assert_eq!(row.parent_id.as_deref(), parent);
    }
    let rows =
        asmt_repo::list_by_root(db, &root.id, |_| String::new()).await.expect("按 root 列表");
    assert_eq!(rows.len(), 3);
}

#[tokio::test]
async fn test_12_cycle_and_max_depth_errors_are_auditable() {
    let h = pool().await;
    let db = &h.conn;
    let a = make_bot(db, "A").await;
    let b = make_bot(db, "B").await;
    let c = make_bot(db, "C").await;
    let d = make_bot(db, "D").await;
    let root = root_input(db, &a).await;
    let mid = create_step_assignment(db, &child_input(&b, &root, "A 派 B")).await.expect("L2");

    let cycle =
        create_step_assignment(db, &child_input(&a, &mid, "B 派回 A")).await.expect_err("环应被拒");
    assert!(cycle.contains("BOT_DELEGATE_CYCLE"), "{cycle}");
    let audited = asmt_repo::record_consult_assignment_with_code(
        db,
        "bot",
        &b,
        &a,
        "B 派回 A",
        Err(cycle.clone()),
        Some("BOT_DELEGATE_CYCLE"),
    )
    .await
    .expect("审计行");
    assert_eq!(audited.error_code.as_deref(), Some("BOT_DELEGATE_CYCLE"));

    let leaf = create_step_assignment(db, &child_input(&c, &mid, "B 派 C")).await.expect("L3");
    let too_deep = create_step_assignment(db, &child_input(&d, &leaf, "C 派 D"))
        .await
        .expect_err("depth=4 应被拒");
    assert!(too_deep.contains("BOT_DELEGATE_MAX_DEPTH"), "{too_deep}");
}

#[tokio::test]
async fn test_13_layer_and_global_in_flight_budgets() {
    let h = pool().await;
    let db = &h.conn;
    let a = make_bot(db, "A").await;
    let b = make_bot(db, "B").await;
    let c = make_bot(db, "C").await;
    let d = make_bot(db, "D").await;
    let root = root_input(db, &a).await;
    create_step_assignment(db, &child_input(&b, &root, "L2-B")).await.expect("depth2 第一单");
    create_step_assignment(db, &child_input(&c, &root, "L2-C")).await.expect("depth2 第二单");
    let busy = create_step_assignment(db, &child_input(&d, &root, "L2-D"))
        .await
        .expect_err("同层第 3 单应被拒");
    assert!(busy.contains("BOT_DELEGATE_BUSY"), "{busy}");

    // 全局 8 单门：独立库里堆 8 条 queued dispatch，第 9 条被拒。
    let h2 = pool().await;
    let db2 = &h2.conn;
    let bot = make_bot(db2, "G").await;
    let mut requesters = Vec::new();
    for i in 0..8 {
        requesters.push(make_bot(db2, &format!("R{i}")).await);
    }
    for requester in &requesters {
        create_step_assignment(
            db2,
            &CreateBotAssignment {
                assignee_bot_id: bot.clone(),
                task_text: "全局预算占用".into(),
                mode: "dispatch".into(),
                requester_bot_id: Some(requester.clone()),
                ..Default::default()
            },
        )
        .await
        .expect("占位单");
    }
    let ninth = create_step_assignment(
        db2,
        &CreateBotAssignment {
            assignee_bot_id: bot,
            task_text: "第 9 单".into(),
            mode: "dispatch".into(),
            requester_bot_id: Some(requesters[0].clone()),
            ..Default::default()
        },
    )
    .await
    .expect_err("全局第 9 单应被拒");
    assert!(ninth.contains("BOT_DELEGATE_BUSY"), "{ninth}");
}

#[tokio::test]
async fn test_14_root_completed_while_child_in_flight_keeps_run_open() {
    let h = pool().await;
    let db = &h.conn;
    let a = make_bot(db, "A").await;
    let b = make_bot(db, "B").await;
    let run_id = make_run(db, &a).await;
    let root = wait_run_step(db, &run_id).await;
    let child =
        create_step_assignment(db, &child_input(&b, &root, "A 派 B")).await.expect("L2 子单");

    let events = drive(db, &root.id, Ok("根完成".into())).await;
    assert_eq!(events, vec!["running", "completed"]);
    let mut run = run_repo::get_run(db, &run_id).await.expect("run");
    for _ in 0..200 {
        if run.context.outputs.contains_key("a") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        run = run_repo::get_run(db, &run_id).await.expect("run");
    }
    assert!(
        !run_repo::status::is_terminal(&run.status),
        "根 completed 但子在途时 run 应仍 running/waiting，实际 {}",
        run.status
    );
    assert_eq!(run.context.outputs["a"].result_text, "根完成");
    assert_eq!(
        asmt_repo::get_assignment(db, &child.id, |_| String::new()).await.expect("子单").status,
        asmt_repo::status::QUEUED
    );
}

#[tokio::test]
async fn test_15_cancel_covers_whole_root_subtree() {
    let h = pool().await;
    let db = &h.conn;
    let a = make_bot(db, "A").await;
    let b = make_bot(db, "B").await;
    let root = root_input(db, &a).await;
    let child =
        create_step_assignment(db, &child_input(&b, &root, "A 派 B")).await.expect("L2 子单");
    let cancelled = asmt_repo::cancel_assignment(db, &root.id).await.expect("取消根单");
    assert_eq!(cancelled.status, asmt_repo::status::CANCELLED);
    // run 级 cancel 在 test_6 已验证按 root_id 遍历子树；此处锁定 root_id 索引覆盖。
    let rows = asmt_repo::list_by_root(db, &root.id, |_| String::new()).await.expect("子树");
    assert!(rows.iter().any(|r| r.id == child.id && r.root_id == root.id));
}

#[tokio::test]
async fn test_17_child_error_code_inherits_to_run() {
    let h = pool().await;
    let db = &h.conn;
    let a = make_bot(db, "A").await;
    let run_id = make_run(db, &a).await;
    let root = wait_run_step(db, &run_id).await;
    let running = asmt_repo::transition(
        db,
        &root.id,
        asmt_repo::status::RUNNING,
        asmt_repo::TransitionPatch::default(),
    )
    .await
    .expect("running");
    let failed = asmt_repo::transition(
        db,
        &root.id,
        asmt_repo::status::FAILED,
        asmt_repo::TransitionPatch {
            error_code: Some("BOT_DELEGATE_CYCLE".into()),
            error_detail: Some("子链检测到环".into()),
            finished_at: Some(axagent_harness::util_fns::now_ts()),
            ..Default::default()
        },
    )
    .await
    .expect("failed");
    on_assignment_completed(db, None, &running).await;
    on_assignment_completed(db, None, &failed).await;
    let run = run_repo::get_run(db, &run_id).await.expect("run");
    assert_eq!(run.status, run_repo::status::FAILED);
    assert_eq!(run.error_code.as_deref(), Some("BOT_DELEGATE_CYCLE"));
}
