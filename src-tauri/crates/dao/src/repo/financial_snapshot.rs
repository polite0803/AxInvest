//! `financial_snapshots` 的读辅助（写路径在命令层 `backfill_financial_snapshots`）。
//!
//! 目前只有一个消费者：每日「估值归档」扫（`init/services.rs`）用「近 N 天已归档过就跳过」
//! 做幂等判断 —— 否则每天对候选池 190 只全部重拉 2 年历史，会反复删写十万级行。

use axagent_entities::financial_snapshots;
use axagent_harness::DatabaseConnection;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use std::collections::HashSet;

/// `snapshot_date >= since` 的代码集合。
pub async fn codes_with_snapshot_since(
    db: &DatabaseConnection,
    since: &str,
) -> Result<HashSet<String>, sea_orm::DbErr> {
    let rows = financial_snapshots::Entity::find()
        .select_only()
        .column(financial_snapshots::Column::StockCode)
        .filter(financial_snapshots::Column::SnapshotDate.gte(since))
        .into_tuple::<String>()
        .all(db)
        .await?;
    Ok(rows.into_iter().collect())
}
