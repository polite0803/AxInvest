use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// OPC 强化学习训练统计
///
/// 每个域包的 RL 训练状态汇总，用于快速查询统计数据
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "opc_rl_training_stats")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub domain_pack_id: String,
    #[sea_orm(default_value = 0)]
    pub total_experiences: i32,
    #[sea_orm(default_value = 0.0)]
    pub total_reward: f64,
    #[sea_orm(default_value = 0.0)]
    pub avg_reward: f64,
    #[sea_orm(default_value = 0.0)]
    pub success_rate: f64,
    pub last_trained_at: Option<i64>,
    pub policy_updated_at: Option<i64>,
    #[sea_orm(default_value = "[]")]
    pub optimization_goals: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
