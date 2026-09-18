// SPDX-License-Identifier: AGPL-3.0-only

//! 能力域**覆盖层** —— 用户可改的域子集（启用/停用 + 追加别名）。
//!
//! # 覆盖层语义（不是「域定义表」）
//!
//! 域定义的**唯一权威源仍是编译期枚举** `axagent_harness::capability::CapabilityDomain`
//! 与声明 `axagent_harness::domain_registry::DOMAIN_NODES`。
//! 本表只存「与内置默认**不同**的部分」：
//!
//! - **无行 = 无覆盖** ⇒ 用内置默认（启用、无额外别名）；
//! - 有行 ⇒ 只覆盖 `enabled` 与 `extra_aliases` 两项；
//! - **不能增删域**：域的存在性由枚举决定，本表无法创造新域。
//!
//! # 为什么 `domain` 列不加 CHECK 约束
//!
//! 加 `CHECK (domain IN ('general', …))` 会把 9 个 id **再抄一份进 SQL** ——
//! 正是 `PLAN-domain-single-source.md` 要消灭的副本形态，且漏改时**静默拒绝写入**。
//! 取值合法性由命令层**解析成枚举**保证（`CapabilityDomain::from_str`）：
//! 枚举是单点真相源，新增域时编译器会强制走查全部消费端。
//!
//! # 与 `capability_policies` 的区别
//!
//! `capability_policies` 是**排除型 **过滤器规则**（策略对象化），作用于候选能力列表；
//! 本表是**域自身的**启用状态与别名，作用于 L1 分类器 prompt、L1 路由与能力过滤闸门。
//! 两者可叠加（都被停用的域仍然被策略排除，互不冲突）。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "capability_domain_overrides")]
pub struct Model {
    /// 域 id（协议 slug，如 `finance`）。主键 ⇒ 每域至多一行覆盖。
    ///
    /// ⚠ 合法性（必须是 9 个枚举变体之一）**不由本层保证**，由命令层解析枚举保证。
    #[sea_orm(primary_key, auto_increment = false)]
    pub domain: String,
    /// 是否启用该域（停用后 L1 prompt / L1 路由 / 能力过滤三处都会排除它）。
    #[sea_orm(default_value = true)]
    pub enabled: bool,
    /// **追加**别名（JSON 字符串数组，如 `["股票分析","选股"]`）。
    ///
    /// 语义是「在内置别名 `DOMAIN_NODES[].aliases` 之上**追加**」，不是替换：
    /// 内置别名（27 条存量兼容别名）永不因本表被删。
    #[sea_orm(default_value = "[]")]
    pub extra_aliases: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
