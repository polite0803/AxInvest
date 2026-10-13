// SPDX-License-Identifier: AGPL-3.0-only

//! 股票进化计划人工审批命令（P4 两段式：计划-审批）。
//!
//! 定时聚合（`commands::evolution_proposal`）只产 `pending_approval` 计划行、
//! 不执行任何变异；本模块提供审批出口：
//! - [`list_pending_evolution_plans`]：待审批列表（任务中心概览 / EvolutionDriftPanel 双入口）；
//! - [`approve_evolution_plan`]：接受 → 执行进化 → UPDATE 原行 accepted（不插新行），
//!   参数配置随后被 `evolution_drift::load_active_weight_decay_config` 读到；
//! - [`reject_evolution_plan`]：拒绝（原因必填）→ UPDATE 原行 rejected；
//! - [`rollback_evolution`]：回滚当前生效版本到上一 accepted（插新 accepted 行 +
//!   当前行置 rejected，全部留痕）。
//!
//! 刻意**不复用** `AppState.evolution_consent_senders`：那是对话内实时 oneshot 同意通道
//! （内存态、进程重启即失），审批是跨天离线待办，状态以 `stock_evolution_history` 表为准。

use axagent_agent_macro::agent_command;
use axagent_analysis_engine::stock_self_evolution::{
    EvolutionPlan, EvolutionTrigger, EvolutionType,
};
use axagent_dao::repo::stock_evolution_history::{
    self as evo_hist, AcceptedHistoryRecord, EvolutionHistoryModel,
};
use tauri::State;

use crate::AppState;
use crate::commands::error::ErrorResponse;
use crate::commands::error_code::stock_workflow as wf_err;

/// 审批人缺省值（桌面单用户，命令入参可覆盖）。
const DEFAULT_REVIEWER: &str = "local-user";

fn internal_err(detail: impl Into<String>) -> String {
    ErrorResponse::new(wf_err::INTERNAL).with_detail(detail.into()).to_string()
}

fn evolution_type_from_str(s: &str) -> Result<EvolutionType, String> {
    match s {
        "parameter" => Ok(EvolutionType::ParameterEvolution),
        "workflow" => Ok(EvolutionType::WorkflowEvolution),
        "hybrid" => Ok(EvolutionType::HybridEvolution),
        other => Err(format!("未知进化类型: {other}")),
    }
}

/// 从待审批历史行重建进化计划（pending 行只持久化了 plan_id/trigger/type，
/// 其余字段用审批期占位值——参数进化执行链不消费它们）。
fn rebuild_plan(row: &EvolutionHistoryModel) -> Result<EvolutionPlan, String> {
    let trigger: EvolutionTrigger = serde_json::from_str(&row.trigger)
        .map_err(|e| format!("反序列化进化 trigger 失败: {e}"))?;
    Ok(EvolutionPlan {
        plan_id: row.plan_id.clone(),
        trigger,
        reflection_report_id: format!("approval-{}", row.plan_id),
        evolution_type: evolution_type_from_str(&row.evolution_type)?,
        description: row.improvement_summary.clone(),
        template_id: None,
        created_at: chrono::DateTime::from_timestamp_millis(row.created_at)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default(),
    })
}

/// 查待审批进化计划（按创建时间升序，先到先审）。
#[agent_command(domain = "finance", safety = Safe, call_mode = StateInput, description = "查询待审批进化计划")]
#[tauri::command]
pub async fn list_pending_evolution_plans(
    state: State<'_, AppState>,
) -> Result<Vec<EvolutionHistoryModel>, String> {
    let db = state.harness.db();
    evo_hist::list_pending_asc(db)
        .await
        .map_err(|e| internal_err(format!("查询待审批进化计划失败: {e}")))
}

/// 接受一条待审批进化计划：执行进化并把结果 UPDATE 回原行（status=accepted）。
///
/// 条件 UPDATE 带 `status=pending_approval` 过滤做幂等闸门：已被审批过的行
/// rows_affected=0，报「已处理」错，避免重复接受。
#[agent_command(domain = "finance", safety = Caution, call_mode = StateInput, description = "接受进化计划")]
#[tauri::command]
pub async fn approve_evolution_plan(
    state: State<'_, AppState>,
    history_id: String,
    reviewer: Option<String>,
) -> Result<EvolutionHistoryModel, String> {
    let db = state.harness.db();
    let reviewer = reviewer
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_REVIEWER.to_string());

    let row = evo_hist::find_by_id(db, &history_id)
        .await
        .map_err(|e| internal_err(format!("读取进化计划失败: {e}")))?
        .ok_or_else(|| internal_err(format!("进化计划不存在: {history_id}")))?;
    if row.status != evo_hist::STATUS_PENDING_APPROVAL {
        return Err(internal_err(format!(
            "进化计划 {history_id} 当前状态为 {}，不可接受（仅待审批计划可接受）",
            row.status
        )));
    }

    let plan = rebuild_plan(&row)?;

    // 执行进化（审批专用路径：不 insert 新 history 行，结果回写本行）。
    let result = state
        .stock_adaptive_engine
        .evolution_engine()
        .execute_pending_plan(&plan)
        .await
        .map_err(|e| internal_err(format!("进化执行失败: {e}")))?;

    let now_ms = chrono::Utc::now().timestamp_millis();

    // 进化执行失败（result.success=false / 无参数结果）：行置 rejected 并带原因，不留僵尸 pending。
    if !result.success || result.parameter_result.is_none() {
        let reason = result.error.clone().unwrap_or_else(|| "进化未产生有效参数结果".to_string());
        let affected = evo_hist::reject_pending(
            db,
            &history_id,
            &reviewer,
            now_ms,
            &format!("接受后执行失败: {reason}"),
        )
        .await
        .map_err(|e| internal_err(format!("进化执行失败，计划状态回写失败: {e}")))?;
        if affected == 0 {
            return Err(internal_err(format!(
                "进化执行失败，且计划 {history_id} 已被其他审批动作处理，状态未更新"
            )));
        }
        return Err(internal_err(format!("进化执行失败，计划已标记 rejected: {reason}")));
    }

    let parameter_result = result
        .parameter_result
        .as_ref()
        .ok_or_else(|| internal_err("进化未产生有效参数结果".to_string()))?;
    let parameter_json = serde_json::to_string(parameter_result)
        .map_err(|e| internal_err(format!("序列化进化结果失败: {e}")))?;

    // 回滚目标：此前最近一条 accepted 行（无则 None = 第一代，无版本可回滚）。
    let prev_accepted_id = evo_hist::latest_accepted_id(db, None)
        .await
        .map_err(|e| internal_err(format!("查询上一已接受版本失败: {e}")))?;

    // 条件 UPDATE：仅仍是 pending_approval 才落接受（并发幂等闸门）。
    let affected = evo_hist::approve_pending(
        db,
        &history_id,
        &parameter_json,
        &reviewer,
        now_ms,
        prev_accepted_id.as_deref(),
        &result.improvement_summary,
    )
    .await
    .map_err(|e| internal_err(format!("接受进化计划落库失败: {e}")))?;
    if affected == 0 {
        return Err(internal_err(format!(
            "进化计划 {history_id} 已被其他审批动作处理，接受未生效"
        )));
    }

    tracing::info!(
        "[evolution-approval] 计划 {history_id} 已接受（reviewer={reviewer}，\
         回滚目标={}）",
        prev_accepted_id.as_deref().unwrap_or("无")
    );

    evo_hist::find_by_id(db, &history_id)
        .await
        .map_err(|e| internal_err(format!("回读已接受计划失败: {e}")))?
        .ok_or_else(|| internal_err("接受后回读计划失败：行不存在"))
}

/// 拒绝一条待审批进化计划（原因必填，截断 500 字）。
#[agent_command(domain = "finance", safety = Caution, call_mode = StateInput, description = "拒绝进化计划")]
#[tauri::command]
pub async fn reject_evolution_plan(
    state: State<'_, AppState>,
    history_id: String,
    reason: String,
    reviewer: Option<String>,
) -> Result<EvolutionHistoryModel, String> {
    let db = state.harness.db();
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(internal_err("拒绝原因不能为空"));
    }
    let reviewer = reviewer
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_REVIEWER.to_string());
    let reason: String = reason.chars().take(500).collect();

    let row = evo_hist::find_by_id(db, &history_id)
        .await
        .map_err(|e| internal_err(format!("读取进化计划失败: {e}")))?
        .ok_or_else(|| internal_err(format!("进化计划不存在: {history_id}")))?;
    if row.status != evo_hist::STATUS_PENDING_APPROVAL {
        return Err(internal_err(format!(
            "进化计划 {history_id} 当前状态为 {}，不可拒绝（仅待审批计划可拒绝）",
            row.status
        )));
    }

    let now_ms = chrono::Utc::now().timestamp_millis();
    let affected = evo_hist::reject_pending(db, &history_id, &reviewer, now_ms, &reason)
        .await
        .map_err(|e| internal_err(format!("拒绝进化计划落库失败: {e}")))?;
    if affected == 0 {
        return Err(internal_err(format!(
            "进化计划 {history_id} 已被其他审批动作处理，拒绝未生效"
        )));
    }
    // 反例回灌挂接点：当前仅在 reject_reason 留痕；reflection_lessons 与具体反思行的
    // 业务字段（horizon/action/confidence）绑定，不宜硬塞审批事件。后续若新增通用
    // 负反馈事件表，在此把 (history_id, reason, reviewer) 写入供进化器消费。
    evo_hist::find_by_id(db, &history_id)
        .await
        .map_err(|e| internal_err(format!("回读已拒绝计划失败: {e}")))?
        .ok_or_else(|| internal_err("拒绝后回读计划失败：行不存在"))
}

/// 回滚当前生效（最近 accepted）的进化版本到上一已接受版本。
///
/// 做法（全部留痕、不删数据）：新插一条 accepted 行，参数结果复制自回滚目标行；
/// 当前行置 rejected，reject_reason 指向新行。`load_active_weight_decay_config`
/// 取「最新 accepted」自然读到旧配置。
#[agent_command(domain = "finance", safety = Caution, call_mode = StateInput, description = "回滚进化版本")]
#[tauri::command]
pub async fn rollback_evolution(
    state: State<'_, AppState>,
    history_id: Option<String>,
    reviewer: Option<String>,
) -> Result<EvolutionHistoryModel, String> {
    let db = state.harness.db();
    let reviewer = reviewer
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_REVIEWER.to_string());
    let now_ms = chrono::Utc::now().timestamp_millis();

    // 当前生效版本：显式传入则校验为 accepted；否则取最近 accepted。
    let current = match history_id {
        Some(id) => {
            let row = evo_hist::find_by_id(db, &id)
                .await
                .map_err(|e| internal_err(format!("读取进化版本失败: {e}")))?
                .ok_or_else(|| internal_err(format!("进化版本不存在: {id}")))?;
            if row.status != evo_hist::STATUS_ACCEPTED {
                return Err(internal_err(format!(
                    "版本 {id} 状态为 {}，仅已接受版本可回滚",
                    row.status
                )));
            }
            row
        },
        None => evo_hist::latest_accepted_row(db)
            .await
            .map_err(|e| internal_err(format!("读取当前生效版本失败: {e}")))?
            .ok_or_else(|| internal_err("当前没有已接受的进化版本，无需回滚"))?,
    };

    let target_id = current
        .rollback_ref
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| internal_err("当前版本是第一代已接受版本，无历史版本可回滚"))?;
    let target = evo_hist::find_by_id(db, target_id)
        .await
        .map_err(|e| internal_err(format!("读取回滚目标版本失败: {e}")))?
        .ok_or_else(|| internal_err(format!("回滚目标版本不存在: {target_id}")))?;
    let target_config = target
        .parameter_result_json
        .clone()
        .ok_or_else(|| internal_err(format!("回滚目标版本 {target_id} 无参数结果，不可回滚")))?;

    let new_id = format!("evo-rollback-{}", uuid::Uuid::new_v4());
    let new_plan_id = format!("rollback-{}", uuid::Uuid::new_v4());
    let summary = format!("回滚到上一已接受版本（{}）：{}", target.id, target.improvement_summary);

    let new_row = AcceptedHistoryRecord {
        id: new_id.clone(),
        plan_id: new_plan_id,
        trigger_json: target.trigger.clone(),
        evolution_type: target.evolution_type.clone(),
        parameter_result_json: Some(target_config),
        workflow_result_json: target.workflow_result_json.clone(),
        improvement_summary: summary,
        trigger_sample_json: target.trigger_sample_json.clone(),
        reviewer: reviewer.clone(),
        // 沿用目标版本的回滚链（再上一代），保持链条不断。
        rollback_ref: target.rollback_ref.clone(),
        created_at_ms: now_ms,
    };
    evo_hist::insert_accepted(db, &new_row)
        .await
        .map_err(|e| internal_err(format!("回滚版本落库失败: {e}")))?;

    // 当前版本置 rejected 并指向回滚新行。
    let affected = evo_hist::reject_accepted(
        db,
        &current.id,
        &reviewer,
        now_ms,
        &format!("rolled back to {new_id}"),
    )
    .await
    .map_err(|e| internal_err(format!("回滚后更新原版本失败: {e}")))?;
    if affected == 0 {
        return Err(internal_err(format!(
            "版本 {} 已被其他审批动作改变状态，回滚未完成",
            current.id
        )));
    }

    tracing::info!(
        "[evolution-approval] 版本 {} 已回滚 -> 新行 {}（配置来自 {}）",
        current.id,
        new_id,
        target.id
    );

    evo_hist::find_by_id(db, &new_id)
        .await
        .map_err(|e| internal_err(format!("回读回滚版本失败: {e}")))?
        .ok_or_else(|| internal_err("回滚后回读失败：新行不存在"))
}
