// SPDX-License-Identifier: AGPL-3.0-only

//! 任务事件脊（Task Ledger — P1-C）
//!
//! ## 定位
//!
//! `tasks` 的**追加型审计流**：每次状态迁移写一行，记录「谁、何时、从什么状态、
//! 到什么状态、为什么」。它是账本的**回执**，不参与状态判定 ——
//! 当前状态仍以业务表（本阶段为 `background_tasks.status`）为准。
//!
//! ## 为什么不把来源/父任务塞进 JSON
//!
//! 参照项目对 EvoFlow 的审计结论：EvoFlow 把 `source` / `parent_task_id` /
//! `handlers` 塞进 `extra_json`，导致无列约束、无索引、无法用 SQL 校验。
//! 本表把**会被查询的字段**全部做成真列 + 显式索引，`payload` 只承载
//! 「不确定形态的附加上下文」（如退出码、错误摘要）。
//!
//! ## 追加型保证
//!
//! 本表**只 insert，不 update、不 delete**。状态迁移与事件写入在同一事务内完成
//! （见 `axagent_dao::task_ledger::transition_task`），保证「状态变了但查不到事件」
//! 这种半写不会落库。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "task_events")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 所属任务的业务 id（对应 `background_tasks.id`；后续跨域汇入时为 `tasks.id`）。
    pub task_id: String,
    /// 任务的来源域，取值见 `harness::task_state::TaskSource`
    /// （`command` | `tool` | `restore` | `other`）。
    ///
    /// 做成真列的原因：P1-C 的终态是「三源写同一账本」，
    /// 而**排查问题时第一个要回答的就是「这条记录是哪个入口写的」**。
    pub source: String,
    /// 迁移前状态。创建事件为 NULL（此时没有前态）。
    pub from_status: Option<String>,
    /// 迁移后状态。取值见 `harness::task_state::TaskStatus`。
    pub to_status: String,
    /// 触发者：`user` | `system` | `scheduler` | `agent` | `unknown`。
    ///
    /// 与 `plans.authorized_by` 的白名单正交：那个字段回答「谁授权执行」，
    /// 这个字段回答「谁改了这一行状态」。
    pub actor: String,
    /// 人类可读的原因（写日志/排障用，不保证本地化）。
    pub reason: Option<String>,
    /// 附加上下文 JSON（如 `{"exitCode": -1}`）。形态不固定，故不做列。
    #[sea_orm(column_type = "Text", nullable)]
    pub payload: Option<String>,
    /// 事件时间（**毫秒**时间戳），且**同一任务内严格递增**。
    ///
    /// 后半条是「按时间排序」能成立的前提：同一任务内两次写入若落在同一毫秒，
    /// 排序键相等，而数据库对等值键之间的顺序不作保证 ⇒ 时间线可能倒过来。
    /// 因此写入侧（`axagent_dao::task_ledger::next_event_at`）取「该任务当前最大值 + 1」
    /// 而不是裸墙钟，代价是它可能略**超前**墙钟（同一毫秒内写 N 条时最多超前 N-1 毫秒）
    /// —— 这是**有意**的取舍：顺序语义比与墙钟逐毫秒对齐更重要。故本列只用于排序与展示，
    /// 任何超时 / 租约 / 过期判定都不得借用它。
    ///
    /// 全项目时间戳单位不一致是本项目的历史包袱（`stock_analyses` 用毫秒、
    /// `workflow_executions` 用秒）。本表**强制毫秒**，且写入侧
    /// （`task_ledger`）同时把 `background_tasks.created_at/updated_at`
    /// 统一为毫秒 —— 同域两套单位会让「按时间排序」静默错序。
    pub created_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
