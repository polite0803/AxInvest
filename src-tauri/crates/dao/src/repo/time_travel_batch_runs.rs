// SPDX-License-Identifier: AGPL-3.0-only
//! 时间旅行批次复盘历史表 `time_travel_batch_runs` 的读写收口
//! —— **命令层不得直连的下沉实现**（分层护栏规则 1）。
//!
//! ## 为什么在这里
//!
//! `commands/time_travel_batch.rs` 负责批次流水线编排与运行态内存
//! （`BATCH_RUNS`），历史表的 upsert / 查询 / 僵尸收敛属于数据访问：
//! 实体（`axagent_entities::time_travel_batch_runs`）、sea-orm 查询组合子
//! 只能出现在本模块；命令层拿到的是 [`BatchRunRecord`] 纯数据载体与
//! 实体 `Model`（反序列化用，先例同目录 `stock_lesson_queries.rs`）。
//!
//! 状态字符串（running / interrupted 等）的语义归命令层：收敛函数以
//! `from_status` / `to_status` 参数化，repo 不内置业务状态常量。

/// 实体行类型重导出：命令层 `state_from_model` 的入参类型走 `axagent_dao::`
/// 合规出口，不得在命令层出现 `axagent_entities::` 路径。
pub use axagent_entities::time_travel_batch_runs::Model as BatchRunModel;
use axagent_entities::time_travel_batch_runs::{self, ActiveModel, Column, Entity};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect,
};

/// 批次历史行的 upsert 载体（命令层组装的纯数据，不泄漏 `ActiveModel`）。
///
/// 字段与 `time_travel_batch_runs` 表一一对应；计数字段在命令层是 `usize`，
/// 落库前收窄为 `i32`。
#[derive(Debug, Clone)]
pub struct BatchRunRecord {
    pub id: String,
    pub as_date: String,
    pub trading_days_ago: Option<i32>,
    pub status: String,
    pub phase: String,
    pub message: Option<String>,
    pub total: i32,
    pub ok: i32,
    pub fail: i32,
    /// 智能荐股链候选行数（合并去重前）
    pub reco_picks: i32,
    /// 趋势智选链候选行数（合并去重前）
    pub serenity_picks: i32,
    /// 趋势智选链失败原因（best-effort 降级）
    pub serenity_error: Option<String>,
    /// 发起请求参数快照 JSON
    pub request_json: Option<String>,
    /// 逐只分析结果 JSON
    pub analyses_json: Option<String>,
    /// 四档汇总 JSON
    pub horizons_json: Option<String>,
    /// 批次反思摘要 JSON
    pub reflection_summary: Option<String>,
    pub error_code: Option<String>,
    pub error_detail: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub finished_at_ms: Option<i64>,
}

/// 全量 upsert 一行批次历史（与命令层 `save_state` 同阶：创建 / 分析完成 /
/// 反思阶段切换 / 反思汇总 / 终态，不做逐只分析的高频写）。
///
/// 主键冲突（同批次后续状态更新）时覆盖除 id / 创建时间 / 原始请求外的列。
/// `request_json` 首插后永不更新：`execute_batch` 中途若从库还原（request 为
/// `#[serde(skip)]` 的 None）再 upsert，不能让 None 覆盖原始请求。
pub async fn upsert_batch_run(db: &DatabaseConnection, r: &BatchRunRecord) -> Result<(), DbErr> {
    let am = ActiveModel {
        id: Set(r.id.clone()),
        as_date: Set(r.as_date.clone()),
        trading_days_ago: Set(r.trading_days_ago),
        status: Set(r.status.clone()),
        phase: Set(r.phase.clone()),
        message: Set(r.message.clone()),
        total: Set(r.total),
        ok: Set(r.ok),
        fail: Set(r.fail),
        reco_picks: Set(r.reco_picks),
        serenity_picks: Set(r.serenity_picks),
        serenity_error: Set(r.serenity_error.clone()),
        request_json: Set(r.request_json.clone()),
        analyses_json: Set(r.analyses_json.clone()),
        horizons_json: Set(r.horizons_json.clone()),
        reflection_summary: Set(r.reflection_summary.clone()),
        error_code: Set(r.error_code.clone()),
        error_detail: Set(r.error_detail.clone()),
        created_at_ms: Set(r.created_at_ms),
        updated_at_ms: Set(r.updated_at_ms),
        finished_at_ms: Set(r.finished_at_ms),
    };

    Entity::insert(am)
        .on_conflict(
            OnConflict::column(Column::Id)
                .update_columns([
                    Column::AsDate,
                    Column::TradingDaysAgo,
                    Column::Status,
                    Column::Phase,
                    Column::Message,
                    Column::Total,
                    Column::Ok,
                    Column::Fail,
                    Column::RecoPicks,
                    Column::SerenityPicks,
                    Column::SerenityError,
                    Column::AnalysesJson,
                    Column::HorizonsJson,
                    Column::ReflectionSummary,
                    Column::ErrorCode,
                    Column::ErrorDetail,
                    Column::UpdatedAtMs,
                    Column::FinishedAtMs,
                ])
                .to_owned(),
        )
        .exec(db)
        .await?;
    Ok(())
}

/// 把库内所有 `from_status` 状态的批次收敛为 `to_status`，返回受影响行数。
///
/// 进程启动时用它收敛异常退出遗留的 running 批次（as_of 封闭世界栈是进程级的，
/// 不支持跨进程续跑）。状态语义由命令层以参数传入。
pub async fn mark_stale_batches(
    db: &DatabaseConnection,
    now_ms: i64,
    from_status: &str,
    to_status: &str,
    message: &str,
) -> Result<u64, DbErr> {
    let result = Entity::update_many()
        .col_expr(Column::Status, Expr::value(to_status.to_owned()))
        .col_expr(Column::Message, Expr::value(message.to_owned()))
        .col_expr(Column::UpdatedAtMs, Expr::value(now_ms))
        .col_expr(Column::FinishedAtMs, Expr::value(now_ms))
        .filter(Column::Status.eq(from_status.to_owned()))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

/// 按主键取单个批次历史行。
pub async fn fetch_batch_by_id(
    db: &DatabaseConnection,
    batch_id: &str,
) -> Result<Option<time_travel_batch_runs::Model>, DbErr> {
    Entity::find_by_id(batch_id.to_owned()).one(db).await
}

/// 按创建时间倒序取批次历史（任务中心列表用）。
///
/// `status` / `as_of` 为 None 或空串时不加该过滤；`limit` 由命令层钳制。
pub async fn list_recent_batch_runs(
    db: &DatabaseConnection,
    limit: u64,
    status: Option<&str>,
    as_of: Option<&str>,
) -> Result<Vec<time_travel_batch_runs::Model>, DbErr> {
    let mut query = Entity::find().order_by_desc(Column::CreatedAtMs);
    if let Some(status) = status.filter(|s| !s.is_empty()) {
        query = query.filter(Column::Status.eq(status.to_owned()));
    }
    if let Some(as_of) = as_of.filter(|s| !s.is_empty()) {
        query = query.filter(Column::AsDate.eq(as_of.to_owned()));
    }
    query.limit(limit).all(db).await
}
