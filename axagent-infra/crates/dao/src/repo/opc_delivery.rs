// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 交付数据访问层（v134）
//!
//! won 线索的发票账本：开票（从线索元数据自动填充）、发票状态机、
//! 交付汇总（转化率）。开票前置校验（线索必须 won / 一线索单张有效发票）
//! 在命令层，本模块只管存储与查询。

use sea_orm::*;

use axagent_entities::opc_invoices;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::types::{
    CreateInvoiceFromLeadInput, DeliveryInvoiceDto, DeliverySummary, RevenueByCurrency,
};
use axagent_harness::util_fns::gen_id;

// ── 实体 ↔ DTO 转换 ──────────────────────────────────────────

fn invoice_from_entity(m: opc_invoices::Model) -> DeliveryInvoiceDto {
    DeliveryInvoiceDto {
        id: m.id,
        lead_id: m.lead_id.unwrap_or_default(),
        linked_workflow_id: m.linked_workflow_id,
        title: m.title.unwrap_or_default(),
        amount: m.amount,
        currency: m.currency,
        status: m.status,
        issued_at: m.issued_at,
        paid_at: m.paid_at,
        notes: m.notes,
        created_at: m.created_at,
        updated_at: m.updated_at,
    }
}

// ── 发票状态机 ──────────────────────────────────────────

/// 发票状态机：draft → sent → paid 单向，同状态幂等
///
/// paid 是终态；不引入 void —— 作废用删除代替（简单账本，别造第二种终态）。
pub fn is_legal_invoice_transition(from: &str, to: &str) -> bool {
    from == to || (from == "draft" && to == "sent") || (from == "sent" && to == "paid")
}

// ── 发票 CRUD ──────────────────────────────────────────

/// 从线索开票（命令层已完成 won 校验与幂等检查，这里只管写入）
pub async fn create_invoice(
    db: &DatabaseConnection,
    lead_id: &str,
    linked_workflow_id: Option<String>,
    input: &CreateInvoiceFromLeadInput,
    now: i64,
) -> Result<DeliveryInvoiceDto> {
    let active = opc_invoices::ActiveModel {
        id: Set(gen_id()),
        lead_id: Set(Some(lead_id.to_string())),
        linked_workflow_id: Set(linked_workflow_id),
        title: Set(Some(input.title.clone().unwrap_or_else(|| "未命名发票".to_string()))),
        // 客户账单场景字段留空（交付场景不填）
        customer_id: Set(None),
        invoice_number: Set(None),
        line_items_json: Set(None),
        subtotal: Set(None),
        tax_total: Set(None),
        total: Set(None),
        due_at: Set(None),
        amount: Set(input.amount.unwrap_or(0.0)),
        currency: Set(input.currency.clone().unwrap_or_else(|| "CNY".to_string())),
        status: Set("draft".to_string()),
        issued_at: Set(None),
        paid_at: Set(None),
        notes: Set(input.notes.clone()),
        created_at: Set(now),
        updated_at: Set(now),
    };
    let inserted = active.insert(db).await.map_err(AxAgentError::Database)?;
    Ok(invoice_from_entity(inserted))
}

pub async fn list_invoices(
    db: &DatabaseConnection,
    status: Option<String>,
) -> Result<Vec<DeliveryInvoiceDto>> {
    let mut sel = opc_invoices::Entity::find().order_by_asc(opc_invoices::Column::Id);
    if let Some(s) = status {
        sel = sel.filter(opc_invoices::Column::Status.eq(s));
    }
    let rows = sel.all(db).await.map_err(AxAgentError::Database)?;
    Ok(rows.into_iter().map(invoice_from_entity).collect())
}

/// 查某线索的全部有效发票（开票幂等检查用）
pub async fn list_invoices_by_lead(
    db: &DatabaseConnection,
    lead_id: &str,
) -> Result<Vec<DeliveryInvoiceDto>> {
    let rows = opc_invoices::Entity::find()
        .filter(opc_invoices::Column::LeadId.eq(lead_id))
        .all(db)
        .await
        .map_err(AxAgentError::Database)?;
    Ok(rows.into_iter().map(invoice_from_entity).collect())
}

pub async fn get_invoice(db: &DatabaseConnection, invoice_id: &str) -> Result<DeliveryInvoiceDto> {
    opc_invoices::Entity::find_by_id(invoice_id)
        .one(db)
        .await
        .map_err(AxAgentError::Database)?
        .map(invoice_from_entity)
        .ok_or_else(|| AxAgentError::NotFound(format!("发票不存在: {invoice_id}")))
}

/// 推进发票状态机；落 issued_at / paid_at 时间戳
pub async fn update_invoice_status(
    db: &DatabaseConnection,
    invoice_id: &str,
    to_status: &str,
    now: i64,
) -> Result<DeliveryInvoiceDto> {
    let existing = get_invoice(db, invoice_id).await?;
    if !is_legal_invoice_transition(&existing.status, to_status) {
        return Err(AxAgentError::Validation(format!(
            "非法发票状态迁移: {} → {}（合法路径 draft → sent → paid）",
            existing.status, to_status
        )));
    }

    // 从 DTO 重建 ActiveModel（只改状态与时间戳字段，主键 Set 走 update 语义）
    let mut active: opc_invoices::ActiveModel =
        opc_invoices::ActiveModel { id: Set(existing.id.clone()), ..Default::default() };
    active.status = Set(to_status.to_string());
    match to_status {
        "sent" => active.issued_at = Set(Some(now)),
        "paid" => {
            // sent → paid：issued_at 没落过的话补上（同状态幂等调用 paid 不覆盖）
            active.issued_at = Set(existing.issued_at.or(Some(now)));
            active.paid_at = Set(Some(now));
        },
        _ => {},
    }
    active.updated_at = Set(now);
    let updated = active.update(db).await.map_err(AxAgentError::Database)?;
    Ok(invoice_from_entity(updated))
}

pub async fn delete_invoice(db: &DatabaseConnection, invoice_id: &str) -> Result<()> {
    opc_invoices::Entity::delete_by_id(invoice_id)
        .exec(db)
        .await
        .map_err(AxAgentError::Database)?;
    Ok(())
}

// ── 交付汇总 ──────────────────────────────────────────

/// 单币种回款/开票小计（内部累加用）
#[derive(Default)]
struct CurrencyAccum {
    paid_total: f64,
    issued_total: f64,
}

/// 交付汇总：won 数、发票数、回款（按币种分组）、转化率
///
/// `conversion_rate` = won / (全部线索 − lost)；won/active 由调用方从线索表
/// 统计后传入（线索统计属于需求域 repo，不在交付域重复查询）。
pub async fn delivery_summary(
    db: &DatabaseConnection,
    won_leads: u32,
    active_leads: u32,
) -> Result<DeliverySummary> {
    let invoices = list_invoices(db, None).await?;

    let mut by_currency: std::collections::BTreeMap<String, CurrencyAccum> =
        std::collections::BTreeMap::new();
    let mut paid_count = 0u32;
    for inv in &invoices {
        let acc = by_currency.entry(inv.currency.clone()).or_default();
        if inv.status == "paid" {
            acc.paid_total += inv.amount;
            acc.issued_total += inv.amount;
            paid_count += 1;
        } else if inv.status == "sent" {
            acc.issued_total += inv.amount;
        }
    }

    Ok(DeliverySummary {
        won_leads,
        active_leads,
        invoice_count: invoices.len() as u32,
        paid_count,
        revenues: by_currency
            .into_iter()
            .map(|(currency, acc)| RevenueByCurrency {
                currency,
                paid_total: acc.paid_total,
                issued_total: acc.issued_total,
            })
            .collect(),
        conversion_rate: if active_leads == 0 {
            0.0
        } else {
            f64::from(won_leads) / f64::from(active_leads)
        },
    })
}

#[cfg(test)]
mod tests {
    use super::is_legal_invoice_transition;

    #[test]
    fn invoice_transition_is_forward_only() {
        // 单向推进
        assert!(is_legal_invoice_transition("draft", "sent"));
        assert!(is_legal_invoice_transition("sent", "paid"));
        // 同状态幂等
        assert!(is_legal_invoice_transition("draft", "draft"));
        assert!(is_legal_invoice_transition("sent", "sent"));
        assert!(is_legal_invoice_transition("paid", "paid"));
        // 逆向/跨级/未知状态一律非法
        assert!(!is_legal_invoice_transition("sent", "draft"));
        assert!(!is_legal_invoice_transition("paid", "sent"));
        assert!(!is_legal_invoice_transition("paid", "draft"));
        assert!(!is_legal_invoice_transition("draft", "paid"));
        assert!(!is_legal_invoice_transition("unknown", "sent"));
    }
}
