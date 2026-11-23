// SPDX-License-Identifier: AGPL-3.0-only

//! `bot_workflows` —— 多 Bot 编排定义（docs/bot-integration/P3/MODEL-P3 §2.1）。
//!
//! 步骤 DAG 存 `steps_json`（整行 JSON 演进，沿用 `cron_jobs.data` 先例）；
//! 归属恒实例所有，`department_id` 只做可见性筛选与成员校验（ADR-P3-3）。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[sea_orm(table_name = "bot_workflows")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub name: String,
    #[sea_orm(default_value = "")]
    pub description: String,
    #[sea_orm(default_value = "instance")]
    pub owner_scope: String,
    pub department_id: Option<String>,
    #[sea_orm(default_value = "[]")]
    pub steps_json: String,
    #[sea_orm(default_value = 2)]
    pub max_concurrency: i32,
    #[sea_orm(default_value = 900)]
    pub total_timeout_secs: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
