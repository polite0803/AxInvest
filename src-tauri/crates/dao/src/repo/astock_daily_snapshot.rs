// SPDX-License-Identifier: AGPL-3.0-only
//! `astock_daily_snapshot`（vendor 每日快照）的读侧小查询 —— 命令层不得直连的下沉实现。
//!
//! 用途只有一个：**时间旅行批次在进 guard 之前，判断所选日期到底能不能建出候选池**。
//! 热股榜与行业榜都申报 `NoHistoricalSemantic`，它们在回放里的唯一历史正门就是这张表
//! （由 live 每日采集累积而来，早于累积起点的日期物理上不可回补）。缺快照时建池得到空池，
//! 整批候选为 0 —— 如果不在这里显式拦下来，批次只会留下一句「均无候选（可能当日数据缺失
//! 或阈值过高）」，把「这个日期根本不可回放」和「阈值太高所以没选出来」挤进同一句话。
//!
//! 要查哪几个源由**调用方传入**（权威是 `recommender::pool::POOL_SNAPSHOT_METHODS`，
//! 即建池真实取的那两个 vendor 方法），本模块不抄一份清单，免得两处各说各话。

use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};

use axagent_entities::astock_daily_snapshot;
use axagent_harness::DatabaseConnection;

/// 该日期**缺失**的快照源（返回空 = 所查各源齐备，该日可回放）。
pub async fn missing_snapshot_sources(
    db: &DatabaseConnection,
    date: &str,
    methods: &[&str],
) -> Result<Vec<String>, sea_orm::DbErr> {
    let mut missing = Vec::new();
    for method in methods {
        let n = astock_daily_snapshot::Entity::find()
            .filter(astock_daily_snapshot::Column::Method.eq(*method))
            .filter(astock_daily_snapshot::Column::SnapshotDate.eq(date))
            .count(db)
            .await?;
        if n == 0 {
            missing.push((*method).to_string());
        }
    }
    Ok(missing)
}

/// 某源最早的快照日期；库里一行都没有时返回 `None`（= 采集尚未跑过，无日期可回放）。
pub async fn earliest_snapshot_date(
    db: &DatabaseConnection,
    method: &str,
) -> Result<Option<String>, sea_orm::DbErr> {
    Ok(astock_daily_snapshot::Entity::find()
        .filter(astock_daily_snapshot::Column::Method.eq(method))
        .order_by_asc(astock_daily_snapshot::Column::SnapshotDate)
        .limit(1)
        .one(db)
        .await?
        .map(|r| r.snapshot_date))
}

/// 拉取指定源的快照行 `(snapshot_date, method, payload)` —— 供调用方按
/// **快照内容**判定「该日能否建出候选池」（判据权威在
/// `recommender::pool::snapshot_payload_yields_stocks`，dao 不理解 vendor 业务语义）。
///
/// 为什么把 payload 原样带回而不是在 SQL 里判：jsonb 谓词会把 vendor 的 payload
/// 结构焊死进 dao 的 SQL（换 vendor / 改字段名就得改这里）；行数 = 天数 × 源数，
/// 快照积累期内存量可控。
pub async fn list_snapshot_rows_with_payload(
    db: &DatabaseConnection,
    methods: &[&str],
) -> Result<Vec<(String, String, String)>, sea_orm::DbErr> {
    if methods.is_empty() {
        return Ok(Vec::new());
    }
    let rows = astock_daily_snapshot::Entity::find()
        .filter(astock_daily_snapshot::Column::Method.is_in(methods.to_vec()))
        .all(db)
        .await?;
    Ok(rows.into_iter().map(|r| (r.snapshot_date, r.method, r.payload)).collect())
}
