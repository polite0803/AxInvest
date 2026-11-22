// SPDX-License-Identifier: AGPL-3.0-only

//! `bot_department_members` —— 部门归属 join 表（docs/bot-integration/P2/MODEL-P2B §2.2）。
//!
//! UNIQUE(department_id, bot_id) 是**功能必需**（Q2：一个 Bot 只属一个部门），
//! 复合 UNIQUE 由 L2 例外清单 `reconcile::extras::MULTI_COL_INDEXES` 声明
//! （实体语法表达不了复合键）。
//!
//! 删除级联全部在命令层：删部门 → 删成员行；Bot 被删 → `delete_bot` 追加清成员行
//! （GAP G-10 同款扫描）。不依赖 SQL 外键。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[sea_orm(table_name = "bot_department_members")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    #[sea_orm(indexed)]
    pub department_id: String,
    #[sea_orm(indexed)]
    pub bot_id: String,
    pub joined_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
