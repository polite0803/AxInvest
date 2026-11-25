// SPDX-License-Identifier: AGPL-3.0-only

//! `bot_cost_quotas` —— Bot / 部门费用配额（docs/bot-integration/P5/MODEL-P5.md §3）。
//!
//! `UNIQUE(scope, scope_id)` 由 `reconcile/extras.rs` 的 `MULTI_COL_INDEXES`
//! 声明。`0 = 不限`（与 `BudgetConfig` 语义一致，ADR-P5-3）。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[sea_orm(table_name = "bot_cost_quotas")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// `bot` / `department`
    pub scope: String,
    /// scope 对应的 id（bot id 或 department id）
    pub scope_id: String,
    /// 窗口内 token 上限；0 = 不限
    #[sea_orm(default_value = 0)]
    pub max_total_tokens: i64,
    /// 窗口内费用上限（估算 USD）；0 = 不限
    #[sea_orm(default_value = 0.0)]
    pub max_cost_usd: f64,
    /// 计量窗口（小时）；默认 168 = 7 天
    #[sea_orm(default_value = 168)]
    pub window_hours: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
