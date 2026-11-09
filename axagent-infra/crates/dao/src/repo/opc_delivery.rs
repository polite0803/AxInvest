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
    // 交付账本只列「交付来源」发票：`opc_invoices` 与域包发票（company-runtime
    // `InvoiceService`）共用同一张表，两套投影字段互斥 —— 交付投影写 `lead_id`/`amount`，
    // 域包投影写 `customer_id`/`total`。不加这条过滤，交付账本会把域包发票一并列出
    // （title/lead_id 落空串），`delivery_summary` 的 paid_count 也会虚增。
    let mut sel = opc_invoices::Entity::find()
        .filter(opc_invoices::Column::LeadId.is_not_null())
        .order_by_asc(opc_invoices::Column::Id);
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

/// 按 id 取**交付投影**发票
///
/// 投影归属校验与 [`list_invoices`] 同源：`opc_invoices` 与域包发票共用一张表
/// （交付投影写 `lead_id`/`amount`，域包投影写 `customer_id`/`total`）。按 id 直取时
/// 若只认主键，传入域包发票 id 也能被交付域读到；`update_invoice_status` 走本函数
/// 取现状，于是「改状态」也一并越权。交付域看不到域包发票 ⇒ 表现为 `NotFound`
/// （与「行不存在」同一响应，不向调用方泄露另一套投影的存在）。
pub async fn get_invoice(db: &DatabaseConnection, invoice_id: &str) -> Result<DeliveryInvoiceDto> {
    opc_invoices::Entity::find_by_id(invoice_id)
        .filter(opc_invoices::Column::LeadId.is_not_null())
        .one(db)
        .await
        .map_err(AxAgentError::Database)?
        .map(invoice_from_entity)
        .ok_or_else(|| AxAgentError::NotFound(format!("发票不存在: {invoice_id}")))
}

/// 推进发票状态机；落 issued_at / paid_at 时间戳
///
/// 投影归属校验由 [`get_invoice`] 承担（域包发票 id ⇒ `NotFound`），本函数沿用其结论，
/// 不必二次判断。
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

/// 删除交付投影发票
///
/// 投影归属校验与 [`get_invoice`] 同源（`lead_id IS NOT NULL`），但落在**同一条**
/// DELETE 上而不是「先查后删」：查与删之间存在竞态窗口，且两段式会多一次往返。
/// `rows_affected == 0` 同时覆盖「行不存在」与「行属域包投影」，对交付域一律
/// `NotFound` —— 既满足「看不见就表现为不存在」，也不泄露另一套投影。
pub async fn delete_invoice(db: &DatabaseConnection, invoice_id: &str) -> Result<()> {
    let res = opc_invoices::Entity::delete_many()
        .filter(opc_invoices::Column::Id.eq(invoice_id))
        .filter(opc_invoices::Column::LeadId.is_not_null())
        .exec(db)
        .await
        .map_err(AxAgentError::Database)?;
    if res.rows_affected == 0 {
        return Err(AxAgentError::NotFound(format!("发票不存在: {invoice_id}")));
    }
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
    use super::{
        create_invoice, delete_invoice, get_invoice, list_invoices, update_invoice_status,
    };
    use axagent_entities::opc_invoices;
    use axagent_harness::core_error::{AxAgentError, Result};
    use axagent_harness::types::CreateInvoiceFromLeadInput;
    use axagent_harness::util_fns::gen_id;
    use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};

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

    // ── 同表两投影：交付域不得越界读写域包发票 ──────────────────
    //
    // 建库走 `create_test_pool`（与生产同一条建表链，见 `db::initialize_schema`）。

    async fn setup() -> DatabaseConnection {
        let handle = crate::db::create_test_pool().await.expect("测试库应可建立");
        // `DbHandle` 析构后克隆体仍持有连接池（`min_connections = 1`）
        handle.conn.clone()
    }

    /// 造两行：交付投影（`lead_id` 有值）+ 域包投影（`lead_id` 为空、`customer_id` 有值）
    ///
    /// 返回 `(交付发票 id, 域包发票 id)`。域包行直接走 entity 写入并显式置
    /// `lead_id: Set(None)` —— 刻意不经过本模块的 `create_invoice`，因为那正是
    /// 「交付投影」的定义；用生产写入路径造对照行会让判据退化成自证。
    async fn seed_two_projections(db: &DatabaseConnection) -> (String, String) {
        let delivery = create_invoice(
            db,
            "lead-1",
            None,
            &CreateInvoiceFromLeadInput {
                title: Some("交付发票".to_string()),
                amount: Some(100.0),
                currency: Some("CNY".to_string()),
                notes: None,
            },
            1_000,
        )
        .await
        .expect("交付投影应可写入");

        let pack_id = gen_id();
        let pack = opc_invoices::ActiveModel {
            id: Set(pack_id.clone()),
            lead_id: Set(None),
            linked_workflow_id: Set(None),
            title: Set(Some("域包账单".to_string())),
            customer_id: Set(Some("cust-1".to_string())),
            invoice_number: Set(Some("INV-0001".to_string())),
            line_items_json: Set(Some("[]".to_string())),
            subtotal: Set(Some(100.0)),
            tax_total: Set(Some(0.0)),
            total: Set(Some(100.0)),
            amount: Set(0.0),
            currency: Set("CNY".to_string()),
            status: Set("draft".to_string()),
            issued_at: Set(None),
            due_at: Set(None),
            paid_at: Set(None),
            notes: Set(None),
            created_at: Set(1_000),
            updated_at: Set(1_000),
        };
        pack.insert(db).await.expect("域包投影应可写入");

        (delivery.id, pack_id)
    }

    fn assert_not_found<T: std::fmt::Debug>(res: Result<T>, what: &str) {
        match res {
            Err(AxAgentError::NotFound(_)) => {},
            other => panic!("{what} 对域包发票必须返回 NotFound，实际: {other:?}"),
        }
    }

    #[tokio::test]
    async fn list_invoices_excludes_domain_pack_projection() {
        let db = setup().await;
        let (delivery_id, pack_id) = seed_two_projections(&db).await;

        let listed = list_invoices(&db, None).await.expect("列表应成功");
        let ids: Vec<&str> = listed.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, vec![delivery_id.as_str()], "交付账本只应列出交付投影行");
        assert!(!ids.contains(&pack_id.as_str()), "域包发票不得混入交付账本");
    }

    #[tokio::test]
    async fn get_and_delete_reject_domain_pack_projection() {
        let db = setup().await;
        let (delivery_id, pack_id) = seed_two_projections(&db).await;

        // 正控：交付投影行可读
        let got = get_invoice(&db, &delivery_id).await.expect("交付投影应可读");
        assert_eq!(got.lead_id, "lead-1");

        // 负控：域包行按 id 直取一律 NotFound
        assert_not_found(get_invoice(&db, &pack_id).await, "get_invoice");
        assert_not_found(delete_invoice(&db, &pack_id).await, "delete_invoice");

        // 域包行必须**仍在**表里（NotFound 不等于被静默删掉）
        assert!(
            opc_invoices::Entity::find_by_id(&pack_id)
                .one(&db)
                .await
                .expect("查询应成功")
                .is_some(),
            "delete_invoice 拒绝域包行后该行不得消失"
        );

        // 正控：交付投影行可删
        delete_invoice(&db, &delivery_id).await.expect("交付投影应可删");
        assert!(get_invoice(&db, &delivery_id).await.is_err(), "删除后应不可读");
    }

    #[tokio::test]
    async fn update_status_rejects_domain_pack_projection() {
        let db = setup().await;
        let (delivery_id, pack_id) = seed_two_projections(&db).await;

        assert_not_found(
            update_invoice_status(&db, &pack_id, "sent", 2_000).await,
            "update_invoice_status",
        );
        // 域包行状态不得被改动
        let row = opc_invoices::Entity::find_by_id(&pack_id)
            .one(&db)
            .await
            .expect("查询应成功")
            .expect("域包行应存在");
        assert_eq!(row.status, "draft", "交付域改状态必须对域包行无效");
        assert!(row.issued_at.is_none());

        // 正控：交付投影行状态机照常推进
        let updated = update_invoice_status(&db, &delivery_id, "sent", 2_000)
            .await
            .expect("交付投影应可推进状态");
        assert_eq!(updated.status, "sent");
        assert_eq!(updated.issued_at, Some(2_000));
    }
}
