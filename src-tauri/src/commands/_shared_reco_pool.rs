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
/// - `batch`：批次键三列（`generated_at` / `as_of_date` / `purpose`）**具名同传**。
///   此前它们是三个平铺参数 —— 与 [`persist_reco_pick`] 用的 `BatchKey` 形态不一致，
///   后果是代际在调用点读不出统一证据：`as-of` 接线门 R2 原先盯命令层的
///   `pick.insert(db)` 字面量，落库下沉到 dao 后该字面量消失，四条链的代际写入
///   **一度完全不在任何证据之下**（2026-10-29 实测）。收成 `BatchKey` 后
///   `as_of_date` 是必填字段，漏传直接编译不过。
pub async fn persist_reco_picks(
    db: &DatabaseConnection,
    picks: &[RecoPick],
    seed_pool_json: Option<String>,
    strategy_weights_json: Option<String>,
    batch: &BatchKey<'_>,
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
        batch,
        // 算法代际随样本一起落库，否则闭环 IC 会把跨代样本混成一格
        Some(recommender::RECO_ALGORITHM_VERSION),
    )
    .await
}

/// 落**一行** `reco_picks`（同业务键覆盖，保留既有 `id`），返回**技术详情文本**。
///
/// 与 [`persist_reco_picks`] 同一份幂等实现（dao `upsert_pick`），区别只在：
/// 调用方需要逐行的失败信息（趋势智选要把「哪只 / 为什么」回传前端），
/// 且每行的候选池快照不同（智选按**候选对象**逐行存 `seed_pool_json`）。
///
/// ⚠ 返回值不是命令边界的错误载荷 —— 命令边界（`run_serenity_screening_inner`）已经把
/// 错误码 `wf_err::PERSIST_FAILED` 与 `persistenceStockCode` 一并发给了前端，这里只补
/// `persistenceError` 那条**技术详情**。所以它既不能是裸的 `e.to_string()`（读者不知道
/// 是哪一步失败，`check-rust-raw-map-err.mjs` 也据此判身份式违规），也不该压成
/// `ErrorResponse` JSON（那会让 detail 里再套一坨 JSON、错误码出现两遍）。
pub async fn persist_reco_pick(
    db: &DatabaseConnection,
    pick: &NewRecoPick,
    seed_pool_json: Option<&str>,
    batch: &BatchKey<'_>,
) -> Result<(), String> {
    pool_dao::upsert_pick(
        db,
        pick,
        seed_pool_json,
        None,
        batch,
        Some(recommender::RECO_ALGORITHM_VERSION),
    )
    .await
    .map_err(|e| {
        format!("落库 reco_picks 失败（{} / {} 档，业务键覆盖）: {e}", pick.stock_code, pick.period)
    })
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

/// `stock_code → 行业名`（`market_stock_universe.industry`，现网覆盖 4,491/6,165）。
///
/// 三个候选来源共用它（全市场快照、用户标的、涨幅榜），行业判据才只有一套词表。
/// 读失败 ⇒ 空表（行业缺席 ≠ 候选作废；`attach_industries` 保持 `None`，可分辨）。
pub(crate) async fn universe_industries(
    db: &DatabaseConnection,
) -> std::collections::HashMap<String, String> {
    use axagent_dao::repo::reco_seed;
    match reco_seed::load_universe_industries(db).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("[reco] 行业映射读取失败（候选按无行业处理）: {e}");
            Default::default()
        },
    }
}

/// 全市场快照 → 引擎（W3，2026-11-01）。
///
/// 放在共享模块而不是 `stock_analysis`：`mover_recall_analysis` 的 as-of 回放也要用，
/// 命令模块之间直接互调会命中分层门禁的 `commands-no-sibling-call`。
/// 本模块已在 `check-layer-discipline.mjs` 的 `SHARED_INFRA_MODULES` 内。
///
/// 只读 DB 的 `market_daily_close`（最近 30 个交易日 + 截止日那天的估值两列）与
/// `market_stock_universe.industry`，不增加网络请求；实盘取最新交易日、as-of 取
/// `generation` 截止日（包含当日）。失败 ⇒ 返回空表（调用方回落基础池），绝不保留上一轮快照。
///
/// 估值与历史**同一个交易日**：日期由历史查询返回后显式传给估值查询，不在估值侧再反查
/// 截止日 —— 两次独立反查之间若落了新一天，快照与估值截面就会跨代。
pub(crate) async fn build_universe_seed_items(
    db: &DatabaseConnection,
    generation: Option<&str>,
) -> Vec<recommender::universe::UniverseItem> {
    use axagent_dao::repo::reco_seed;

    const TRADE_DAYS: usize = 30;
    let (latest, rows) =
        match reco_seed::load_close_history_on_or_before(db, generation, TRADE_DAYS).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("[reco] 全市场快照读取失败（回落基础池）: {e}");
                return Vec::new();
            },
        };
    let row_count = rows.len();
    let industries = universe_industries(db).await;
    // 估值缺失不阻断快照：价值桶自己按覆盖率作废并声明降级（其余三桶照常建）。
    let valuations = match latest.as_deref() {
        Some(day) => match reco_seed::load_valuations_on_date(db, day).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("[reco] 估值列读取失败（价值桶将声明缺席）: {e}");
                Default::default()
            },
        },
        None => Default::default(),
    };
    let valued = valuations.len();
    let close_rows: Vec<recommender::universe::CloseRow> = rows
        .into_iter()
        .map(|(code, name, trade_date, close, amount, change_pct)| {
            recommender::universe::CloseRow { code, name, trade_date, close, amount, change_pct }
        })
        .collect();
    let items = recommender::universe::build_universe(&close_rows, &industries, &valuations);
    tracing::info!(
        "[reco] 全市场快照（日 {:?}）：{} 行 → {} 只可用于分桶；带估值 {} 只",
        latest,
        row_count,
        items.len(),
        valued
    );
    items
}

/// 构建并**写入进程级快照**（live 单档/定时入口的回落来源），返回 `(条数, 条目)`。
pub(crate) async fn refresh_universe_seed_db(
    db: &DatabaseConnection,
    generation: Option<&str>,
) -> (usize, Vec<recommender::universe::UniverseItem>) {
    let items = build_universe_seed_items(db, generation).await;
    let n = items.len();
    recommender::set_universe_seed(items.clone());
    (n, items)
}

/// 用途值域的唯一权威（dao 侧定义，命令层转出给引擎/批次复用）。
pub use axagent_dao::repo::reco_pool::purpose;
