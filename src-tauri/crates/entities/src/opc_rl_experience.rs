use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// OPC 强化学习经验记录
///
/// 每次工作流执行后记录的经验数据，用于 RL 经验池积累
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "opc_rl_experiences")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    #[sea_orm(indexed)]
    pub domain_pack_id: String,
    #[sea_orm(indexed)]
    pub workflow_id: String,
    #[sea_orm(indexed)]
    pub timestamp_ms: i64,
    #[sea_orm(default_value = 0.0)]
    pub quality_score: f64,
    #[sea_orm(default_value = 0.0)]
    pub efficiency_score: f64,
    #[sea_orm(default_value = 0.0)]
    pub cost_score: f64,
    #[sea_orm(default_value = 0.0)]
    pub innovation_score: f64,
    #[sea_orm(default_value = 0.0)]
    pub satisfaction_score: f64,
    #[sea_orm(default_value = 0.0)]
    pub total_reward: f64,
    #[sea_orm(default_value = 0)]
    pub step_count: i32,
    #[sea_orm(default_value = false)]
    pub success: bool,
    #[sea_orm(default_value = "{}")]
    pub metadata: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
