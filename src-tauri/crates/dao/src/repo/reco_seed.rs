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
