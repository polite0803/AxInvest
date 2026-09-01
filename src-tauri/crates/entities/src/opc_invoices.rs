// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 交付 — 发票（v136）
//!
//! 一张表承载两种场景：
//! 1. **交付发票**：won 线索开票生成（`opc_create_invoice_from_lead`），
//!    lead_id 溯源到需求线索；状态机 draft → sent → paid 单向推进。
//! 2. **客户账单**：analysis-engine 完整 InvoiceService 使用，
//!    customer_id + invoice_number + line_items_json（行项目 JSON）。
//!
//! 两种场景字段互斥为可空：交付场景填 lead_id，客户场景填 customer_id。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "opc_invoices")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    // ── 交付场景字段（可空：客户账单场景不填）──
    /// 来源线索（won）；无外键约束 —— 线索删除不应连带清账
    pub lead_id: Option<String>,
    /// P2 转化出的交付工作流（可空：人工交付无工作流）
    pub linked_workflow_id: Option<String>,
    /// 交付发票标题（默认取线索标题，可改）
    pub title: Option<String>,
    // ── 客户账单场景字段（可空：交付场景不填）──
    /// 客户 ID（客户账单场景）
    pub customer_id: Option<String>,
    /// 发票编号（客户账单场景）
    pub invoice_number: Option<String>,
    /// 行项目 JSON（客户账单场景，存 Vec<InvoiceLineItem>）
    pub line_items_json: Option<String>,
    /// 小计金额（客户账单场景）
    pub subtotal: Option<f64>,
    /// 税额（客户账单场景）
    pub tax_total: Option<f64>,
    /// 总额（客户账单场景）
    pub total: Option<f64>,
    // ── 通用字段 ──
    /// 金额（交付场景取线索预算上限，客户场景取 total）
    pub amount: f64,
    /// 币种（ISO 4217，默认 CNY）
    pub currency: String,
    /// 状态机：draft → sent → paid（可扩展 overdue / cancelled / refunded）
    pub status: String,
    /// 标记 sent 的时间戳（秒）
    pub issued_at: Option<i64>,
    /// 到期时间戳（秒，客户账单场景）
    pub due_at: Option<i64>,
    /// 标记 paid 的时间戳（秒）
    pub paid_at: Option<i64>,
    /// 备注
    pub notes: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
