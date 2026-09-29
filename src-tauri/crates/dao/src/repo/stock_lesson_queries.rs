// SPDX-License-Identifier: AGPL-3.0-only
//! 反思教训注入的三组查询 + 主档代理 —— **命令层不得直连的下沉实现**。
//!
//! ## 为什么在这里
//!
//! `commands/stock_workflow/core.rs::fetch_stock_lessons` 与
//! `commands/stock_workflow/hooks.rs::latest_known_horizon` 在组装反思注入上下文时
//! 需要按「周期档（horizon）」过滤 `stock_reflections` / `reflection_lessons` /
//! `stock_analyses`。`sea_orm::Condition::any()` 这类查询组合子一旦写在命令层，
//! 即命中分层门禁 `scripts/check-layer-discipline.mjs` 规则 1
//! （`commands-no-direct-db`：命令层不得出现 `sea_orm::` / `axagent_entities::`
//! 字面量，须经 dao / service）。
//!
//! 把查询搬到这里是**真下沉**（先例见同目录 `stock_analysis_snapshot.rs`）：
//! 实体、列、查询组合子不再出现在命令层，命令层只拿到 `Model` / `Option<String>`。
//!
//! ## 档过滤语义（调用方契约，勿改）
//!
//! `horizon = Some(h)` ⇒ 取「复盘 h 档产出的行」+ `horizon IS NULL` 的行
//! （NULL = 引入该列之前的旧行，复盘档未知，按既有行为继续注入）；
//! `horizon = None` ⇒ 不过滤（兼容旧调用方）。
//!
//! 跨档的行**不进**结果 —— 这是 〇-B v2 第 4 条「同档优先 + 通用规则，跨档不进 prompt」。

use axagent_entities::{reflection_lessons, stock_analyses, stock_reflections};
use sea_orm::{ColumnTrait, Condition, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};

/// 同 ticker 近期已完成反思（按 `CreatedAt` 倒序）。
///
/// 调用方（`fetch_stock_lessons` 的 same_ticker 段）传「90 天前毫秒时间戳」，
/// 取回后自行 `take(3)` —— 截断数是呈现策略，留给调用方。
pub async fn fetch_same_ticker_completed(
    db: &DatabaseConnection,
    stock_code: &str,
    since_ms: i64,
    horizon: Option<&str>,
) -> Result<Vec<stock_reflections::Model>, sea_orm::DbErr> {
    let q = stock_reflections::Entity::find()
        .filter(stock_reflections::Column::StockCode.eq(stock_code))
        .filter(stock_reflections::Column::Status.eq("completed")) // 只注入已 resolve 的教训
        .filter(stock_reflections::Column::CreatedAt.gte(since_ms));
    let q = match horizon {
        Some(h) => q.filter(
            Condition::any()
                .add(stock_reflections::Column::Horizon.eq(h))
                .add(stock_reflections::Column::Horizon.is_null()),
        ),
        None => q,
    };
    q.order_by_desc(stock_reflections::Column::CreatedAt).all(db).await
}

/// 全市场近期已完成反思（按 `CreatedAt` 倒序，调用方自行排除本股并 `take(2)`）。
pub async fn fetch_recent_completed(
    db: &DatabaseConnection,
    since_ms: i64,
    horizon: Option<&str>,
) -> Result<Vec<stock_reflections::Model>, sea_orm::DbErr> {
    let q = stock_reflections::Entity::find()
        .filter(stock_reflections::Column::CreatedAt.gte(since_ms))
        .filter(stock_reflections::Column::Status.eq("completed")); // 只看已 resolve 的
    let q = match horizon {
        Some(h) => q.filter(
            Condition::any()
                .add(stock_reflections::Column::Horizon.eq(h))
                .add(stock_reflections::Column::Horizon.is_null()),
        ),
        None => q,
    };
    q.order_by_desc(stock_reflections::Column::CreatedAt).all(db).await
}

/// 规则化教训（按 `Confidence` 倒序，调用方自行 `take(5)`）。
pub async fn fetch_rule_lessons(
    db: &DatabaseConnection,
    stock_code: &str,
    min_confidence: f64,
    horizon: Option<&str>,
) -> Result<Vec<reflection_lessons::Model>, sea_orm::DbErr> {
    let q = reflection_lessons::Entity::find()
        .filter(reflection_lessons::Column::StockCode.eq(stock_code))
        .filter(reflection_lessons::Column::Confidence.gte(min_confidence)); // 过滤低质量/已废弃规则
    let q = match horizon {
        Some(h) => q.filter(
            Condition::any()
                .add(reflection_lessons::Column::Horizon.eq(h))
                .add(reflection_lessons::Column::Horizon.is_null()),
        ),
        None => q,
    };
    q.order_by_desc(reflection_lessons::Column::Confidence).all(db).await
}

/// 该股最近一条带主档（`decision_time_horizon` 非空）的分析的主档。
///
/// 返回 `Ok(None)` 仅表示「没有符合条件的行」；查询失败走 `Err` ——
// 调用方（`latest_known_horizon`）把两种情形都按「不按档过滤」处理并 WARN。
pub async fn latest_analysis_horizon(
    db: &DatabaseConnection,
    stock_code: &str,
) -> Result<Option<String>, sea_orm::DbErr> {
    let row = stock_analyses::Entity::find()
        .filter(stock_analyses::Column::StockCode.eq(stock_code))
        .filter(stock_analyses::Column::DecisionTimeHorizon.is_not_null())
        .order_by_desc(stock_analyses::Column::CreatedAt)
        .one(db)
        .await?;
    Ok(row.and_then(|a| a.decision_time_horizon))
}
