// SPDX-License-Identifier: AGPL-3.0-only

//! 进化产物执行统计表（DDL 由 v122 建表）。
//!
//! 一行 = 某个会话内某个工具（进化产物）的真实执行反馈累计值。复合主键
//! `(conversation_id, tool_id)`。
//!
//! 写入方：`EvolutionFeedbackSinkImpl::record` 在更新内存统计后异步落库；
//! 应用启动时一次性读回内存（`load_all_execution_stats`），保证重启后真实
//! 执行证据不丢失（阶段四后置闭环 D3）。
//!
//! 持久化入口为 [`axagent_dao::repo::evolution_execution_stats`]。
//! **列宽注意**：DDL 声明为 `INTEGER`（PG = int4），故此处用 `i32` 而非 `i64`；
//! 若某库被 `schema_diff` 的无损加宽规则改成了 `BIGINT`，本实体应随之调整。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "evolution_execution_stats")]
pub struct Model {
    /// 会话 ID
    #[sea_orm(primary_key, auto_increment = false)]
    pub conversation_id: String,
    /// 工具 ID（进化产物标识）
    #[sea_orm(primary_key, auto_increment = false)]
    pub tool_id: String,
    /// 累计使用次数
    #[sea_orm(default_value = 0)]
    pub usage_count: i32,
    /// 累计成功次数
    #[sea_orm(default_value = 0)]
    pub successes: i32,
    /// 累计失败次数
    #[sea_orm(default_value = 0)]
    pub failures: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
