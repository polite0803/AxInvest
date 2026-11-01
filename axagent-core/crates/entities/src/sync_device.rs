// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// 同步设备表
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "sync_devices")]
pub struct Model {
    /// 设备 ID（UUID）
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 设备名称
    pub name: String,
    /// 设备类型（desktop/mobile/server）
    pub device_type: String,
    /// 操作系统
    pub os: String,
    /// 应用版本
    pub app_version: String,
    /// 唯一设备标识符
    ///
    /// ⚠ **`unique` 必须保留**：注释声称「唯一」，而不带该属性时引擎不会给列加唯一约束
    /// —— 全新库上就会出现**允许重复设备标识**的表，注释与实态相反。
    /// 出处：已删的 `v116` 迁移的
    /// `unique_id TEXT NOT NULL UNIQUE`（P6 删迁移时靠**本属性**保住这条语义，
    /// 不是靠那个索引名字 —— 索引名是 `idx-{表}-{列}` 形态，与迁移里的
    /// `idx_{表}_{列}` 永远对不上，靠名字比对永远看不见这条损失）。
    ///
    /// 为什么不写 `#[sea_orm(indexed)]`：sea-orm `sea-orm-2.0.2/src/schema/entity.rs:156` 的条件是
    /// `indexed && !unique` ⇒ unique 列**不会**派生普通索引。写了也只是个
    /// 「声明了但没产出」的死标志，且会让人误以为索引另有来源。
    #[sea_orm(unique)]
    pub unique_id: String,
    /// 公钥（用于加密通信）
    pub public_key: String,
    /// IP 地址
    pub ip_address: Option<String>,
    /// 配对状态
    #[sea_orm(indexed)]
    #[sea_orm(default_value = false)]
    pub is_paired: bool,
    /// 受信任级别（backup_only/standard/full）
    #[sea_orm(default_value = "standard")]
    pub trust_level: String,
    /// 最后同步时间（Unix 毫秒）
    pub last_synced_at: Option<i64>,
    /// 最后心跳时间（Unix 毫秒）
    pub last_heartbeat_at: Option<i64>,
    /// 是否启用
    #[sea_orm(default_value = true)]
    pub is_enabled: bool,
    /// 创建时间（Unix 毫秒）
    pub created_at: i64,
    /// 更新时间（Unix 毫秒）
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
