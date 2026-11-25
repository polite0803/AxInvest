// SPDX-License-Identifier: AGPL-3.0-only

//! P4-b 持久化待答请求集成测试骨架（PLAN-P4 §6.4 B5 场景 / MODEL-P4 §3 / IPC-P4 §1·§3·§4）。
//!
//! ## 文件状态（2026-11-25）
//!
//! B1 的实体文件 `axagent-core/crates/entities/src/bot_input_requests.rs` 已落盘但未在
//! `entities/src/lib.rs` 注册，DAO 仓库 `axagent-infra/crates/dao/src/repo/bot_input_request.rs`
//! 与命令层 `src-tauri/src/commands/bot_input.rs` 尚未存在；因此本文件当前**只编译不运行任何
//! pending 场景**：
//!
//! - 可运行部分：冻结数值守卫（ADR-P4 D5 的 300s 与 P3 派发超时同源）。
//! - 待点亮部分：`mod p4b` 整体被 `#[cfg(any())]` 屏蔽；**不改断言，只改门**。
//!
//! ## 点亮步骤（B1–B3 落地后一次性完成）
//!
//! 1. `entities/src/lib.rs` 注册 `bot_input_requests`；DAO 仓库按 MODEL-P4 §3 提供
//!    `create / get_by_id / get_by_ask_id / list / transition / expire_stale /
//!    count_pending_by_bot`。
//! 2. 把下方 `#[cfg(any())]` 改为 `#[cfg(all())]`。
//! 3. 按 DAO 实际入参（`create` 取 entity Model 还是 harness DTO、是否首参 `db`）校一次
//!    import 与 `make_row` 构造；**断言文本不改**。
//! 4. 运行 `cargo test --manifest-path src-tauri/Cargo.toml --test bot_input`，全绿后
//!    在 ACCEPTANCE-P4 登记「B5 骨架已点亮」。
//!
//! ## B5 场景 → 归属（哪些在这、哪些在别处，避免重复造断言）
//!
//! | B5 场景 | 归属 | 说明 |
//! |---|---|---|
//! | ① Bot 线程提问 → 行 pending + 事件 | B2（`AppAskUserBridge` 注入 `request_context`） | 需真实交互桥与 oneshot，本文件只锁 DAO 的落行契约 |
//! | ② 作答 → answered + 事件 + 模型继续 | B2 + 本文件 `answer_only_from_pending` | 模型继续由 B2 的 oneshot 测试锁 |
//! | ③ 300s 超时 → expired + `AGENT_INPUT_WAIT_REPLY_TIMEOUT` | B2 | DAO 侧只保证行状态迁移，超时触发在桥 |
//! | ④ 重启清扫 → pending 全 expired | B2（`init/services.rs` 调 `expire_stale`）+ 本文件 `expire_stale_sweeps_all_pending` | DAO 批量语义在这，启动时机在 B2 |
//! | ⑤ 通用聊天（无 Bot）不落行、行为零变化 | B2 | 行写入在桥的 Bot 分支内，DAO 不参与判定；e2e 侧由 `bot-needs-input.spec.ts` 的 `AC-B5-UI-5` 待点亮 |
//! | ⑥ 无活 sender 答复 → `BOT_INPUT_REQUEST_NOT_PENDING` | B3（命令层） | 命令层错误码，DAO 只回 `transition == false` |
//!
//! 命令层错误码（`BOT_INPUT_ANSWER_EMPTY` / `BOT_INPUT_REQUEST_NOT_PENDING` /
//! `BOT_INPUT_REQUEST_NOT_FOUND`）与过滤/排序由 B3 的命令测试覆盖；本文件不复制一份
//! 命令层替身，避免出现「测了替身、漏了真命令」的假绿。

use std::time::Duration;

/// ADR-P4 D5：pending 请求 300s 过期。
///
/// 该值与 P3 单轮派发超时 [`axagent_lib::bot_assignment_runner::DISPATCH_TIMEOUT_SECS`]
/// 同源；B2 落库时 `expires_at = created_at + 300_000`。若 P3 常量被改动而这里不跟，
/// 待答卡会先于模型轮被判过期（或反之），属于规格级事故，因此用编译期可见的守卫钉住。
const P4B_INPUT_EXPIRY_MS: i64 = 300_000;

#[test]
fn frozen_expiry_matches_p3_dispatch_cap() {
    assert_eq!(
        P4B_INPUT_EXPIRY_MS,
        axagent_lib::bot_assignment_runner::DISPATCH_TIMEOUT_SECS as i64 * 1_000,
        "ADR-P4 D5 的 300s 过期必须与 P3 派发超时同源；变更需先改 ADR"
    );
    assert_eq!(Duration::from_millis(P4B_INPUT_EXPIRY_MS as u64), Duration::from_secs(300));
}

/// 待点亮：B1–B3 落地后把 `cfg(any())` 改为 `cfg(all())`（见文件头「点亮步骤」）。
/// 断言按 MODEL-P4 §3 的生命周期与 IPC-P4 §1 的 DTO 语义书写，不依赖 UI。
#[cfg(any())]
mod p4b {
    use axagent_dao::DatabaseConnection;
    use axagent_dao::repo::bot_input_request as input_repo;
    use axagent_entities::bot_input_requests as entity;

    async fn pool() -> axagent_dao::db::DbHandle {
        axagent_dao::db::create_test_pool().await.expect("测试库")
    }

    /// 构造 pending 行：`expires_at = created_at + 300_000`（ADR-P4 D5）。
    fn make_row(ask: &str, conv: &str, bot: Option<&str>, created_at: i64) -> entity::Model {
        entity::Model {
            id: format!("input-{ask}"),
            ask_id: ask.to_string(),
            conversation_id: conv.to_string(),
            bot_id: bot.map(str::to_string),
            workflow_run_id: None,   // D4=A 恒 NULL
            assignment_id: None,     // D4=A 恒 NULL
            origin: "person".into(), // D4=A 恒 person
            question: "请确认标的池范围？".into(),
            options_json: r#"["沪深300","中证500"]"#.into(),
            status: "pending".into(),
            answer: None,
            answered_at: None,
            expires_at: created_at + 300_000,
            created_at,
            updated_at: created_at,
        }
    }

    /// B1：落 pending 行后按 id 与 ask_id 双路可读，字段 round-trip 不丢。
    #[tokio::test]
    async fn create_pending_row_and_dual_lookup() {
        let h = pool().await;
        let db = &h.conn;
        let row = make_row("ask-dual", "conv-1", Some("bot-1"), 1_700_000_000_000);
        input_repo::create(db, &row).await.expect("落行");

        let by_id = input_repo::get_by_id(db, &row.id).await.expect("读行").expect("存在");
        assert_eq!(by_id.ask_id, "ask-dual");
        assert_eq!(by_id.status, "pending");
        assert_eq!(by_id.options_json, r#"["沪深300","中证500"]"#);
        assert_eq!(by_id.expires_at - by_id.created_at, 300_000);

        let by_ask = input_repo::get_by_ask_id(db, "ask-dual").await.expect("按 ask 读").expect("存在");
        assert_eq!(by_ask.id, row.id, "ask_id 必须唯一定位同一行");
        assert!(input_repo::get_by_id(db, "input-ghost").await.expect("读空").is_none());
    }

    /// B1：`UNIQUE(ask_id)` —— 同一 askId 重复落行必须失败（答复投递靠它去重）。
    #[tokio::test]
    async fn duplicate_ask_id_rejected() {
        let h = pool().await;
        let db = &h.conn;
        let first = make_row("ask-dup", "conv-1", Some("bot-1"), 1_700_000_000_000);
        input_repo::create(db, &first).await.expect("首次落行");

        let mut second = make_row("ask-dup", "conv-1", Some("bot-1"), 1_700_000_001_000);
        second.id = "input-different-id".into();
        assert!(
            input_repo::create(db, &second).await.is_err(),
            "ask_id 唯一约束必须生效（否则答复会打到两行）"
        );
    }

    /// B1/B3：`answer` 只接受 pending；终态不可回退（`transition` 返回 false）。
    #[tokio::test]
    async fn answer_only_from_pending() {
        let h = pool().await;
        let db = &h.conn;
        let row = make_row("ask-answer", "conv-1", Some("bot-1"), 1_700_000_000_000);
        input_repo::create(db, &row).await.expect("落行");

        let now = 1_700_000_010_000;
        assert!(
            input_repo::transition(db, &row.id, "pending", "answered", Some("沪深300"), now)
                .await
                .expect("迁移"),
            "pending → answered 必须命中"
        );
        let answered = input_repo::get_by_id(db, &row.id).await.expect("读行").expect("存在");
        assert_eq!(answered.status, "answered");
        assert_eq!(answered.answer.as_deref(), Some("沪深300"));
        assert_eq!(answered.answered_at, Some(now));

        // 重复答复 / 取消已答行：from_status 不再匹配 → 未命中（命令层据此回 NOT_PENDING）
        assert!(
            !input_repo::transition(db, &row.id, "pending", "answered", Some("中证500"), now + 1)
                .await
                .expect("迁移"),
            "answered 行不得再次被作答"
        );
        assert!(
            !input_repo::transition(db, &row.id, "pending", "cancelled", None, now + 2)
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
        let a = make_row("ask-stale-a", "conv-1", Some("bot-1"), 1_700_000_000_000);
        let b = make_row("ask-stale-b", "conv-2", Some("bot-2"), 1_700_000_000_000);
        let done = make_row("ask-done", "conv-1", Some("bot-1"), 1_700_000_000_000);
        for row in [&a, &b, &done] {
            input_repo::create(db, row).await.expect("落行");
        }
        input_repo::transition(db, &done.id, "pending", "answered", Some("已答"), 1_700_000_001_000)
            .await
            .expect("先答一行");

        let swept = input_repo::expire_stale(db, 1_700_000_100_000).await.expect("清扫");
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
    #[tokio::test]
    async fn list_filters_and_created_at_desc() {
        let h = pool().await;
        let db = &h.conn;
        let base = 1_700_000_000_000;
        let old = make_row("ask-old", "conv-1", Some("bot-1"), base);
        let mid = make_row("ask-mid", "conv-2", Some("bot-1"), base + 1_000);
        let mut run_row = make_row("ask-run", "conv-1", Some("bot-1"), base + 2_000);
        run_row.origin = "workflow".into();
        run_row.workflow_run_id = Some("run-1".into());
        for row in [&old, &mid, &run_row] {
            input_repo::create(db, row).await.expect("落行");
        }

        let filter = axagent_harness::types::BotInputRequestFilter {
            conversation_id: Some("conv-1".into()),
            bot_id: None,
            status: None,
            workflow_run_id: None,
        };
        let listed = input_repo::list(db, &filter).await.expect("列表");
        assert_eq!(
            listed.iter().map(|r| r.ask_id.as_str()).collect::<Vec<_>>(),
            vec!["ask-run", "ask-old"],
            "同线程按 created_at DESC"
        );

        let by_run = axagent_harness::types::BotInputRequestFilter {
            workflow_run_id: Some("run-1".into()),
            ..filter.clone()
        };
        let listed = input_repo::list(db, &by_run).await.expect("按 run 列表");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].ask_id, "ask-run");

        let by_bot = axagent_harness::types::BotInputRequestFilter {
            conversation_id: None,
            bot_id: Some("bot-1".into()),
            status: Some("pending".into()),
            workflow_run_id: None,
        };
        assert_eq!(input_repo::list(db, &by_bot).await.expect("按 Bot+状态").len(), 3);
    }

    /// B4 侧栏徽章的 DAO 依据：pending 计数按 bot 分组，终态不计。
    #[tokio::test]
    async fn count_pending_by_bot_for_badge() {
        let h = pool().await;
        let db = &h.conn;
        let base = 1_700_000_000_000;
        for (ask, bot, conv) in [
            ("ask-1", "bot-1", "conv-1"),
            ("ask-2", "bot-1", "conv-1"),
            ("ask-3", "bot-2", "conv-2"),
        ] {
            input_repo::create(db, &make_row(ask, conv, Some(bot), base)).await.expect("落行");
        }
        let answered = make_row("ask-4", "conv-1", Some("bot-1"), base);
        input_repo::create(db, &answered).await.expect("落行");
        input_repo::transition(db, &answered.id, "pending", "answered", Some("好"), base + 1)
            .await
            .expect("作答");

        assert_eq!(input_repo::count_pending_by_bot(db, "bot-1").await.expect("计数"), 2);
        assert_eq!(input_repo::count_pending_by_bot(db, "bot-2").await.expect("计数"), 1);
        assert_eq!(input_repo::count_pending_by_bot(db, "bot-none").await.expect("计数"), 0);
    }
}