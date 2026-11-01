// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 能力缺口记录表实体
//!
//! 需求匹配时，若热门/高价值需求的能力集暂不满足，则落档一条能力缺口，
//! 供后续能力建设（新增工具/技能/工作流模板）驱动。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "opc_capability_gap")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    #[sea_orm(indexed)]
    pub lead_id: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub title: String,
    #[sea_orm(column_type = "Text")]
    #[sea_orm(default_value = "")]
    pub description: String,
    #[sea_orm(column_type = "Text")]
    #[sea_orm(default_value = "")]
    pub missing_capability: String,
    #[sea_orm(default_value = "capability")]
    pub gap_type: String,
    #[sea_orm(column_type = "Text")]
    #[sea_orm(default_value = "")]
    pub suggested_action: String,
    #[sea_orm(default_value = 3)]
    pub priority: i32,
    #[sea_orm(indexed)]
    #[sea_orm(default_value = "open")]
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub closed_at: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
