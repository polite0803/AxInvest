use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// 复盘→进化：每次权重调整的全量留痕。
/// 包含旧/新权重、变化幅度、触发原因、样本量、胜率、可读解释。
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "strategy_weight_history")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    #[sea_orm(indexed)]
    pub strategy_id: String,
    pub period: String,
    /// 旧权重（1.0 为基准）
    pub old_weight: f64,
    /// 外乘线（mover/熔断）之前的基准权重；NULL = 该列引入前的旧行（按 new_weight 兼容）。
    /// 下一轮 EWMA 以它为输入 ⇒ 同一批证据重复重算不会复利衰减。
    pub base_weight: Option<f64>,
    /// 本轮外乘惩罚（mover × 熔断）；NULL = 旧行/无外乘线。
    pub outer_penalty: Option<f64>,
    /// 新权重（生效值 = base_weight × outer_penalty）
    pub new_weight: f64,
    /// 变化百分比
    pub delta_pct: f64,
    /// 触发类型："cron" | "manual" | "rule"
    pub trigger: String,
    /// 触发的 reflection 行 ID（cron 触发时填写，manual 可空）
    pub source_reflection_id: Option<String>,
    /// 计算权重时所基于的 strategy_performance 行数
    pub sample_size: i32,
    /// 计算时的胜率（0-1）
    pub win_rate: f64,
    /// 可读解释（模板或 LLM 生成的归因）
    pub rationale: Option<String>,
    /// 权重应用时间（ms）
    pub applied_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
