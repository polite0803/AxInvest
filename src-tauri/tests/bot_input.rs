// SPDX-License-Identifier: AGPL-3.0-only

//! P4-b 持久化待答请求集成测试（PLAN-P4 §6.4 B5 / MODEL-P4 §3 / IPC-P4 §1·§3·§4）。
//!
//! ## 覆盖范围（2026-11-25，B1 已落盘故本文件已点亮）
//!
//! 锁 DAO 层冻结语义：双路查询、`UNIQUE(ask_id)`、只从 `pending` 迁移、终态不可回退、
//! 300s 过期窗口与 P3 派发超时同源、启动清扫只扫 `pending`、四过滤组合 + `created_at DESC`、
//! 侧栏徽章的 pending 计数。
//!
//! 不在此文件（避免用替身冒充真命令）：
//!
//! | B5 场景 | 归属 | 说明 |
//! |---|---|---|
//! | ① Bot 线程提问 → 行 pending + 事件 | B2（`AppAskUserBridge` + `bot_input_request.rs` helper） | 需真实交互桥与 oneshot |
//! | ② 作答 → answered + 事件 + 模型继续 | B2；DAO 侧见 `answer_only_from_pending` | 模型继续由 B2 的 oneshot 测试锁 |
//! | ③ 300s 超时 → expired + `AGENT_INPUT_WAIT_REPLY_TIMEOUT` | B2 | DAO 侧只保证行状态迁移，超时触发在桥 |
//! | ④ 重启清扫 → pending 全 expired | B2 调 `expire_stale`；DAO 批量语义见 `expire_stale_sweeps_all_pending` | 启动时机在 B2 |
//! | ⑤ 通用聊天（无 Bot）不落行、行为零变化 | B2 | 行写入在桥的 Bot 分支内，DAO 不参与判定；e2e 侧见 `bot-needs-input.spec.ts` 的 `AC-B5-UI-5`（待 B4 点亮） |
//! | ⑥ 无活 sender 答复 → `BOT_INPUT_REQUEST_NOT_PENDING` | B3（命令层） | DAO 只回 `transition == false`，命令层翻译成错误码 |
//!
//! 命令层错误码（`BOT_INPUT_ANSWER_EMPTY` / `BOT_INPUT_REQUEST_NOT_PENDING` /
//! `BOT_INPUT_REQUEST_NOT_FOUND`）与事件三态各一次，由 B3 的命令测试与
//! `e2e/bot-needs-input.spec.ts` 的契约自检共同覆盖。

use std::time::Duration;

use axagent_dao::repo::bot_input_request as input_repo;
use axagent_harness::types::{BotInputRequest, BotInputRequestFilter};

/// ADR-P4 D5：pending 请求 300s 过期。
///
/// 该值与 P3 单轮派发超时 [`axagent_lib::bot_assignment_runner::DISPATCH_TIMEOUT_SECS`]
/// 同源；若 P3 常量被改动而 TTL 不跟，待答卡会先于模型轮被判过期（或反之），
/// 属于规格级事故，因此用三层守卫钉住：DAO 常量、P3 常量、Duration 换算。
#[test]
fn frozen_expiry_matches_p3_dispatch_cap() {
    assert_eq!(
        input_repo::INPUT_REQUEST_TTL_MS,
        axagent_lib::bot_assignment_runner::DISPATCH_TIMEOUT_SECS as i64 * 1_000,
        "ADR-P4 D5 的 300s 过期必须与 P3 派发超时同源；变更需先改 ADR"
    );
    assert_eq!(
        Duration::from_millis(input_repo::INPUT_REQUEST_TTL_MS as u64),
        Duration::from_secs(300)
    );
}

async fn pool() -> axagent_dao::db::DbHandle {
    axagent_dao::db::create_test_pool().await.expect("测试库")
}

/// 构造 pending 入参。`created_at` 由 DAO 落库时取 `now_ts()`，故此处不给；
/// `expires_at` 按 ADR-P4 D5 由调用方算好（DAO 不重复推导）。
fn make_req(ask: &str, conv: &str, bot: Option<&str>, now: i64) -> BotInputRequest {
    BotInputRequest {
        id: String::new(), // 空 = DAO 生成
        ask_id: ask.to_string(),
        conversation_id: conv.to_string(),
        bot_id: bot.map(str::to_string),
        workflow_run_id: None,   // D4=A 恒 None
        assignment_id: None,     // D4=A 恒 None
        origin: "person".into(), // D4=A 恒 person
        question: "请确认标的池范围？".into(),
        options: vec!["沪深300".into(), "中证500".into()],
        status: "pending".into(),
        answer: None,
        answered_at: None,
        expires_at: now + input_repo::INPUT_REQUEST_TTL_MS,
        created_at: 0,
        updated_at: 0,
    }
}

/// B1：落 pending 行后按 id 与 ask_id 双路可读，options JSON 往返成 `string[]`。
#[tokio::test]
async fn create_pending_row_and_dual_lookup() {
    let h = pool().await;
    let db = &h.conn;
    let now = 1_700_000_000_000;
    let created = input_repo::create(db, &make_req("ask-dual", "conv-1", Some("bot-1"), now))
        .await
        .expect("落行");
    assert!(!created.id.is_empty(), "空 id 必须由 DAO 生成");
    assert_eq!(created.status, "pending");
    assert_eq!(created.expires_at, now + input_repo::INPUT_REQUEST_TTL_MS);
    assert_eq!(created.options, vec!["沪深300".to_string(), "中证500".to_string()]);

    let by_id = input_repo::get_by_id(db, &created.id).await.expect("读行").expect("存在");
    assert_eq!(by_id, created, "DTO round-trip 不丢字段");

    let by_ask = input_repo::get_by_ask_id(db, "ask-dual")
        .await
        .expect("按 ask 读")
        .expect("存在");
    assert_eq!(by_ask.id, created.id, "ask_id 必须唯一定位同一行");
    assert!(input_repo::get_by_id(db, "input-ghost").await.expect("读空").is_none());
}

/// B1：`UNIQUE(ask_id)` —— 同一 askId 重复落行必须失败（答复投递靠它去重）。
#[tokio::test]
async fn duplicate_ask_id_rejected() {
    let h = pool().await;
    let db = &h.conn;
    let now = 1_700_000_000_000;
    input_repo::create(db, &make_req("ask-dup", "conv-1", Some("bot-1"), now))
        .await
        .expect("首次落行");
    assert!(
        input_repo::create(db, &make_req("ask-dup", "conv-1", Some("bot-1"), now + 1_000))
            .await
            .is_err(),
        "ask_id 唯一约束必须生效（否则答复会打到两行）"
    );
}

/// B1/B3：`answer` 只接受 pending；终态不可回退（`transition` 返回 false）。
#[tokio::test]
async fn answer_only_from_pending() {
    let h = pool().await;
    let db = &h.conn;
    let now = 1_700_000_000_000;
    let row = input_repo::create(db, &make_req("ask-answer", "conv-1", Some("bot-1"), now))
        .await
        .expect("落行");

    let answered_at = now + 10_000;
    assert!(
        input_repo::transition(
            db, &row.id, "pending", "answered", Some("沪深300"), answered_at
        )
        .await
        .expect("迁移"),
        "pending → answered 必须命中"
    );
    let answered = input_repo::get_by_id(db, &row.id).await.expect("读行").expect("存在");
    assert_eq!(answered.status, "answered");
    assert_eq!(answered.answer.as_deref(), Some("沪深300"));
    assert_eq!(answered.answered_at, Some(answered_at));

    // 重复答复 / 取消已答行：from_status 不再匹配 → 未命中（命令层据此回 NOT_PENDING）
    assert!(
        !input_repo::transition(db, &row.id, "pending", "answered", Some("中证500"), now + 20_000)
            .await
            .expect("迁移"),
        "answered 行不得再次被作答"
    );
    assert!(
        !input_repo::transition(db, &row.id, "pending", "cancelled", None, now + 30_000)
            .await
            .expect("迁移"),
        "answered 行不得被取消"
    );
    let after = input_repo::get_by_id(db, &row.id).await.expect("读行").expect("存在");
    assert_eq!(after.answer.as_deref(), Some("沪深300"), "终态不可回退");
}

/// B2：启动清扫把所有遗留 pending 置 expired，answered 不受影响（MODEL-P4 §3）。
#[tokio::test]
async fn expire_stale_sweeps_all_pending() {
    let h = pool().await;
    let db = &h.conn;
    let now = 1_700_000_000_000;
    let a = input_repo::create(db, &make_req("ask-stale-a", "conv-1", Some("bot-1"), now))
        .await
        .expect("落行");
    let b = input_repo::create(db, &make_req("ask-stale-b", "conv-2", Some("bot-2"), now))
        .await
        .expect("落行");
    let done = input_repo::create(db, &make_req("ask-done", "conv-1", Some("bot-1"), now))
        .await
        .expect("落行");
    input_repo::transition(db, &done.id, "pending", "answered", Some("已答"), now + 1_000)
        .await
        .expect("先答一行");

    let swept = input_repo::expire_stale(db, now + 100_000).await.expect("清扫");
    assert_eq!(swept, 2, "只清扫 pending 行");

    for id in [&a.id, &b.id] {
        let row = input_repo::get_by_id(db, id).await.expect("读行").expect("存在");
        assert_eq!(row.status, "expired");
        assert_eq!(row.answer, None, "过期原因不写 answer");
    }
    let kept = input_repo::get_by_id(db, &done.id).await.expect("读行").expect("存在");
    assert_eq!(kept.status, "answered", "清扫不得改动终态行");
}

/// B3：list 四个过滤字段可组合，顺序固定 `created_at DESC`（IPC-P4 §1）。
/// `create` 用 `now_ts()` 覆盖 `created_at`，故按落库顺序 + 毫秒间隔制造先后。
#[tokio::test]
async fn list_filters_and_created_at_desc() {
    let h = pool().await;
    let db = &h.conn;
    let now = 1_700_000_000_000;
    let old = input_repo::create(db, &make_req("ask-old", "conv-1", Some("bot-1"), now))
        .await
        .expect("落行");
    tokio::time::sleep(Duration::from_millis(5)).await;
    let mid = input_repo::create(db, &make_req("ask-mid", "conv-2", Some("bot-1"), now))
        .await
        .expect("落行");
    tokio::time::sleep(Duration::from_millis(5)).await;
    let mut run_req = make_req("ask-run", "conv-1", Some("bot-1"), now);
    run_req.origin = "workflow".into();
    run_req.workflow_run_id = Some("run-1".into());
    let run_row = input_repo::create(db, &run_req).await.expect("落行");
    assert!(old.created_at <= mid.created_at && mid.created_at <= run_row.created_at);

    let filter = BotInputRequestFilter {
        conversation_id: Some("conv-1".into()),
        ..Default::default()
    };
    let listed = input_repo::list(db, &filter).await.expect("列表");
    assert_eq!(
        listed.iter().map(|r| r.ask_id.as_str()).collect::<Vec<_>>(),
        vec!["ask-run", "ask-old"],
        "同线程按 created_at DESC"
    );

    let by_run = BotInputRequestFilter {
        workflow_run_id: Some("run-1".into()),
        ..Default::default()
    };
    let listed = input_repo::list(db, &by_run).await.expect("按 run 列表");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].ask_id, "ask-run");

    let by_bot = BotInputRequestFilter {
        bot_id: Some("bot-1".into()),
        status: Some("pending".into()),
        ..Default::default()
    };
    assert_eq!(input_repo::list(db, &by_bot).await.expect("按 Bot+状态").len(), 3);
}

/// B4 侧栏徽章的 DAO 依据：pending 计数按 bot 分组，终态不计。
#[tokio::test]
async fn count_pending_by_bot_for_badge() {
    let h = pool().await;
    let db = &h.conn;
    let now = 1_700_000_000_000;
    for (ask, bot, conv) in [
        ("ask-1", "bot-1", "conv-1"),
        ("ask-2", "bot-1", "conv-1"),
        ("ask-3", "bot-2", "conv-2"),
    ] {
        input_repo::create(db, &make_req(ask, conv, Some(bot), now)).await.expect("落行");
    }
    let answered = input_repo::create(db, &make_req("ask-4", "conv-1", Some("bot-1"), now))
        .await
        .expect("落行");
    input_repo::transition(db, &answered.id, "pending", "answered", Some("好"), now + 1)
        .await
        .expect("作答");

    assert_eq!(input_repo::count_pending_by_bot(db, "bot-1").await.expect("计数"), 2);
    assert_eq!(input_repo::count_pending_by_bot(db, "bot-2").await.expect("计数"), 1);
    assert_eq!(input_repo::count_pending_by_bot(db, "bot-none").await.expect("计数"), 0);
}

/// 预留列在 D4=A 下必须恒为 None（防止 B2 顺手写入 run/assignment 关联）。
#[tokio::test]
async fn reserved_columns_stay_null_in_interactive_mode() {
    let h = pool().await;
    let db = &h.conn;
    let row = input_repo::create(db, &make_req("ask-reserved", "conv-1", Some("bot-1"), 1_700_000_000_000))
        .await
        .expect("落行");
    assert_eq!(row.workflow_run_id, None);
    assert_eq!(row.assignment_id, None);
    assert_eq!(row.origin, "person");
}

// ─────────────────────────────────────────────────────────────────────────────
// B2：交互桥生命周期（落库 / 答复 / 超时 / 重启清扫 / 通用聊天零变化）
//
// 桥本体在 `commands/agent/mod.rs`（同步 `AskUserBridge` 实现，依赖 AppHandle 与
// oneshot，无法在集成测试里驱动真实模型轮）；这里锁的是桥所调用的全部生命周期
// 函数：`persist_pending` / `settle_by_ask_id` / `terminal_status_for` /
// `startup_sweep`。`app = None` 表示不发事件（发射是投影，不是状态）。
// ─────────────────────────────────────────────────────────────────────────────
mod bridge_lifecycle {
    use super::*;
    use axagent_lib::bot_input_request as lifecycle;
    use axagent_harness::types::bot::input_status;

    fn ask_row(ask: &str, conv: &str) -> BotInputRequest {
        BotInputRequest {
            id: String::new(),
            ask_id: ask.into(),
            conversation_id: conv.into(),
            bot_id: Some("bot-1".into()),
            workflow_run_id: None,
            assignment_id: None,
            origin: "person".into(),
            question: "继续吗？".into(),
            options: vec!["继续".into(), "停止".into()],
            status: "pending".into(),
            answer: None,
            answered_at: None,
            expires_at: 0,
            created_at: 0,
            updated_at: 0,
        }
    }

    /// ① 落库：Bot 线程提问 ⇒ pending 行，expires_at = now + 300s，origin=person。
    #[tokio::test]
    async fn bridge_persists_pending_with_300s_expiry() {
        let h = pool().await;
        let db = &h.conn;
        let before = axagent_harness::util_fns::now_ms();
        let row = lifecycle::persist_pending(
            db,
            None,
            "conv-1",
            Some("bot-1"),
            "ask-persist",
            "继续吗？",
            vec!["继续".into()],
        )
        .await
        .expect("Bot 线程必须落行");
        let after = axagent_harness::util_fns::now_ms();

        assert_eq!(row.status, input_status::PENDING);
        assert_eq!(row.bot_id.as_deref(), Some("bot-1"));
        assert_eq!(row.origin, "person");
        assert_eq!(row.options, vec!["继续".to_string()]);
        assert!(row.expires_at >= before + lifecycle::EXPIRY_MS);
        assert!(row.expires_at <= after + lifecycle::EXPIRY_MS);
        assert_eq!(lifecycle::EXPIRY_MS, 300_000, "D5=A 冻结 300s");
    }

    /// ② 答复：settle 后 answered + 正文 + 时间；重复答复不再改写（首个胜出）。
    #[tokio::test]
    async fn bridge_settles_answer_once() {
        let h = pool().await;
        let db = &h.conn;
        lifecycle::persist_pending(db, None, "conv-1", Some("bot-1"), "ask-a", "继续吗？", vec![])
            .await
            .expect("persist");

        let answered = lifecycle::settle_by_ask_id(
            db, None, "ask-a", input_status::ANSWERED, Some("继续"),
        )
        .await
        .expect("answer");
        assert_eq!(answered.status, input_status::ANSWERED);
        assert_eq!(answered.answer.as_deref(), Some("继续"));
        assert!(answered.answered_at.is_some());

        let again = lifecycle::settle_by_ask_id(
            db, None, "ask-a", input_status::ANSWERED, Some("停止"),
        )
        .await;
        assert!(again.is_none(), "terminal row must not re-settle");
        let fresh = input_repo::get_by_ask_id(db, "ask-a").await.expect("read").expect("row");
        assert_eq!(fresh.answer.as_deref(), Some("继续"));
    }

    /// ③ 超时：桥的超时错误码映射为 expired（不写答复正文）。
    #[tokio::test]
    async fn bridge_timeout_marks_expired() {
        let h = pool().await;
        let db = &h.conn;
        lifecycle::persist_pending(db, None, "conv-1", Some("bot-1"), "ask-t", "继续吗？", vec![])
            .await
            .expect("persist");

        let status = lifecycle::terminal_status_for("AGENT_INPUT_WAIT_REPLY_TIMEOUT");
        assert_eq!(status, input_status::EXPIRED);
        let row = lifecycle::settle_by_ask_id(db, None, "ask-t", status, None)
            .await
            .expect("expire");
        assert_eq!(row.status, input_status::EXPIRED);
        assert_eq!(row.answer, None);
        assert_eq!(row.answered_at, None);

        // 通道关闭（run 被取消）= cancelled，与超时区分。
        assert_eq!(
            lifecycle::terminal_status_for("AGENT_INPUT_CHANNEL_CLOSED"),
            input_status::CANCELLED
        );
    }

    /// ④ 重启清扫：遗留 pending 全 expired，已答复行不受影响，且幂等。
    #[tokio::test]
    async fn startup_sweep_expires_only_leftovers() {
        let h = pool().await;
        let db = &h.conn;
        lifecycle::persist_pending(db, None, "conv-1", Some("bot-1"), "ask-1", "q1", vec![]).await.expect("p1");
        lifecycle::persist_pending(db, None, "conv-1", Some("bot-1"), "ask-2", "q2", vec![]).await.expect("p2");
        lifecycle::persist_pending(db, None, "conv-1", Some("bot-1"), "ask-3", "q3", vec![]).await.expect("p3");
        lifecycle::settle_by_ask_id(db, None, "ask-3", input_status::ANSWERED, Some("好"))
            .await
            .expect("answer");

        assert_eq!(lifecycle::startup_sweep(db, None).await, 2);
        for ask in ["ask-1", "ask-2"] {
            let r = input_repo::get_by_ask_id(db, ask).await.expect("read").expect("row");
            assert_eq!(r.status, input_status::EXPIRED, "{ask}");
        }
        let answered = input_repo::get_by_ask_id(db, "ask-3").await.expect("read").expect("row");
        assert_eq!(answered.status, input_status::ANSWERED);
        assert_eq!(lifecycle::startup_sweep(db, None).await, 0, "second sweep is a no-op");
    }

    /// ⑤ 通用聊天（无 Bot）：不落行、settle 无行可动 —— 行为零变化。
    #[tokio::test]
    async fn generic_chat_writes_nothing() {
        let h = pool().await;
        let db = &h.conn;
        let row = lifecycle::persist_pending(db, None, "conv-chat", None, "ask-chat", "继续吗？", vec![]).await;
        assert!(row.is_none(), "non-Bot thread must not persist");
        assert!(
            input_repo::list(db, &BotInputRequestFilter::default()).await.expect("list").is_empty(),
            "non-Bot thread must not touch the table"
        );
        assert!(
            lifecycle::settle_by_ask_id(db, None, "ask-chat", input_status::ANSWERED, Some("x"))
                .await
                .is_none()
        );
    }

    /// 桥的落库入参与 DAO 草稿字段一致（防止 helper 少传列导致行不可见）。
    #[test]
    fn ask_row_matches_helper_contract() {
        let draft = ask_row("ask-x", "conv-x");
        assert_eq!(draft.bot_id.as_deref(), Some("bot-1"));
        assert_eq!(draft.status, "pending");
        assert!(draft.workflow_run_id.is_none() && draft.assignment_id.is_none(), "D4=A 恒 NULL");
    }
}
