//! 全市场收盘快照与代码清单采集（`PLAN-mover-recall-attribution.md` Phase 1）
//!
//! 命令层（`src/commands/market_close_sweep.rs`）只做薄封装，此处承载全部 DB 落库。
//! 分层原因：`scripts/check-layer-discipline.mjs` 的 `commands-no-direct-db`
//! 禁止新命令文件直连 `sea_orm` / `axagent_entities`，须经 dao / service 层。
//!
//! 两件事：
//! 1. `market_stock_universe` —— 核查用的枚举域。权威扩张只能靠东财 `clist` 分页，
//!    而该接口连续爬 60 页实测 11/60 成功即触发连接级封禁（见
//!    `AUDIT-mover-universe-feasibility-2026-09-30.md`），所以**只提供「本轮取哪几页」**，
//!    全量由调用方跨 tick 摊薄；本地既有代码（自选股、已分析股）作 bootstrap 补录。
//! 2. `market_daily_close` —— 事件地基。走腾讯批量行情（80 码/请求，实测 74 请求覆盖
//!    全市场、零失败）。窗口累计涨幅不在这里存，由消费侧复利连乘算。
//!
//! ⚠ 停牌/缺值**不落行**，也绝不写 `change_pct = 0.0`：写 0 会把「拿不到」伪装成
//! 「没涨」，直接污染漏检率的分母。缺值票数必须计进 `skipped_no_data` 并上报。

use sea_orm::{
    sea_query::OnConflict, ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QuerySelect,
};
use serde::Serialize;

use axagent_astock_data::AStockClient;
use axagent_entities::{
    market_daily_close, market_stock_universe, stock_analyses, watchlist_items,
};

/// 腾讯批量端点实测可完整兑现的批量（80 码/请求）；调用方可下调，不得凭经验上调。
pub const DEFAULT_BATCH_SIZE: usize = 80;
/// 批量之间的间隔（ms）——实测 0.4s 下零失败，风控收紧时优先加大这里而不是改批量。
pub const DEFAULT_BATCH_INTERVAL_MS: u64 = 400;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseSweepReport {
    pub trade_date: String,
    /// 本轮参与采集的代码数（= 清单规模）
    pub universe_size: usize,
    pub batches: usize,
    /// 成功批次里返回的行数合计
    pub rows_returned: usize,
    /// 实际落库行数
    pub written: usize,
    /// 返回了但取不到有效收盘/涨幅（停牌、缺昨收、非有限值）的票数
    pub skipped_no_data: usize,
    /// 整批失败（网络/429）的批次数；非空 ⇒ 本轮数据不完整，不得当全量用
    pub failed_batches: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UniverseReport {
    pub pages_requested: usize,
    pub pages_failed: usize,
    pub rows_upserted: usize,
    /// 接口申报的全市场总数；0 表示本轮没拿到（封禁/失败）
    pub reported_total: i64,
    pub universe_size: usize,
}

/// 清单与快照的覆盖状况（面板据此显式声明枚举域是否完整，禁止把「没采到」演成「没涨」）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageData {
    pub universe_size: i64,
    /// 清单里被 clist 全量确认过的票数（低于 universe_size ⇒ 枚举域由本地票拼出，不完整）
    pub confirmed_by_clist: i64,
    pub close_dates: Vec<String>,
    pub rows_per_date: Vec<i64>,
}

/// 把批量返回的一行折算成待落库行；**无效输入返回 None（不落行、不补 0）**。
fn close_row(
    code: &str,
    name: &str,
    trade_date: &str,
    now_ms: i64,
    price: f64,
    pre_close: f64,
    change_pct: f64,
) -> Option<market_daily_close::ActiveModel> {
    if code.is_empty() || trade_date.is_empty() {
        return None;
    }
    // 三个量都必须有限且为正，否则视为「拿不到」而不是「没涨」
    if !(price.is_finite() && pre_close.is_finite() && change_pct.is_finite()) {
        return None;
    }
    if price <= 0.0 || pre_close <= 0.0 {
        return None;
    }
    Some(market_daily_close::ActiveModel {
        id: Set(format!("{code}@{trade_date}")),
        stock_code: Set(code.to_string()),
        stock_name: Set(name.to_string()),
        trade_date: Set(trade_date.to_string()),
        close: Set(price),
        prev_close: Set(pre_close),
        change_pct: Set(change_pct),
        collected_at: Set(now_ms),
        source: Set("tencent_batch".to_string()),
    })
}

async fn upsert_universe(
    db: &sea_orm::DatabaseConnection,
    entries: &[(String, String)],
    origin: &str,
    now_ms: i64,
    confirm: bool,
) -> usize {
    let mut written = 0usize;
    for (code, name) in entries {
        if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let market_type = axagent_harness::market_data::detect_market_type(code).to_string();
        let am = market_stock_universe::ActiveModel {
            stock_code: Set(code.clone()),
            stock_name: Set(name.clone()),
            market_type: Set(market_type),
            first_seen_at: Set(now_ms),
            last_confirmed_at: Set(if confirm { now_ms } else { 0 }),
            origin: Set(origin.to_string()),
        };
        let res = market_stock_universe::Entity::insert(am)
            .on_conflict(
                OnConflict::column(market_stock_universe::Column::StockCode)
                    .update_columns([
                        market_stock_universe::Column::StockName,
                        market_stock_universe::Column::MarketType,
                    ])
                    .to_owned(),
            )
            .exec(db)
            .await;
        match res {
            Ok(_) => written += 1,
            Err(e) => tracing::warn!("[universe] {code} 落库失败: {e}"),
        }
    }
    written
}

/// bootstrap：把本地已有代码（自选股 + 已分析股）补进清单。
///
/// 它**不能**替代全量刷新 —— 覆盖的是「我们本来就碰得到的票」；
/// 唯一的全市场扩张正门是 [`refresh_universe_pages`]。
pub async fn bootstrap_universe_from_local(
    db: &sea_orm::DatabaseConnection,
) -> Result<usize, String> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut entries: Vec<(String, String)> = Vec::new();

    let watch =
        watchlist_items::Entity::find().all(db).await.map_err(|e| format!("读自选股失败: {e}"))?;
    for w in watch {
        if seen.insert(w.stock_code.clone()) {
            entries.push((w.stock_code, w.stock_name));
        }
    }

    let analysed = stock_analyses::Entity::find()
        .limit(20_000)
        .all(db)
        .await
        .map_err(|e| format!("读已分析股失败: {e}"))?;
    for a in analysed {
        if seen.insert(a.stock_code.clone()) {
            entries.push((a.stock_code, a.stock_name));
        }
    }

    Ok(upsert_universe(db, &entries, "local", now_ms, false).await)
}

/// 分 tick 扩清单：本轮只取 `pages` 指定的页（每页 100 只）。
///
/// 封禁期失败是**预期内**的，故不返 Err 而是把 `pages_failed` 报出去；
/// 调用方（cron tick）据此决定下一轮从哪续。
pub async fn refresh_universe_pages(
    db: &sea_orm::DatabaseConnection,
    client: &AStockClient,
    pages: &[u32],
) -> Result<UniverseReport, String> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut pages_failed = 0usize;
    let mut rows_upserted = 0usize;
    let mut reported_total = 0i64;

    for pn in pages {
        match client.market_list_page(*pn).await {
            Ok((entries, total)) => {
                if total > 0 {
                    reported_total = total;
                }
                if entries.is_empty() {
                    pages_failed += 1;
                    continue;
                }
                rows_upserted += upsert_universe(db, &entries, "clist", now_ms, true).await;
            },
            Err(e) => {
                pages_failed += 1;
                tracing::warn!("[universe] 第 {pn} 页取数失败: {e}");
            },
        }
        // 页间限速：连续快爬是本机进入封禁窗的直接原因
        tokio::time::sleep(std::time::Duration::from_millis(1_500)).await;
    }

    let universe_size = market_stock_universe::Entity::find()
        .count(db)
        .await
        .map_err(|e| format!("统计清单失败: {e}"))?;

    Ok(UniverseReport {
        pages_requested: pages.len(),
        pages_failed,
        rows_upserted,
        reported_total,
        universe_size: universe_size as usize,
    })
}

/// 采集指定交易日的收盘快照。
pub async fn run_close_sweep(
    db: &sea_orm::DatabaseConnection,
    client: &AStockClient,
    trade_date: &str,
    batch_size: usize,
    interval_ms: u64,
) -> Result<CloseSweepReport, String> {
    let batch_size = batch_size.clamp(1, DEFAULT_BATCH_SIZE);
    let now_ms = chrono::Utc::now().timestamp_millis();

    let rows = market_stock_universe::Entity::find()
        .all(db)
        .await
        .map_err(|e| format!("读清单失败: {e}"))?;
    let codes: Vec<String> = rows.into_iter().map(|r| r.stock_code).collect();
    let universe_size = codes.len();
    if universe_size == 0 {
        return Err("股票清单为空：先跑 bootstrap 或分 tick 扩清单，本轮不产出快照".to_string());
    }

    let mut batches = 0usize;
    let mut failed_batches = 0usize;
    let mut rows_returned = 0usize;
    let mut written = 0usize;
    let mut skipped_no_data = 0usize;

    for chunk in codes.chunks(batch_size) {
        batches += 1;
        match client.market_snapshot_batch(chunk).await {
            Ok(quotes) => {
                rows_returned += quotes.len();
                for q in quotes {
                    let row_id = format!("{}@{trade_date}", q.code);
                    let Some(am) = close_row(
                        &q.code,
                        &q.name,
                        trade_date,
                        now_ms,
                        q.price,
                        q.pre_close,
                        q.change_pct,
                    ) else {
                        skipped_no_data += 1;
                        continue;
                    };
                    let res = market_daily_close::Entity::insert(am)
                        .on_conflict(
                            OnConflict::column(market_daily_close::Column::Id)
                                .update_columns([
                                    market_daily_close::Column::StockName,
                                    market_daily_close::Column::Close,
                                    market_daily_close::Column::PrevClose,
                                    market_daily_close::Column::ChangePct,
                                    market_daily_close::Column::CollectedAt,
                                    market_daily_close::Column::Source,
                                ])
                                .to_owned(),
                        )
                        .exec(db)
                        .await;
                    match res {
                        Ok(_) => written += 1,
                        Err(e) => tracing::warn!("[close] {row_id} 落库失败: {e}"),
                    }
                }
            },
            Err(e) => {
                failed_batches += 1;
                tracing::warn!("[close] 批量取数失败（{} 只）: {e}", chunk.len());
            },
        }
        tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
    }

    Ok(CloseSweepReport {
        trade_date: trade_date.to_string(),
        universe_size,
        batches,
        rows_returned,
        written,
        skipped_no_data,
        failed_batches,
    })
}

/// 清单与快照的覆盖状况（面板据此显式声明枚举域是否完整，禁止把「没采到」演成「没涨」）。
pub async fn load_market_coverage(
    db: &sea_orm::DatabaseConnection,
) -> Result<CoverageData, String> {
    let universe_size = market_stock_universe::Entity::find()
        .count(db)
        .await
        .map_err(|e| format!("统计清单失败: {e}"))?;
    let confirmed_by_clist = market_stock_universe::Entity::find()
        .filter(market_stock_universe::Column::LastConfirmedAt.gt(0))
        .count(db)
        .await
        .map_err(|e| format!("统计确认数失败: {e}"))?;

    // 按日聚合行数：限定近 400 天窗口读，避免整表进内存（覆盖率视图只看近期，历史聚合走事件表）
    let cutoff = (chrono::Utc::now().date_naive() - chrono::Duration::days(400))
        .format("%Y-%m-%d")
        .to_string();
    let rows = market_daily_close::Entity::find()
        .filter(market_daily_close::Column::TradeDate.gte(&cutoff))
        .all(db)
        .await
        .map_err(|e| format!("读快照失败: {e}"))?;
    let mut by_date: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    for r in rows {
        *by_date.entry(r.trade_date).or_default() += 1;
    }
    let (close_dates, rows_per_date): (Vec<String>, Vec<i64>) = by_date.into_iter().unzip();

    Ok(CoverageData {
        universe_size: universe_size as i64,
        confirmed_by_clist: confirmed_by_clist as i64,
        close_dates,
        rows_per_date,
    })
}

/// 某交易日已落库的快照行数（cron 用它判「这天采齐了没有」）。
pub async fn rows_for_date(
    db: &sea_orm::DatabaseConnection,
    trade_date: &str,
) -> Result<u64, String> {
    let n = market_daily_close::Entity::find()
        .filter(market_daily_close::Column::TradeDate.eq(trade_date))
        .count(db)
        .await
        .map_err(|e| format!("统计 {trade_date} 快照行数失败: {e}"))?;
    Ok(n as u64)
}

/// 清单规模。
pub async fn universe_size(db: &sea_orm::DatabaseConnection) -> Result<u64, String> {
    let n = market_stock_universe::Entity::find()
        .count(db)
        .await
        .map_err(|e| format!("统计清单规模失败: {e}"))?;
    Ok(n as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_row_is_written_with_date_scoped_id() {
        let am = close_row("600059", "古越龙山", "2026-09-30", 1, 12.27, 11.15, 10.04)
            .expect("有效行情应落行");
        assert_eq!(am.id.unwrap(), "600059@2026-09-30");
        assert_eq!(am.change_pct.unwrap(), 10.04);
    }

    #[test]
    fn suspended_or_zero_prev_close_is_skipped_not_zeroed() {
        // 停牌/缺昨收 ⇒ None（不落行），绝不能写成 change_pct=0 的"没涨"
        assert!(close_row("600059", "古越龙山", "2026-09-30", 1, 12.27, 0.0, 0.0).is_none());
        assert!(close_row("600059", "古越龙山", "2026-09-30", 1, 0.0, 11.15, 10.04).is_none());
        assert!(close_row("600059", "古越龙山", "2026-09-30", 1, f64::NAN, 11.15, 10.04).is_none());
        assert!(close_row("", "x", "2026-09-30", 1, 1.0, 1.0, 1.0).is_none());
    }
}
