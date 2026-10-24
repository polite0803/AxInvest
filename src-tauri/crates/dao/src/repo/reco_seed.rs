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
    let latest = market_daily_close::Entity::find()
        .filter(market_daily_close::Column::TradeDate.lte(day.to_string()))
        .order_by_desc(market_daily_close::Column::TradeDate)
        .one(db)
        .await?
        .map(|r| r.trade_date);
    match latest {
        Some(d) => top_close_rows_by_change(db, &d, limit).await,
        None => Ok(Vec::new()),
    }
}
