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

/// 列出**至少一个**所查源有快照的快照日期（升序）。
///
/// 为什么要有这个查询：`missing_snapshot_sources` 只能逐日判断，
/// 而「今天能回放哪些日」需要**一次列出全部候选日**（R-2 每日批次用）。
/// 逐日循环在快照积累到几百天后会退化成 N 次查询。
///
/// 返回 `(snapshot_date, 该日命中的源数)`；`methods` 为空时返回空 Vec
/// （空清单 ⇒ 无源可命中，不返回「全部日期都命中」）。
///
/// ## 为什么判据是「**一**源」而不是「全部源」（2026-10-26，R-3h）
///
/// 实测 `astock_daily_snapshot` 的历史分布：
/// `2026-05-27 / 06-10 / 06-11 / 08-18 / 09-04 / 09-07 / 09-09` 这些日期**只有
/// `get_industry_ranking` 一源**（`get_hot_stocks` 的快照最早只到 2026-09-28）。
/// 用「全部源」判 ⇒ 这些日期全部不可回放 ⇒ 只能从 2026-09-28 起回放。
///
/// 但「缺一源」≠「池子空」：
/// ① 缺的那源在 as-of 下**仍走 vendor 历史通道**（如 THS 热榜回溯），取不到返回**空榜**
///    （`Ok(vec![])`，不是失败）；
/// ② 建池 = `get_hot_stocks(30) ∪ get_industry_ranking 龙头(20) ∪ FALLBACK_STOCKS`，
///    **FALLBACK 是静态大盘股列表**（与日期无关 ⇒ **无前视偏差**）⇒ 少一源时池子仍有真实成分。
///
/// ## 为什么这决定 long 档能不能攒到校准样本
///
/// 档位成熟判定只看「行情库里决策日之后有没有 N 根 K 线」，**不看今天**
/// （`compute_market_snapshots` 取最近 500 根，`exit_idx = entry_idx + expected_days`）。
/// 实测行情库 6575 个交易日：**2026-06-10 之后恰好 90 根** ⇒ 回放 06-10 立刻让
/// long 档（T+90）成熟；**09-07 之后恰好 28 根** ⇒ mid 档（T+28）同理。
/// 不放宽 ⇒ long/mid 的校准样本只能等 wall-clock（long 需 2027-01），
/// 而 `CALIBRATION_MIN_SAMPLES = 30` 的门槛又不允许降 ⇒ 长期档**永远校准不了**
/// ⇒ 置信被永久封顶 ⇒ 顶层「买入」永远为 0（R-3h 死循环）。
///
/// ⚠ 零源日（两源都没快照）**必须由调用方排除** —— 那时池子 100% 是
/// FALLBACK_STOCKS，不是「当时的候选池」，回放与系统真实行为不同构。
pub async fn list_snapshot_dates_with_any_method(
    db: &DatabaseConnection,
    methods: &[&str],
) -> Result<Vec<(String, usize)>, sea_orm::DbErr> {
    if methods.is_empty() {
        return Ok(Vec::new());
    }
    let rows = astock_daily_snapshot::Entity::find()
        .filter(astock_daily_snapshot::Column::Method.is_in(methods.to_vec()))
        .all(db)
        .await?;
    // 内存聚合：方法数只有个位数、行数在快照积累期内可控（每天 2~13 行）。
    let mut per_day: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    for r in rows {
        per_day.entry(r.snapshot_date.clone()).or_default().insert(r.method.clone());
    }
    Ok(per_day.into_iter().filter(|(_, ms)| !ms.is_empty()).map(|(d, ms)| (d, ms.len())).collect())
}
