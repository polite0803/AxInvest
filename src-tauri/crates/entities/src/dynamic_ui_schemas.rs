// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "dynamic_ui_schemas")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub title: String,
    #[sea_orm(default_value = "")]
    pub description: String,
    #[sea_orm(column_type = "Text")]
    pub schema_json: String,
    #[sea_orm(indexed)]
    #[sea_orm(default_value = "custom")]
    pub category: String,
    #[sea_orm(default_value = "[]")]
    pub tags: String,
    #[sea_orm(default_value = "1.0.0")]
    pub version: String,
    #[sea_orm(default_value = 0)]
    pub is_builtin: i32,
    /// 来源维度（`builtin` / `user` / `ai` / `plugin`）。
    ///
    /// `is_builtin` 是二值，判不开「AI 生成」与「插件注入」—— 二者信任级别不同。
    /// 本列是**纯展示元数据**，不参与任何权限分支（与 `is_builtin` 的守卫职责分离）。
    #[sea_orm(indexed)]
    #[sea_orm(default_value = "user")]
    pub origin: String,
    /// 来源归属：`origin = "plugin"` 时为 pluginId，用于卸载插件时**精确撤销**其 UI 贡献；
    /// 其余来源为空串。
    #[sea_orm(default_value = "")]
    pub owner_id: String,
    pub created_at: String,
    #[sea_orm(indexed)]
    pub updated_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
