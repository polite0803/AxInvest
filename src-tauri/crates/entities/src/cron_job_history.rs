// SPDX-License-Identifier: AGPL-3.0-only

//! `cron_job_history` —— 定时任务单次执行记录。
//!
//! ## 时间戳必须是 64 位
//! `started_at` / `completed_at` 存 **epoch 毫秒**（约 1.8e12），而 PostgreSQL 的
//! `INTEGER` 是 int4（上限 2.1e9）。若建表成 `integer`，首次 `record_run` 就会报
//! `value out of range for type integer`。
//!
//! 本实体用 `i64`：SQLite 生成 `INTEGER`（本就是 64 位），PG 生成 `BIGINT`，
//! 两边都安全。改这两个字段的类型前请先算一遍毫秒量级。
//!
//! ## 本文件即建表契约
//! 说明见同目录 `cron_job.rs`。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "cron_job_history")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 所属任务 ID（对应 `cron_jobs.id`）
    pub task_id: String,
    /// 开始时间（epoch 毫秒）
    pub started_at: i64,
    /// 完成时间（epoch 毫秒）；任务中途崩溃时为 NULL
    pub completed_at: Option<i64>,
    /// 是否成功（0/1）。用 `i32` 而非 `bool`：SQLite 无原生布尔列类型，
    /// 用 `bool` 会让 SQLite / PG 两边的存储形态不一致。
    #[sea_orm(default_value = 0)]
    pub success: i32,
    /// 任务输出（截断后）
    #[sea_orm(column_type = "Text")]
    pub output: Option<String>,
    /// 失败原因
    #[sea_orm(column_type = "Text")]
    pub error: Option<String>,
    /// 耗时（毫秒）
    #[sea_orm(default_value = 0)]
    pub duration_ms: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
