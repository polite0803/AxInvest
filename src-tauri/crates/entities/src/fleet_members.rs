// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// 舰队成员 — 办公室里的一个 agent
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "fleet_members")]
pub struct Model {
    /// 唯一 ID（UUID）
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 所属舰队 ID
    #[sea_orm(indexed)]
    pub fleet_id: String,
    /// 关联的 AgentSession ID
    pub agent_id: String,
    /// agent slug（业务标识，用于 Dispatcher 路由）
    #[sea_orm(indexed)]
    pub agent_slug: String,
    /// 显示名称
    pub display_name: String,
    /// 角色描述
    #[sea_orm(default_value = "")]
    pub role: String,
    /// 关联的 AgentProfile ID（NULL = 旧成员，回退自由文本 role）
    pub agent_profile_id: Option<String>,
    /// 房间 ID（前端 Phaser 渲染位置）
    #[sea_orm(default_value = "workspace")]
    pub room_id: String,
    /// 成员状态：idle / busy / paused / error / offline
    #[sea_orm(indexed)]
    #[sea_orm(default_value = "idle")]
    pub status: String,
    /// 加入时间（Unix 毫秒）
    pub joined_at: i64,
    /// 今日 token 用量
    #[sea_orm(default_value = 0)]
    pub today_tokens: i64,
    /// 累计 token 用量
    #[sea_orm(default_value = 0)]
    pub total_tokens: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    /// 多个成员属于一个舰队
    #[sea_orm(
        belongs_to = "super::fleets::Entity",
        from = "Column::FleetId",
        to = "super::fleets::Column::Id",
        on_delete = "Cascade"
    )]
    Fleet,
}

impl Related<super::fleets::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Fleet.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
