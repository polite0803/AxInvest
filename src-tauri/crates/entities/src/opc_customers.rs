// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 客户表实体

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "opc_customers")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub name: String,
    #[sea_orm(indexed)]
    pub email: String,
    pub phone: Option<String>,
    pub company: Option<String>,
    #[sea_orm(default_value = "unknown")]
    pub customer_type: String,
    pub country: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,
    pub address: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub source: Option<String>,
    #[sea_orm(column_type = "Text")]
    #[sea_orm(default_value = "[]")]
    pub tags_json: String,
    #[sea_orm(column_type = "Text")]
    #[sea_orm(default_value = "")]
    pub notes: String,
    #[sea_orm(default_value = 0.0)]
    pub total_revenue: f64,
    #[sea_orm(default_value = 0)]
    pub invoice_count: u32,
    #[sea_orm(indexed)]
    #[sea_orm(default_value = "lead")]
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
