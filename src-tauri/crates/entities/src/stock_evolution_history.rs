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
    /// 执行状态：
    /// `"pending_approval"` = 定时聚合产出、待人工审批（P4 两段式）；
    /// `"accepted"` = 人工接受并已执行回写；
    /// `"rejected"` = 人工拒绝（含拒绝原因）或接受后被回滚；
    /// 历史值 `"success" | "failed"`（P4 前直接执行期写入，只读保留）。
    pub status: String,
    /// 改进说明
    pub improvement_summary: String,
    /// 触发样本留痕（P4）：JSON 结构
    /// `{reflectionIds, validationIds, moverRunIds（驱动进化的业务样本 id）,
    ///   dataBlocked: {reflectionIds, moverRunIds}（数据可达性失败、只进数据质量看板的样本）,
    ///   counts: {reflections, validations, moverMisses, dataBlocked}}`。
    /// NULL = P4 前的历史行。
    pub trigger_sample_json: Option<String>,
    /// 审批人（命令入参，桌面端缺省 local-user）。NULL = 尚未审批。
    pub reviewer: Option<String>,
    /// 审批时间（ms）。NULL = 尚未审批。
    pub reviewed_at: Option<i64>,
    /// 拒绝原因（拒绝时必填）；回滚时写 `rolled back to evo-xxx`。
    pub reject_reason: Option<String>,
    /// 回滚目标链：接受本计划时盖的「上一条 accepted 行 id」，用于一键回滚；
    /// 回滚新插行沿用目标版本的本字段，保持链条不断。
    pub rollback_ref: Option<String>,
    /// 进化执行时间（ms）
    pub created_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
