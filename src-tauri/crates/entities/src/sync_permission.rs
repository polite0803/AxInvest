// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// 设备权限表
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "sync_permissions")]
pub struct Model {
    /// 权限 ID（UUID）
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 设备 ID（一台设备只能有一条权限记录）
    ///
    /// ⚠ **`unique` 必须保留**：出处是已删的 `v116` 迁移的
    /// `device_id TEXT NOT NULL REFERENCES sync_devices(id) ON DELETE CASCADE UNIQUE`
    /// —— 那里的 `UNIQUE` 是**一台设备只有一条权限**这条业务约束的唯一载体。
    /// 删掉后全新库会允许同设备多行权限，`repo` 侧「先查后插」的兜底随之失效。
    /// （该行的 FK 部分由下面的 `Relation::Device` 承接，`UNIQUE` 部分只能由本属性承接。）
    ///
    /// 为什么不写 `#[sea_orm(indexed)]`：sea-orm `sea-orm-2.0.2/src/schema/entity.rs:156` 的条件是
    /// `indexed && !unique` ⇒ unique 列不派生普通索引，写了是无产出的死标志。
    #[sea_orm(unique)]
    pub device_id: String,
    /// 信任级别（backup_only/standard/full）
    #[sea_orm(default_value = "standard")]
    pub trust_level: String,
    /// 是否允许推送
    #[sea_orm(default_value = true)]
    pub can_push: bool,
    /// 是否允许拉取
    #[sea_orm(default_value = true)]
    pub can_pull: bool,
    /// 是否允许全量同步
    #[sea_orm(default_value = false)]
    pub can_full_sync: bool,
    /// 是否允许解决冲突
    #[sea_orm(default_value = false)]
    pub can_resolve_conflicts: bool,
    /// 是否允许管理设备
    #[sea_orm(default_value = false)]
    pub can_manage_devices: bool,
    /// 是否允许修改策略
    #[sea_orm(default_value = false)]
    pub can_modify_policy: bool,
    /// 权限过期时间（Unix 毫秒，null 表示永不过期）
    #[sea_orm(indexed)]
    pub expires_at: Option<i64>,
    /// 创建时间（Unix 毫秒）
    pub created_at: i64,
    /// 更新时间（Unix 毫秒）
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    /// 属于一个设备
    #[sea_orm(
        belongs_to = "super::sync_device::Entity",
        from = "Column::DeviceId",
        to = "super::sync_device::Column::Id",
        on_delete = "Cascade"
    )]
    Device,
}

impl Related<super::sync_device::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Device.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
