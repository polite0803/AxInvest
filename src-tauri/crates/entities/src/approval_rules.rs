// SPDX-License-Identifier: AGPL-3.0-only

//! 审批规则（PLAN-codex-parity R2-1）—— 「批准沉淀」的落库表。
//!
//! 主键 `id` 由 `program` + `args_prefix` 派生（见 wiring 层
//! `init/approval_rule_store.rs` 的 `rule_id`），使同一规则的 upsert 幂等：
//! 同一 `(program, args_prefix)` 只会有一行。
//!
//! `args_prefix` 以 `\u{1f}`（Unit Separator）连接存储 —— 程序名与参数中
//! 不会出现该控制字符，且无需引入 JSON 序列化开销。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "approval_rules")]
pub struct Model {
    /// 派生主键：`{program}\u{1f}{args_prefix 以 \u{1f} 连接}`。
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 程序名（小写归一化后）。
    #[sea_orm(indexed)]
    pub program: String,
    /// 参数前缀，以 `\u{1f}` 连接；空串表示 program 级规则。
    #[sea_orm(default_value = "")]
    pub args_prefix: String,
    /// 规则裁决：`allow` / `prompt` / `forbidden`（对应 `RuleDecision::as_str`）。
    #[sea_orm(default_value = "allow")]
    pub decision: String,
    /// 沉淀来源（会话 id 等，用于追溯）。
    #[sea_orm(default_value = "")]
    pub source: String,
    /// 创建时间（Unix 秒）。
    pub created_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
