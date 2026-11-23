// SPDX-License-Identifier: AGPL-3.0-only

//! P2-b T3''（PLAN-P2B §3）：dispatch 委派执行器集成测试。
//!
//! 无模型环境用注入的 FakeExecutor 锁死 queued→running→completed/failed 全链、
//! 事件三态发射点、回执落线程、取消与超时。 consult 工具层测试在
//! `axagent-tools::tools::bot_delegate`（`cargo test -p axagent-tools bot_delegate`）。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axagent_dao::repo::bot as bot_repo;
use axagent_dao::repo::bot_assignment as asmt_repo;
use axagent_dao::DatabaseConnection;
use axagent_harness::types::{BotAssignment, CreateBotAssignment, MessageRole};
use axagent_lib::bot_assignment_runner::{
    run_assignment_with_timeout, AssignmentTurnExecutor, DISPATCH_TIMEOUT_SECS,
};

struct FakeExecutor {
    result: std::result::Result<String, String>,
    delay: Duration,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl AssignmentTurnExecutor for FakeExecutor {
    async fn execute(&self, _bot_id: &str, _task_text: &str) -> Result<String, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        self.result.clone()
    }
}

async fn pool() -> axagent_dao::db::DbHandle {
    axagent_dao::db::create_test_pool().await.expect("测试库")
}

async fn make_bot(db: &DatabaseConnection, name: &str) -> String {
    bot_repo::create_bot(
        db,
        &axagent_harness::types::CreateBot { name: name.to_string(), ..Default::default() },
    )
    .await
    .expect("建 Bot")
    .id
}

async fn make_assignment(db: &DatabaseConnection, assignee: &str, conversation: Option<&str>) -> String {
    asmt_repo::create_assignment(
        db,
        &CreateBotAssignment {
            assignee_bot_id: assignee.to_string(),
            task_text: "汇总今天盘面".to_string(),
            mode: "dispatch".to_string(),
            requester_conversation_id: conversation.map(str::to_string),
            requester_bot_id: None,
        },
    )
    .await
    .expect("建单")
    .id
}

fn sink(events: Arc<tokio::sync::Mutex<Vec<String>>>) -> impl Fn(&BotAssignment) + Send + Sync {
    move |a: &BotAssignment| {
        // sink 是同步闭包；用 try_lock 只在本测试单任务串行场景使用。
        if let Ok(mut v) = events.try_lock() {
            v.push(a.status.clone());
        }
    }
}

#[tokio::test]
async fn dispatch_happy_path_completes_emits_and_appends_receipt() {
    let h = pool().await;
    let db = &h.conn;
    let bot = make_bot(db, "执行者").await;
    let conv = axagent_dao::repo::conversation::create_conversation_with_defaults(
        db,
        "发起线程",
        "m",
        "p",
        None,
        axagent_dao::repo::conversation::ConversationParamDefaults::default(),
        None,
    )
    .await
    .expect("建线程");
    let id = make_assignment(db, &bot, Some(&conv.id)).await;
    let events = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
    let calls = Arc::new(AtomicUsize::new(0));

    run_assignment_with_timeout(
        db,
        &sink(events.clone()),
        &id,
        &FakeExecutor {
            result: Ok("结论：震荡".into()),
            delay: Duration::ZERO,
            calls: calls.clone(),
        },
        Duration::from_secs(5),
    )
    .await;

    let done = asmt_repo::get_assignment(db, &id, |_| "执行者".into()).await.expect("读回");
    assert_eq!(done.status, asmt_repo::status::COMPLETED);
    assert_eq!(done.result_text.as_deref(), Some("结论：震荡"));
    assert!(done.started_at.is_some() && done.finished_at.is_some());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(*events.lock().await, vec!["running", "completed"]);

    let messages = axagent_dao::repo::message::list_messages(db, &conv.id).await.expect("读消息");
    assert!(
        messages.iter().any(|m| m.role == MessageRole::Assistant && m.content.contains("结论：震荡")),
        "completed 结果应落回执线程的普通 assistant 消息"
    );
}

#[tokio::test]
async fn permission_denial_marks_failed_with_readable_detail() {
    let h = pool().await;
    let db = &h.conn;
    let bot = make_bot(db, "ask 档 Bot").await;
    let id = make_assignment(db, &bot, None).await;
    let events = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));

    run_assignment_with_timeout(
        db,
        &sink(events.clone()),
        &id,
        &FakeExecutor {
            result: Err("写操作被权限策略拒绝: ERR_PERMISSION".into()),
            delay: Duration::ZERO,
            calls: Arc::new(AtomicUsize::new(0)),
        },
        Duration::from_secs(5),
    )
    .await;

    let failed = asmt_repo::get_assignment(db, &id, |_| String::new()).await.expect("读回");
    assert_eq!(failed.status, asmt_repo::status::FAILED);
    assert!(failed.error_detail.as_deref().unwrap_or("").contains("ERR_PERMISSION"));
    assert_eq!(*events.lock().await, vec!["running", "failed"]);
}

#[tokio::test]
async fn timeout_marks_failed_with_timeout_code() {
    let h = pool().await;
    let db = &h.conn;
    let bot = make_bot(db, "慢 Bot").await;
    let id = make_assignment(db, &bot, None).await;
    assert_eq!(DISPATCH_TIMEOUT_SECS, 300, "Q5 冻结的超时预算");

    run_assignment_with_timeout(
        db,
        &sink(Arc::new(tokio::sync::Mutex::new(Vec::new()))),
        &id,
        &FakeExecutor {
            result: Ok("太慢".into()),
            delay: Duration::from_millis(500),
            calls: Arc::new(AtomicUsize::new(0)),
        },
        Duration::from_millis(20),
    )
    .await;

    let failed = asmt_repo::get_assignment(db, &id, |_| String::new()).await.expect("读回");
    assert_eq!(failed.status, asmt_repo::status::FAILED);
    assert_eq!(failed.error_code.as_deref(), Some("BOT_DELEGATE_TIMEOUT"));
}

#[tokio::test]
async fn cancelled_assignment_is_not_executed() {
    let h = pool().await;
    let db = &h.conn;
    let bot = make_bot(db, "Bot").await;
    let id = make_assignment(db, &bot, None).await;
    asmt_repo::cancel_assignment(db, &id).await.expect("先取消");
    let events = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
    let calls = Arc::new(AtomicUsize::new(0));

    run_assignment_with_timeout(
        db,
        &sink(events.clone()),
        &id,
        &FakeExecutor {
            result: Ok("不应执行".into()),
            delay: Duration::ZERO,
            calls: calls.clone(),
        },
        Duration::from_secs(5),
    )
    .await;

    let after = asmt_repo::get_assignment(db, &id, |_| String::new()).await.expect("读回");
    assert_eq!(after.status, asmt_repo::status::CANCELLED);
    assert!(after.result_text.is_none(), "已取消的单不得落结果");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(events.lock().await.is_empty(), "取消态不发 running/终态事件");
}
