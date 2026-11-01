// SPDX-License-Identifier: AGPL-3.0-only

//! 反思链「依据不可得」守卫的运行门（2026-10-14）。
//!
//! # 为什么要有这条
//!
//! 单股 / 每日自动化通道历史上不落黑板快照，于是反思链会在**空记忆**上跑完一整轮
//! （`build_sub_analysis_from_snapshot` 对这种行注入空 map 并 WARN），产出的教训文本
//! 看着像结论、实际没有任何逐节点依据 —— 而它会经 `fetch_stock_lessons` 注入后续分析的
//! prompt。实证：15 条无快照的分析行已产出 14 条 `completed` 反思行 + 5 条 pending。
//!
//! 守卫落在 `retire_orphan_pending_reflections`（每轮批处理开头执行一次），把这种 pending
//! 行落成显式终局而不是让它们到期后照跑。本门验的是**这个函数在真库上的行为**，
//! 不是它看起来会这样。
//!
//! # 三条判据各锁一格
//!
//! | 情形 | 期望 | 若实现写错 |
//! |---|---|---|
//! | 源分析 completed 且无快照 | `failed: ANALYSIS_SNAPSHOT_MISSING` | 与孤儿互串 ⇒ 用户以为分析被删了，其实重跑可救 |
//! | 源分析行不存在 | `failed: ORIGINAL_ANALYSIS_MISSING` | 被新码抢走 ⇒ 同上，方向相反 |
//! | 源分析仍在 running | **保持 pending** | 抢先判死 ⇒ 分析跑完也等不到反思 |
//!
//! 幂等性另断一次：退役后的行不再是 pending ⇒ 下一轮不得重复处置。

use axagent_dao::db::create_test_pool;
use axagent_dao::repo::stock_lesson_queries::retire_orphan_pending_reflections;
use axagent_entities::{stock_analyses, stock_reflections};
use sea_orm::{ActiveModelTrait, EntityTrait, Set};

async fn put_analysis(
    db: &sea_orm::DatabaseConnection,
    id: &str,
    status: &str,
    snapshot: Option<&str>,
) {
    let am = stock_analyses::ActiveModel {
        id: Set(id.to_string()),
        stock_code: Set("000001".to_string()),
        stock_name: Set("测试票".to_string()),
        analysis_date: Set("2026-10-14".to_string()),
        provider_id: Set("test".to_string()),
        conversation_id: Set("test-conv".to_string()),
        status: Set(status.to_string()),
        blackboard_snapshot: Set(snapshot.map(str::to_string)),
        created_at: Set(0),
        updated_at: Set(0),
        ..Default::default()
    };
    am.insert(db).await.expect("播种分析行应成功");
}

async fn put_pending(db: &sea_orm::DatabaseConnection, id: &str, analysis_id: &str) {
    let am = stock_reflections::ActiveModel {
        id: Set(id.to_string()),
        stock_code: Set("000001".to_string()),
        stock_name: Set("测试票".to_string()),
        original_analysis_id: Set(analysis_id.to_string()),
        as_of_date: Set("2026-10-14".to_string()),
        hindsight_date: Set("2026-11-11".to_string()),
        min_confidence_threshold: Set(60),
        reflection_depth: Set("light".to_string()),
        actual_outcome: Set(String::new()),
        source: Set("due".to_string()),
        status: Set("pending".to_string()),
        created_at: Set(0),
        updated_at: Set(0),
        ..Default::default()
    };
    am.insert(db).await.expect("播种 pending 反思行应成功");
}

async fn status_of(db: &sea_orm::DatabaseConnection, id: &str) -> String {
    stock_reflections::Entity::find_by_id(id.to_string())
        .one(db)
        .await
        .expect("读反思行应成功")
        .unwrap_or_else(|| panic!("反思行 {id} 不见了"))
        .status
}

#[tokio::test]
async fn snapshot_less_analyses_retire_pending_rows_with_their_own_code() {
    let h = create_test_pool().await.expect("建临时测试库失败");
    let db = &h.conn;

    put_analysis(db, "a-ok", "completed", Some("{\"report.a-market-analyst\":{}}")).await;
    put_analysis(db, "a-no-snap", "completed", None).await;
    put_analysis(db, "a-running", "running", None).await;
    // "a-gone" 故意不播种 ⇒ 孤儿
    put_pending(db, "r-ok", "a-ok").await;
    put_pending(db, "r-no-snap", "a-no-snap").await;
    put_pending(db, "r-running", "a-running").await;
    put_pending(db, "r-gone", "a-gone").await;

    let retired =
        retire_orphan_pending_reflections(db, 1_760_000_000_000).await.expect("守卫执行应成功");
    assert_eq!(retired, 2, "只有「无依据」与「孤儿」两族该被处置，期望 2 实得 {retired}");

    let snap_text = status_of(db, "r-no-snap").await;
    assert!(
        snap_text.starts_with("failed: ANALYSIS_SNAPSHOT_MISSING:"),
        "无依据那族要落 ANALYSIS_SNAPSHOT_MISSING，实得：{snap_text}"
    );
    assert!(snap_text.contains("a-no-snap"), "终局串必须点名是哪条分析：{snap_text}");

    let orphan_text = status_of(db, "r-gone").await;
    assert!(
        orphan_text.starts_with("failed: ORIGINAL_ANALYSIS_MISSING:"),
        "孤儿必须仍落 ORIGINAL_ANALYSIS_MISSING —— 两码互串会让人以为分析被删（其实重跑可救）。\
         实得：{orphan_text}"
    );

    // 第三格：有依据的不动，还在跑的也不判死。
    assert_eq!(status_of(db, "r-ok").await, "pending", "源分析留有依据 ⇒ 反思照常待跑");
    assert_eq!(
        status_of(db, "r-running").await,
        "pending",
        "源分析仍在 running ⇒ 快照缺席是正常中间态，抢先判死会让它跑完也等不到反思"
    );

    // 幂等：终局行不再是 pending，下一轮不得重复计数。
    let again = retire_orphan_pending_reflections(db, 1_760_000_000_001)
        .await
        .expect("第二轮守卫执行应成功");
    assert_eq!(again, 0, "守卫必须幂等，第二轮实处置 {again} 行");
}
