// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 落地页表实体

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "opc_landing_pages")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub title: String,
    /// URL slug（唯一，落地页按 slug 对外可达）
    ///
    /// ⚠ **`unique` 必须保留**：出处是已删的 `v210` 迁移的
    /// `slug TEXT NOT NULL UNIQUE`。slug 是**对外路由键**，重复即两个页面争同一路径。
    /// （2026-09-16 P6 删迁移时靠本属性承接该语义。）
    ///
    /// 为什么不写 `#[sea_orm(indexed)]`：sea-orm `sea-orm-2.0.2/src/schema/entity.rs:156` 的条件是
    /// `indexed && !unique` ⇒ unique 列不派生普通索引，写了是无产出的死标志。
    #[sea_orm(unique)]
    pub slug: String,
    #[sea_orm(column_type = "Text")]
    #[sea_orm(default_value = "")]
    pub description: String,
    #[sea_orm(column_type = "Text")]
    #[sea_orm(default_value = "")]
    pub content: String,
    #[sea_orm(default_value = 0)]
    pub published: i32,
    pub published_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
