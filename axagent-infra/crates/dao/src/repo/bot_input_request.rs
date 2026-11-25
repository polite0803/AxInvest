// SPDX-License-Identifier: AGPL-3.0-only

//! `bot_input_requests` 表的仓库层（docs/bot-integration/P4/MODEL-P4 §3）。
//!
//! 待答请求的生命周期独立于 assignment / run（ADR-P4-3）：
//! `pending → answered | expired | cancelled`，终态不可回退。所有状态迁移走
//! [`transition`] 的行级条件更新（`WHERE id = ? AND status = ?`），返回是否命中，
//! 由命令层翻译成 `BOT_INPUT_REQUEST_NOT_PENDING`（并发答复只有一个赢家）。
//!
//! 本层只做数据访问；`options_json` → `options: string[]` 的解析在这里完成，
//! 上层只见 harness DTO（IPC-P4 §2：前端永不接触 JSON 字符串）。

use sea_orm::sea_query::Expr;
use sea_orm::*;

use axagent_entities::bot_input_requests;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::types::bot::{input_origin, input_status};
use axagent_harness::types::{BotInputRequest, BotInputRequestFilter};
use axagent_harness::util_fns::{gen_id, now_ts};

/// 待答请求过期窗口：300s（ADR-P4-4 / D5=A，与 P3 派发超时同源）。
pub const INPUT_REQUEST_TTL_MS: i64 = 300_000;

fn parse_options(raw: &str) -> Result<Vec<String>> {
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(raw)
        .map_err(|e| AxAgentError::Validation(format!("input options_json 损坏: {e}")))
}

fn to_dto(m: bot_input_requests::Model) -> Result<BotInputRequest> {
    Ok(BotInputRequest {
        id: m.id,
        ask_id: m.ask_id,
        conversation_id: m.conversation_id,
        bot_id: m.bot_id,
        workflow_run_id: m.workflow_run_id,
        assignment_id: m.assignment_id,
        origin: m.origin,
        question: m.question,
        options: parse_options(&m.options_json)?,
        status: m.status,
        answer: m.answer,
        answered_at: m.answered_at,
        expires_at: m.expires_at,
        created_at: m.created_at,
        updated_at: m.updated_at,
    })
}

/// 落一行 `pending`。`expires_at` 由调用方按 [`INPUT_REQUEST_TTL_MS`] 计算好
/// （命令层/B2 桥同口径），本层不重复推导。
pub async fn create(db: &DatabaseConnection, req: &BotInputRequest) -> Result<BotInputRequest> {
    let options_json = serde_json::to_string(&req.options)
        .map_err(|e| AxAgentError::Validation(format!("input options 序列化失败: {e}")))?;
    let now = now_ts();
    let am = bot_input_requests::ActiveModel {
        id: Set(if req.id.is_empty() { gen_id() } else { req.id.clone() }),
        ask_id: Set(req.ask_id.clone()),
        conversation_id: Set(req.conversation_id.clone()),
        bot_id: Set(req.bot_id.clone()),
        workflow_run_id: Set(req.workflow_run_id.clone()),
        assignment_id: Set(req.assignment_id.clone()),
        origin: Set(if req.origin.is_empty() {
            "person".to_string()
        } else {
            req.origin.clone()
        }),
        question: Set(req.question.clone()),
        options_json: Set(options_json),
        status: Set(input_status::PENDING.to_string()),
        answer: Set(None),
        answered_at: Set(None),
        expires_at: Set(req.expires_at),
        created_at: Set(now),
        updated_at: Set(now),
    };
    let inserted = bot_input_requests::Entity::insert(am)
        .exec_with_returning(db)
        .await?;
    to_dto(inserted)
}

/// 按主键读一行。
pub async fn get_by_id(db: &DatabaseConnection, id: &str) -> Result<Option<BotInputRequest>> {
    let row = bot_input_requests::Entity::find_by_id(id).one(db).await?;
    row.map(to_dto).transpose()
}

/// 按 `ask_id` 读一行（答复投递与去重的主路径）。
pub async fn get_by_ask_id(db: &DatabaseConnection, ask_id: &str) -> Result<Option<BotInputRequest>> {
    let row = bot_input_requests::Entity::find()
        .filter(bot_input_requests::Column::AskId.eq(ask_id))
        .one(db)
        .await?;
    row.map(to_dto).transpose()
}

/// 过滤查询：四项全可选，按 `created_at DESC`、`id DESC` 稳定排序。
pub async fn list(db: &DatabaseConnection, filter: &BotInputRequestFilter) -> Result<Vec<BotInputRequest>> {
    let mut query = bot_input_requests::Entity::find();
    if let Some(v) = filter.conversation_id.as_deref() {
        query = query.filter(bot_input_requests::Column::ConversationId.eq(v));
    }
    if let Some(v) = filter.bot_id.as_deref() {
        query = query.filter(bot_input_requests::Column::BotId.eq(v));
    }
    if let Some(v) = filter.status.as_deref() {
        query = query.filter(bot_input_requests::Column::Status.eq(v));
    }
    if let Some(v) = filter.workflow_run_id.as_deref() {
        query = query.filter(bot_input_requests::Column::WorkflowRunId.eq(v));
    }
    let rows = query
        .order_by_desc(bot_input_requests::Column::CreatedAt)
        .order_by_desc(bot_input_requests::Column::Id)
        .all(db)
        .await?;
    rows.into_iter().map(to_dto).collect()
}

/// 行级条件迁移：仅当当前状态等于 `from_status` 时改成 `to_status`。
/// 返回是否命中（`false` = 已被并发答复/取消，调用方按
/// `BOT_INPUT_REQUEST_NOT_PENDING` 报错）。`answered` 同时写答复正文与时间；
/// 终态迁移把 `answer` 置空以外的列保持不动。
pub async fn transition(
    db: &DatabaseConnection,
    id: &str,
    from_status: &str,
    to_status: &str,
    answer: Option<&str>,
    now: i64,
) -> Result<bool> {
    if !input_status::is_terminal(to_status) {
        return Err(AxAgentError::Validation(format!(
            "待答请求只能迁移到终态，收到 {to_status}"
        )));
    }
    let mut update = bot_input_requests::Entity::update_many()
        .col_expr(
            bot_input_requests::Column::Status,
            Expr::value(to_status.to_string()),
        )
        .col_expr(bot_input_requests::Column::UpdatedAt, Expr::value(now));
    if let Some(text) = answer {
        update = update
            .col_expr(
                bot_input_requests::Column::Answer,
                Expr::value(text.to_string()),
            )
            .col_expr(bot_input_requests::Column::AnsweredAt, Expr::value(now));
    }
    let res = update
        .filter(bot_input_requests::Column::Id.eq(id))
        .filter(bot_input_requests::Column::Status.eq(from_status))
        .exec(db)
        .await?;
    Ok(res.rows_affected == 1)
}

/// 重启清扫（PLAN-P4 §1.3 / D7=A）：把所有遗留 `pending` 置 `expired`。
/// 只作用于 `pending`，终态行不受影响；返回被清扫行数。
pub async fn expire_stale(db: &DatabaseConnection, now: i64) -> Result<usize> {
    let res = bot_input_requests::Entity::update_many()
        .col_expr(
            bot_input_requests::Column::Status,
            Expr::value(input_status::EXPIRED.to_string()),
        )
        .col_expr(bot_input_requests::Column::Answer, Expr::value(Option::<String>::None))
        .col_expr(bot_input_requests::Column::UpdatedAt, Expr::value(now))
        .filter(bot_input_requests::Column::Status.eq(input_status::PENDING))
        .exec(db)
        .await?;
    Ok(res.rows_affected as usize)
}

/// 侧栏徽章：某 Bot 当前 `pending` 数。
pub async fn count_pending_by_bot(db: &DatabaseConnection, bot_id: &str) -> Result<i64> {
    let n = bot_input_requests::Entity::find()
        .filter(bot_input_requests::Column::BotId.eq(bot_id))
        .filter(bot_input_requests::Column::Status.eq(input_status::PENDING))
        .count(db)
        .await?;
    Ok(n as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> crate::db::DbHandle {
        crate::db::create_test_pool().await.expect("test pool")
    }

    fn row(ask_id: &str, conversation_id: &str, bot_id: Option<&str>) -> BotInputRequest {
        BotInputRequest {
            id: String::new(),
            ask_id: ask_id.to_string(),
            conversation_id: conversation_id.to_string(),
            bot_id: bot_id.map(str::to_string),
            workflow_run_id: None,
            assignment_id: None,
            origin: input_origin::PERSON.to_string(),
            question: "选哪个方案？".to_string(),
            options: vec!["A".to_string(), "B".to_string()],
            status: input_status::PENDING.to_string(),
            answer: None,
            answered_at: None,
            expires_at: now_ts() + INPUT_REQUEST_TTL_MS,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn filter() -> BotInputRequestFilter {
        BotInputRequestFilter::default()
    }

    /// 落库即 `pending`，`options` 往返为字符串数组，`id` 自动生成。
    #[tokio::test]
    async fn create_persists_pending_and_roundtrips_options() {
        let h = pool().await;
        let db = &h.conn;
        let created = create(db, &row("ask-1", "conv-1", Some("bot-1")))
            .await
            .expect("create");
        assert!(!created.id.is_empty());
        assert_eq!(created.status, input_status::PENDING);
        assert_eq!(created.options, vec!["A".to_string(), "B".to_string()]);
        assert_eq!(created.answer, None);
        assert_eq!(created.answered_at, None);
        assert_eq!(created.origin, input_origin::PERSON);

        let by_id = get_by_id(db, &created.id).await.expect("by id").expect("row");
        assert_eq!(by_id.ask_id, "ask-1");
        let by_ask = get_by_ask_id(db, "ask-1").await.expect("by ask").expect("row");
        assert_eq!(by_ask.id, created.id);
        assert!(get_by_ask_id(db, "ask-missing").await.expect("miss").is_none());
    }

    /// 同一 askId 不得落两行（UNIQUE(ask_id) 是答复去重的第一道防线）。
    #[tokio::test]
    async fn duplicate_ask_id_is_rejected() {
        let h = pool().await;
        let db = &h.conn;
        create(db, &row("ask-dup", "conv-1", Some("bot-1"))).await.expect("first");
        let err = create(db, &row("ask-dup", "conv-2", Some("bot-1")))
            .await
            .expect_err("second must violate UNIQUE(ask_id)");
        let text = err.to_string();
        assert!(text.contains("UNIQUE") || text.contains("unique"), "{text}");
    }

    /// 四项过滤各自独立生效；省略即不过滤。
    #[tokio::test]
    async fn list_applies_each_filter() {
        let h = pool().await;
        let db = &h.conn;
        create(db, &row("a1", "conv-a", Some("bot-1"))).await.expect("a1");
        create(db, &row("a2", "conv-a", Some("bot-2"))).await.expect("a2");
        create(db, &row("b1", "conv-b", Some("bot-1"))).await.expect("b1");

        assert_eq!(list(db, &filter()).await.expect("all").len(), 3);

        let mut f = filter();
        f.conversation_id = Some("conv-a".to_string());
        assert_eq!(list(db, &f).await.expect("by conv").len(), 2);

        let mut f = filter();
        f.bot_id = Some("bot-1".to_string());
        assert_eq!(list(db, &f).await.expect("by bot").len(), 2);

        let mut f = filter();
        f.status = Some(input_status::PENDING.to_string());
        assert_eq!(list(db, &f).await.expect("by status").len(), 3);

        let mut f = filter();
        f.workflow_run_id = Some("run-x".to_string());
        assert!(list(db, &f).await.expect("by run").is_empty());
    }

    /// 条件迁移：pending → answered 只命中一次；重复答复返回 false（命令层据此
    /// 报 `BOT_INPUT_REQUEST_NOT_PENDING`），且不改写首次答复。
    #[tokio::test]
    async fn transition_is_conditional_and_idempotent_guard() {
        let h = pool().await;
        let db = &h.conn;
        let r = create(db, &row("ask-t", "conv-1", Some("bot-1"))).await.expect("create");
        let now = now_ts();

        assert!(transition(db, &r.id, input_status::PENDING, input_status::ANSWERED, Some("A"), now)
            .await
            .expect("answer"));
        let answered = get_by_id(db, &r.id).await.expect("read").expect("row");
        assert_eq!(answered.status, input_status::ANSWERED);
        assert_eq!(answered.answer.as_deref(), Some("A"));
        assert_eq!(answered.answered_at, Some(now));

        let again = transition(db, &r.id, input_status::PENDING, input_status::ANSWERED, Some("B"), now + 1)
            .await
            .expect("second answer");
        assert!(!again, "terminal row must not transition again");
        let still = get_by_id(db, &r.id).await.expect("read").expect("row");
        assert_eq!(still.answer.as_deref(), Some("A"), "first answer wins");

        // 非终态目标被拒绝（终态不可回退的入口校验）。
        assert!(transition(db, &r.id, input_status::ANSWERED, input_status::PENDING, None, now).await.is_err());
    }

    /// 重启清扫只动 pending：answered/cancelled 保持原样。
    #[tokio::test]
    async fn expire_stale_only_touches_pending() {
        let h = pool().await;
        let db = &h.conn;
        let p1 = create(db, &row("p1", "conv-1", Some("bot-1"))).await.expect("p1");
        let p2 = create(db, &row("p2", "conv-1", Some("bot-1"))).await.expect("p2");
        let done = create(db, &row("done", "conv-1", Some("bot-1"))).await.expect("done");
        transition(db, &done.id, input_status::PENDING, input_status::ANSWERED, Some("ok"), now_ts())
            .await
            .expect("answer done");

        let swept = expire_stale(db, now_ts() + 1).await.expect("sweep");
        assert_eq!(swept, 2, "two pending rows swept");
        for id in [&p1.id, &p2.id] {
            let r = get_by_id(db, id).await.expect("read").expect("row");
            assert_eq!(r.status, input_status::EXPIRED);
            assert_eq!(r.answer, None);
        }
        let d = get_by_id(db, &done.id).await.expect("read").expect("row");
        assert_eq!(d.status, input_status::ANSWERED);
        assert_eq!(d.answer.as_deref(), Some("ok"));

        // 幂等：再扫一次为 0。
        assert_eq!(expire_stale(db, now_ts() + 2).await.expect("sweep2"), 0);
    }

    /// 侧栏徽章：只数某 Bot 的 pending。
    #[tokio::test]
    async fn count_pending_by_bot_excludes_terminal_and_other_bots() {
        let h = pool().await;
        let db = &h.conn;
        create(db, &row("x1", "conv-1", Some("bot-1"))).await.expect("x1");
        create(db, &row("x2", "conv-1", Some("bot-1"))).await.expect("x2");
        create(db, &row("y1", "conv-1", Some("bot-2"))).await.expect("y1");
        let answered = create(db, &row("x3", "conv-1", Some("bot-1"))).await.expect("x3");
        transition(db, &answered.id, input_status::PENDING, input_status::ANSWERED, Some("v"), now_ts())
            .await
            .expect("answer");

        assert_eq!(count_pending_by_bot(db, "bot-1").await.expect("bot1"), 2);
        assert_eq!(count_pending_by_bot(db, "bot-2").await.expect("bot2"), 1);
        assert_eq!(count_pending_by_bot(db, "bot-none").await.expect("none"), 0);
    }
}
