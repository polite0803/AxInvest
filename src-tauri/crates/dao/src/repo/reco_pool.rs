// SPDX-License-Identifier: AGPL-3.0-only
//! 候选池两张表（`reco_picks` / `reco_scan_audit`）的写入 —— **持久化边界的唯一实现**。
//!
//! ## 为什么要从命令层搬下来
//!
//! `commands/stock_analysis.rs::persist_reco_picks` 与 `persist_reco_scan_audit` 原本在
//! 命令层直接构造 `reco_picks::ActiveModel` 并 `insert` ⇒ 命令层出现 `sea_orm::` /
//! `axagent_entities::` 字面量，命中分层门禁
//! [`scripts/check-layer-discipline.mjs`] 规则 1（`commands-no-direct-db`）。
//! 而时间旅行批次链 A 也要落同一张表，从命令层调过去又命中规则 2
//! （`commands-no-sibling-call`）。两处都收敛到 dao 之后，三条入口（命令层手动刷新、
//! 00:00 荐股 cron、时间旅行批次）写的是**同一个实现**，字段不会各写各的。
//!
//! 先例见同目录 `stock_analysis_snapshot.rs` 与 `stock_lesson_queries.rs`（同样是
//! 为这道门做的真下沉）。
//!
//! ## 入参为什么不是 `RecoPick`
//!
//! `RecoPick` 定义在 `axagent-analysis-engine`，而该 crate 依赖本 crate —— dao 反向
//! 收它会成环。故这里定义写入用的纯数据结构，由主 crate 侧做一次性映射；
//! `reco_version`（荐股算法代际）同理由调用方传入，dao 不引用那个常量。
//!
//! ## 代际（`as_of_date`）契约
//!
//! `None` = 实盘批次；`Some(YYYY-MM-DD)` = 某次时间旅行回放的截止日。
//! 2026-10-12 裁定：回放的候选同样落库（不落就没有样本可供反思），实盘与回放靠这一列
//! 分代 ⇒ 只读实盘的下游必须显式带 `as_of_date IS NULL`。
//! 读侧漏加谓词由 `scripts/check-reco-generation.mjs` 拦。

use sea_orm::{ActiveModelTrait, Set};

use axagent_entities::{reco_picks, reco_scan_audit};
use axagent_harness::DatabaseConnection;

/// 一条待落库的候选出票（`reco_picks` 的一行）。
pub struct NewRecoPick {
    pub period: String,
    pub stock_code: String,
    pub stock_name: String,
    pub style: String,
    pub confidence: i32,
    pub synthetic: bool,
    /// 完整 pick 的 JSON —— `get_cached_recommendation` 靠它还原缓存，
    /// 必须与实时拉取结果 schema 等价，故由调用方序列化后整体传入。
    pub pick_data: Option<String>,
}

/// 落 `reco_picks`，返回写入行数。
///
/// 写失败**不静默**（逐行 warn 带代码/档位/风格）：此前 `let _ = insert(..)` 失败无感知，
/// 前端表现为"无缓存"、定时链路表现为"候选池为空"，两头都查不到根因。
pub async fn insert_picks(
    db: &DatabaseConnection,
    picks: &[NewRecoPick],
    seed_pool_json: Option<String>,
    strategy_weights_json: Option<String>,
    generated_at: &str,
    as_of_date: Option<&str>,
    reco_version: Option<i32>,
) -> usize {
    let mut written = 0usize;
    for pick in picks {
        let am = reco_picks::ActiveModel {
            id: Set(uuid::Uuid::new_v4().to_string()),
            generated_at: Set(generated_at.to_string()),
            period: Set(pick.period.clone()),
            stock_code: Set(pick.stock_code.clone()),
            stock_name: Set(pick.stock_name.clone()),
            style: Set(pick.style.clone()),
            confidence: Set(pick.confidence),
            synthetic: Set(i32::from(pick.synthetic)),
            seed_pool_json: Set(seed_pool_json.clone()),
            strategy_weights_json: Set(strategy_weights_json.clone()),
            pick_data: Set(pick.pick_data.clone()),
            reco_version: Set(reco_version),
            as_of_date: Set(as_of_date.map(|s| s.to_string())),
            created_at: Set(generated_at.to_string()),
        };
        match am.insert(db).await {
            Ok(_) => written += 1,
            Err(e) => tracing::warn!(
                "[reco_pool::insert_picks] 写入 reco_picks 失败 ({} {} {}): {e}",
                pick.period,
                pick.stock_code,
                pick.style
            ),
        }
    }
    written
}

/// 一条 L3 截断留痕（算出该票、但在组内 top-N 被截掉）。
pub struct NewScanAuditTrim {
    pub stock_code: String,
    pub stock_name: String,
    pub style: String,
    pub confidence: i32,
    pub rank: i32,
}

/// 落 `reco_scan_audit`，返回写入行数。
///
/// 语义边界见 `axagent_entities::reco_scan_audit` 的文档：**无行**不等于该风格把票选出去了。
/// 与同批 picks 共用同一个 `generated_at`（批次键）与同一个 `as_of_date`（代际），
/// 否则漏检归因会出现「实盘的票 + 回放的留痕」这种跨代拼出来的证据。
pub async fn insert_scan_audit(
    db: &DatabaseConnection,
    trimmed: &[NewScanAuditTrim],
    period: &str,
    generated_at: &str,
    as_of_date: Option<&str>,
) -> usize {
    let mut written = 0usize;
    for t in trimmed {
        let am = reco_scan_audit::ActiveModel {
            id: Set(uuid::Uuid::new_v4().to_string()),
            generated_at: Set(generated_at.to_string()),
            period: Set(period.to_string()),
            stock_code: Set(t.stock_code.clone()),
            stock_name: Set(t.stock_name.clone()),
            style: Set(t.style.clone()),
            confidence: Set(t.confidence),
            rank: Set(t.rank),
            as_of_date: Set(as_of_date.map(|s| s.to_string())),
            created_at: Set(generated_at.to_string()),
        };
        match am.insert(db).await {
            Ok(_) => written += 1,
            Err(e) => tracing::warn!(
                "[reco_pool::insert_scan_audit] 写入失败 ({} {} {}): {e}",
                period,
                t.stock_code,
                t.style
            ),
        }
    }
    written
}

/// 最近一批荐股的候选池代码（`reco_picks.seed_pool_json` 的 `[["code","name"], …]`）。
///
/// 用途（2026-10-20，P-n 的覆盖前提）：每日估值归档要补的正是**荐股扫描的那批票**，
/// 而不是全市场 681 只逐个拉估值（收益与请求量不成比例）。此处直接复用已经落库的池快照，
/// 零额外网络。解析失败按空处理（归档跳过，不影响收盘快照）。
pub async fn latest_seed_pool_codes(
    db: &DatabaseConnection,
) -> Result<Vec<String>, sea_orm::DbErr> {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

    let row = reco_picks::Entity::find()
        .filter(reco_picks::Column::SeedPoolJson.ne(""))
        .order_by_desc(reco_picks::Column::GeneratedAt)
        .one(db)
        .await?;
    let Some(r) = row else {
        return Ok(Vec::new());
    };
    let Some(raw) = r.seed_pool_json.as_deref().filter(|s| !s.is_empty()) else {
        return Ok(Vec::new());
    };
    let pairs: Vec<(String, String)> = serde_json::from_str(raw).unwrap_or_default();
    Ok(pairs.into_iter().map(|(code, _)| code).collect())
}
