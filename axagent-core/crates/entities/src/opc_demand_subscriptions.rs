// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 需求发现 — 需求订阅词表（v133）
//!
//! 一行 = 一个长期跟踪的需求关键词。`opc_demand_scan` 定时任务按
//! `interval_hours` 挑出到期的订阅，逐个跑扫描→评估→入库，
//! 命中 `min_score` 的高价值线索才走 delivery 推送。
//!
//! 与线索表（`opc_demand_leads`）无外键：订阅是**扫描意图**，线索是**扫描产物**，
//! 两者是多对多（同一线索可被多个订阅词命中），用外键反而束缚。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "opc_demand_subscriptions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 订阅关键词（唯一，大小写敏感由 DB collation 决定，入库前 trim）
    ///
    /// ⚠ **`unique` 必须保留**：本注释声称「唯一」，而不带该属性时引擎不会给列加
    /// 唯一约束 —— 全新库上就会出现允许重复关键词的表，注释与实态相反。
    /// 出处：已删的 `v135` 迁移的
    /// `CREATE UNIQUE INDEX idx_opc_demand_subs_keyword`（P6 删迁移时是靠本属性
    /// 保住这条语义的，不是靠那个索引名字）。
    #[sea_orm(unique)]
    pub keyword: String,
    /// 是否启用（布尔列按项目规范用 INTEGER，0/1）
    #[sea_orm(default_value = 1)]
    pub enabled: i32,
    /// 扫描间隔（小时），到期判定 = last_scanned_at + interval_hours*3600 <= now
    #[sea_orm(default_value = 6)]
    pub interval_hours: i32,
    /// 推送门槛：商业价值分低于此值不计入高价值命中
    #[sea_orm(default_value = 60)]
    pub min_score: f64,
    /// 限定平台 ID 列表（JSON 字符串数组）；空数组 = 跟随全局启用的平台
    #[sea_orm(default_value = "[]")]
    pub platforms_json: String,
    /// 最近一次扫描时间戳（秒），NULL = 从未扫描（立即到期）
    pub last_scanned_at: Option<i64>,
    /// 最近一次扫描的高价值命中数（前端展示用，不参与到期判定）
    #[sea_orm(default_value = 0)]
    pub last_hit_count: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
