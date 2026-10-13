// SPDX-License-Identifier: AGPL-3.0-only
//! `mover_recall_runs` 核查轮次快照 + 同事务事件全集写入。
//!
//! **命令层不得直连的下沉实现**（分层护栏规则 1）：`persist_mover_recall_run`
//! 需要把轮次汇总与漏检事件放进同一个事务，事务原语一旦写在命令层即命中门禁，
//! 故连同两个实体的 `ActiveModel` 构造一并收口到这里；命令层只组装纯数据记录。

use axagent_entities::mover_recall_events::{
    ActiveModel as EventActiveModel, Entity as EventEntity,
};
/// 轮次行类型重导出（命令侧分类 `misses_json` 走 dao 合规出口）。
pub use axagent_entities::mover_recall_runs::Model as RunModel;
use axagent_entities::mover_recall_runs::{
    ActiveModel as RunActiveModel, Column as RunColumn, Entity as RunEntity,
};
use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, TransactionTrait};

/// 一轮核查的汇总快照（命令层组装的纯数据，不泄漏 `ActiveModel`）。
#[derive(Debug, Clone)]
pub struct RunSnapshotRecord {
    pub id: String,
    pub date_from: String,
    pub date_to: String,
    pub data_since: String,
    pub collected_days: i32,
    pub universe_size: i64,
    pub universe_confirmed: i64,
    pub rules_json: String,
    pub rates_json: String,
    pub layers_json: String,
    pub events_total: i32,
    pub miss_count: i32,
    pub misses_json: Option<String>,
    pub status: String,
    pub trigger_source: String,
    pub created_at_ms: i64,
}

/// 漏检事件全集的单行（与 [`RunSnapshotRecord`] 同事务落库）。
#[derive(Debug, Clone)]
pub struct RunEventRecord {
    pub id: String,
    pub run_id: String,
    pub stock_code: String,
    pub stock_name: String,
    pub period: String,
    pub anchor_date: String,
    pub window_start: String,
    pub window_end: String,
    pub window_days: i32,
    pub cum_gain_pct: f64,
    pub max_daily_pct: f64,
    pub threshold_pct: f64,
    pub market_type: String,
    pub miss_layer: String,
    /// 1 = 窗口内有过任一档推荐（档位错配），0 = 无
    pub picked_any_period: i32,
    pub scored_out_styles_json: Option<String>,
    /// pending / excluded（批次认领 / 分析 / 反思回写由事件仓库的其它原语负责）
    pub status: String,
    pub created_at_ms: i64,
}

/// 同事务落一轮核查汇总与其漏检事件全集（事件为空时只插汇总行）。
pub async fn insert_run_snapshot(
    db: &DatabaseConnection,
    run: &RunSnapshotRecord,
    events: &[RunEventRecord],
) -> Result<(), DbErr> {
    let run_row = RunActiveModel {
        id: Set(run.id.clone()),
        date_from: Set(run.date_from.clone()),
        date_to: Set(run.date_to.clone()),
        data_since: Set(run.data_since.clone()),
        collected_days: Set(run.collected_days),
        universe_size: Set(run.universe_size),
        universe_confirmed: Set(run.universe_confirmed),
        rules_json: Set(run.rules_json.clone()),
        rates_json: Set(run.rates_json.clone()),
        layers_json: Set(run.layers_json.clone()),
        events_total: Set(run.events_total),
        miss_count: Set(run.miss_count),
        misses_json: Set(run.misses_json.clone()),
        status: Set(run.status.clone()),
        trigger_source: Set(run.trigger_source.clone()),
        created_at_ms: Set(run.created_at_ms),
    };

    let event_rows: Vec<EventActiveModel> = events
        .iter()
        .map(|e| EventActiveModel {
            id: Set(e.id.clone()),
            run_id: Set(e.run_id.clone()),
            stock_code: Set(e.stock_code.clone()),
            stock_name: Set(e.stock_name.clone()),
            period: Set(e.period.clone()),
            anchor_date: Set(e.anchor_date.clone()),
            window_start: Set(e.window_start.clone()),
            window_end: Set(e.window_end.clone()),
            window_days: Set(e.window_days),
            cum_gain_pct: Set(e.cum_gain_pct),
            max_daily_pct: Set(e.max_daily_pct),
            threshold_pct: Set(e.threshold_pct),
            market_type: Set(e.market_type.clone()),
            miss_layer: Set(e.miss_layer.clone()),
            picked_any_period: Set(e.picked_any_period),
            scored_out_styles_json: Set(e.scored_out_styles_json.clone()),
            batch_id: Set(None),
            analysis_id: Set(None),
            reflection_id: Set(None),
            status: Set(e.status.clone()),
            error_detail: Set(None),
            created_at_ms: Set(e.created_at_ms),
            updated_at_ms: Set(e.created_at_ms),
        })
        .collect();

    db.transaction::<_, _, DbErr>(|txn| {
        Box::pin(async move {
            RunEntity::insert(run_row).exec(txn).await?;
            if !event_rows.is_empty() {
                EventEntity::insert_many(event_rows).exec(txn).await?;
            }
            Ok(())
        })
    })
    .await
    // sea-orm 2.0 事务结果包一层 TransactionError，两种变体的载荷都是 DbErr。
    .map_err(|e| match e {
        sea_orm::TransactionError::Connection(db_err)
        | sea_orm::TransactionError::Transaction(db_err) => db_err,
    })?;
    Ok(())
}

/// 取某时间戳之后落库的全部核查轮次（进化计划聚合读 `misses_json` 用）。
pub async fn list_runs_since(
    db: &DatabaseConnection,
    since_ms: i64,
) -> Result<Vec<RunModel>, DbErr> {
    RunEntity::find().filter(RunColumn::CreatedAtMs.gte(since_ms)).all(db).await
}
