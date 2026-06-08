use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "stock_analyses")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub stock_code: String,
    pub stock_name: String,
    pub analysis_date: String,
    pub provider_id: String,
    pub conversation_id: String,
    pub status: String,
    pub decision_action: Option<String>,
    pub decision_position_pct: Option<f64>,
    pub decision_reasoning: Option<String>,
    pub decision_json: Option<String>,
    pub blackboard_snapshot: Option<String>,
    pub config_id: Option<String>,
    /// Time-travel mode: 'live' | 'replay' | 'ab_test'
    #[sea_orm(default_value = "live")]
    pub analysis_kind: String,
    /// Time-travel mode: replay 模式的数据截止日 (YYYY-MM-DD)
    pub as_of_date: Option<String>,
    /// 决策所用 LLM 的版本标识（用于复现实验）
    pub model_version: Option<String>,
    /// 关联到 L2 disk-cache 的快照 ID
    pub data_snapshot_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
