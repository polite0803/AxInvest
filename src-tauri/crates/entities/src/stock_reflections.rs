use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "stock_reflections")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub stock_code: String,
    pub stock_name: String,
    /// 原始分析的 ID（关联 stock_analyses.id）
    pub original_analysis_id: String,
    /// as-of 时间（原始分析日期，YYYY-MM-DD）
    pub as_of_date: String,
    /// 后见信息时间（校验/反思触发日期，YYYY-MM-DD）
    pub hindsight_date: String,
    /// 反思触发时的置信度阈值
    pub min_confidence_threshold: i32,
    /// 反思深度：light | deep（deep 会详述 reasoning chain）
    pub reflection_depth: String,
    /// 实际走势描述，如 "30天跌-8.3% → 失败"
    pub actual_outcome: String,
    /// 反思摘要：错因
    pub what_went_wrong: Option<String>,
    /// 反思摘要：被忽视的信号（JSON 数组字符串）
    pub missed_signals: Option<String>,
    /// 反思摘要：改进建议
    pub fix_for_future: Option<String>,
    /// portfolio-manager 完整输出 JSON
    pub decision_json: Option<String>,
    /// 工作流完整结果（用于追溯）
    pub blackboard_snapshot: Option<String>,
    /// 所用 LLM 的版本标识
    pub model_version: Option<String>,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
