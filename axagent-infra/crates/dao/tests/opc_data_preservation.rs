// SPDX-License-Identifier: AGPL-3.0-only

//! P6.2 升级前后的 `opc_*` 数据保留实测。
//!
//! 判据：声明式收敛引擎（`initialize_schema` / `bootstrap_schema`）在既有 SQLite 数据上
//! 再跑一次升级收敛时，**不得删除 / 改写既有业务行**。以 `opc_invoices` 为代表
//! （一张表承载交付 + 客户账单两种投影），插入两种形态后重跑收敛并逐字段对账。
//! 这不是「真机升级」，但直接锁住引擎层面的数据保留语义；生产库另有端到端验收。

use axagent_dao::db::create_test_pool;
use axagent_dao::reconcile::apply::bootstrap_schema;
use axagent_entities::opc_invoices;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};

/// 交付场景发票（lead_id 非空，customer_id 为空）。
fn delivery_invoice(id: &str, lead_id: &str, amount: f64) -> opc_invoices::ActiveModel {
    opc_invoices::ActiveModel {
        id: Set(id.to_string()),
        lead_id: Set(Some(lead_id.to_string())),
        linked_workflow_id: Set(None),
        title: Set(Some("交付发票".to_string())),
        customer_id: Set(None),
        invoice_number: Set(None),
        line_items_json: Set(None),
        subtotal: Set(None),
        tax_total: Set(None),
        total: Set(None),
        amount: Set(amount),
        currency: Set("CNY".to_string()),
        status: Set("draft".to_string()),
        issued_at: Set(None),
        due_at: Set(None),
        paid_at: Set(None),
        notes: Set(None),
        created_at: Set(1_700_000_000),
        updated_at: Set(1_700_000_000),
    }
}

/// 客户账单发票（customer_id 非空，lead_id 为空）。
fn customer_invoice(
    id: &str,
    customer_id: &str,
    invoice_number: &str,
    total: f64,
) -> opc_invoices::ActiveModel {
    opc_invoices::ActiveModel {
        id: Set(id.to_string()),
        lead_id: Set(None),
        linked_workflow_id: Set(None),
        title: Set(None),
        customer_id: Set(Some(customer_id.to_string())),
        invoice_number: Set(Some(invoice_number.to_string())),
        line_items_json: Set(Some(r#"[{"description":"咨询","quantity":1}]"#.to_string())),
        subtotal: Set(Some(total)),
        tax_total: Set(Some(0.0)),
        total: Set(Some(total)),
        amount: Set(0.0),
        currency: Set("CNY".to_string()),
        status: Set("draft".to_string()),
        issued_at: Set(None),
        due_at: Set(None),
        paid_at: Set(None),
        notes: Set(None),
        created_at: Set(1_700_000_100),
        updated_at: Set(1_700_000_100),
    }
}

async fn count_invoices(db: &DatabaseConnection) -> u64 {
    opc_invoices::Entity::find().all(db).await.expect("查询发票应成功").len() as u64
}

#[tokio::test]
async fn upgrade_reconciliation_preserves_opc_invoices() {
    let h = create_test_pool().await.expect("测试：创建数据库连接池应成功");
    let db = &h.conn;

    // ① 升级前：同一张表写两种投影形态。
    opc_invoices::Entity::insert(delivery_invoice("inv-delivery-1", "lead-1", 500.0))
        .exec(db)
        .await
        .expect("插入交付发票应成功");
    opc_invoices::Entity::insert(customer_invoice("inv-customer-1", "cust-1", "INV-001", 226.0))
        .exec(db)
        .await
        .expect("插入客户账单发票应成功");
    assert_eq!(count_invoices(db).await, 2, "升级前应有 2 行");

    // ② 模拟升级：与生产同源的声明式收敛引擎再跑一遍。手工 migration 清单为空，
    //    这一步就是「升级」的建表/加列/索引收敛面。
    let outcome = bootstrap_schema(db).await.expect("升级收敛应成功");
    assert!(
        outcome.refusal.is_none(),
        "升级收敛不得被拒：{:?}",
        outcome.refusal.map(|r| r.reason())
    );

    // ③ 升级后：行数不变；两种投影逐字段保留。
    assert_eq!(count_invoices(db).await, 2, "升级不得删除既有业务行");

    let delivery = opc_invoices::Entity::find()
        .filter(opc_invoices::Column::Id.eq("inv-delivery-1"))
        .one(db)
        .await
        .expect("查询交付发票应成功")
        .expect("交付发票必须保留");
    assert_eq!(delivery.lead_id.as_deref(), Some("lead-1"));
    assert_eq!(delivery.amount, 500.0);
    assert_eq!(delivery.currency, "CNY");
    assert_eq!(delivery.customer_id, None);

    let customer = opc_invoices::Entity::find()
        .filter(opc_invoices::Column::Id.eq("inv-customer-1"))
        .one(db)
        .await
        .expect("查询客户账单应成功")
        .expect("客户账单必须保留");
    assert_eq!(customer.customer_id.as_deref(), Some("cust-1"));
    assert_eq!(customer.invoice_number.as_deref(), Some("INV-001"));
    assert_eq!(customer.total, Some(226.0));
    assert_eq!(
        customer.line_items_json.as_deref(),
        Some(r#"[{"description":"咨询","quantity":1}]"#)
    );
    assert_eq!(customer.lead_id, None);
}
