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

use std::collections::HashMap;

use sea_orm::{
    sea_query::OnConflict, ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect,
};
use serde::Serialize;

use axagent_astock_data::AStockClient;
use axagent_entities::{
    market_daily_close, market_stock_universe, market_universe_stats, stock_analyses,
    watchlist_items,
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
    /// 返回了但取不到有效收盘/涨幅（缺昨收、非有限值）的票数
    pub skipped_no_data: usize,
    /// 涨幅为 0 且成交额/成交量都为 0 的票数 —— 停牌/无成交，不落行。
    /// 与「平盘有量」区分：后者成交量为正，照常落 0% 行。
    pub skipped_suspended: usize,
    /// 整批失败（网络/429）的批次数；非空 ⇒ 本轮数据不完整，不得当全量用
    pub failed_batches: usize,
    /// 清单里有、但行情源**一个报价都没回**的代码数（≠ 停牌/无有效值）。
    /// 让「全 A 股样本」的缺口显式可见，而不是只看 written < universe_size 猜原因。
    pub missing_quotes: usize,
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
    /// 行情源申报的全市场总数（`market_universe_stats.reported_total`；0 = 还没拿到）
    pub reported_total: i64,
    pub close_dates: Vec<String>,
    pub rows_per_date: Vec<i64>,
    /// 最新收盘日的代码数（= 当日 `market_daily_close` 行数；行与 (code,date) 一对一）
    pub latest_close_codes: i64,
}

/// 把批量返回的一行折算成待落库行；**无效输入返回 None（不落行、不补 0）**。
// 纯行构造器：8 个标量全部来自批量行情的同一行，拆成结构体只会把调用点写成 `Struct { .. }`
// 而不增加任何可读性 ⇒ 与 `recommender/strategies/trend.rs` 的 `trend_evidence_strength` 同形态。
#[allow(clippy::too_many_arguments)]
fn close_row(
    code: &str,
    name: &str,
    trade_date: &str,
    now_ms: i64,
    price: f64,
    pre_close: f64,
    change_pct: f64,
    amount: f64,
    volume: f64,
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
    // 成交额：源不给（0/非有限）时用 close × volume 推定 —— 解析层已把「手」换成「股」、
    // 「万」换成「元」⇒ 乘积单位就是元，与阈值的 1 亿同单位。
    // 两者都拿不到 ⇒ 保留 0 = 「未采集」（读侧按 0 跳过流动性判据），绝不写一个假的 0 成交。
    let (amount_value, source_tag) = if amount.is_finite() && amount > 0.0 {
        (amount, "tencent_batch")
    } else if volume.is_finite() && volume > 0.0 {
        (price * volume, "tencent_batch+amount_est")
    } else {
        (0.0, "tencent_batch")
    };
    Some(market_daily_close::ActiveModel {
        id: Set(format!("{code}@{trade_date}")),
        stock_code: Set(code.to_string()),
        stock_name: Set(name.to_string()),
        trade_date: Set(trade_date.to_string()),
        close: Set(price),
        prev_close: Set(pre_close),
        change_pct: Set(change_pct),
        amount: Set(amount_value),
        collected_at: Set(now_ms),
        // 推定值与真值必须可分辨：读侧判据（amount=0 OR ≥1亿）不变，但审计能看出这一行是推的
        source: Set(source_tag.to_string()),
    })
}

/// 段扫专用入口：只登记代码 —— `confirm = false` ⇒ `last_confirmed_at` 保持 0（不算东财全量确认）、
/// 行业列不覆写。清单计数不会被段扫灌成「已确认」。
/// 收盘历史回补：用日线把 `market_daily_close` 补到 `target_days` 个交易日。
///
/// 为什么需要：四档窗口是 2/5/28/90 个**交易日**，而本地快照是逐日攒的（现网只有 9 天）
/// ⇒ 中/长档永远「窗口未满」，用户看到的自动窗口等于没生效。这里从日线一次性补齐，
/// 之后每天由 16:00 的快照只追加当天。
///
/// 口径：
/// - **只补缺失行**（`on_conflict do_nothing`）—— 既有行来自当日快照，amount 真值/推定与
///   `source` 更权威，回补不得覆盖；
/// - `prev_close` 由相邻两根日线推出，首根没有前值 ⇒ 跳过（不猜）；
/// - `source='tencent_kline'`，与 `tencent_batch` / `+amount_est` 可分辨。
pub async fn backfill_close_history(
    db: &sea_orm::DatabaseConnection,
    client: &axagent_astock_data::AStockClient,
    target_days: u32,
    interval_ms: u64,
) -> Result<BackfillStats, String> {
    let universe = market_stock_universe::Entity::find()
        .all(db)
        .await
        .map_err(|e| format!("读市场清单失败: {e}"))?;
    let entries: Vec<(String, String)> =
        universe.into_iter().map(|m| (m.stock_code, m.stock_name)).collect();
    backfill_close_history_for_codes(db, client, &entries, target_days, interval_ms).await
}

/// 只补指定代码的历史（逐票覆盖判据的入口）。
///
/// 全清单逐票回补在清单扩到 2500+ 后会把行情通道打爆，且已补过的票被反复请求；
/// 调用方先用 [`codes_needing_history`] 选「行数不足」的票，再按批调用本函数。
pub async fn backfill_close_history_for_codes(
    db: &sea_orm::DatabaseConnection,
    client: &axagent_astock_data::AStockClient,
    entries: &[(String, String)],
    target_days: u32,
    interval_ms: u64,
) -> Result<BackfillStats, String> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    // 只认**已收盘**交易日：盘中今天那根 K 线是半日数据，写进来会被当收盘价用
    // （2026-10-23 12:19 实测：一次回补把 10-23 盘中价写成收盘，90 日窗口还多算一根）。
    let now_local = chrono::Local::now();
    let latest_completed = axagent_astock_data::daily_snapshot::snapshot_target_date(&now_local)
        .unwrap_or_else(|| {
            axagent_astock_data::calendar::previous_trading_day(
                now_local.date_naive() - chrono::Duration::days(1),
            )
        });
    let cutoff = latest_completed.format("%Y-%m-%d").to_string();
    // 多取一根：接口返回包含今天（未收盘）那根时，剔除后仍能凑满 target_days 根已完成日线。
    let fetch_days = target_days.saturating_add(1);
    let mut stats = BackfillStats::default();
    for (code, stock_name) in entries {
        stats.codes += 1;
        let mut klines = match client.get_klines(code, "daily", fetch_days).await {
            Ok(v) => v,
            Err(e) => {
                stats.failed += 1;
                if stats.first_error.is_none() {
                    stats.first_error = Some(format!("{code}: {e}"));
                    // 只对首条打 warn：500 只全失败时逐条刷屏会把日志淹掉
                    tracing::warn!("[close] 历史回补 {code} 失败（后续同类只计数）: {e}");
                }
                continue;
            },
        };
        klines.sort_by(|a, b| a.date.cmp(&b.date));
        // 先剔除未收盘/未来的 bar，再截最近 target_days 根（顺序不能反：
        // 先截会把已完成的最老一根挤掉，结果永远差一天、反复选中同一批）。
        klines.retain(|k| k.date.as_str() <= cutoff.as_str());
        // 只保留最近 target_days 根：K 线接口的 limit 实测未生效（返回了 1996 年起的全历史，
        // 55k 行），写入侧自行截断，避免把「数据起点」和核查取数区间拖成十年。
        if klines.len() > target_days as usize {
            let keep_from = klines.len() - target_days as usize;
            klines.drain(..keep_from);
        }
        let mut rows = Vec::new();
        let mut prev: Option<f64> = None;
        for k in klines {
            // 用 Option::filter 表达「有前值且两者都为正」，避免 let-chain（本 crate 是 2021 edition）
            if let Some(p) = prev.filter(|p| *p > 0.0 && k.close > 0.0) {
                rows.push(market_daily_close::ActiveModel {
                    id: Set(format!("{}@{}", code, k.date)),
                    stock_code: Set(code.clone()),
                    stock_name: Set(stock_name.clone()),
                    trade_date: Set(k.date.clone()),
                    close: Set(k.close),
                    prev_close: Set(p),
                    change_pct: Set((k.close / p - 1.0) * 100.0),
                    amount: Set(if k.amount.is_finite() && k.amount > 0.0 {
                        k.amount
                    } else {
                        0.0
                    }),
                    collected_at: Set(now_ms),
                    source: Set("tencent_kline".to_string()),
                });
            }
            prev = Some(k.close);
        }
        for chunk in rows.chunks(500) {
            let res = market_daily_close::Entity::insert_many(chunk.to_vec())
                .on_conflict(
                    OnConflict::column(market_daily_close::Column::Id).do_nothing().to_owned(),
                )
                .exec(db)
                .await;
            match res {
                Ok(_) => stats.rows += chunk.len() as u32,
                Err(e) => {
                    stats.failed += 1;
                    if stats.first_error.is_none() {
                        stats.first_error = Some(format!("{code}: 落库失败: {e}"));
                    }
                    tracing::warn!("[close] {code} 历史回补失败: {e}");
                },
            }
        }
        if interval_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
        }
    }
    tracing::info!(
        "[close] 历史回补完成：代码 {} 只 / 尝试写入 {} 行 / 失败 {} 次",
        stats.codes,
        stats.rows,
        stats.failed
    );
    Ok(stats)
}

#[derive(Debug, Default, Clone)]
pub struct BackfillStats {
    pub codes: u32,
    pub rows: u32,
    pub failed: u32,
    /// 首条失败原因（500 只全失败时用来诊断；后续同类只计数不刷屏）
    pub first_error: Option<String>,
}

/// 选「历史行数不足 `target_days`」的票。
///
/// 判据是**逐票行数**，不是全局日期数：段扫新登记的票在 `market_daily_close` 里
/// 可能是 0 行，而全局日期数早已 ≥90 ⇒ 旧实现永远不补它们。
/// 排序：已有部分历史的优先（0 行票可能是误登记），再按代码稳定排序，便于分批追进度。
pub async fn codes_needing_history(
    db: &sea_orm::DatabaseConnection,
    target_days: u32,
    limit: usize,
) -> Result<Vec<(String, String)>, String> {
    let universe = market_stock_universe::Entity::find()
        .all(db)
        .await
        .map_err(|e| format!("读市场清单失败: {e}"))?;
    let rows = market_daily_close::Entity::find()
        .all(db)
        .await
        .map_err(|e| format!("读收盘历史失败: {e}"))?;
    let mut counts: HashMap<String, usize> = HashMap::new();
    for r in rows {
        *counts.entry(r.stock_code).or_default() += 1;
    }
    let mut need: Vec<(usize, String, String)> = universe
        .into_iter()
        .map(|m| (counts.get(&m.stock_code).copied().unwrap_or(0), m.stock_code, m.stock_name))
        .filter(|(n, _, _)| *n < target_days as usize)
        .collect();
    need.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    need.truncate(limit);
    Ok(need.into_iter().map(|(_, code, name)| (code, name)).collect())
}

pub(crate) async fn upsert_universe_registered(
    db: &sea_orm::DatabaseConnection,
    entries: &[(String, String, Option<String>)],
    origin: &str,
    now_ms: i64,
) -> Result<u32, String> {
    Ok(upsert_universe(db, entries, origin, now_ms, false).await as u32)
}

async fn upsert_universe(
    db: &sea_orm::DatabaseConnection,
    entries: &[(String, String, Option<String>)],
    origin: &str,
    now_ms: i64,
    confirm: bool,
) -> usize {
    let mut written = 0usize;
    for (code, name, industry) in entries {
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
            industry: Set(industry.clone()),
        };
        // 只由 clist 全量刷新（confirm=true）覆写行业列：本机 bootstrap 传的是 None，
        // 若也进 update 列表，会把已采集到的行业擦成 NULL。
        let mut cols = vec![
            market_stock_universe::Column::StockName,
            market_stock_universe::Column::MarketType,
        ];
        if confirm {
            cols.push(market_stock_universe::Column::Industry);
        }
        let res = market_stock_universe::Entity::insert(am)
            .on_conflict(
                OnConflict::column(market_stock_universe::Column::StockCode)
                    .update_columns(cols)
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
    let mut entries: Vec<(String, String, Option<String>)> = Vec::new();

    let watch =
        watchlist_items::Entity::find().all(db).await.map_err(|e| format!("读自选股失败: {e}"))?;
    for w in watch {
        if seen.insert(w.stock_code.clone()) {
            entries.push((w.stock_code, w.stock_name, None));
        }
    }

    let analysed = stock_analyses::Entity::find()
        .limit(20_000)
        .all(db)
        .await
        .map_err(|e| format!("读已分析股失败: {e}"))?;
    for a in analysed {
        if seen.insert(a.stock_code.clone()) {
            entries.push((a.stock_code, a.stock_name, None));
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

    // 0-D（2026-10-18，PLAN-mover-coverage-and-reflection-exit）：把**接口自报的分母**一起落库。
    // 原先 `reported_total` 只进 `tracing::info!`，于是「清单到底缺多少」在库里无法回答
    // —— 而这张清单就是荐股的候选宇宙，覆盖率等于「智能荐股能看见多少只票」。
    // ⚠ **只在有分母时落行**：页全失败时 `reported_total` 还是初值 0，落一行 0 分母
    //   会被读侧当成「全市场就 0 只票」——「拿不到」与「确实是 0」是两件事。
    if reported_total > 0 {
        use axagent_entities::market_universe_stats;
        let now_ms = chrono::Utc::now().timestamp_millis();
        let am = market_universe_stats::ActiveModel {
            observed_at: Set(now_ms),
            universe_size: Set(universe_size as i32),
            reported_total: Set(reported_total),
            // 存两位小数的比值：读侧现算也行，但那样「当初按哪条公式算的」会留给
            // 下一个改公式的人 —— 落库的那个数就是本次 tick 的口径快照。
            coverage_pct: Set(
                ((universe_size as f64 / reported_total as f64) * 10000.0).round() / 100.0
            ),
            pages_requested: Set(pages.len() as i32),
            pages_failed: Set(pages_failed as i32),
            rows_upserted: Set(rows_upserted as i32),
            created_at: Set(now_ms),
        };
        if let Err(e) = market_universe_stats::Entity::insert(am).exec(db).await {
            tracing::warn!("[universe] 覆盖率读数落库失败: {e}");
        }
    }

    Ok(UniverseReport {
        pages_requested: pages.len(),
        pages_failed,
        rows_upserted,
        reported_total,
        universe_size: universe_size as usize,
    })
}

/// 最近一次行情源申报的全市场总数（0 = 还没取到）。
///
/// 命令层不得直连 `axagent_entities`（分层门 `commands-no-direct-db`），读侧统一走这里。
pub async fn latest_reported_total(db: &sea_orm::DatabaseConnection) -> Result<i64, String> {
    use axagent_entities::market_universe_stats;
    market_universe_stats::Entity::find()
        .order_by_desc(market_universe_stats::Column::ObservedAt)
        .one(db)
        .await
        .map(|r| r.map_or(0, |m| m.reported_total))
        .map_err(|e| format!("读覆盖率读数失败: {e}"))
}

/// 段扫后落一行覆盖率读数：分母沿用最近一次 clist 自报的全市场总数。
///
/// 段扫不申报 `reported_total`（它只按段问行情），所以不能自己编一个分母；
/// 没有历史分母时**不落行**（0 分母会被读成「全市场 0 只」）。
/// 不落这一行的话，清单已扩到 2563，面板/门仍会读到 10-21 的 universe=684。
pub async fn record_universe_stats_for_scan(
    db: &sea_orm::DatabaseConnection,
    registered: u32,
    failed_batches: u32,
) -> Result<(), String> {
    use axagent_entities::market_universe_stats;
    let last = market_universe_stats::Entity::find()
        .order_by_desc(market_universe_stats::Column::ObservedAt)
        .one(db)
        .await
        .map_err(|e| format!("读覆盖率读数失败: {e}"))?;
    let Some(last) = last.filter(|r| r.reported_total > 0) else {
        return Ok(());
    };
    let universe_size = market_stock_universe::Entity::find()
        .count(db)
        .await
        .map_err(|e| format!("统计清单失败: {e}"))? as i64;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let am = market_universe_stats::ActiveModel {
        observed_at: Set(now_ms),
        universe_size: Set(universe_size as i32),
        reported_total: Set(last.reported_total),
        coverage_pct: Set(
            ((universe_size as f64 / last.reported_total as f64) * 10000.0).round() / 100.0
        ),
        pages_requested: Set(0),
        pages_failed: Set(failed_batches as i32),
        rows_upserted: Set(registered as i32),
        created_at: Set(now_ms),
    };
    market_universe_stats::Entity::insert(am)
        .exec(db)
        .await
        .map_err(|e| format!("覆盖率读数落库失败: {e}"))?;
    Ok(())
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
    let mut skipped_suspended = 0usize;

    for chunk in codes.chunks(batch_size) {
        batches += 1;
        match client.market_snapshot_batch(chunk).await {
            Ok(quotes) => {
                rows_returned += quotes.len();
                for q in quotes {
                    let row_id = format!("{}@{trade_date}", q.code);
                    // 停牌/无成交：涨幅 0 且量、额都拿不到。不能落 0% 行 —— 那会把
                    // 「拿不到」伪装成「没涨」，还会把停牌日当作交易日填进窗口。
                    // 显式写有限性，别用 `!(x > 0.0)`：NaN/INF 都要归到「拿不到」，
                    // 而 `x > 0.0` 对 +INF 为 true、对 NaN 为 false，取反后语义分裂。
                    let amount_missing = !q.amount.is_finite() || q.amount <= 0.0;
                    let volume_missing = !q.volume.is_finite() || q.volume <= 0.0;
                    let no_trade = amount_missing && volume_missing;
                    if q.change_pct == 0.0 && no_trade {
                        skipped_suspended += 1;
                        continue;
                    }
                    let Some(am) = close_row(
                        &q.code,
                        &q.name,
                        trade_date,
                        now_ms,
                        q.price,
                        q.pre_close,
                        q.change_pct,
                        q.amount,
                        q.volume,
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
                                    market_daily_close::Column::Amount,
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
        skipped_suspended,
        failed_batches,
        missing_quotes: universe_size.saturating_sub(rows_returned),
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
    let latest_close_codes = rows_per_date.last().copied().unwrap_or(0);
    let reported_total = market_universe_stats::Entity::find()
        .order_by_desc(market_universe_stats::Column::ObservedAt)
        .one(db)
        .await
        .map_err(|e| format!("读全市场申报总数失败: {e}"))?
        .map(|r| r.reported_total)
        .unwrap_or(0);

    Ok(CoverageData {
        universe_size: universe_size as i64,
        confirmed_by_clist: confirmed_by_clist as i64,
        reported_total,
        close_dates,
        rows_per_date,
        latest_close_codes,
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
        let am =
            close_row("600059", "古越龙山", "2026-09-30", 1, 12.27, 11.15, 10.04, 1.23e8, 1.0e7)
                .expect("有效行情应落行");
        assert_eq!(am.id.unwrap(), "600059@2026-09-30");
        assert_eq!(am.change_pct.unwrap(), 10.04);
        assert_eq!(am.amount.unwrap(), 1.23e8, "成交额必须落库（P-m ① 的数据源）");
        assert_eq!(am.source.unwrap(), "tencent_batch", "真值行不得带推定后缀");
    }

    #[test]
    fn suspended_or_zero_prev_close_is_skipped_not_zeroed() {
        // 停牌/缺昨收 ⇒ None（不落行），绝不能写成 change_pct=0 的"没涨"
        assert!(
            close_row("600059", "古越龙山", "2026-09-30", 1, 12.27, 0.0, 0.0, 0.0, 0.0).is_none()
        );
        assert!(
            close_row("600059", "古越龙山", "2026-09-30", 1, 0.0, 11.15, 10.04, 0.0, 0.0).is_none()
        );
        assert!(close_row("600059", "古越龙山", "2026-09-30", 1, f64::NAN, 11.15, 10.04, 0.0, 0.0)
            .is_none());
        assert!(close_row("", "x", "2026-09-30", 1, 1.0, 1.0, 1.0, 0.0, 0.0).is_none());
    }

    /// C-3：源不给成交额时用 close × volume 推定（单位同为元），并打 `+amount_est` 后缀。
    /// 没有这条，`amount=0` 的股票在流动门上恒走「未采集」分支 ⇒ 流动性维度长期空转。
    #[test]
    fn missing_amount_is_estimated_from_close_times_volume() {
        let am = close_row("600059", "古越龙山", "2026-09-30", 1, 12.0, 11.0, 9.09, 0.0, 1.0e7)
            .expect("有价格与成交量就应落行");
        assert_eq!(am.amount.unwrap(), 1.2e8, "12.0 元 × 1e7 股 = 1.2e8 元");
        assert_eq!(am.source.unwrap(), "tencent_batch+amount_est", "推定值必须可与真值分辨");
    }

    /// 负控：真值在 ⇒ 绝不改成推定值（即便 volume 也在）。
    #[test]
    fn source_amount_wins_over_estimate() {
        let am = close_row("600059", "古越龙山", "2026-09-30", 1, 12.0, 11.0, 9.09, 5.0e7, 1.0e7)
            .expect("应落行");
        assert_eq!(am.amount.unwrap(), 5.0e7, "真值优先");
        assert_eq!(am.source.unwrap(), "tencent_batch");
    }

    /// 「两样都没有」仍是 0 = 未采集（读侧跳过流动性判据），不得用别的数假装有值。
    #[test]
    fn missing_both_keeps_zero_amount_and_base_source() {
        let am = close_row("600059", "古越龙山", "2026-09-30", 1, 12.0, 11.0, 9.09, 0.0, 0.0)
            .expect("价格有效仍应落行");
        assert_eq!(am.amount.unwrap(), 0.0);
        assert_eq!(am.source.unwrap(), "tencent_batch");
    }

    /// 负控：NaN 成交量不得被当成「有量」算出 NaN 成交额（NaN 会让 ≥1亿 判据恒假且无告警）。
    #[test]
    fn nan_volume_falls_back_to_uncollected() {
        let am = close_row("600059", "古越龙山", "2026-09-30", 1, 12.0, 11.0, 9.09, 0.0, f64::NAN)
            .expect("价格有效仍应落行");
        assert_eq!(am.amount.unwrap(), 0.0);
    }
}
