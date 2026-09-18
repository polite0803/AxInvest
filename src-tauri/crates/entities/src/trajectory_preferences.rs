// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "trajectory_preferences")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 偏好键（唯一，写入按 key upsert）
    ///
    /// ⚠ **`unique` 必须保留**：出处是已删的 `v100` 迁移的
    /// `key TEXT NOT NULL UNIQUE`。偏好表是「一键一值」的 KV 语义，重复键会让
    /// 按 key 读到的结果取决于物理行序 —— 且 upsert 的「查不到就插」会退化成无限插入。
    /// （2026-09-16 P6 删迁移时靠本属性承接该语义。）
    ///
    /// 为什么不写 `#[sea_orm(indexed)]`：sea-orm `sea-orm-2.0.2/src/schema/entity.rs:156` 的条件是
    /// `indexed && !unique` ⇒ unique 列不派生普通索引，写了是无产出的死标志。
    #[sea_orm(unique)]
    pub key: String,
    pub value: String,
    #[sea_orm(default_value = 0.0)]
    pub confidence: f64,
    pub updated_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
