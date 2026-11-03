// SPDX-License-Identifier: AGPL-3.0-only
//! `trades` 表「同键查重」的唯一持久化实现。
//!
//! ## 为什么从命令层搬下来
//!
//! `trading.engine` 接缝的宿主实现 `commands/stock_workflow/trading_engine_service.rs`
//! 原先自己写 `trades::Entity::find()` 加六个 `.filter(...)` ⇒ 命令层出现 `sea_orm::` /
//! `axagent_entities::` 字面量，命中分层门禁 [`scripts/check-layer-discipline.mjs`]
//! 规则 1（`commands-no-direct-db`，该文件基线为 0）。
//! 与 `price_alert.rs` 同一处置：SQL 与实体引用收在 dao，命令层只做「命中即按
//! `duplicate` 拒绝」的编排与错误映射。
//!
//! dao 不能依赖 `axagent_harness`（接缝契约类型在对面），故六个查重键一律由调用方
//! 拆成标量传入，本文件不引入 `TradeExecutionRequest`。

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

use axagent_entities::trades;

/// 同键查重：键 = 股票 + 方向 + 日期 + 时间 + 价格 + 数量（**六字段**）。
///
/// 为什么要在 dao 而不是命令层：`scripts/check-layer-discipline.mjs` 规则 1 禁止
/// `src/commands/**` 出现 `sea_orm::` / `axagent_entities::` 字面量（新文件基线为 0），
/// 而该判据必须逐列比对 `trades::Column`，留在命令层就只能违反门禁；错误文案由调用方
/// 决定，本函数只原样回 `sea_orm::DbErr`。
///
/// 与 CSV 导入的 `is_duplicate_trade`
/// （`axagent-quant/crates/analysis-engine/src/trade_import.rs:431-447`）口径逐字一致：
/// 两条写入路径必须认同一套「同一笔交易」。否则同一笔单子 CSV 导一次、插件再录一次
/// 就能重复入账。`trades` 表无唯一约束（加约束要写 migration），故这里只能做
/// **尽力而为**的查重：窗口期内两笔真重复的、并发绕过该检查的，仍会入库。
///
/// 返回 `bool` 而不是行：调用方只判存在性。搬迁前后同为「六个等值 filter + `.one()` +
/// `is_some()`」，原查询没有 `order_by` / `limit`，故不存在排序与分页形状的变化。
pub async fn exists_same_key(
    db: &DatabaseConnection,
    stock_code: &str,
    direction: &str,
    trade_date: &str,
    trade_time: &str,
    price: f64,
    quantity: i32,
) -> Result<bool, sea_orm::DbErr> {
    let existing = trades::Entity::find()
        .filter(trades::Column::StockCode.eq(stock_code))
        .filter(trades::Column::Direction.eq(direction))
        .filter(trades::Column::TradeDate.eq(trade_date))
        .filter(trades::Column::TradeTime.eq(trade_time))
        .filter(trades::Column::Price.eq(price))
        .filter(trades::Column::Quantity.eq(quantity))
        .one(db)
        .await?;

    Ok(existing.is_some())
}
