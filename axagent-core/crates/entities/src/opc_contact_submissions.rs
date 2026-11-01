// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 联系表单提交表实体

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "opc_contact_submissions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub name: String,
    pub email: String,
    #[sea_orm(column_type = "Text")]
    #[sea_orm(default_value = "")]
    pub message: String,
    #[sea_orm(default_value = "")]
    pub source: String,
    #[sea_orm(column_name = "is_read")]
    #[sea_orm(default_value = 0)]
    pub is_read: i32,
    pub created_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
