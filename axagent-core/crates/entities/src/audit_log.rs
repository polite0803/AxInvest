// SPDX-License-Identifier: AGPL-3.0-only

//! `audit_log` —— 工具调用审计流水。
//!
//! 每次工具执行写一行（成功/失败、耗时、输出预览、是否含敏感信息）。
//!
//! ## 时间戳必须是 64 位（原实现有 bug）
//! `timestamp` 写入的是 `chrono::Utc::now().timestamp_millis()`，约 **1.8e12**。
//! 原手写 DDL 把它声明成 `INTEGER`，即 PostgreSQL 的 int4（上限 2.1e9）
//! ⇒ 在 PG 上首次写入必然 `value out of range for type integer`。
//! 本实体用 `i64`：SQLite 生成 `INTEGER`（本就 64 位），PG 生成 `BIGINT`。
//!
//! 该 bug 此前未被发现，是因为审计写入路径从未真正启用
//! （`ToolAuditor` 的 db 恒为 `None`，见 `audit.rs` 的说明）。
//!
//! ## 本文件即建表契约
//! 说明见 `cron_job.rs`。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "audit_log")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    /// 调用时间（epoch 毫秒）—— 见上文「必须是 64 位」
    pub timestamp: i64,
    pub tool_name: String,
    /// 会话 ID；非会话上下文（如 fleet / plan 批量执行）时为 NULL
    pub conversation_id: Option<String>,
    /// 是否成功（0/1）。用 `i32` 而非 `bool`：SQLite 无原生布尔列类型。
    pub success: i32,
    /// 执行耗时（毫秒）
    pub duration_ms: i64,
    /// 输出前 200 字符（超长截断）
    #[sea_orm(column_type = "Text")]
    pub output_preview: String,
    /// 入参是否命中敏感信息脱敏
    pub has_sensitive_input: i32,
    /// 出参是否扫描出敏感信息
    pub has_sensitive_output: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
