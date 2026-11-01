// SPDX-License-Identifier: AGPL-3.0-only
//! `price_alerts` 表「同股同类型未触发行复用」的写入 —— 持久化边界的唯一实现。
//!
//! ## 为什么从命令层搬下来
//!
//! 面板手建、agent 工具 `stock_create_price_alert`、工作流分析后自动挂单三条写入路共用
//! `commands/price_alert_upsert.rs` 这个唯一入口（裁定 10），而该入口原先自己构造
//! `price_alerts::ActiveModel` 并 `find/update/insert` ⇒ 命令层出现 `sea_orm::` /
//! `axagent_entities::` 字面量，命中分层门禁 [`scripts/check-layer-discipline.mjs`]
//! 规则 1（`commands-no-direct-db`，新文件基线为 0）。
//! 与 `reco_pool.rs` 同一处置：SQL 与实体构造收在 dao，命令层只做 alert_type 归一化
//! （在 `axagent_analysis_engine::alert_mapping`）与「复用还是新建」的编排。
//!
//! dao 不能依赖 analysis-engine（它反向依赖本 crate，会成环），故归一化结果
//! （`alert_type` / `legacy_condition` / `condition_type`）一律由调用方算好后以标量传入。
//!
//! ## 复用语义（调用方不得绕过本文件各写各的）
//!
//! - 未触发行按 `(stock_code, alert_type)` 取**最新一条**复用并更新阈值，`id` 不变；
//! - 已触发行是终态，不参与复用（新一轮监控从一条新行开始）。

use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
};

use axagent_entities::price_alerts;

/// 价格告警行；命令层经此别名引用，避免在 `commands/` 出现 `axagent_entities::` 字面量。
pub type AlertRow = price_alerts::Model;

/// 查同一 `(stock_code, alert_type)` 的最新一条**未触发**行；没有则 `None`。
pub async fn find_open_untriggered(
    db: &DatabaseConnection,
    stock_code: &str,
    alert_type: &str,
) -> Result<Option<AlertRow>, sea_orm::DbErr> {
    price_alerts::Entity::find()
        .filter(price_alerts::Column::StockCode.eq(stock_code))
        .filter(price_alerts::Column::AlertType.eq(alert_type))
        .filter(price_alerts::Column::IsTriggered.eq(0))
        .order_by_desc(price_alerts::Column::CreatedAt)
        .one(db)
        .await
}

/// 列出某只股票的**全部**告警行（含已触发），按创建时间倒序 —— 去重口径的对照读。
///
/// 判据用它在两轮 upsert 之后数行数：只看未触发行会漏掉「已触发行被新单复活」这种缺陷。
pub async fn find_by_stock(
    db: &DatabaseConnection,
    stock_code: &str,
) -> Result<Vec<AlertRow>, sea_orm::DbErr> {
    price_alerts::Entity::find()
        .filter(price_alerts::Column::StockCode.eq(stock_code))
        .order_by_desc(price_alerts::Column::CreatedAt)
        .all(db)
        .await
}

/// 复用既有未触发行：更新名称 / 兼容列方向 / 双阈值与更新时间，`id` 不变。
///
/// 入参 `row` 即 [`find_open_untriggered`] 返回的行 —— 命令层据此保留「复用」分支，
/// 实体到 `ActiveModel` 的转换仍只在 dao 内发生。
pub async fn reuse_open_alert(
    db: &DatabaseConnection,
    row: AlertRow,
    stock_name: &str,
    legacy_condition: &str,
    condition_type: &str,
    threshold: f64,
    now: i64,
) -> Result<AlertRow, sea_orm::DbErr> {
    let mut active: price_alerts::ActiveModel = row.into();
    active.stock_name = Set(stock_name.to_string());
    active.condition = Set(legacy_condition.to_string());
    active.target_price = Set(threshold);
    active.condition_type = Set(Some(condition_type.to_string()));
    active.threshold = Set(Some(threshold));
    active.updated_at = Set(now);
    active.update(db).await
}

/// 新建一条未触发告警行（新一轮监控周期的起点）。
///
/// `target_price` 与 `threshold` 同源同值：老列继续写以兼容仍在读它的代码。
#[allow(clippy::too_many_arguments)]
pub async fn insert_open_alert(
    db: &DatabaseConnection,
    stock_code: &str,
    stock_name: &str,
    alert_type: &str,
    legacy_condition: &str,
    condition_type: &str,
    threshold: f64,
    now: i64,
) -> Result<AlertRow, sea_orm::DbErr> {
    let model = price_alerts::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        stock_code: Set(stock_code.to_string()),
        stock_name: Set(stock_name.to_string()),
        condition: Set(legacy_condition.to_string()),
        target_price: Set(threshold),
        alert_type: Set(Some(alert_type.to_string())),
        condition_type: Set(Some(condition_type.to_string())),
        threshold: Set(Some(threshold)),
        is_triggered: Set(0),
        triggered_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };
    model.insert(db).await
}
