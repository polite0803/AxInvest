// SPDX-License-Identifier: AGPL-3.0-only

//! P3 T3'''（PLAN-P3 §3 测试①–⑦）：编排 runner 集成测试。
//! 无模型环境用 ScriptedExecutor 驱动 `run_assignment_inner`，完成钩子走真实
//! `on_assignment_completed`；锁 DB 状态 / context / 回执 parent / run 终态与错误码。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axagent_dao::DatabaseConnection;
use axagent_dao::repo::bot as bot_repo;
use axagent_dao::repo::bot_assignment as asmt_repo;
use axagent_dao::repo::bot_department as dept_repo;
use axagent_dao::repo::bot_workflow as workflow_repo;
use axagent_dao::repo::bot_workflow_run as run_repo;
use axagent_harness::types::{
    BotAssignment, BotWorkflowStep, CreateBotWorkflow, CreateBotWorkflowRun,
};
use axagent_lib::bot_assignment_runner::{AssignmentTurnExecutor, run_assignment_inner};
use axagent_lib::bot_workflow_runner::{cancel_run, on_assignment_completed, start_run};

type Script = Arc<Mutex<HashMap<String, Result<String, String>>>>;

struct ScriptedExecutor {
    script: Script,
    prompts: Arc<Mutex<Vec<String>>>,
    delay: Duration,
}

#[async_trait]
impl AssignmentTurnExecutor for ScriptedExecutor {
    async fn execute(&self, bot_id: &str, task_text: &str) -> Result<String, String> {
        self.prompts.lock().expect("prompts").push(task_text.to_string());
        // 延迟只作用于显式脚本化的 Bot；未脚本化的独立分支瞬时完成。
        let scripted = self.script.lock().expect("script").get(bot_id).cloned();
        if scripted.is_some() && !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        scripted.unwrap_or_else(|| Ok(format!("{bot_id} 的结论")))
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

fn step(key: &str, bot: &str, deps: &[&str], on_failure: &str) -> BotWorkflowStep {
    BotWorkflowStep {
        step_key: key.into(),
        bot_id: bot.into(),
        prompt_template: format!(
            "分析 {{inputs.topic}} {}",
            deps.iter().map(|d| format!("{{{{steps.{d}.output}}}}")).collect::<Vec<_>>().join(" ")
        ),
        depends_on: deps.iter().map(|s| s.to_string()).collect(),
        timeout_secs: 30,
        on_failure: on_failure.into(),
    }
}

async fn make_run(
    db: &DatabaseConnection,
    steps: Vec<BotWorkflowStep>,
    dept: Option<&str>,
    timeout: i32,
    max_concurrency: i32,
) -> (String, String) {
    let total_steps = steps.len() as i32;
    let workflow = workflow_repo::create_workflow(
        db,
        &CreateBotWorkflow {
            name: "测试编排".into(),
            owner_scope: if dept.is_some() {
                "department"
            } else {
                "instance"
            }
            .into(),
            department_id: Some(dept.map(str::to_string)),
            steps,
            max_concurrency,
            total_timeout_secs: timeout,
            ..Default::default()
        },
    )
    .await
    .expect("建编排");
    let conv = axagent_dao::repo::conversation::create_conversation_with_defaults(
        db,
        "编排回执线程",
        "m",
        "p",
        None,
        axagent_dao::repo::conversation::ConversationParamDefaults::default(),
        None,
    )
    .await
    .expect("建线程");
    let run = run_repo::create_run(
        db,
        &CreateBotWorkflowRun {
            workflow_id: workflow.id.clone(),
            workflow_name: workflow.name,
            origin: "person".into(),
            requester_bot_id: None,
            requester_conversation_id: conv.id,
            total_steps,
            inputs: [("topic".to_string(), "盘面".to_string())].into_iter().collect(),
        },
    )
    .await
    .expect("建 run");
    (run.id, workflow.id)
}

async fn list_step_rows(db: &DatabaseConnection, run_id: &str) -> Vec<BotAssignment> {
    asmt_repo::list_by_workflow_run(db, run_id, |_| String::new()).await.expect("列出 run 步骤")
}

async fn wait_assignment_count(
    db: &DatabaseConnection,
    run_id: &str,
    count: usize,
) -> Vec<BotAssignment> {
    for _ in 0..200 {
        let rows = list_step_rows(db, run_id).await;
        if rows.len() >= count {
            return rows;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("编排器只创建了 {} 个 assignment，期望 {count}", list_step_rows(db, run_id).await.len());
}

fn complete_hook(db: &DatabaseConnection) -> impl Fn(&BotAssignment) + Send + Sync + 'static {
    let db = db.clone();
    move |a: &BotAssignment| {
        let db = db.clone();
        let row = a.clone();
        tokio::spawn(async move {
            on_assignment_completed(&db, None, &row).await;
        });
    }
}

async fn drive_queued(
    db: &DatabaseConnection,
    script: &Script,
    prompts: &Arc<Mutex<Vec<String>>>,
    delay: Duration,
    timeout: Duration,
) -> usize {
    let rows = asmt_repo::list_assignments(
        db,
        &asmt_repo::AssignmentFilter {
            status: Some(asmt_repo::status::QUEUED.to_string()),
            ..Default::default()
        },
        |_| String::new(),
    )
    .await
    .expect("queued assignments");
    let queued: Vec<BotAssignment> =
        rows.into_iter().filter(|a| a.workflow_run_id.is_some()).collect();
    for row in &queued {
        let executor = ScriptedExecutor { script: script.clone(), prompts: prompts.clone(), delay };
        let sink = |_: &BotAssignment| {};
        run_assignment_inner(db, &sink, &row.id, &executor, timeout, &complete_hook(db)).await;
    }
    queued.len()
}

async fn wait_run_terminal(
    db: &DatabaseConnection,
    run_id: &str,
) -> axagent_harness::types::BotWorkflowRun {
    for _ in 0..300 {
        let run = run_repo::get_run(db, run_id).await.expect("读 run");
        if run_repo::status::is_terminal(&run.status) {
            return run;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("run 未进入终态");
}

fn script_one(bot: &str, result: Result<String, String>) -> Script {
    let script: Script = Arc::new(Mutex::new(HashMap::new()));
    script.lock().expect("script").insert(bot.to_string(), result);
    script
}

#[tokio::test]
async fn test_1_three_step_chain_substitutes_outputs_and_anchor_receipts() {
    let h = pool().await;
    let db = &h.conn;
    let a = make_bot(db, "A").await;
    let b = make_bot(db, "B").await;
    let c = make_bot(db, "C").await;
    let (run_id, _) = make_run(
        db,
        vec![
            step("a", &a, &[], "abort"),
            step("b", &b, &["a"], "abort"),
            step("c", &c, &["b"], "abort"),
        ],
        None,
        60,
        2,
    )
    .await;
    start_run(db, None, &run_id).await.expect("启动 run");

    let script: Script = Arc::new(Mutex::new(HashMap::new()));
    {
        let mut guard = script.lock().expect("script");
        guard.insert(a.clone(), Ok("A 的结论".into()));
        guard.insert(b.clone(), Ok("B 的结论".into()));
        guard.insert(c.clone(), Ok("C 的结论".into()));
    }
    let prompts = Arc::new(Mutex::new(Vec::new()));
    for _ in 0..100 {
        drive_queued(db, &script, &prompts, Duration::ZERO, Duration::from_secs(10)).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        if run_repo::status::is_terminal(
            &run_repo::get_run(db, &run_id).await.expect("读 run").status,
        ) {
            break;
        }
    }
    let run = wait_run_terminal(db, &run_id).await;
    assert_eq!(run.status, run_repo::status::COMPLETED);
    assert_eq!(run.completed_steps, 3);
    assert_eq!(run.context.outputs["b"].result_text, "B 的结论");
    let texts = prompts.lock().expect("prompts").clone();
    assert!(texts.iter().any(|t| t.contains("A 的结论")), "B 应收到 A 的输出替换: {texts:?}");
    assert!(texts.iter().any(|t| t.contains("B 的结论")), "C 应收到 B 的输出替换: {texts:?}");

    let anchor = run.anchor_message_id.expect("anchor");
    let messages = axagent_dao::repo::message::list_messages(db, &run.requester_conversation_id)
        .await
        .expect("消息");
    let receipts: Vec<_> =
        messages.iter().filter(|m| m.parent_message_id.as_deref() == Some(&anchor)).collect();
    assert_eq!(receipts.len(), 3, "三条回执同线程且 parent=anchor");
    assert!(receipts.iter().all(|m| m.content.starts_with("[步骤 ")));
}

#[tokio::test]
async fn test_2_independent_steps_respect_max_concurrency() {
    let h = pool().await;
    let db = &h.conn;
    let a = make_bot(db, "A").await;
    let b = make_bot(db, "B").await;
    let c = make_bot(db, "C").await;
    let (run_id, _) = make_run(
        db,
        vec![step("a", &a, &[], "abort"), step("b", &b, &[], "abort"), step("c", &c, &[], "abort")],
        None,
        60,
        2,
    )
    .await;
    start_run(db, None, &run_id).await.expect("启动");
    let _rows = wait_assignment_count(db, &run_id, 2).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(list_step_rows(db, &run_id).await.len(), 2, "maxConcurrency=2 时只放两单");

    let script = script_one(&a, Ok("done".into()));
    let prompts = Arc::new(Mutex::new(Vec::new()));
    drive_queued(db, &script, &prompts, Duration::ZERO, Duration::from_secs(10)).await;
    let rows = wait_assignment_count(db, &run_id, 3).await;
    assert_eq!(rows.len(), 3, "有空位后应补第三单");
    assert!(rows.iter().any(|r| r.step_key.as_deref() == Some("c")));
    for _ in 0..100 {
        drive_queued(db, &script, &prompts, Duration::ZERO, Duration::from_secs(10)).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        if run_repo::status::is_terminal(
            &run_repo::get_run(db, &run_id).await.expect("读 run").status,
        ) {
            break;
        }
    }
    let run = wait_run_terminal(db, &run_id).await;
    assert_eq!(run.status, run_repo::status::COMPLETED);
}

#[tokio::test]
async fn test_3_cycle_definition_and_runtime_bot_checks() {
    let h = pool().await;
    let db = &h.conn;
    let a = make_bot(db, "A").await;
    let (_, wf_id) = make_run(db, vec![step("a", &a, &[], "abort")], None, 60, 2).await;
    let cyclic = vec![step("a", &a, &["b"], "abort"), step("b", &a, &["a"], "abort")];
    assert_eq!(
        axagent_lib::bot_workflow_steps::validate_dag(&cyclic).unwrap_err().code,
        "BOT_WORKFLOW_CYCLE"
    );

    bot_repo::update_bot(
        db,
        &a,
        &axagent_harness::types::UpdateBotPatch { is_enabled: Some(false), ..Default::default() },
    )
    .await
    .expect("禁用 Bot");
    let conv = axagent_dao::repo::conversation::create_conversation_with_defaults(
        db,
        "线程",
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
            workflow_id: wf_id,
            workflow_name: "wf".into(),
            origin: "person".into(),
            requester_bot_id: None,
            requester_conversation_id: conv.id,
            total_steps: 1,
            inputs: HashMap::new(),
        },
    )
    .await
    .expect("run");
    start_run(db, None, &run.id).await.expect("start 返回 DTO");
    let failed = wait_run_terminal(db, &run.id).await;
    assert_eq!(failed.status, run_repo::status::FAILED);
    assert_eq!(failed.error_code.as_deref(), Some("BOT_WORKFLOW_STEP_BOT_MISSING"));
}

#[tokio::test]
async fn test_4_abort_cancels_in_flight_and_inherits_error_code() {
    let h = pool().await;
    let db = &h.conn;
    let a = make_bot(db, "A").await;
    let b = make_bot(db, "B").await;
    let (run_id, _) = make_run(
        db,
        vec![step("a", &a, &[], "abort"), step("b", &b, &["a"], "abort")],
        None,
        60,
        2,
    )
    .await;
    start_run(db, None, &run_id).await.expect("启动");
    wait_assignment_count(db, &run_id, 1).await;
    // 用执行超时制造带语义码的步骤失败（executor 返回裸 Err 不带码）。
    let script = script_one(&a, Ok("never".into()));
    let prompts = Arc::new(Mutex::new(Vec::new()));
    drive_queued(db, &script, &prompts, Duration::from_millis(60), Duration::from_millis(5)).await;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        if run_repo::status::is_terminal(
            &run_repo::get_run(db, &run_id).await.expect("读 run").status,
        ) {
            break;
        }
    }
    let run = wait_run_terminal(db, &run_id).await;
    assert_eq!(run.status, run_repo::status::FAILED);
    assert_eq!(run.error_code.as_deref(), Some("BOT_DELEGATE_TIMEOUT"));
    assert!(run.context.skipped.contains(&"b".to_string()), "下游应 skipped");
    assert!(
        list_step_rows(db, &run_id).await.iter().all(|a| a.status != asmt_repo::status::RUNNING)
    );
}

#[tokio::test]
async fn test_5_continue_skips_downstream_but_finishes_independent_branch() {
    let h = pool().await;
    let db = &h.conn;
    let a = make_bot(db, "A").await;
    let b = make_bot(db, "B").await;
    let c = make_bot(db, "C").await;
    let (run_id, _) = make_run(
        db,
        vec![
            step("a", &a, &[], "continue"),
            step("b", &b, &["a"], "abort"),
            step("c", &c, &[], "abort"),
        ],
        None,
        60,
        2,
    )
    .await;
    start_run(db, None, &run_id).await.expect("启动");
    let script = script_one(&a, Ok("A done".into()));
    let prompts = Arc::new(Mutex::new(Vec::new()));
    for _ in 0..100 {
        drive_queued(db, &script, &prompts, Duration::from_millis(30), Duration::from_millis(5))
            .await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        if run_repo::status::is_terminal(
            &run_repo::get_run(db, &run_id).await.expect("读 run").status,
        ) {
            break;
        }
    }
    let run = wait_run_terminal(db, &run_id).await;
    assert_eq!(run.status, run_repo::status::FAILED, "Q6：run 终态一律 failed");
    assert!(run.error_code.is_some());
    let rows = list_step_rows(db, &run_id).await;
    assert!(
        run.context.outputs.contains_key("c"),
        "独立分支应完成: status={} code={:?} detail={:?} ctx={:?} rows={:?}",
        run.status,
        run.error_code,
        run.error_detail,
        run.context,
        rows.iter()
            .map(|a| (a.step_key.clone(), a.status.clone(), a.error_code.clone()))
            .collect::<Vec<_>>()
    );
    assert!(run.context.skipped.contains(&"b".to_string()));
}

#[tokio::test]
async fn test_6_total_timeout_and_cancel_cover_run() {
    let h = pool().await;
    let db = &h.conn;
    let a = make_bot(db, "A").await;
    let (run_id, _) = make_run(db, vec![step("a", &a, &[], "abort")], None, 1, 2).await;
    start_run(db, None, &run_id).await.expect("启动");
    let run = wait_run_terminal(db, &run_id).await;
    assert_eq!(run.status, run_repo::status::FAILED);
    assert_eq!(run.error_code.as_deref(), Some("BOT_WORKFLOW_TIMEOUT"));

    let (run_id2, _) = make_run(db, vec![step("a", &a, &[], "abort")], None, 60, 2).await;
    start_run(db, None, &run_id2).await.expect("启动");
    let rows = wait_assignment_count(db, &run_id2, 1).await;
    let cancelled = cancel_run(db, None, &run_id2).await.expect("取消");
    assert_eq!(cancelled.status, run_repo::status::CANCELLED);
    let row =
        asmt_repo::get_assignment(db, &rows[0].id, |_| String::new()).await.expect("assignment");
    assert_eq!(row.status, asmt_repo::status::CANCELLED);
    assert!(cancel_run(db, None, &run_id2).await.is_err(), "终态取消非法");
}

#[tokio::test]
async fn test_7_member_change_is_detected_at_run_start() {
    let h = pool().await;
    let db = &h.conn;
    let a = make_bot(db, "A").await;
    let dept = dept_repo::create_department(db, "研究部", "", None).await.expect("部门");
    dept_repo::add_bot_to_department(db, &dept.id, &a).await.expect("入部");
    let (run_id, _) = make_run(db, vec![step("a", &a, &[], "abort")], Some(&dept.id), 60, 2).await;
    dept_repo::remove_bot_from_department(db, &dept.id, &a).await.expect("出部");
    start_run(db, None, &run_id).await.expect("返回 run DTO");
    let run = wait_run_terminal(db, &run_id).await;
    assert_eq!(run.status, run_repo::status::FAILED);
    assert_eq!(run.error_code.as_deref(), Some("BOT_WORKFLOW_MEMBER_CHANGED"));
}

/// P4 收口（验收条件④）：cron 触发超配额时，失败必须**落 history 且不建 run 行**。
///
/// 这条链路此前只在 `bot_quota.rs` 注释里写着「history 记 failed 属调度器闭包内既有
/// 错误路径，本文件无法驱动」——本模块直接驱动抽出来的 `run_scheduled_bot_workflow`
/// （闭包体，行为不变），把「校验拒绝」与「失败落历史」两半一次覆盖。
mod p4_scheduled_cron {
    use super::*;
    use axagent_dao::repo::bot_department_quota as quota_repo;
    use axagent_harness::types::BotDepartmentQuotaInput;
    use axagent_lib::init::services::run_scheduled_bot_workflow;
    use axagent_runtime_core::{CronJob, CronJobStore};

    fn cron_job(id: &str, workflow_id: &str) -> CronJob {
        let mut job = CronJob::new("编排定时", "0 9 * * *", "编排定时", "编排定时");
        job.id = id.to_string();
        job.task_type = Some("bot_workflow".to_string());
        job.params = Some(serde_json::json!({ "workflowId": workflow_id }));
        job
    }

    async fn make_department_workflow(
        db: &DatabaseConnection,
        dept: &str,
        steps: Vec<BotWorkflowStep>,
    ) -> String {
        workflow_repo::create_workflow(
            db,
            &CreateBotWorkflow {
                name: "定时编排".into(),
                owner_scope: "department".into(),
                department_id: Some(Some(dept.to_string())),
                steps,
                max_concurrency: 2,
                total_timeout_secs: 900,
                ..Default::default()
            },
        )
        .await
        .expect("建编排")
        .id
    }

    /// ① 部门上限 1 步、cron job 的定义是 3 步 ⇒ 触发函数返回 Err（BOT_QUOTA_EXCEEDED，
    /// detail 带 maxSteps），history 记 failed，run 表零新增。
    #[tokio::test]
    async fn over_quota_cron_trigger_fails_history_and_creates_no_run() {
        let h = pool().await;
        let db = &h.conn;
        let dept = dept_repo::create_department(db, "配额部门", "", None).await.expect("建部门").id;
        let bot = make_bot(db, "配额 Bot").await;
        quota_repo::upsert(
            db,
            &BotDepartmentQuotaInput {
                department_id: dept.clone(),
                max_concurrency: 2,
                total_timeout_secs: 900,
                max_steps: 1,
                max_in_flight_per_depth: 2,
                step_timeout_secs: 300,
                max_concurrent_runs: 4,
            },
        )
        .await
        .expect("写部门配额");
        let workflow_id = make_department_workflow(
            db,
            &dept,
            vec![
                step("a", &bot, &[], "abort"),
                step("b", &bot, &[], "abort"),
                step("c", &bot, &[], "abort"),
            ],
        )
        .await;

        let job = cron_job("cron-over-quota", &workflow_id);
        let store = Arc::new(CronJobStore::new_ephemeral());
        store.add(job.clone()).await;

        let err = run_scheduled_bot_workflow(db.clone(), None, store.clone(), job)
            .await
            .expect_err("超配额必须返回 Err");
        assert!(err.contains("BOT_QUOTA_EXCEEDED"), "错误码应可读: {err}");
        assert!(err.contains("maxSteps"), "detail 应带越界字段: {err}");

        let after = store.get("cron-over-quota").await.expect("job 仍在 store");
        assert_eq!(after.run_count, 1, "失败轮也推进 run_count");
        let recorded = after.last_result.expect("history 必须记下失败轮");
        assert!(!recorded.success, "history 应记 failed");
        let recorded_err = recorded.error.unwrap_or_default();
        assert!(
            recorded_err.contains("BOT_QUOTA_EXCEEDED"),
            "history.error 应带错误码: {recorded_err}"
        );

        let runs = run_repo::list_runs(db, Some(&workflow_id), None).await.expect("列出 run");
        assert!(runs.is_empty(), "超配额不得创建 run 行，实际 {} 行", runs.len());
    }

    /// ② 未超配额 ⇒ 正常创建 run 并跑到终态，history 记 success（配额只拦超限）。
    #[tokio::test]
    async fn under_quota_cron_trigger_creates_run_and_records_success() {
        let h = pool().await;
        let db = &h.conn;
        let dept = dept_repo::create_department(db, "放行部门", "", None).await.expect("建部门").id;
        let bot = make_bot(db, "执行 Bot").await;
        // 部门编排在 start_run 会校验成员快照：Bot 必须在部门内（否则 MEMBER_CHANGED）。
        dept_repo::add_bot_to_department(db, &dept, &bot).await.expect("入部");
        quota_repo::upsert(
            db,
            &BotDepartmentQuotaInput {
                department_id: dept.clone(),
                max_concurrency: 2,
                total_timeout_secs: 900,
                max_steps: 3,
                max_in_flight_per_depth: 2,
                step_timeout_secs: 300,
                max_concurrent_runs: 4,
            },
        )
        .await
        .expect("写部门配额");
        let workflow_id =
            make_department_workflow(db, &dept, vec![step("a", &bot, &[], "abort")]).await;

        let job = cron_job("cron-under-quota", &workflow_id);
        let store = Arc::new(CronJobStore::new_ephemeral());
        store.add(job.clone()).await;

        // 无窗口 + 无模型：用脚本执行器驱动 assignment（与既有用例同一套 drive_queued）。
        let driver_db = db.clone();
        let driver_workflow_id = workflow_id.clone();
        let script = script_one(&bot, Ok("定时结论".to_string()));
        let prompts: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (outcome, _) =
            tokio::join!(run_scheduled_bot_workflow(db.clone(), None, store.clone(), job), async {
                for _ in 0..200 {
                    drive_queued(
                        &driver_db,
                        &script,
                        &prompts,
                        Duration::ZERO,
                        Duration::from_secs(10),
                    )
                    .await;
                    let runs = run_repo::list_runs(&driver_db, Some(&driver_workflow_id), None)
                        .await
                        .expect("列出 run");
                    if runs.iter().any(|r| run_repo::status::is_terminal(&r.status)) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            });
        outcome.expect("未超配额应成功");

        let runs = run_repo::list_runs(db, Some(&workflow_id), None).await.expect("列出 run");
        assert_eq!(runs.len(), 1, "未超配额须创建 1 个 run");
        assert_eq!(runs[0].origin, "routine", "定时触发 origin=routine");
        assert_eq!(runs[0].status, run_repo::status::COMPLETED, "单步编排应完成");

        let after = store.get("cron-under-quota").await.expect("job 仍在 store");
        assert_eq!(after.run_count, 1);
        let recorded = after.last_result.expect("history 必须记下本轮");
        assert!(recorded.success, "history 应记成功: {:?}", recorded.error);
        assert!(
            recorded.output.unwrap_or_default().contains("completed"),
            "output 应带 run 完成说明"
        );
    }
}
