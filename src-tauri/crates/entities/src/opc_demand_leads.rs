// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 需求发现 — 需求线索表（v131）
//!
//! 一行 = 一条扫描到并完成评估的需求线索。评分因子（pain/market_gap/
//! commercial_value）落列存储，支持按价值分排序查询；评估在 `axagent_tools`
//! 的 `marketplace_scanner` 完成，本表只做持久化。
//!
//! 去重：`(platform, source_url)` 上的唯一索引（NULL 不参与唯一约束，
//! 手动补录无 URL 的线索可重复插入）。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "opc_demand_leads")]
pub struct Model {
    /// 线索 ID（`{platform}_{uuid}`，生成逻辑见 DemandLead::new_from_raw）
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 来源平台标识
    pub platform: String,
    pub title: String,
    pub description: String,
    /// 预算下限
    pub budget_min: Option<f64>,
    /// 预算上限
    pub budget_max: Option<f64>,
    /// 币种（默认 CNY）
    pub budget_currency: String,
    pub contact_name: Option<String>,
    pub contact_email: Option<String>,
    pub contact_phone: Option<String>,
    /// 来源 URL（与 platform 组成去重键）
    pub source_url: Option<String>,
    /// 平台原始返回数据（JSON 字符串）
    pub raw_snapshot: String,
    /// 生命周期：new / evaluated / contacted / won / lost
    pub status: String,
    /// 评估置信度 0-1
    pub confidence: f64,
    /// 痛点强度 0-100
    pub pain_score: f64,
    /// 市场空白度 0-100
    pub market_gap_score: f64,
    /// 商业价值综合分 0-100
    pub commercial_value_score: f64,
    /// 需求类型（snake_case 标识）
    pub demand_type: String,
    /// 入库时间戳（秒）
    pub created_at: i64,
    /// 更新时间戳（秒）
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
