// SPDX-License-Identifier: AGPL-3.0-only

//! `bot_skill_bindings` —— 每 Bot 技能归属声明（docs/bot-integration/P5/MODEL-P5.md §4）。
//!
//! `UNIQUE(bot_id, skill_name)` 由 `reconcile/extras.rs` 的 `MULTI_COL_INDEXES`
//! 声明。目录是真相源，本表只作归属声明与审计（ADR-P5-5）。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[sea_orm(table_name = "bot_skill_bindings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub bot_id: String,
    pub skill_name: String,
    /// `workspace` / `preset`
    #[sea_orm(default_value = "workspace")]
    pub source: String,
    pub preset_id: Option<String>,
    /// 本 Bot 停用标记（与全局 skill_states 分层）
    #[sea_orm(default_value = true)]
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
