// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 能力缺口数据访问层（#264 批 6 自 `commands::demand_discovery` 下沉）
//!
//! 被动缺口（匹配失败）与主动缺口（高频需求未被覆盖，`proactive`）共用一张表。
//! 关闭前置校验（缺口存在性）在读取时即判，本模块只管存储与查询。

use sea_orm::*;

use axagent_entities::opc_capability_gap;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::opc_store::CapabilityGapView;

// ── 实体 ↔ DTO 转换 ──────────────────────────────────────────

fn gap_view_from_entity(m: opc_capability_gap::Model) -> CapabilityGapView {
    CapabilityGapView {
        id: m.id,
        lead_id: m.lead_id,
        title: m.title,
        description: m.description,
        missing_capability: m.missing_capability,
        gap_type: m.gap_type,
        suggested_action: m.suggested_action,
        priority: m.priority,
        status: m.status,
        created_at: m.created_at,
        updated_at: m.updated_at,
        closed_at: m.closed_at,
    }
}

fn view_to_active_model(v: &CapabilityGapView) -> opc_capability_gap::ActiveModel {
    opc_capability_gap::ActiveModel {
        id: Set(v.id.clone()),
        lead_id: Set(v.lead_id.clone()),
        title: Set(v.title.clone()),
        description: Set(v.description.clone()),
        missing_capability: Set(v.missing_capability.clone()),
        gap_type: Set(v.gap_type.clone()),
        suggested_action: Set(v.suggested_action.clone()),
        priority: Set(v.priority),
        status: Set(v.status.clone()),
        created_at: Set(v.created_at),
        updated_at: Set(v.updated_at),
        closed_at: Set(v.closed_at),
    }
}

// ── 查询 ──────────────────────────────────────────────────

/// 列出能力缺口（`status` 可选过滤，按创建时间降序 —— 迁移前 `opc_list_capability_gaps` 同序）。
pub async fn list_gaps(
    db: &DatabaseConnection,
    status: Option<String>,
) -> Result<Vec<CapabilityGapView>> {
    let mut qs = opc_capability_gap::Entity::find();
    if let Some(ref s) = status {
        qs = qs.filter(opc_capability_gap::Column::Status.eq(s));
    }
    let rows = qs
        .order_by_desc(opc_capability_gap::Column::CreatedAt)
        .all(db)
        .await
        .map_err(|e| AxAgentError::Provider(format!("读取能力缺口失败: {e}")))?;
    Ok(rows.into_iter().map(gap_view_from_entity).collect())
}

/// 取单个缺口；缺行报 `NotFound`（id 由调用方持有，读到不存在是异常输入）。
pub async fn get_gap(db: &DatabaseConnection, gap_id: &str) -> Result<CapabilityGapView> {
    let row = opc_capability_gap::Entity::find_by_id(gap_id)
        .one(db)
        .await
        .map_err(|e| AxAgentError::Provider(format!("读取能力缺口失败: {e}")))?
        .ok_or_else(|| AxAgentError::NotFound(format!("能力缺口不存在: {gap_id}")))?;
    Ok(gap_view_from_entity(row))
}

/// 关闭缺口（`resolved` + 关闭时间；迁移前 `opc_close_capability_gap` 同语义）。
pub async fn close_gap(
    db: &DatabaseConnection,
    gap_id: &str,
    now: i64,
) -> Result<CapabilityGapView> {
    let row = opc_capability_gap::Entity::find_by_id(gap_id)
        .one(db)
        .await
        .map_err(|e| AxAgentError::Provider(format!("读取能力缺口失败: {e}")))?
        .ok_or_else(|| AxAgentError::NotFound(format!("能力缺口不存在: {gap_id}")))?;
    let mut am: opc_capability_gap::ActiveModel = row.into();
    am.status = Set("resolved".to_string());
    am.closed_at = Set(Some(now));
    am.updated_at = Set(now);
    let saved = am
        .update(db)
        .await
        .map_err(|e| AxAgentError::Provider(format!("关闭能力缺口失败: {e}")))?;
    Ok(gap_view_from_entity(saved))
}

/// 插入缺口行（主动分析自动建行复用；`view` 需自带 id / gap_type / status）。
pub async fn insert_gap(db: &DatabaseConnection, view: &CapabilityGapView) -> Result<()> {
    view_to_active_model(view)
        .insert(db)
        .await
        .map_err(|e| AxAgentError::Provider(format!("创建能力缺口失败: {e}")))?;
    Ok(())
}
