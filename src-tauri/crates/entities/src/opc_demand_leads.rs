// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 需求发现 — 需求线索表（v131）
//!
//! 一行 = 一条扫描到并完成评估的需求线索。评分因子（pain/market_gap/
//! commercial_value）落列存储，支持按价值分排序查询；评估在 `axagent_tools`
//! 的 `marketplace_scanner` 完成，本表只做持久化。
//!
//! 去重（v136）：`(platform, content_fingerprint)` 上的唯一索引（NULL 不参与
//! 唯一约束，无指纹线索可重复插入）。旧键 `(platform, source_url)` 会在
//! 所有线索共享同一搜索页 URL 的平台上互相踩踏，已废弃。

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
    /// 来源 URL（展示用；v136 起去重键迁移为内容指纹）
    pub source_url: Option<String>,
    /// 内容指纹（标题+描述归一化哈希，v136）：去重主键；NULL = 旧数据/空内容不参与
    pub content_fingerprint: Option<String>,
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
    /// 转化生成的实现工作流模板 ID（v132；NULL = 未转化）
    pub linked_workflow_id: Option<String>,
    /// 首次启动实现工作流执行的时间戳（秒；NULL = 未执行）
    pub implemented_at: Option<i64>,
    /// 入库时间戳（秒）
    pub created_at: i64,
    /// 更新时间戳（秒）
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
