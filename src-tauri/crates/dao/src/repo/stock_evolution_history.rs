// SPDX-License-Identifier: AGPL-3.0-only
//! `stock_evolution_history` 的读写收口 —— P4「计划-审批两段式」的唯一持久化边界。
//!
//! 命令层（`commands::evolution_proposal` 聚合产计划 / `commands::stock_evolution_approval`
//! 审批 / 拒绝 / 回滚）不得直连实体与 sea_orm（分层护栏规则 1），全部经本原语；
//! 条件 UPDATE 的幂等闸门（`status = 'pending_approval'` / `'accepted'`）也收口在这里，
//! 调用方只按返回的 `rows_affected` 判幂等冲突。

/// 进化历史行类型重导出（审批命令的返回类型走 dao 合规出口）。
pub use axagent_entities::stock_evolution_history::Model as EvolutionHistoryModel;
use axagent_entities::stock_evolution_history::{ActiveModel, Column, Entity};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, DatabaseConnection, DbErr, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
};

/// 待审批
pub const STATUS_PENDING_APPROVAL: &str = "pending_approval";
/// 已接受（参数已生效）
pub const STATUS_ACCEPTED: &str = "accepted";
/// 已拒绝（含接受后被回滚）
pub const STATUS_REJECTED: &str = "rejected";

/// 定时聚合产出的待审批计划（插入时固定 `parameter` 类、无执行结果）。
#[derive(Debug, Clone)]
pub struct PendingProposalRecord {
    pub id: String,
    pub plan_id: String,
    /// EvolutionTrigger 的 JSON 序列化
    pub trigger_json: String,
    pub improvement_summary: String,
    /// 触发样本留痕（业务样本 / 数据可达性失败样本分桶）
    pub trigger_sample_json: String,
    pub created_at_ms: i64,
}

/// 回滚时新插的 accepted 行（参数结果复制自回滚目标版本）。
#[derive(Debug, Clone)]
pub struct AcceptedHistoryRecord {
    pub id: String,
    pub plan_id: String,
    pub trigger_json: String,
    pub evolution_type: String,
    pub parameter_result_json: Option<String>,
    pub workflow_result_json: Option<String>,
    pub improvement_summary: String,
    pub trigger_sample_json: Option<String>,
    pub reviewer: String,
    /// 沿用目标版本的回滚链（再上一代）
    pub rollback_ref: Option<String>,
    pub created_at_ms: i64,
}

/// 落一条待审批计划（固定 `parameter` 类、无参数/流程结果、无审批字段）。
pub async fn insert_pending_proposal(
    db: &DatabaseConnection,
    r: &PendingProposalRecord,
) -> Result<(), DbErr> {
    let row = ActiveModel {
        id: Set(r.id.clone()),
        plan_id: Set(r.plan_id.clone()),
        trigger: Set(r.trigger_json.clone()),
        evolution_type: Set("parameter".to_string()),
        quality_before: Set(0),
        parameter_result_json: Set(None),
        workflow_result_json: Set(None),
        status: Set(STATUS_PENDING_APPROVAL.to_string()),
        improvement_summary: Set(r.improvement_summary.clone()),
        trigger_sample_json: Set(Some(r.trigger_sample_json.clone())),
        reviewer: Set(None),
        reviewed_at: Set(None),
        reject_reason: Set(None),
        rollback_ref: Set(None),
        created_at: Set(r.created_at_ms),
    };
    Entity::insert(row).exec(db).await?;
    Ok(())
}

/// 窗口内待审批计划计数（防每晚重复堆计划的防抖闸口）。
pub async fn count_pending_since(db: &DatabaseConnection, since_ms: i64) -> Result<u64, DbErr> {
    Entity::find()
        .filter(Column::Status.eq(STATUS_PENDING_APPROVAL))
        .filter(Column::CreatedAt.gte(since_ms))
        .count(db)
        .await
}

/// 待审批计划列表（创建时间升序，先到先审）。
pub async fn list_pending_asc(
    db: &DatabaseConnection,
) -> Result<Vec<EvolutionHistoryModel>, DbErr> {
    Entity::find()
        .filter(Column::Status.eq(STATUS_PENDING_APPROVAL))
        .order_by_asc(Column::CreatedAt)
        .all(db)
        .await
}

/// 按主键取单条历史行。
pub async fn find_by_id(
    db: &DatabaseConnection,
    history_id: &str,
) -> Result<Option<EvolutionHistoryModel>, DbErr> {
    Entity::find_by_id(history_id.to_owned()).one(db).await
}

/// 最近一条 accepted 行（可选排除某行 id）；无则 None。
pub async fn latest_accepted_id(
    db: &DatabaseConnection,
    exclude_id: Option<&str>,
) -> Result<Option<String>, DbErr> {
    let mut q =
        Entity::find().filter(Column::Status.eq(STATUS_ACCEPTED)).order_by_desc(Column::CreatedAt);
    if let Some(ex) = exclude_id {
        q = q.filter(Column::Id.ne(ex.to_owned()));
    }
    q.one(db).await.map(|r| r.map(|m| m.id))
}

/// 最近一条 accepted 整行（回滚命令未显式指定版本时的当前生效版本）。
pub async fn latest_accepted_row(
    db: &DatabaseConnection,
) -> Result<Option<EvolutionHistoryModel>, DbErr> {
    Entity::find()
        .filter(Column::Status.eq(STATUS_ACCEPTED))
        .order_by_desc(Column::CreatedAt)
        .one(db)
        .await
}

/// 接受待审批计划的条件 UPDATE（仅仍是 pending_approval 才落），返回受影响行数。
#[allow(clippy::too_many_arguments)]
pub async fn approve_pending(
    db: &DatabaseConnection,
    history_id: &str,
    parameter_result_json: &str,
    reviewer: &str,
    reviewed_at_ms: i64,
    rollback_ref: Option<&str>,
    improvement_summary: &str,
) -> Result<u64, DbErr> {
    let result = Entity::update_many()
        .col_expr(Column::Status, Expr::value(STATUS_ACCEPTED.to_string()))
        .col_expr(Column::ParameterResultJson, Expr::value(parameter_result_json.to_owned()))
        .col_expr(Column::Reviewer, Expr::value(reviewer.to_owned()))
        .col_expr(Column::ReviewedAt, Expr::value(reviewed_at_ms))
        .col_expr(Column::RollbackRef, Expr::value(rollback_ref.map(str::to_owned)))
        .col_expr(Column::ImprovementSummary, Expr::value(improvement_summary.to_owned()))
        .filter(Column::Id.eq(history_id.to_owned()))
        .filter(Column::Status.eq(STATUS_PENDING_APPROVAL))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

/// 拒绝待审批计划的条件 UPDATE（原因必填），返回受影响行数。
pub async fn reject_pending(
    db: &DatabaseConnection,
    history_id: &str,
    reviewer: &str,
    reviewed_at_ms: i64,
    reason: &str,
) -> Result<u64, DbErr> {
    let result = Entity::update_many()
        .col_expr(Column::Status, Expr::value(STATUS_REJECTED.to_string()))
        .col_expr(Column::RejectReason, Expr::value(reason.to_owned()))
        .col_expr(Column::Reviewer, Expr::value(reviewer.to_owned()))
        .col_expr(Column::ReviewedAt, Expr::value(reviewed_at_ms))
        .filter(Column::Id.eq(history_id.to_owned()))
        .filter(Column::Status.eq(STATUS_PENDING_APPROVAL))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

/// 落一条 accepted 行（回滚新版本，审批时间=创建时间）。
pub async fn insert_accepted(
    db: &DatabaseConnection,
    r: &AcceptedHistoryRecord,
) -> Result<(), DbErr> {
    let row = ActiveModel {
        id: Set(r.id.clone()),
        plan_id: Set(r.plan_id.clone()),
        trigger: Set(r.trigger_json.clone()),
        evolution_type: Set(r.evolution_type.clone()),
        quality_before: Set(0),
        parameter_result_json: Set(r.parameter_result_json.clone()),
        workflow_result_json: Set(r.workflow_result_json.clone()),
        status: Set(STATUS_ACCEPTED.to_string()),
        improvement_summary: Set(r.improvement_summary.clone()),
        trigger_sample_json: Set(r.trigger_sample_json.clone()),
        reviewer: Set(Some(r.reviewer.clone())),
        reviewed_at: Set(Some(r.created_at_ms)),
        reject_reason: Set(None),
        rollback_ref: Set(r.rollback_ref.clone()),
        created_at: Set(r.created_at_ms),
    };
    Entity::insert(row).exec(db).await?;
    Ok(())
}

/// 回滚后把当前 accepted 版本置 rejected（条件 status=accepted），返回受影响行数。
pub async fn reject_accepted(
    db: &DatabaseConnection,
    history_id: &str,
    reviewer: &str,
    reviewed_at_ms: i64,
    reject_reason: &str,
) -> Result<u64, DbErr> {
    let result = Entity::update_many()
        .col_expr(Column::Status, Expr::value(STATUS_REJECTED.to_string()))
        .col_expr(Column::RejectReason, Expr::value(reject_reason.to_owned()))
        .col_expr(Column::Reviewer, Expr::value(reviewer.to_owned()))
        .col_expr(Column::ReviewedAt, Expr::value(reviewed_at_ms))
        .filter(Column::Id.eq(history_id.to_owned()))
        .filter(Column::Status.eq(STATUS_ACCEPTED))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}
