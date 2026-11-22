// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Bot —— 身份 + SOUL + 工作区 + 记忆 + 线程归属的宿主记录。
/// 与 `agent_profiles`（专家/角色模板）职责不同：Bot 是主人，profile 是模板。
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[sea_orm(table_name = "bots")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    #[sea_orm(default_value = "")]
    pub soul: String,
    #[sea_orm(default_value = "")]
    pub soul_hash: String,
    #[sea_orm(default_value = 0)]
    pub soul_drift: i32,
    #[sea_orm(default_value = "🤖")]
    pub icon: String,
    pub color: Option<String>,
    #[sea_orm(indexed)]
    #[sea_orm(default_value = "general")]
    pub section: String,
    pub model_id: Option<String>,
    pub provider_id: Option<String>,
    pub effort: Option<String>,
    pub workdir: Option<String>,
    #[sea_orm(default_value = "full")]
    pub approval_mode: String,
    pub agent_profile_id: Option<String>,
    #[sea_orm(default_value = 1)]
    pub is_enabled: i32,
    #[sea_orm(default_value = 0)]
    pub sort_order: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
