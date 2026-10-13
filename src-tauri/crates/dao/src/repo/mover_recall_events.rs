// SPDX-License-Identifier: AGPL-3.0-only
//! 漏检事件表 `mover_recall_events` 的读写收口（补分析批次的数据访问层）。
//!
//! 事件状态机（pending / excluded / running / analyzed / failed / reflected）
//! 的语义归命令层；本模块只提供按状态筛选、认领、回写、批次反查四类原语。

pub use axagent_entities::mover_recall_events::Model as EventModel;
use axagent_entities::mover_recall_events::{self, Column, Entity};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, DatabaseConnection, DbErr, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use std::collections::HashMap;

/// 可进补分析批次的状态（待补 + 上批失败允许跨批重试）。
pub const CANDIDATE_STATUSES: [&str; 2] = ["pending", "failed"];

/// 取补分析候选事件：pending/failed，按累计涨幅降序（先补最妖的），
/// 同分按创建时间升序（先老后新）。`run_id` 为 None 时跨全部轮次选样。
pub async fn list_candidate_events(
    db: &DatabaseConnection,
    run_id: Option<&str>,
    limit: u64,
) -> Result<Vec<mover_recall_events::Model>, DbErr> {
    let mut query = Entity::find()
        .filter(Column::Status.is_in(CANDIDATE_STATUSES))
        .order_by_desc(Column::CumGainPct)
        .order_by_asc(Column::CreatedAtMs);
    if let Some(run_id) = run_id.filter(|s| !s.is_empty()) {
        query = query.filter(Column::RunId.eq(run_id.to_owned()));
    }
    query.limit(limit).all(db).await
}

/// 从各轮核查的 `rates_json` 提取 unexplained 占比：run_id → 占比。
///
/// 占比为 null（NaN，如无漏检）或缺字段时**缺键**；调用方据此判「无信息不拦截」。
/// runs 表为周频小表，直接全量读（避免在 dao 层依赖 analysis-engine 的 DTO，
/// JSON 字段名 `unexplainedShare` 与 `RecallRates` 的 camelCase 序列化对齐）。
pub async fn load_run_unexplained_shares(
    db: &DatabaseConnection,
) -> Result<HashMap<String, f64>, DbErr> {
    let rows = axagent_entities::mover_recall_runs::Entity::find().all(db).await?;
    let mut out = HashMap::new();
    for r in rows {
        // rates_json 为非空 String 列（空率时存 null 字面量的 JSON）
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&r.rates_json)
            && let Some(share) = v.get("unexplainedShare").and_then(|x| x.as_f64())
        {
            out.insert(r.id, share);
        }
    }
    Ok(out)
}

/// 认领事件：pending/failed → running 并打批次标。仅认领仍处于候选态的行
/// （防并发/重入下重复认领），返回受影响行数。
pub async fn claim_events(
    db: &DatabaseConnection,
    event_ids: &[String],
    batch_id: &str,
    now_ms: i64,
) -> Result<u64, DbErr> {
    if event_ids.is_empty() {
        return Ok(0);
    }
    let result = Entity::update_many()
        .col_expr(Column::Status, Expr::value("running"))
        .col_expr(Column::BatchId, Expr::value(batch_id.to_owned()))
        .col_expr(Column::ErrorDetail, Expr::value::<Option<String>>(None))
        .col_expr(Column::UpdatedAtMs, Expr::value(now_ms))
        .filter(Column::Id.is_in(event_ids.to_vec()))
        .filter(Column::Status.is_in(CANDIDATE_STATUSES))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

/// 单只事件终态回写（analyzed 带 analysis_id；failed 带 error_detail）。
#[allow(clippy::too_many_arguments)]
pub async fn finish_event(
    db: &DatabaseConnection,
    event_id: &str,
    status: &str,
    analysis_id: Option<&str>,
    error_detail: Option<&str>,
    now_ms: i64,
) -> Result<(), DbErr> {
    Entity::update_many()
        .col_expr(Column::Status, Expr::value(status.to_owned()))
        .col_expr(Column::AnalysisId, Expr::value(analysis_id.map(str::to_owned)))
        .col_expr(Column::ErrorDetail, Expr::value(error_detail.map(str::to_owned)))
        .col_expr(Column::UpdatedAtMs, Expr::value(now_ms))
        .filter(Column::Id.eq(event_id.to_owned()))
        .exec(db)
        .await?;
    Ok(())
}

/// 批次异常中断后释放其仍在 running 的事件：回 pending，清批次标，
/// 使其可被后续批次重新认领。
pub async fn release_running_events(
    db: &DatabaseConnection,
    batch_id: &str,
    error_detail: &str,
    now_ms: i64,
) -> Result<u64, DbErr> {
    let result = Entity::update_many()
        .col_expr(Column::Status, Expr::value("pending"))
        .col_expr(Column::BatchId, Expr::value::<Option<String>>(None))
        .col_expr(Column::ErrorDetail, Expr::value(error_detail.to_owned()))
        .col_expr(Column::UpdatedAtMs, Expr::value(now_ms))
        .filter(Column::BatchId.eq(batch_id.to_owned()))
        .filter(Column::Status.eq("running"))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

/// 按主键取单个事件（反思工作流按补分析行盖章的 `anchor_event_id` 反查用）。
pub async fn fetch_by_id(
    db: &DatabaseConnection,
    event_id: &str,
) -> Result<Option<mover_recall_events::Model>, DbErr> {
    Entity::find_by_id(event_id.to_owned()).one(db).await
}

/// 三键（stock_code + anchor_date + period）定位事件，同三键跨多轮核查时
/// 取最新一轮（created_at_ms DESC）；查无返回 `Ok(None)`。
pub async fn find_latest_by_key(
    db: &DatabaseConnection,
    stock_code: &str,
    anchor_date: &str,
    period: &str,
) -> Result<Option<mover_recall_events::Model>, DbErr> {
    Entity::find()
        .filter(Column::StockCode.eq(stock_code.to_owned()))
        .filter(Column::AnchorDate.eq(anchor_date.to_owned()))
        .filter(Column::Period.eq(period.to_owned()))
        .order_by_desc(Column::CreatedAtMs)
        .one(db)
        .await
}

/// 漏检事件闭环盖章：置 reflected 并记 reflection_id。带 `status != 'reflected'`
/// 行级守卫 —— 一个事件多档反思时只让首个 completed 的档生效，返回 0 表示
/// 已被更早的档盖章（幂等，不是冲突）。
pub async fn stamp_reflected(
    db: &DatabaseConnection,
    event_id: &str,
    reflection_id: Option<&str>,
    now_ms: i64,
) -> Result<u64, DbErr> {
    let result = Entity::update_many()
        .col_expr(Column::Status, Expr::value("reflected"))
        .col_expr(Column::ReflectionId, Expr::value(reflection_id.map(str::to_owned)))
        .col_expr(Column::UpdatedAtMs, Expr::value(now_ms))
        .filter(Column::Id.eq(event_id.to_owned()))
        .filter(Column::Status.ne("reflected"))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

/// 反查某批次的逐票明细（明细抽屉）：累计涨幅降序。
pub async fn list_by_batch(
    db: &DatabaseConnection,
    batch_id: &str,
) -> Result<Vec<mover_recall_events::Model>, DbErr> {
    Entity::find()
        .filter(Column::BatchId.eq(batch_id.to_owned()))
        .order_by_desc(Column::CumGainPct)
        .all(db)
        .await
}

/// 统计某批次各状态事件数（批次表计数的校验口径，测试用）。
pub async fn count_by_batch_status(
    db: &DatabaseConnection,
    batch_id: &str,
    status: &str,
) -> Result<u64, DbErr> {
    Entity::find()
        .filter(Column::BatchId.eq(batch_id.to_owned()))
        .filter(Column::Status.eq(status.to_owned()))
        .count(db)
        .await
}
