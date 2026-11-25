// SPDX-License-Identifier: AGPL-3.0-only

//! `bot_usage_ledger` —— Bot 用量账本（docs/bot-integration/P5/MODEL-P5.md §2）。
//!
//! append-only：每轮一行，写入时固化四类 token、估算费用与归因快照；
//! 不 UPDATE、不 DELETE（ADR-P5-2）。账本与会话生命周期解耦。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[sea_orm(table_name = "bot_usage_ledger")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub bot_id: String,
    pub department_id: Option<String>,
    pub conversation_id: Option<String>,
    pub execution_id: Option<String>,
    pub workflow_run_id: Option<String>,
    pub assignment_id: Option<String>,
    pub model_id: String,
    #[sea_orm(default_value = "routine")]
    pub origin: String,
    #[sea_orm(default_value = 0)]
    pub input_tokens: i64,
    #[sea_orm(default_value = 0)]
    pub output_tokens: i64,
    #[sea_orm(default_value = 0)]
    pub cache_creation_tokens: i64,
    #[sea_orm(default_value = 0)]
    pub cache_read_tokens: i64,
    #[sea_orm(default_value = 0)]
    pub total_tokens: i64,
    #[sea_orm(default_value = 0.0)]
    pub estimated_cost_usd: f64,
    #[sea_orm(default_value = "heuristic")]
    pub pricing_source: String,
    pub created_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
