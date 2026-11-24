// SPDX-License-Identifier: AGPL-3.0-only

//! `bot_department_quotas` —— 部门运行配额（docs/bot-integration/P4/MODEL-P4.md §2）。
//!
//! 部门是现有唯一组织维度，配额挂部门即无需引入新的 scope 体系（ADR-P4-1）。
//! 六个上限的 CHECK 上限**逐项等于 P3 默认值**：配额只能收紧、不能放大 P3 行为
//! （ADR-P4-2）；运行期生效值 = `min(定义值, 配额)`，超额一律拒绝并报字段。
//!
//! `department_id` 的 FK 语义在命令层校验（与 `chief_bot_id` 同口径，不建 SQL
//! 外键）；`UNIQUE(department_id)` 由 `reconcile/extras.rs` 的
//! `MULTI_COL_INDEXES` 声明（实体语法表达不了复合键，单列唯一索引同队列）。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[sea_orm(table_name = "bot_department_quotas")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 部门 id；一个部门至多一行配额（UNIQUE）。
    pub department_id: String,
    /// 单编排步骤并发上限（对应 `bot_workflow_runner` P3 默认 2，硬顶 4）。
    #[sea_orm(default_value = 2)]
    pub max_concurrency: i32,
    /// 单编排总超时上限（对应 P3 默认 900s，硬顶 3600）。
    #[sea_orm(default_value = 900)]
    pub total_timeout_secs: i32,
    /// 单编排步数上限（对应 P3 硬顶 12，不可放宽）。
    #[sea_orm(default_value = 12)]
    pub max_steps: i32,
    /// 委派链同层在途上限（对应 P3 默认 2，硬顶 4）。
    #[sea_orm(default_value = 2)]
    pub max_in_flight_per_depth: i32,
    /// 单步 / 链总超时上限（对应 P3 默认 300s，硬顶 900）。
    #[sea_orm(default_value = 300)]
    pub step_timeout_secs: i32,
    /// 该部门同时在跑的编排数上限（默认 2，硬顶 8）。
    #[sea_orm(default_value = 2)]
    pub max_concurrent_runs: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
