// SPDX-License-Identifier: AGPL-3.0-only
//! 候选池落库的**共享入口** —— 手动刷新命令、00:00 荐股 cron、时间旅行批次三条入口共用。
//!
//! ## 为什么单独成一个模块
//!
//! 三条入口原先有两种坏形态：命令层的 `persist_reco_picks` 被 cron 与批次跨模块调用
//! （命中分层门禁规则 2 `commands-no-sibling-call`），而函数本体在命令层直接构造
//! `ActiveModel` 写库（命中规则 1 `commands-no-direct-db`）。
//! 现在拆成两层：SQL 与实体构造在 dao（`repo::reco_pool`），这里只做
//! `RecoPick → 写入行` 的映射与代际透传 ⇒ 命令层不再出现 `sea_orm::` /
//! `axagent_entities::` 字面量。
//! 本模块自身属于「被多个命令模块共用的基础设施」形态（同 `_shared_state`），
//! 已按门禁要求登记进 `check-layer-discipline.mjs` 的 `SHARED_INFRA_MODULES`。
//!
//! ## 代际（`as_of_date`）
//!
//! `None` = 实盘批次，`Some(YYYY-MM-DD)` = 该次回放的截止日。2026-10-12 裁定：回放候选
//! 同样落库（不落就没有样本可供反思），实盘与回放靠这一列分代；只读实盘的下游必须显式
//! 带 `as_of_date IS NULL`，漏加由 `scripts/check-reco-generation.mjs` 拦。

use axagent_analysis_engine::recommender::{self, Period, RecoPick, TrimmedPick};
use axagent_dao::repo::reco_pool::{
    self as pool_dao, BatchKey, NewPoolBatch, NewRecoPick, NewScanAuditTrim,
};
use axagent_harness::DatabaseConnection;

/// 落 `reco_picks`，返回写入行数。
///
/// - `seed_pool_json`：本次扫描**实际使用**的候选池快照（回测负向样本的来源）
/// - `strategy_weights_json`：当时的策略权重快照（回溯某次荐股用的权重配置）
/// - `generated_at`：批次标识，同批各档共用一个值，供缓存按批取回整批
pub async fn persist_reco_picks(
    db: &DatabaseConnection,
    picks: &[RecoPick],
    seed_pool_json: Option<String>,
    strategy_weights_json: Option<String>,
    generated_at: &str,
    as_of_date: Option<&str>,
    purpose: &str,
) -> usize {
    let rows: Vec<NewRecoPick> = picks
        .iter()
        .map(|p| NewRecoPick {
            period: p.period.as_str().to_string(),
            stock_code: p.stock_code.clone(),
            stock_name: p.stock_name.clone(),
            style: p.style.as_str().to_string(),
            // `reco_picks.confidence` 是 i32 列，引擎侧是 0-100 的整数评分
            confidence: p.confidence as i32,
            synthetic: p.synthetic,
            // 完整 pick 的 JSON：缓存回读靠它还原成与实时拉取同 schema 的结果
            pick_data: serde_json::to_string(p).ok(),
        })
        .collect();
    pool_dao::insert_picks(
        db,
        &rows,
        seed_pool_json,
        strategy_weights_json,
        &BatchKey { generated_at, as_of_date, purpose },
        // 算法代际随样本一起落库，否则闭环 IC 会把跨代样本混成一格
        Some(recommender::RECO_ALGORITHM_VERSION),
    )
    .await
}

/// 落一个候选池批次快照（每批一行，**0 票也写**）。
///
/// 漏检归因需要「这批扫过、池子是什么」；只在出票行上带 `seed_pool_json` 时，
/// 0 票批次在库里完全不可见 ⇒ `in_pool` 只能判「不知道」，还会连带整轮被
/// `unexplained_share` 过滤出补分析样本。写失败只 warn（不阻塞荐股主链）。
pub async fn persist_reco_pool_batch(
    db: &DatabaseConnection,
    chain: &str,
    period: &str,
    seed_pool_json: Option<&str>,
    generated_at: &str,
    as_of_date: Option<&str>,
    purpose: &str,
) -> bool {
    let stock_count = seed_pool_json
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .map(|v| match v {
            serde_json::Value::Array(a) => a.len(),
            serde_json::Value::Object(_) => 1,
            _ => 0,
        })
        .unwrap_or(0);
    let degraded = match seed_pool_json {
        None => true,
        Some(raw) => {
            let trimmed = raw.trim();
            trimmed.is_empty() || trimmed == "[]"
        },
    };
    let batch = NewPoolBatch {
        id: format!("rpb-{}", uuid::Uuid::new_v4()),
        chain: chain.to_string(),
        period: period.to_string(),
        generated_at: generated_at.to_string(),
        as_of_date: as_of_date.map(str::to_string),
        purpose: purpose.to_string(),
        seed_pool_json: seed_pool_json.map(str::to_string),
        stock_count: stock_count as i32,
        degraded,
        created_at_ms: chrono::Utc::now().timestamp_millis(),
    };
    match pool_dao::insert_pool_batch(db, &batch).await {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("[reco] 候选池批次快照落库失败（不影响荐股）: {e}");
            false
        },
    }
}

/// 落 L3 截断留痕（`reco_scan_audit`）。
///
/// 语义边界见实体文档：**有行** = 该风格算出了该票、只是组内 top-N 排不上；
/// **无行** ≠ 该风格把它选出去了。与同批 picks 共用 `generated_at` 与 `as_of_date`。
pub async fn persist_reco_scan_audit(
    db: &DatabaseConnection,
    trimmed: &[TrimmedPick],
    period: Period,
    generated_at: &str,
    as_of_date: Option<&str>,
    purpose: &str,
) -> usize {
    let rows: Vec<NewScanAuditTrim> = trimmed
        .iter()
        .map(|t| NewScanAuditTrim {
            stock_code: t.stock_code.clone(),
            stock_name: t.stock_name.clone(),
            style: t.style.as_str().to_string(),
            confidence: t.confidence,
            rank: t.rank,
        })
        .collect();
    pool_dao::insert_scan_audit(
        db,
        &rows,
        period.as_str(),
        &BatchKey { generated_at, as_of_date, purpose },
    )
    .await
}

/// 用途值域的唯一权威（dao 侧定义，命令层转出给引擎/批次复用）。
pub use axagent_dao::repo::reco_pool::purpose;
