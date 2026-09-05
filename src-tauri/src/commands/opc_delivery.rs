// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 交付命令层（P4 交付闭环，v134）
//!
//! 打通「实现 → 交付」断点（全链路审计断点 4）：won 线索开票入账，
//! 发票状态机 `draft → sent → paid`，交付汇总（转化率）只统计不回写 ——
//! 样本量不够时自动调评分权重是过拟合，先让数据积累起来。
//!
//! - 开票：`opc_create_invoice_from_lead`（前置校验线索 must be won；幂等）
//! - 账本：`opc_list_invoices` / `opc_update_invoice_status` / `opc_delete_invoice`
//! - 汇总：`opc_get_delivery_summary`
//!
//! 设计决策：
//! - 不碰 artifacts 表（它绑 conversation_id，与线索域不同源；硬接是污染）。
//!   交付产物溯源走 `linked_workflow_id`（P2 转化链已有）。
//! - 多币种汇总按币种分组，不做汇率换算 —— 换算错比不汇总更糟。
//! - 错误码复用 common::INVALID_INPUT / opc_setup::INTERNAL，不新增 11 语言翻译。

use crate::AppState;
use crate::commands::error_code::common as common_err;
use crate::commands::error_code::opc_setup as opc_setup_err;
use axagent_agent_macro::agent_command;
use axagent_dao::repo::opc_delivery;
use axagent_harness::types::{CreateInvoiceFromLeadInput, DeliveryInvoiceDto, DeliverySummary};
use axagent_harness::util_fns::now_ts;
use tauri::State;

/// 把 won 线索开成发票
///
/// 幂等：该线索已有发票时直接返回第一张（按创建时间最早），不重复开票。
/// 缺省字段从线索元数据自动填充：标题 = 线索标题、金额 = 预算上限
/// （无上限退下限，再无则 0）、币种 = 线索预算币种（无则 CNY）。
#[tauri::command]
pub async fn opc_create_invoice_from_lead(
    state: State<'_, AppState>,
    lead_id: String,
    input: Option<CreateInvoiceFromLeadInput>,
) -> Result<DeliveryInvoiceDto, String> {
    let db = state.harness.db();

    let lead = axagent_dao::repo::opc_demand::get_lead(db, &lead_id).await.map_err(err)?;
    if lead.status != "won" {
        return Err(crate::commands::error::ErrorResponse::err_with_detail(
            common_err::INVALID_INPUT,
            format!("仅 won 线索可开票，当前状态: {}（线索 {lead_id}）", lead.status),
        ));
    }

    // 幂等：已有发票直接返回，不重复开
    let existing = opc_delivery::list_invoices_by_lead(db, &lead_id).await.map_err(err)?;
    if let Some(inv) = existing.first() {
        return Ok(inv.clone());
    }

    // 缺省填充：金额优先预算上限 → 预算下限 → 0
    let amount = input
        .as_ref()
        .and_then(|i| i.amount)
        .or(lead.budget_max)
        .or(lead.budget_min)
        .unwrap_or(0.0);
    let filled = CreateInvoiceFromLeadInput {
        title: input.as_ref().and_then(|i| i.title.clone()).or(Some(lead.title.clone())),
        amount: Some(amount),
        currency: Some(
            input
                .as_ref()
                .and_then(|i| i.currency.clone())
                .or_else(|| {
                    if lead.budget_currency.is_empty() {
                        None
                    } else {
                        Some(lead.budget_currency.clone())
                    }
                })
                .unwrap_or_else(|| "CNY".to_string()),
        ),
        notes: input.as_ref().and_then(|i| i.notes.clone()),
    };

    opc_delivery::create_invoice(db, &lead_id, lead.linked_workflow_id.clone(), &filled, now_ts())
        .await
        .map_err(err)
}

/// 发票列表（可按状态过滤）
#[agent_command(domain = "automation", safety = Safe, call_mode = StateInput, description = "列出发票")]
#[tauri::command]
pub async fn opc_list_invoices(
    state: State<'_, AppState>,
    status: Option<String>,
) -> Result<Vec<DeliveryInvoiceDto>, String> {
    opc_delivery::list_invoices(state.harness.db(), status).await.map_err(err)
}

/// 推进发票状态机（draft → sent → paid，同状态幂等，逆向报错）
#[tauri::command]
pub async fn opc_update_invoice_status(
    state: State<'_, AppState>,
    invoice_id: String,
    status: String,
) -> Result<DeliveryInvoiceDto, String> {
    if !matches!(status.as_str(), "draft" | "sent" | "paid") {
        return Err(crate::commands::error::ErrorResponse::err_with_detail(
            common_err::INVALID_INPUT,
            format!("非法发票状态: {status}（合法值 draft/sent/paid）"),
        ));
    }
    opc_delivery::update_invoice_status(state.harness.db(), &invoice_id, &status, now_ts())
        .await
        .map_err(err)
}

/// 删除发票（作废用删除代替 —— 简单账本，不造第二种终态）
#[agent_command(domain = "automation", safety = Dangerous, call_mode = StateInput, description = "删除发票")]
#[tauri::command]
pub async fn opc_delete_invoice(
    state: State<'_, AppState>,
    invoice_id: String,
) -> Result<(), String> {
    opc_delivery::delete_invoice(state.harness.db(), &invoice_id).await.map_err(err)
}

/// 交付汇总：won 数、发票数、回款（按币种分组）、转化率
///
/// 转化率 = won / (全部线索 − lost)。只统计暴露，不自动回写评分权重。
#[tauri::command]
pub async fn opc_get_delivery_summary(
    state: State<'_, AppState>,
) -> Result<DeliverySummary, String> {
    let db = state.harness.db();

    // 线索侧统计：个人公司量级（几十~几百条）全量拉取无压力，不值得上聚合 SQL
    let leads =
        axagent_dao::repo::opc_demand::list_leads(db, 10_000, None, None).await.map_err(err)?;
    let won_leads = leads.iter().filter(|l| l.status == "won").count() as u32;
    let active_leads = leads.iter().filter(|l| l.status != "lost").count() as u32;

    let mut summary =
        opc_delivery::delivery_summary(db, won_leads, active_leads).await.map_err(err)?;

    // 空库兜底：发票表为零时给 0 转化率而非 NaN（active=0 时 dao 已处理）
    if summary.active_leads == 0 {
        summary.conversion_rate = 0.0;
    }
    tracing::info!(
        won = summary.won_leads,
        active = summary.active_leads,
        invoices = summary.invoice_count,
        conversion = summary.conversion_rate,
        "[opc_delivery] 交付汇总完成"
    );
    Ok(summary)
}

/// DAO/校验错误 → 命令层错误串（走错误码映射层）
fn err(e: axagent_harness::core_error::AxAgentError) -> String {
    String::from(crate::commands::error::ErrorResponse::from_error_with_code(
        opc_setup_err::INTERNAL,
        e,
        crate::commands::error::ErrorCategory::Unrecoverable,
    ))
}
