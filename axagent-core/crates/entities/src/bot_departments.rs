// SPDX-License-Identifier: AGPL-3.0-only

//! `bot_departments` —— Bot 部门（docs/bot-integration/P2/MODEL-P2B §2.1）。
//!
//! 部门是 Bot 的常设组织归属 + Chief 标注，与「AI 办公室」的
//! `fleets`/`fleet_members`（AgentSession 为主键的临时协作场景）并存互不引用
//! （ADR-P2B-1）。`chief_bot_id` 的 FK 语义在命令层校验，不建 SQL 外键
//! （与 `conversations.bot_id` 同口径，P1 MODEL §3.2）。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[sea_orm(table_name = "bot_departments")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub name: String,
    #[sea_orm(default_value = "")]
    pub description: String,
    /// Chief Bot；FK 语义在命令层校验（bots 表存在且 enabled）。
    pub chief_bot_id: Option<String>,
    /// 预留（共享模板白名单等 P2-c 扩展位）。
    #[sea_orm(default_value = "{}")]
    pub metadata_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
