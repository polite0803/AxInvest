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
    /// ⚠ 本列**是授权锚点，不是展示元数据**（2026-10 起由提交 `43803e5ac` 收口）：
    /// 所有按 id 寻址的 `dynamic_ui_*` 命令都先过 `commands/dynamic_ui.rs` 的统一闸门
    /// `load_schema_for_command`，其判据 `plugin_owned_denied` 只看本列 —— 不在
    /// `TRUSTED_SCHEMA_ORIGINS`（`builtin` / `user` / `ai`）内即拒。前端权限面
    /// （`src/lib/dynamicUI/schemaActionPermissions.ts`）是另一道，两者不互相替代。
    /// 展示用的「谁生成的」语义请另加字段，**不要复用本列**。
    #[sea_orm(indexed)]
    #[sea_orm(default_value = "user")]
    pub origin: String,
    /// 来源归属：`origin = "plugin"` 时为 pluginId，用于卸载插件时**精确撤销**其 UI 贡献；
    /// 其余来源为空串。
    ///
    /// ⚠ 本列**刻意不参与授权判定**（不是漏了归属校验）：非插件行的 `owner_id` 恒为空串
    /// （`create_dynamic_ui_schema` 硬写空），拿它比等于零区分度；而命令面上不存在
    /// 「不可自报的调用方身份」⇒ 加它也挡不住插件自称归属。后续请勿再提「补 owner_id 校验」。
    #[sea_orm(default_value = "")]
    pub owner_id: String,
    pub created_at: String,
    #[sea_orm(indexed)]
    pub updated_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
