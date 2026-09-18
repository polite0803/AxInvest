// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 域包注册表实体（Domain Pack 扫描/启用/禁用/版本追踪）

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "opc_domain_packs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub name: String,
    #[sea_orm(default_value = "🏢")]
    pub icon: String,
    #[sea_orm(column_type = "Text")]
    #[sea_orm(default_value = "")]
    pub description: String,
    #[sea_orm(default_value = 1)]
    pub version: i32,
    #[sea_orm(indexed)]
    #[sea_orm(default_value = 1)]
    pub enabled: i32,
    #[sea_orm(column_type = "Text")]
    #[sea_orm(default_value = "")]
    pub pack_path: String,
    pub installed_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
