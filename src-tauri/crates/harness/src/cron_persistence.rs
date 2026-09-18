// SPDX-License-Identifier: AGPL-3.0-only

//! 定时任务的持久化**端口**（port）—— 与 `cron_delivery.rs` 同一套 port/adapter 设计。
//!
//! ## 为什么需要它（2026-09-18，修 `check:contracts` 的 [D] 越界依赖）
//!
//! `crates/runtime-core/src/cron_job.rs` 原先直接
//! `use axagent_entities::{cron_job, cron_job_history}`，并自己 `Schema::create_table_from_entity`
//! 建表、自己 upsert、自己写历史。而本仓分层规则（`scripts/check-contracts.mjs` 的 [D] 项）是
//! **consumer crate（agent / gateway / orchestrator / runtime-core）只允许依赖 `axagent-harness`**，
//! `runtime-core` 正是其中之一 ⇒ 这条依赖被门禁判为「越界依赖实现层」。
//!
//! 这**不是**门禁过严：本 crate 在 `cron_delivery.rs` 的文件头已经把同一条设计写清楚了 ——
//! 「DTO + Trait 在 harness，runtime-core 通过 trait 调用，不直接依赖实现层。
//! 具体实现在 wiring 层」。`cron_job.rs` 只是漏了这一层，于是它既当契约又当实现。
//!
//! ## 形状上的两条约束
//!
//! 1. **只传纯数据**：任务本体以 JSON 字符串出入（`CronJob` 的序列化/反序列化留在
//!    runtime-core），历史记录用 [`CronHistoryRecord`]——它不引用任何 ORM / 表类型。
//!    端口上出现 `sea_orm::Value` 之类的类型就等于把依赖又漏回来了。
//! 2. **错误用 `Result<_, String>`**：端口在 harness，而 harness 不依赖任何错误库；
//!    调用方（runtime-core）负责把它转成自己的错误呈现方式。
//!
//! ## 具体实现
//!
//! `axagent-dao::repo::cron_job_persistence::PgCronJobPersistence`，在 `src/init/state.rs`
//! 里构造并注入。

use async_trait::async_trait;

/// 一条定时任务的**执行历史**（纯数据）。
///
/// 字段与 `cron_job_history` 表逐列对应，但**不引用**该表的实体类型 ——
/// 这样 runtime-core 才能在不知道表存在的前提下读写它。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronHistoryRecord {
    pub id: String,
    pub task_id: String,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    /// 表里是 `integer`（0/1），端口上就是 `bool`。
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<String>,
    pub duration_ms: i64,
}

/// 定时任务的持久化端口。
///
/// ⚠ 实现方必须是 `Send + Sync`：`CronJobStore` 会被放进 `Arc` 跨任务共享。
#[async_trait]
pub trait CronJobPersistence: Send + Sync {
    /// 确保两张表存在（幂等），并按需做列类型修复。
    ///
    /// 幂等是硬要求：本方法在**每次进程启动**时都会被调用，
    /// 而表在绝大多数启动里已经存在。
    async fn ensure_tables(&self);

    /// 读出全部任务本体（`CronJob` 的 JSON）。排序由调用方在应用层做。
    async fn load_jobs(&self) -> Result<Vec<String>, String>;

    /// 按 id upsert 一条任务（不存在则插入）。
    async fn upsert_job(&self, id: &str, data: &str) -> Result<(), String>;

    /// 按 id 删除一条任务。目标不存在不算错（幂等）。
    async fn delete_job(&self, id: &str) -> Result<(), String>;

    /// 追加一条执行历史。
    async fn insert_history(&self, record: CronHistoryRecord) -> Result<(), String>;

    /// 按任务 id 取最近的执行历史（按 `started_at` 倒序，最多 `limit` 条）。
    async fn load_history(
        &self,
        task_id: &str,
        limit: i64,
    ) -> Result<Vec<CronHistoryRecord>, String>;
}
