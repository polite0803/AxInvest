// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 需求发现 — 平台配置表（v131）
//!
//! 一行 = 一个需求平台连接器配置。`platform_type` 决定连接器实现：
//! api / scanner（内置扫描器）/ mock / manual。`id` 与内置扫描器
//! `platform()` 返回值一致（如 "reddit"），是扫描器路由的键。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "opc_demand_platforms")]
pub struct Model {
    /// 平台标识（自然主键，如 "reddit"）
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 展示名
    pub name: String,
    /// 连接器类型：api / scanner / mock / manual
    pub platform_type: String,
    /// 是否启用（布尔列按项目规范用 INTEGER，0/1）
    pub enabled: i32,
    /// 平台基础 URL，NULL 时用连接器默认端点
    pub base_url: Option<String>,
    /// 连接器扩展配置（JSON 字符串）
    pub config_json: String,
    /// 最近一次扫描成功时间戳（秒）
    pub last_sync_at: Option<i64>,
    /// 连接器状态：idle / ok / error
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
