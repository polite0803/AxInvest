// SPDX-License-Identifier: AGPL-3.0-only

//! 荐股候选池的表侧来源（用户标的 / 收盘快照涨幅榜）。
//!
//! 放在 dao 而不是命令层：`src/commands/**` 有分层棘轮（不得直连 sea_orm），而这三条
//! 都是纯表读取、没有业务判断。失败语义留给调用方 —— 各来源独立 fail-open，不因一条
//! 查询失败把整批候选清零。

use axagent_entities::{market_daily_close, market_stock_universe, watchlist_items};
use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use std::collections::HashMap;

/// 自选股 `(code, name)`。
pub async fn load_watchlist_items(db: &DatabaseConnection) -> Result<Vec<(String, String)>, DbErr> {
    let rows = watchlist_items::Entity::find().all(db).await?;
    Ok(rows.into_iter().map(|r| (r.stock_code, r.stock_name)).collect())
}

/// `market_stock_universe` 里 `origin='local'` 的本机标的 `(code, name)`。
pub async fn load_local_universe_items(
    db: &DatabaseConnection,
) -> Result<Vec<(String, String)>, DbErr> {
    let rows = market_stock_universe::Entity::find()
        .filter(market_stock_universe::Column::Origin.eq("local"))
        .all(db)
        .await?;
    Ok(rows.into_iter().map(|r| (r.stock_code, r.stock_name)).collect())
}

/// 收盘快照里最新的交易日（无快照时为 None）。
pub async fn latest_close_trade_date(db: &DatabaseConnection) -> Result<Option<String>, DbErr> {
    Ok(market_daily_close::Entity::find()
        .order_by_desc(market_daily_close::Column::TradeDate)
        .one(db)
        .await?
        .map(|r| r.trade_date))
}

/// 某交易日按涨幅降序的收盘快照 `(code, name)`；ST 过滤留给调用方（按名称判定）。
///
/// 流动性门（2026-10-20，存档带上 `amount` 之后）：与建池 `amount < 1 亿 ⇒ 丢` 同口径，
/// 但 `amount = 0` 是**存量行未采集**（该列 2026-10-20 才加）⇒ 放行 —— 把老数据判成
/// 「没有流动性」会让涨幅榜来源在列新增后的第一天整批消失。
pub async fn top_close_rows_by_change(
    db: &DatabaseConnection,
    day: &str,
    limit: u64,
) -> Result<Vec<(String, String)>, DbErr> {
    let rows = market_daily_close::Entity::find()
        .filter(market_daily_close::Column::TradeDate.eq(day))
        .filter(
            Condition::any()
                .add(market_daily_close::Column::Amount.eq(0.0))
                .add(market_daily_close::Column::Amount.gte(100_000_000.0)),
        )
        .order_by_desc(market_daily_close::Column::ChangePct)
        .limit(limit)
        .all(db)
        .await?;
    Ok(rows.into_iter().map(|r| (r.stock_code, r.stock_name)).collect())
}

/// **as-of 版涨幅榜**：截至 `day`（含）最近一个有收盘快照的交易日，按涨幅降序取前 N。
///
/// 与 [`top_close_rows_by_change`] 的差别只有一处：调用方给的是窗口起点，那个日期
/// 可能不是交易日（或当天快照尚未落入），直接精确匹配会返回空表并把 as-of 回放的
/// 涨幅榜来源静默清零。这里先反查「≤ day 的最近交易日」，复用同一套流动性谓词，
/// **不引入第二套取数口径**。
pub async fn top_close_rows_by_change_on_or_before(
    db: &DatabaseConnection,
    day: &str,
    limit: u64,
) -> Result<Vec<(String, String)>, DbErr> {
    match latest_close_date_on_or_before(db, Some(day)).await? {
        Some(d) => top_close_rows_by_change(db, &d, limit).await,
        None => Ok(Vec::new()),
    }
}

/// `day`（含）之前最近一个有收盘快照的交易日；`day = None` ⇒ 全局最新。
///
/// 两条读路径（涨幅榜、全市场历史）共用它：各自反查一次就会出现「涨幅榜取 10-30、历史取
/// 10-29」的错位回放，而响应里的 `as_of_date` 只有一个。估值截面用历史查出的那个日期
/// 显式取（[`load_valuations_on_date`]），同样不许自己再反查。
pub async fn latest_close_date_on_or_before(
    db: &DatabaseConnection,
    day: Option<&str>,
) -> Result<Option<String>, DbErr> {
    let mut q = market_daily_close::Entity::find();
    if let Some(d) = day {
        q = q.filter(market_daily_close::Column::TradeDate.lte(d.to_string()));
    }
    Ok(q.order_by_desc(market_daily_close::Column::TradeDate).one(db).await?.map(|r| r.trade_date))
}

/// `trade_date` 那一天的 `code → (pe_ttm, pb)`（W3b，2026-11-01）。
///
/// 只返回**至少一列非空**的行：调用方用 `map.len() / 快照只数` 算覆盖率，把「有行但两列
/// 都 NULL」算进分子会让 0 覆盖被判成有覆盖。
///
/// 日期由调用方给（而不是内部再反查一次截止日）：同一轮的收盘历史与估值截面**必须是同一个
/// 交易日**，两次独立反查之间若落了新一天，就会出现「涨幅榜取 10-30、估值取 10-29」这种
/// 响应里只有一个 `as_of_date` 掩盖不住的错位。
///
/// ⚠ 这两列 2026-11-01 才加入 ⇒ 之前的历史行是 NULL ⇒ 对旧日期返回空表（如实缺席，不做
/// 「最近一个有值的日期」回填 —— 那会把后一天的估值贴到前一天的候选上）。
pub async fn load_valuations_on_date(
    db: &DatabaseConnection,
    trade_date: &str,
) -> Result<HashMap<String, (Option<f64>, Option<f64>)>, DbErr> {
    let rows = market_daily_close::Entity::find()
        .select_only()
        .column(market_daily_close::Column::StockCode)
        .column(market_daily_close::Column::PeTtm)
        .column(market_daily_close::Column::Pb)
        .filter(market_daily_close::Column::TradeDate.eq(trade_date.to_string()))
        .filter(
            Condition::any()
                .add(market_daily_close::Column::PeTtm.is_not_null())
                .add(market_daily_close::Column::Pb.is_not_null()),
        )
        .into_tuple::<(String, Option<f64>, Option<f64>)>()
        .all(db)
        .await?;
    Ok(rows.into_iter().map(|(code, pe, pb)| (code, (pe, pb))).collect())
}

/// 全市场收盘历史的窄投影（W3，2026-11-01）：
/// `(code, name, trade_date, close, amount, change_pct)`。
///
/// 只取最近 `trade_days` 个**交易日**（不是日历日），实盘取最新交易日、as-of 取
/// `day`（含）之前最近的一个交易日 —— 与 [`top_close_rows_by_change_on_or_before`]
/// 同一条代际纪律：回放绝不读今天之后的行情。
///
/// 为什么不在 SQL 里算 20 日收益/回撤：窗口函数会引入第二套口径，且 `check-sql-dialect`
/// 对 PG 专有语法有额外约束；把原始收盘交给 consumer crate 的纯函数计算，单测可直接喂表。
pub async fn load_close_history_on_or_before(
    db: &DatabaseConnection,
    day: Option<&str>,
    trade_days: usize,
) -> Result<(Option<String>, Vec<(String, String, String, f64, f64, f64)>), DbErr> {
    let Some(latest) = latest_close_date_on_or_before(db, day).await? else {
        return Ok((None, Vec::new()));
    };

    let dates: Vec<String> = market_daily_close::Entity::find()
        .select_only()
        .column(market_daily_close::Column::TradeDate)
        .filter(market_daily_close::Column::TradeDate.lte(latest.clone()))
        .distinct()
        .order_by_desc(market_daily_close::Column::TradeDate)
        .limit(trade_days as u64)
        .into_tuple::<String>()
        .all(db)
        .await?;
    if dates.is_empty() {
        return Ok((Some(latest), Vec::new()));
    }

    let rows = market_daily_close::Entity::find()
        .select_only()
        .column(market_daily_close::Column::StockCode)
        .column(market_daily_close::Column::StockName)
        .column(market_daily_close::Column::TradeDate)
        .column(market_daily_close::Column::Close)
        .column(market_daily_close::Column::Amount)
        .column(market_daily_close::Column::ChangePct)
        .filter(market_daily_close::Column::TradeDate.is_in(dates))
        .into_tuple::<(String, String, String, f64, f64, f64)>()
        .all(db)
        .await?;
    Ok((Some(latest), rows))
}

/// `stock_code → industry`（东财 f100 词表；空值不写入，保持「缺席」可区分）。
pub async fn load_universe_industries(
    db: &DatabaseConnection,
) -> Result<HashMap<String, String>, DbErr> {
    let rows = market_stock_universe::Entity::find()
        .select_only()
        .column(market_stock_universe::Column::StockCode)
        .column(market_stock_universe::Column::Industry)
        .into_tuple::<(String, Option<String>)>()
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(code, industry)| industry.filter(|i| !i.is_empty()).map(|i| (code, i)))
        .collect())
}
