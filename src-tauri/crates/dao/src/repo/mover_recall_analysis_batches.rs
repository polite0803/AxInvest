// SPDX-License-Identifier: AGPL-3.0-only
//! 漏检补分析批次历史表 `mover_recall_analysis_batches` 的读写收口
//! —— **命令层不得直连的下沉实现**（分层护栏规则 1），形态仿
//! `repo/time_travel_batch_runs.rs`。
//!
//! 逐票明细不存本表 JSON：`mover_recall_events.batch_id` 反查是权威，
//! 见 [`super::mover_recall_events::list_by_batch`]。

/// 实体行类型重导出（命令层反序列化/明细查询走 dao 合规出口）。
pub use axagent_entities::mover_recall_analysis_batches::Model as BatchRunModel;
use axagent_entities::mover_recall_analysis_batches::{self, ActiveModel, Column, Entity};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect,
};

/// 批次历史行的 upsert 载体（命令层组装的纯数据，不泄漏 `ActiveModel`）。
#[derive(Debug, Clone)]
pub struct BatchRunRecord {
    pub id: String,
    pub status: String,
    pub message: Option<String>,
    pub total: i32,
    pub ok: i32,
    pub fail: i32,
    pub request_json: Option<String>,
    pub error_detail: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub trigger: Option<String>,
    /// 阶段 E-4 选样留痕四数（`None` = 选样未跑／这几列引入前的旧行）
    pub scanned: Option<i32>,
    pub skipped_by_share: Option<i32>,
    pub skipped_by_period: Option<i32>,
    pub skipped_by_window: Option<i32>,
}

/// 全量 upsert 一行批次历史。主键冲突时覆盖除 id / 创建时间 / 原始请求外的列
/// （`request_json` 首插后不更新：内存态丢失后从库还原再 upsert 不能让 None 覆盖原请求）。
pub async fn upsert_batch_run(db: &DatabaseConnection, r: &BatchRunRecord) -> Result<(), DbErr> {
    let am = ActiveModel {
        id: Set(r.id.clone()),
        status: Set(r.status.clone()),
        message: Set(r.message.clone()),
        total: Set(r.total),
        ok: Set(r.ok),
        fail: Set(r.fail),
        request_json: Set(r.request_json.clone()),
        error_detail: Set(r.error_detail.clone()),
        created_at_ms: Set(r.created_at_ms),
        updated_at_ms: Set(r.updated_at_ms),
        finished_at_ms: Set(r.finished_at_ms),
        trigger: Set(r.trigger.clone()),
        scanned: Set(r.scanned),
        skipped_by_share: Set(r.skipped_by_share),
        skipped_by_period: Set(r.skipped_by_period),
        skipped_by_window: Set(r.skipped_by_window),
    };

    Entity::insert(am)
        .on_conflict(
            OnConflict::column(Column::Id)
                .update_columns([
                    Column::Status,
                    Column::Message,
                    Column::Total,
                    Column::Ok,
                    Column::Fail,
                    Column::ErrorDetail,
                    Column::UpdatedAtMs,
                    Column::FinishedAtMs,
                    // 选样四数在建批时还是 NULL（选样没跑），必须在后续 upsert 里补写，
                    // 否则「空批到底被什么跳过」永远落不进库。
                    Column::Scanned,
                    Column::SkippedByShare,
                    Column::SkippedByPeriod,
                    Column::SkippedByWindow,
                ])
                .to_owned(),
        )
        .exec(db)
        .await?;
    Ok(())
}

/// 把库内所有 `from_status` 状态的批次收敛为 `to_status`（启动收敛僵尸批次）。
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
) -> Result<Option<mover_recall_analysis_batches::Model>, DbErr> {
    Entity::find_by_id(batch_id.to_owned()).one(db).await
}

/// 按创建时间倒序取批次历史（任务中心列表用）。`status` 为空串时不过滤。
pub async fn list_recent_batch_runs(
    db: &DatabaseConnection,
    limit: u64,
    status: Option<&str>,
) -> Result<Vec<mover_recall_analysis_batches::Model>, DbErr> {
    let mut query = Entity::find().order_by_desc(Column::CreatedAtMs);
    if let Some(status) = status.filter(|s| !s.is_empty()) {
        query = query.filter(Column::Status.eq(status.to_owned()));
    }
    query.limit(limit).all(db).await
}
