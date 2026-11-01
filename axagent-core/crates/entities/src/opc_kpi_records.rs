// SPDX-License-Identifier: AGPL-3.0-only

//! OPC KPI 记录表实体
//!
//! ## `domain_pack_id` 为什么是**必填**（v230 新增）
//!
//! OPC 是**多域包**系统（`config/opc/domain_packs/<id>/`），同一 `name` 的 KPI
//! （如 `word_count`）在不同域包下是完全不同的量。本列缺失时，读写两侧都跨域包串号：
//! 写端把 A 域包的采样值放进一张无归属的表，读端只能按 `name` 取「最新一条」
//! ⇒ B 域包的仪表盘会读到 A 域包的值。
//!
//! 故本列 `NOT NULL`、**无 DEFAULT**（见 `migrations/v230_opc_kpi_capability_pack.rs`）：
//! `DEFAULT ''` 会留一条「写入者忘传 ⇒ 落一个查不出来的行」的静默通道。
//! `''` 仅表示**存量/迁移回填行**的「未标注」，不是新写入的合法取值 ——
//! 新写入必须显式给值（读侧对 `''` 的处理见 `opc::analytics`）。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "opc_kpi_records")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 域包 ID（与 `config/opc/domain_packs/<id>` 同名，下划线形式，如 `content_media`）。
    pub domain_pack_id: String,
    #[sea_orm(indexed)]
    pub name: String,
    pub value: f64,
    #[sea_orm(default_value = "")]
    pub unit: String,
    #[sea_orm(indexed)]
    #[sea_orm(default_value = "")]
    pub period: String,
    pub recorded_at: i64,
    pub created_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
