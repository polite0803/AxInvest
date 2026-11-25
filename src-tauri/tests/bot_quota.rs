// SPDX-License-Identifier: AGPL-3.0-only

//! P4 A2 配额解析集成测试（PLAN-P4 §6.3 测试①–⑤、⑦）：
//! 默认等于 P3、min 生效、超额拒绝带字段、部门并发 run 上限、两部门互不影响、
//! 删部门清配额、每层在途按配额生效。
//!
//! 门禁侧补充（A5 缺口，2026-11-25）：
//! - `step_timeout_violation_names_the_offending_step`：单步超时必须点名越界步骤；
//! - `cron_gate_rejects_before_run_creation_across_workflows`：测试⑥ 可观测部分——
//!   部门容量跨编排共享，cron 预检（`check_run_allowed`）在 `create_run` 之前拒绝且
//!   不留 run 行；「history 记 failed」属调度器闭包内既有错误路径，本文件无法驱动。

use axagent_dao::DatabaseConnection;
use axagent_dao::repo::bot as bot_repo;
use axagent_dao::repo::bot_assignment as asmt_repo;
use axagent_dao::repo::bot_department as dept_repo;
use axagent_dao::repo::bot_department_quota as quota_repo;
use axagent_dao::repo::bot_workflow as workflow_repo;
use axagent_dao::repo::bot_workflow_run as run_repo;
use axagent_harness::types::{
    BotDepartmentQuotaInput, BotWorkflow, BotWorkflowStep, CreateBot, CreateBotAssignment,
    CreateBotWorkflow, CreateBotWorkflowRun,
};
use axagent_lib::bot_quota::{
    AssignmentContext, QuotaLimits, check_definition_allowed, check_run_allowed,
    resolve_workflow_limits,
};

async fn pool() -> axagent_dao::db::DbHandle {
    axagent_dao::db::create_test_pool().await.expect("测试库")
}

async fn make_bot(db: &DatabaseConnection, name: &str) -> String {
    bot_repo::create_bot(db, &CreateBot { name: name.into(), ..Default::default() })
        .await
        .expect("建 Bot")
        .id
}

async fn make_department(db: &DatabaseConnection, name: &str) -> String {
    dept_repo::create_department(db, name, "", None).await.expect("建部门").id
}

fn step(key: &str, bot: &str, timeout_secs: i32) -> BotWorkflowStep {
    BotWorkflowStep {
        step_key: key.into(),
        bot_id: bot.into(),
        prompt_template: format!("处理 {key}"),
        depends_on: Vec::new(),
        timeout_secs,
        on_failure: "abort".into(),
    }
}

async fn make_workflow(
    db: &DatabaseConnection,
    dept: &str,
    steps: Vec<BotWorkflowStep>,
    max_concurrency: i32,
    total_timeout_secs: i32,
) -> BotWorkflow {
    workflow_repo::create_workflow(
        db,
        &CreateBotWorkflow {
            name: "部门编排".into(),
            owner_scope: "department".into(),
            department_id: Some(Some(dept.to_string())),
            steps,
            max_concurrency,
            total_timeout_secs,
            ..Default::default()
        },
    )
    .await
    .expect("建编排")
}

async fn set_quota(
    db: &DatabaseConnection,
    dept: &str,
    max_concurrency: i32,
    total_timeout_secs: i32,
    max_steps: i32,
    per_depth: i32,
    step_timeout_secs: i32,
    max_concurrent_runs: i32,
) {
    quota_repo::upsert(
        db,
        &BotDepartmentQuotaInput {
            department_id: dept.to_string(),
            max_concurrency,
            total_timeout_secs,
            max_steps,
            max_in_flight_per_depth: per_depth,
            step_timeout_secs,
            max_concurrent_runs,
        },
    )
    .await
    .expect("写配额");
}

/// ① 无配额行 → 生效值逐项等于 P3 常量（并发/总超时取定义默认 2/900）。
#[tokio::test]
async fn no_quota_row_equals_p3_defaults() {
    let h = pool().await;
    let db = &h.conn;
    let dept = make_department(db, "无配额部门").await;
    let bot = make_bot(db, "研究员").await;
    let wf = make_workflow(db, &dept, vec![step("a", &bot, 30)], 2, 900).await;

    let limits = resolve_workflow_limits(db, &wf).await.expect("解析");
    assert_eq!(limits, QuotaLimits::p3_defaults(), "无行必须逐项等于 P3");
}

/// 实例级编排零 DB 读路径：resolve 直接返回定义侧基线，不受任何部门配额影响。
#[tokio::test]
async fn instance_workflow_ignores_department_quota() {
    let h = pool().await;
    let db = &h.conn;
    let dept = make_department(db, "被忽略的部门").await;
    set_quota(db, &dept, 1, 300, 1, 1, 60, 1).await;
    let bot = make_bot(db, "研究员").await;
    let wf = workflow_repo::create_workflow(
        db,
        &CreateBotWorkflow {
            name: "实例编排".into(),
            owner_scope: "instance".into(),
            steps: vec![step("a", &bot, 30), step("b", &bot, 30)],
            max_concurrency: 4,
            total_timeout_secs: 1800,
            ..Default::default()
        },
    )
    .await
    .expect("建编排");

    let limits = resolve_workflow_limits(db, &wf).await.expect("解析");
    assert_eq!(limits.max_concurrency, 4, "实例级保留定义值");
    assert_eq!(limits.total_timeout_secs, 1800);
    assert_eq!(limits.max_steps, 12);
    assert_eq!(limits.step_timeout_secs, 300);
}

/// ② 部门配额更小 → 逐项取 min；⑤ 两部门互不影响。
#[tokio::test]
async fn quota_tightens_per_field_and_departments_are_independent() {
    let h = pool().await;
    let db = &h.conn;
    let dept_a = make_department(db, "部门A").await;
    let dept_b = make_department(db, "部门B").await;
    let bot = make_bot(db, "研究员").await;
    set_quota(db, &dept_a, 1, 300, 3, 1, 60, 1).await;

    let wf_a = make_workflow(db, &dept_a, vec![step("a", &bot, 30)], 2, 900).await;
    let wf_b = make_workflow(db, &dept_b, vec![step("a", &bot, 30)], 2, 900).await;

    let limits_a = resolve_workflow_limits(db, &wf_a).await.expect("解析A");
    assert_eq!(limits_a.max_concurrency, 1);
    assert_eq!(limits_a.total_timeout_secs, 300);
    assert_eq!(limits_a.max_steps, 3);
    assert_eq!(limits_a.max_in_flight_per_depth, 1);
    assert_eq!(limits_a.step_timeout_secs, 60);
    assert_eq!(limits_a.max_concurrent_runs, 1);

    let limits_b = resolve_workflow_limits(db, &wf_b).await.expect("解析B");
    assert_eq!(limits_b, QuotaLimits::p3_defaults(), "B 不受 A 的配额影响");
}

/// ③ 定义值超配额 → BOT_QUOTA_EXCEEDED 且 detail/param 带字段。
#[tokio::test]
async fn definition_over_quota_rejected_with_field() {
    let h = pool().await;
    let db = &h.conn;
    let dept = make_department(db, "部门").await;
    set_quota(db, &dept, 1, 300, 2, 1, 60, 1).await;
    let bot = make_bot(db, "研究员").await;

    let err = check_definition_allowed(db, &dept, 2, 900, &[step("a", &bot, 30)])
        .await
        .expect_err("并发超限");
    assert!(err.contains("BOT_QUOTA_EXCEEDED"), "{err}");
    assert!(err.contains("maxConcurrency"), "detail 应带字段名: {err}");

    let steps = vec![step("a", &bot, 30), step("b", &bot, 30), step("c", &bot, 30)];
    let err = check_definition_allowed(db, &dept, 1, 300, &steps).await.expect_err("步数超限");
    assert!(err.contains("maxSteps"), "{err}");

    let err = check_definition_allowed(db, &dept, 1, 300, &[step("a", &bot, 600)])
        .await
        .expect_err("单步超时超限");
    assert!(err.contains("stepTimeoutSecs"), "{err}");

    check_definition_allowed(db, &dept, 1, 300, &[step("a", &bot, 60)])
        .await
        .expect("未超限应通过");
}

async fn queued_run(
    db: &DatabaseConnection,
    wf: &BotWorkflow,
) -> axagent_harness::types::BotWorkflowRun {
    run_repo::create_run(
        db,
        &CreateBotWorkflowRun {
            workflow_id: wf.id.clone(),
            workflow_name: wf.name.clone(),
            origin: "routine".into(),
            requester_bot_id: None,
            requester_conversation_id: "conv-test".into(),
            total_steps: wf.steps.len() as i32,
            inputs: Default::default(),
        },
    )
    .await
    .expect("建 run")
}

/// ④ 部门并发 run 超限 → 拒绝；自身 run 复核时排除；别部门不受影响。
#[tokio::test]
async fn run_capacity_rejected_per_department() {
    let h = pool().await;
    let db = &h.conn;
    let dept = make_department(db, "部门A").await;
    let other = make_department(db, "部门B").await;
    let bot = make_bot(db, "研究员").await;
    set_quota(db, &dept, 2, 900, 12, 2, 300, 1).await;
    let wf = make_workflow(db, &dept, vec![step("a", &bot, 30)], 2, 900).await;
    let wf_other = make_workflow(db, &other, vec![step("a", &bot, 30)], 2, 900).await;

    let first = queued_run(db, &wf).await;
    let err = check_run_allowed(db, &wf, None).await.expect_err("同时跑第 2 个应拒绝");
    assert!(err.contains("BOT_QUOTA_EXCEEDED"), "{err}");
    assert!(err.contains("maxConcurrentRuns"), "{err}");

    // start_run 复核排除自身：同一个 run 不算「另一个在跑」。
    let limits = check_run_allowed(db, &wf, Some(&first.id)).await.expect("排除自身应通过");
    assert_eq!(limits.max_concurrent_runs, 1);

    // 别部门无配额行 → P3 默认，不受 A 的在途 run 影响。
    let other_limits = check_run_allowed(db, &wf_other, None).await.expect("B 应通过");
    assert_eq!(other_limits, QuotaLimits::p3_defaults());

    // 首 run 结束后容量释放。
    run_repo::patch_run(
        db,
        &first.id,
        axagent_harness::types::BotWorkflowRunPatch {
            status: Some(run_repo::status::COMPLETED.into()),
            ..Default::default()
        },
    )
    .await
    .expect("置终态");
    check_run_allowed(db, &wf, None).await.expect("终态后应通过");
}

/// ⑧ 删部门同事务清配额行 + 编排转实例（ADR-P4-1 / ADR-P3-3）。
#[tokio::test]
async fn deleting_department_clears_quota_and_transfers_workflow() {
    let h = pool().await;
    let db = &h.conn;
    let dept = make_department(db, "将被删除").await;
    let bot = make_bot(db, "研究员").await;
    dept_repo::add_bot_to_department(db, &dept, &bot).await.expect("加成员");
    set_quota(db, &dept, 1, 300, 3, 1, 60, 1).await;
    let wf = make_workflow(db, &dept, vec![step("a", &bot, 30)], 2, 900).await;

    let members = dept_repo::delete_department_cascade(db, &dept).await.expect("删部门");
    assert_eq!(members, 1);
    assert_eq!(
        quota_repo::get_by_department(db, &dept).await.expect("读配额"),
        None,
        "配额行必须同事务清掉"
    );
    assert!(dept_repo::get_department(db, &dept).await.is_err(), "部门已删");
    assert_eq!(dept_repo::department_of_bot(db, &bot).await.expect("归属"), None);

    let transferred = workflow_repo::get_workflow(db, &wf.id).await.expect("读编排");
    assert_eq!(transferred.owner_scope, "instance");
    assert_eq!(transferred.department_id, None);
}

/// ⑦ 每层在途按部门配额生效：配额 1 时同层第二个子单被拒。
#[tokio::test]
async fn per_depth_cap_follows_run_quota() {
    let h = pool().await;
    let db = &h.conn;
    let requester = make_bot(db, "发起方").await;
    let assignee = make_bot(db, "执行方").await;
    let root = asmt_repo::create_assignment(
        db,
        &CreateBotAssignment {
            assignee_bot_id: requester.clone(),
            task_text: "根任务".into(),
            mode: "dispatch".into(),
            requester_bot_id: None,
            origin: Some("person".into()),
            ..Default::default()
        },
    )
    .await
    .expect("建根单");

    AssignmentContext::install(
        "run-depth",
        QuotaLimits { max_in_flight_per_depth: 1, ..QuotaLimits::p3_defaults() },
    );
    let child = |n: usize| CreateBotAssignment {
        assignee_bot_id: assignee.clone(),
        task_text: format!("子任务 {n}"),
        mode: "dispatch".into(),
        requester_bot_id: Some(requester.clone()),
        parent_assignment_id: Some(root.id.clone()),
        workflow_run_id: Some("run-depth".into()),
        origin: Some("workflow".into()),
        ..Default::default()
    };
    axagent_lib::create_step_assignment(db, &child(1)).await.expect("第一个子单");
    let err = axagent_lib::create_step_assignment(db, &child(2))
        .await
        .expect_err("同层第二个应被配额 1 拒绝");
    assert!(err.contains("BOT_DELEGATE_BUSY"), "{err}");
    AssignmentContext::remove("run-depth");
}

/// ③ 补充：单步超时的拒绝必须点名具体步骤（ADR-P4 D3 的 detail 可定位要求），
/// 否则运维只能看到「某一步超了」而无法知道是哪一步。
#[tokio::test]
async fn step_timeout_violation_names_the_offending_step() {
    let h = pool().await;
    let db = &h.conn;
    let dept = make_department(db, "点名部门").await;
    set_quota(db, &dept, 2, 900, 12, 2, 60, 2).await;
    let bot = make_bot(db, "研究员").await;

    let steps = vec![step("collect", &bot, 30), step("analyze", &bot, 600)];
    let err = check_definition_allowed(db, &dept, 2, 900, &steps)
        .await
        .expect_err("analyze 单步 600s 超过配额 60s");
    assert!(err.contains("stepTimeoutSecs"), "{err}");
    assert!(err.contains("步骤 analyze"), "detail 应点名越界步骤: {err}");
    assert!(!err.contains("步骤 collect"), "未越界的步骤不应出现在 detail: {err}");
}

/// ⑥ cron/定时触发前的同一道门（PLAN-P4 §6.3 A2 测试⑥）。
///
/// `init/services.rs` 的 `task_type=="bot_workflow"` 分支在
/// `create_bot_workflow_persistent_conversation + create_run` 之前调用的就是
/// `check_run_allowed(db, &workflow, None)`。本测试锁定其中可观测的部分：
/// **容量按部门跨编排共享**——兄弟编排占满 `max_concurrent_runs` 时，cron 目标
/// 编排的预检必须失败，且不得留下任何 run 行。
///
/// 边界（如实记录，不伪造覆盖）：闭包内的「history 记 failed」是调度器既有错误
/// 路径，无法在集成测试内驱动；该段只在端到端启动调度器时可验证。
#[tokio::test]
async fn cron_gate_rejects_before_run_creation_across_workflows() {
    let h = pool().await;
    let db = &h.conn;
    let dept = make_department(db, "定时部门").await;
    let bot = make_bot(db, "研究员").await;
    set_quota(db, &dept, 2, 900, 12, 2, 300, 1).await;

    // 兄弟编排先用掉部门唯一的并发额度（queued_run 的 origin = routine，与 cron 同源）。
    let sibling = make_workflow(db, &dept, vec![step("a", &bot, 30)], 2, 900).await;
    let occupying = queued_run(db, &sibling).await;

    // cron 目标的预检：定义合规，但部门容量已满 → 在 create_run 之前就被拒。
    let cron_target = make_workflow(db, &dept, vec![step("a", &bot, 30)], 2, 900).await;
    let err = check_run_allowed(db, &cron_target, None)
        .await
        .expect_err("部门容量 1 已被兄弟编排占用");
    assert!(err.contains("BOT_QUOTA_EXCEEDED"), "{err}");
    assert!(err.contains("maxConcurrentRuns"), "{err}");

    // 「不创建 run」：cron 目标编排名下 0 条 run。
    let runs = run_repo::list_runs(db, Some(&cron_target.id), None).await.expect("列 run");
    assert!(runs.is_empty(), "预检失败的触发不得写入 run 行: {runs:?}");

    // 占用方仍在跑（预检失败没有误伤兄弟编排）。
    let still = run_repo::get_run(db, &occupying.id).await.expect("读占用 run");
    assert!(!["completed", "failed", "cancelled"].contains(&still.status.as_str()));
}
