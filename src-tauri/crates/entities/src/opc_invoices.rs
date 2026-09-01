// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 交付 — 发票（v134）
//!
//! 一行 = 一张交付发票。由 won 线索开票生成（`opc_create_invoice_from_lead`），
//! 状态机 `draft → sent → paid` 单向推进；`linked_workflow_id` 溯源到
//! P2 转化出的交付工作流。多币种并存，汇总按币种分组。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "opc_invoices")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 来源线索（won）；无外键约束 —— 线索删除不应连带清账
    pub lead_id: String,
    /// P2 转化出的交付工作流（可空：人工交付无工作流）
    pub linked_workflow_id: Option<String>,
    /// 发票标题（默认取线索标题，可改）
    pub title: String,
    /// 金额（默认取线索预算上限，可改）；多币种并存，汇总按币种分组
    pub amount: f64,
    /// 币种（ISO 4217，默认取线索预算币种）
    pub currency: String,
    /// 状态机：draft → sent → paid（单向，同状态幂等）
    pub status: String,
    /// 标记 sent 的时间戳（秒）
    pub issued_at: Option<i64>,
    /// 标记 paid 的时间戳（秒）
    pub paid_at: Option<i64>,
    /// 备注（可空）
    pub notes: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
