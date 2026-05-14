use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// 股票定时分析调度计划
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "analysis_schedules")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub stock_code: String,
    pub stock_name: String,
    /// cron 表达式（5字段: 分 时 日 月 周），如 "0 9 * * 1-5"
    pub cron_expression: String,
    pub provider_id: String,
    pub is_enabled: bool,
    pub last_run_at: Option<i64>,
    pub next_run_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
