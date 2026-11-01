// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// 舰队消息 — 群聊 / 私信消息的**持久化**记录。
///
/// ## 为什么需要这张表
///
/// 此前 Fleet 的消息只存在于前端内存（`officeStore.dispatchEvents`），
/// `DispatchInput.history` 由调用方每次临时传入 ⇒ 刷新即丢、无法回溯，
/// agent 也无法基于「会话的真实历史」决策。
///
/// 本表是协调门的地基：判据全都是「有没有比某个水位更新的消息」，
/// 没有持久化的单调 `seq` 就无从比较。
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "fleet_messages")]
pub struct Model {
    /// 唯一 ID（UUID）
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 所属舰队 ID
    pub fleet_id: String,
    /// **会话作用域** ID —— 消息线程的归属，不是成员站位的房间。
    ///
    /// - 群聊：`"group"`（见 `axagent_harness::fleet::CONVERSATION_GROUP`）
    /// - 私信：`"dm:<agent_slug>"`（见 `axagent_harness::fleet::conversation_dm`）
    ///
    /// ⚠ 与 `fleet_members.room_id` **不是同一个概念**：后者是像素办公室里
    /// 精灵站的物理房间（前端渲染用），从不参与消息查询。曾用同一个名字
    /// `room_id` 导致「DM 与群聊共享一条时间线」而无人察觉，故改名断开歧义。
    #[sea_orm(default_value = "group")]
    pub conversation_id: String,
    /// 单调递增序号（**同一 `(fleet_id, conversation_id)` 内唯一且递增**）
    ///
    /// 由 DAO 以 `MAX(seq)+1` 分配（读与写是两条独立语句，**不在同一事务**），
    /// 并以 `UNIQUE(fleet_id, conversation_id, seq)` 兜底并发写入（冲突则重试）。
    /// 所有协调门的比较基线都是它，因此**必须单调** —— 时间戳不具备这个性质，故不采用。
    pub seq: i64,
    /// 作者类型：`human` / `agent`
    #[sea_orm(default_value = "agent")]
    pub author_kind: String,
    /// 作者 ID（human 用固定本地用户标识；agent 用 `agent_id`）
    pub author_id: String,
    /// 作者 slug（仅 agent 有）
    pub author_slug: Option<String>,
    /// 作者显示名（human 为 NULL，前端按 i18n 渲染「我」）
    pub author_display_name: Option<String>,
    /// 消息正文
    pub content: String,
    /// 创建时间（Unix 毫秒）
    pub created_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    /// 多条消息属于一个舰队
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
