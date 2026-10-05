// SPDX-License-Identifier: AGPL-3.0-only
//! 反思教训注入的查询组（3 取 + 2 个「被筛掉多少」计数）+ 主档代理 —— **命令层不得直连的下沉实现**。
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
//!
//! ## 代过滤语义（§七十一 按版归属 + §五十一-② 起算代际，调用方契约）
//!
//! **规则是下限**：`template_version >= generation_floor` 的行 + `template_version IS NULL`
//! 的行才进 prompt。用下限而不是等号，理由见
//! `axagent_harness::holding_period::HORIZON_BRANCH_GENERATION_FLOOR` 的注释 ——
//! 等号会让每次换代时全部教训整批消失（本仓 v125→v132 只用了两周）。
//!
//! NULL 在此**保留**（代际未知：本列引入前的存量行，或对话直执行通道 —— 那条链不经过
//! `workflow_templates`，NULL 是 A4 裁定的正常形态）：一条来路不明的**文本教训**对模型
//! 仍有参考价值，只要声明它代际未知即可。
//! ⚠ 与统计侧（`reflection_stats`）**刻意不同** —— 那边 NULL 被排除，因为分母不能掺未知代。
//! 同一个 NULL 两处处置不同是有意的：prompt 容得下带标注的未知，分母容不下。
//!
//! 与档过滤不同的是：被筛掉的**旧代**条数要还给调用方（`count_pre_floor_generation_*`），
//! 由注入文本显式声明「另有 N 条早于起算代，未注入」。档可以默认「跨档本就不该要」，
//! 代际不行 —— 一个空结果既可能是「真没教训」也可能是「教训全在起算代之前」，
//! 两者对模型的含义相反，静默合并就是伪造（本仓「结构性缺口不得造成歧义」口径）。

use axagent_entities::{reflection_lessons, stock_analyses, stock_reflections};
// `count(db)` 由 `PaginatorTrait` 提供（sea-orm 2.x 把它从 QuerySelect 挪走了）⇒ 少这一个
// import 就退化成 `Iterator::count`，编译器报的是「not an iterator」，看着完全不像缺 import。
use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder,
};

/// 「同档 OR 复盘档未知(NULL)」组合子（〇-B v2 第 4 条）。
///
/// `fetch_*` 三函数内的同源逻辑；`reflection.rs` 的 pending 扫描因查询主体各异
/// 无法共用整查询函数，但档过滤子句同源 ⇒ 提成泛型 helper，避免各写一遍
/// `Condition::any()`（命令层写它即分层门禁规则 1 命中）。
pub fn horizon_eq_or_null<C>(col: C, horizon: &str) -> Condition
where
    C: ColumnTrait,
{
    Condition::any().add(col.eq(horizon)).add(col.is_null())
}

/// 「**起算代之后** OR 代际未知(NULL)」组合子（§五十一-②，与 `horizon_eq_or_null` 同族）。
///
/// 不复用成一个通用 `eq_or_null`：两条维度的**比较符不同**（档是等值，代是下限），
/// 合成一个函数最容易被后来者想当然写成等值 —— 那会让每次换代教训整批消失。
/// `gte` 是唯一实现，改动它请连同 `HORIZON_BRANCH_GENERATION_FLOOR` 的注释一起读。
pub fn generation_floor_or_null<C>(col: C, generation_floor: i32) -> Condition
where
    C: ColumnTrait,
{
    Condition::any().add(col.gte(generation_floor)).add(col.is_null())
}

/// 同 ticker 近期已完成反思（按 `CreatedAt` 倒序）。
///
/// 调用方（`fetch_stock_lessons` 的 same_ticker 段）传「90 天前毫秒时间戳」，
/// 取回后自行 `take(3)` —— 截断数是呈现策略，留给调用方。
pub async fn fetch_same_ticker_completed(
    db: &DatabaseConnection,
    stock_code: &str,
    since_ms: i64,
    horizon: Option<&str>,
    // 当前分析所属代际（`None` = 不过滤，语义见模块头「代过滤语义」）
    generation_floor: i32,
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
    // §七十一 按版归属：代际同档同理 —— 异代的反思不进 prompt，代际未知的（NULL）保留。
    let q = q.filter(generation_floor_or_null(
        stock_reflections::Column::TemplateVersion,
        generation_floor,
    ));
    q.order_by_desc(stock_reflections::Column::CreatedAt).all(db).await
}

/// 全市场近期已完成反思（按 `CreatedAt` 倒序，调用方自行排除本股并 `take(2)`）。
pub async fn fetch_recent_completed(
    db: &DatabaseConnection,
    since_ms: i64,
    horizon: Option<&str>,
    // 当前分析所属代际（`None` = 不过滤，语义见模块头「代过滤语义」）
    generation_floor: i32,
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
    // §七十一 按版归属：代际同档同理 —— 异代的反思不进 prompt，代际未知的（NULL）保留。
    let q = q.filter(generation_floor_or_null(
        stock_reflections::Column::TemplateVersion,
        generation_floor,
    ));
    q.order_by_desc(stock_reflections::Column::CreatedAt).all(db).await
}

/// 规则化教训（按 `Confidence` 倒序，调用方自行 `take(5)`）。
pub async fn fetch_rule_lessons(
    db: &DatabaseConnection,
    stock_code: &str,
    min_confidence: f64,
    horizon: Option<&str>,
    // 当前分析所属代际（§七十一；`None` = 不过滤）
    generation_floor: i32,
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
    let q = q.filter(generation_floor_or_null(
        reflection_lessons::Column::TemplateVersion,
        generation_floor,
    ));
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

/// 被**起算代**筛掉的已完成反思条数（§五十一-② 的声明侧）。
///
/// 只数「早于起算代」的行（`template_version IS NOT NULL AND < floor`）—— NULL 行本就随
/// 一起注入，把它们也计进「未注入」会把实话写成假话。
///
/// `stock_code = Some(c)` 对应 same_ticker 组，`None` 对应跨股组（调用方另有「排除本股」
/// 的呈现层过滤，不在这里 —— 与 `fetch_*` 的分工一致）。
/// 的呈现层过滤，不在这里 —— 与 `fetch_*` 的分工一致）。
pub async fn count_pre_floor_generation_reflections(
    db: &DatabaseConnection,
    stock_code: Option<&str>,
    since_ms: i64,
    horizon: Option<&str>,
    generation_floor: i32,
) -> Result<u64, sea_orm::DbErr> {
    let q = stock_reflections::Entity::find()
        .filter(stock_reflections::Column::Status.eq("completed"))
        .filter(stock_reflections::Column::CreatedAt.gte(since_ms));
    let q = match stock_code {
        Some(c) => q.filter(stock_reflections::Column::StockCode.eq(c)),
        None => q,
    };
    let q = match horizon {
        Some(h) => q.filter(
            Condition::any()
                .add(stock_reflections::Column::Horizon.eq(h))
                .add(stock_reflections::Column::Horizon.is_null()),
        ),
        None => q,
    };
    let foreign = Condition::all()
        .add(stock_reflections::Column::TemplateVersion.is_not_null())
        .add(stock_reflections::Column::TemplateVersion.lt(generation_floor));
    q.filter(foreign).count(db).await
}

/// 被**起算代**筛掉的规则化教训条数（语义同 `count_pre_floor_generation_reflections`）。
pub async fn count_pre_floor_generation_lessons(
    db: &DatabaseConnection,
    stock_code: &str,
    min_confidence: f64,
    horizon: Option<&str>,
    generation_floor: i32,
) -> Result<u64, sea_orm::DbErr> {
    let q = reflection_lessons::Entity::find()
        .filter(reflection_lessons::Column::StockCode.eq(stock_code))
        .filter(reflection_lessons::Column::Confidence.gte(min_confidence));
    let q = match horizon {
        Some(h) => q.filter(
            Condition::any()
                .add(reflection_lessons::Column::Horizon.eq(h))
                .add(reflection_lessons::Column::Horizon.is_null()),
        ),
        None => q,
    };
    let foreign = Condition::all()
        .add(reflection_lessons::Column::TemplateVersion.is_not_null())
        .add(reflection_lessons::Column::TemplateVersion.lt(generation_floor));
    q.filter(foreign).count(db).await
}
