use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// 进化闭环留痕：每次股票进化执行的触发原因、资源与结果（参数/流程）。
/// 供前端事后回看"某次进化改了啥、为何触发"。
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "stock_evolution_history")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 进化计划 ID
    #[sea_orm(indexed)]
    pub plan_id: String,
    /// 触发原因（EvolutionTrigger 的 JSON 序列化）
    pub trigger: String,
    /// 进化类型："parameter" | "workflow" | "hybrid"
    pub evolution_type: String,
    /// 进化前质量分
    pub quality_before: u8,
    /// 参数进化结果（EvolutionResult JSON，Option）
    pub parameter_result_json: Option<String>,
    /// 流程进化结果（WorkflowModification JSON，含 genome）
    pub workflow_result_json: Option<String>,
    /// 执行状态："success" | "failed"
    pub status: String,
    /// 改进说明
    pub improvement_summary: String,
    /// 进化执行时间（ms）
    pub created_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
