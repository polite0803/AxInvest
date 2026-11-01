// SPDX-License-Identifier: AGPL-3.0-only

//! Fleet 持久化实现 — 基于 SeaORM 的 `FleetRepository` trait 实现。
//!
//! ## 设计
//!
//! - 直接操作 `axagent_entities::fleets` / `fleet_members` / `fleet_messages` 三张表
//! - 不使用内存索引（每次查询直接走 DB），简化并发控制
//! - 错误统一转为 `String` 返回（符合 harness 错误隔离约定）
//! - 状态枚举在 DB 中以字符串存储（snake_case），与 harness DTO 双向映射
//! - **双数据库兼容**：所有 SeaORM 抽象层操作均同时支持 SQLite 和 PostgreSQL
//!   （SQLite 用 INTEGER 存 i64，PG 用 BIGINT；字符串列类型一致）

use async_trait::async_trait;
use axagent_entities::{fleet_members, fleet_messages, fleets};
use axagent_harness::fleet::{
    AUTHOR_KIND_AGENT, Fleet, FleetMember, FleetMemberStatus, FleetMessage, FleetMetadata,
    FleetRepository, FleetStatus,
};
use sea_orm::sea_query::Expr;
use sea_orm::*;

/// SeaORM 实现的 FleetRepository（同时兼容 SQLite 与 PostgreSQL）
pub struct SeaOrmFleetRepository {
    db: DatabaseConnection,
}

impl SeaOrmFleetRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}

// ── 状态枚举与字符串的双向转换（DB 存 snake_case） ──

fn fleet_status_to_str(status: FleetStatus) -> &'static str {
    match status {
        FleetStatus::Active => "active",
        FleetStatus::Paused => "paused",
        FleetStatus::Stopped => "stopped",
    }
}

fn fleet_status_from_str(s: &str) -> FleetStatus {
    match s {
        "paused" => FleetStatus::Paused,
        "stopped" => FleetStatus::Stopped,
        _ => FleetStatus::Active,
    }
}

fn member_status_to_str(status: FleetMemberStatus) -> &'static str {
    match status {
        FleetMemberStatus::Idle => "idle",
        FleetMemberStatus::Busy => "busy",
        FleetMemberStatus::Paused => "paused",
        FleetMemberStatus::Error => "error",
        FleetMemberStatus::Offline => "offline",
    }
}

fn member_status_from_str(s: &str) -> FleetMemberStatus {
    match s {
        "busy" => FleetMemberStatus::Busy,
        "paused" => FleetMemberStatus::Paused,
        "error" => FleetMemberStatus::Error,
        "offline" => FleetMemberStatus::Offline,
        _ => FleetMemberStatus::Idle,
    }
}

// ── Entity → DTO 转换 ──

fn fleet_from_entity(m: fleets::Model) -> Fleet {
    let metadata: FleetMetadata = serde_json::from_str(&m.metadata_json).unwrap_or_default();
    Fleet {
        id: m.id,
        name: m.name,
        scene_template_slug: m.scene_template_slug,
        status: fleet_status_from_str(&m.status),
        created_at: m.created_at,
        updated_at: m.updated_at,
        metadata,
    }
}

fn member_from_entity(m: fleet_members::Model) -> FleetMember {
    FleetMember {
        id: m.id,
        fleet_id: m.fleet_id,
        agent_id: m.agent_id,
        agent_slug: m.agent_slug,
        display_name: m.display_name,
        role: m.role,
        agent_profile_id: m.agent_profile_id,
        room_id: m.room_id,
        status: member_status_from_str(&m.status),
        joined_at: m.joined_at,
        // 防御性处理：DB 字段允许负数（极不可能），转 u64 时先 clamp 到 0
        today_tokens: std::cmp::max(m.today_tokens, 0) as u64,
        total_tokens: std::cmp::max(m.total_tokens, 0) as u64,
    }
}

fn message_from_entity(m: fleet_messages::Model) -> FleetMessage {
    FleetMessage {
        id: m.id,
        fleet_id: m.fleet_id,
        conversation_id: m.conversation_id,
        seq: m.seq,
        author_kind: m.author_kind,
        author_id: m.author_id,
        author_slug: m.author_slug,
        author_display_name: m.author_display_name,
        content: m.content,
        created_at: m.created_at,
    }
}

/// 判断数据库错误是否为**唯一约束冲突**（`seq` 分配竞态的唯一预期失败形态）。
///
/// 双库文本不同：PG 报 `duplicate key value violates unique constraint`（SQLSTATE 23505），
/// SQLite 报 `UNIQUE constraint failed`。
///
/// **只有这一种错误值得重试** —— 外键失败、连接断开等重试只会掩盖真实问题，
/// 且会把「写不进去」伪装成「一直在重试」。
fn is_unique_violation(err_text: &str) -> bool {
    err_text.contains("UNIQUE constraint failed")
        || err_text.contains("duplicate key value violates unique constraint")
        || err_text.contains("23505")
}

#[async_trait]
impl FleetRepository for SeaOrmFleetRepository {
    async fn create_fleet(&self, fleet: Fleet) -> Result<Fleet, String> {
        let metadata_json = serde_json::to_string(&fleet.metadata)
            .map_err(|e| format!("序列化 metadata 失败: {e}"))?;
        let model = fleets::ActiveModel {
            id: Set(fleet.id.clone()),
            name: Set(fleet.name.clone()),
            scene_template_slug: Set(fleet.scene_template_slug.clone()),
            status: Set(fleet_status_to_str(fleet.status).to_string()),
            created_at: Set(fleet.created_at),
            updated_at: Set(fleet.updated_at),
            metadata_json: Set(metadata_json),
        };
        let inserted = fleets::Entity::insert(model)
            .exec_with_returning(&self.db)
            .await
            .map_err(|e| format!("插入舰队失败: {e}"))?;
        Ok(fleet_from_entity(inserted))
    }

    async fn list_fleets(&self, status_filter: Option<FleetStatus>) -> Result<Vec<Fleet>, String> {
        let mut query = fleets::Entity::find().order_by_desc(fleets::Column::CreatedAt);
        if let Some(status) = status_filter {
            query = query.filter(fleets::Column::Status.eq(fleet_status_to_str(status)));
        }
        let rows = query.all(&self.db).await.map_err(|e| format!("查询舰队列表失败: {e}"))?;
        Ok(rows.into_iter().map(fleet_from_entity).collect())
    }

    async fn get_fleet(&self, fleet_id: &str) -> Result<Option<Fleet>, String> {
        let row = fleets::Entity::find_by_id(fleet_id.to_string())
            .one(&self.db)
            .await
            .map_err(|e| format!("查询舰队失败: {e}"))?;
        Ok(row.map(fleet_from_entity))
    }

    async fn update_fleet_status(&self, fleet_id: &str, status: FleetStatus) -> Result<(), String> {
        let now = chrono::Utc::now().timestamp_millis();
        fleets::Entity::update_many()
            .col_expr(fleets::Column::Status, Expr::value(fleet_status_to_str(status)))
            .col_expr(fleets::Column::UpdatedAt, Expr::value(now))
            .filter(fleets::Column::Id.eq(fleet_id))
            .exec(&self.db)
            .await
            .map_err(|e| format!("更新舰队状态失败: {e}"))?;
        Ok(())
    }

    async fn delete_fleet(&self, fleet_id: &str) -> Result<(), String> {
        // 先删除成员（避免外键约束错误，虽然 ON DELETE CASCADE 应该处理但 SQLite 行为不一致）
        fleet_members::Entity::delete_many()
            .filter(fleet_members::Column::FleetId.eq(fleet_id))
            .exec(&self.db)
            .await
            .map_err(|e| format!("删除舰队成员失败: {e}"))?;
        fleets::Entity::delete_by_id(fleet_id.to_string())
            .exec(&self.db)
            .await
            .map_err(|e| format!("删除舰队失败: {e}"))?;
        Ok(())
    }

    async fn list_members(&self, fleet_id: &str) -> Result<Vec<FleetMember>, String> {
        let rows = fleet_members::Entity::find()
            .filter(fleet_members::Column::FleetId.eq(fleet_id))
            .order_by_asc(fleet_members::Column::JoinedAt)
            .all(&self.db)
            .await
            .map_err(|e| format!("查询成员列表失败: {e}"))?;
        Ok(rows.into_iter().map(member_from_entity).collect())
    }

    async fn add_member(&self, member: FleetMember) -> Result<FleetMember, String> {
        let model = fleet_members::ActiveModel {
            id: Set(member.id.clone()),
            fleet_id: Set(member.fleet_id.clone()),
            agent_id: Set(member.agent_id.clone()),
            agent_slug: Set(member.agent_slug.clone()),
            display_name: Set(member.display_name.clone()),
            role: Set(member.role.clone()),
            agent_profile_id: Set(member.agent_profile_id.clone()),
            room_id: Set(member.room_id.clone()),
            status: Set(member_status_to_str(member.status).to_string()),
            joined_at: Set(member.joined_at),
            today_tokens: Set(member.today_tokens as i64),
            total_tokens: Set(member.total_tokens as i64),
        };
        let inserted = fleet_members::Entity::insert(model)
            .exec_with_returning(&self.db)
            .await
            .map_err(|e| format!("添加成员失败: {e}"))?;
        Ok(member_from_entity(inserted))
    }

    async fn get_member(&self, member_id: &str) -> Result<Option<FleetMember>, String> {
        let row = fleet_members::Entity::find_by_id(member_id.to_string())
            .one(&self.db)
            .await
            .map_err(|e| format!("查询成员失败: {e}"))?;
        Ok(row.map(member_from_entity))
    }

    async fn update_member_status(
        &self,
        member_id: &str,
        status: FleetMemberStatus,
    ) -> Result<(), String> {
        fleet_members::Entity::update_many()
            .col_expr(fleet_members::Column::Status, Expr::value(member_status_to_str(status)))
            .filter(fleet_members::Column::Id.eq(member_id))
            .exec(&self.db)
            .await
            .map_err(|e| format!("更新成员状态失败: {e}"))?;
        Ok(())
    }

    async fn add_member_tokens(&self, member_id: &str, tokens: u64) -> Result<(), String> {
        // 用 SQL 表达式 + 累加，避免读改写竞态
        fleet_members::Entity::update_many()
            .col_expr(
                fleet_members::Column::TodayTokens,
                Expr::col(fleet_members::Column::TodayTokens).add(tokens as i64),
            )
            .col_expr(
                fleet_members::Column::TotalTokens,
                Expr::col(fleet_members::Column::TotalTokens).add(tokens as i64),
            )
            .filter(fleet_members::Column::Id.eq(member_id))
            .exec(&self.db)
            .await
            .map_err(|e| format!("累加成员 token 失败: {e}"))?;
        Ok(())
    }

    async fn reset_daily_tokens(&self, fleet_id: &str) -> Result<(), String> {
        fleet_members::Entity::update_many()
            .col_expr(fleet_members::Column::TodayTokens, Expr::value(0i64))
            .filter(fleet_members::Column::FleetId.eq(fleet_id))
            .exec(&self.db)
            .await
            .map_err(|e| format!("重置今日 token 失败: {e}"))?;
        Ok(())
    }

    async fn remove_member(&self, member_id: &str) -> Result<(), String> {
        fleet_members::Entity::delete_by_id(member_id.to_string())
            .exec(&self.db)
            .await
            .map_err(|e| format!("移除成员失败: {e}"))?;
        Ok(())
    }

    // ── 消息持久化（协调门的地基） ──────────────────────────────────────

    async fn append_message(&self, message: FleetMessage) -> Result<FleetMessage, String> {
        // seq 分配的最大重试次数。3 次足够：真正的并发窗口只有「两个 dispatch
        // 在同一瞬间分配」，第 2 次必然拿到新值；给到 3 只为容忍极端调度。
        // 注意：函数体内**不能**用 `///`（那是 unused_doc_comments，-D warnings 下即红）。
        const MAX_SEQ_ATTEMPTS: u32 = 3;

        let mut last_err = String::new();

        for _ in 0..MAX_SEQ_ATTEMPTS {
            // 分配 seq：调用方传入的 seq 被忽略（见 trait 文档）。
            // ⚠ 作用域是**会话** —— 群聊与每条 DM 各有独立序列，
            // 否则 DM 的写入会把群聊水位顶高（或反之），协调门误判。
            let seq = self
                .max_seq(&message.fleet_id, &message.conversation_id, false)
                .await
                .map_err(|e| format!("分配消息 seq 失败: {e}"))?
                + 1;

            let model = fleet_messages::ActiveModel {
                id: Set(message.id.clone()),
                fleet_id: Set(message.fleet_id.clone()),
                conversation_id: Set(message.conversation_id.clone()),
                seq: Set(seq),
                author_kind: Set(message.author_kind.clone()),
                author_id: Set(message.author_id.clone()),
                author_slug: Set(message.author_slug.clone()),
                author_display_name: Set(message.author_display_name.clone()),
                content: Set(message.content.clone()),
                created_at: Set(message.created_at),
            };

            // 不要求返回行：seq 已由本方法掌握，回读反而多一次往返
            match fleet_messages::Entity::insert(model).exec(&self.db).await {
                // 此分支发散（return），故可在循环内 move `message` ——
                // 借用检查器能证明后续迭代不可达，无需 clone
                Ok(_) => return Ok(FleetMessage { seq, ..message }),
                Err(e) => {
                    let text = e.to_string();
                    if !is_unique_violation(&text) {
                        return Err(format!("追加消息失败: {text}"));
                    }
                    // 唯一约束冲突 = 并发事务抢了同一个 seq ⇒ 重新分配
                    last_err = text;
                },
            }
        }

        Err(format!(
            "追加消息失败：seq 分配连续 {MAX_SEQ_ATTEMPTS} 次冲突（并发写入过密）: {last_err}"
        ))
    }

    async fn list_messages(
        &self,
        fleet_id: &str,
        conversation_id: &str,
        after_seq: Option<i64>,
        limit: u32,
    ) -> Result<Vec<FleetMessage>, String> {
        // 会话是过滤的第一维：不隔离会让 DM 混进群聊面板、并混进路由 prompt
        let mut query = fleet_messages::Entity::find()
            .filter(fleet_messages::Column::FleetId.eq(fleet_id))
            .filter(fleet_messages::Column::ConversationId.eq(conversation_id));
        if let Some(after) = after_seq {
            query = query.filter(fleet_messages::Column::Seq.gt(after));
        }
        // 先按 seq 倒序取**最新** limit 条，再翻转回升序 ——
        // 若按升序 + limit 会拿到最早的历史，把最近上下文挤出窗口。
        let mut rows = query
            .order_by_desc(fleet_messages::Column::Seq)
            .limit(limit as u64)
            .all(&self.db)
            .await
            .map_err(|e| format!("查询舰队消息失败: {e}"))?;
        rows.reverse();
        Ok(rows.into_iter().map(message_from_entity).collect())
    }

    async fn max_seq(
        &self,
        fleet_id: &str,
        conversation_id: &str,
        only_agents: bool,
    ) -> Result<i64, String> {
        let mut query = fleet_messages::Entity::find()
            .filter(fleet_messages::Column::FleetId.eq(fleet_id))
            .filter(fleet_messages::Column::ConversationId.eq(conversation_id));
        if only_agents {
            query = query.filter(fleet_messages::Column::AuthorKind.eq(AUTHOR_KIND_AGENT));
        }
        // 用 order_by_desc + one() 取最大，避免 sea-orm 的 `ExprTrait::max` 与
        // `Ord::max` 在作用域内产生方法歧义（见 AGENTS.md 高频报错速查）。
        let row = query
            .order_by_desc(fleet_messages::Column::Seq)
            .one(&self.db)
            .await
            .map_err(|e| format!("查询房间水位失败: {e}"))?;
        Ok(row.map(|m| m.seq).unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_conversion() {
        assert_eq!(fleet_status_to_str(FleetStatus::Active), "active");
        assert_eq!(fleet_status_to_str(FleetStatus::Paused), "paused");
        assert_eq!(fleet_status_to_str(FleetStatus::Stopped), "stopped");
        assert_eq!(fleet_status_from_str("active"), FleetStatus::Active);
        assert_eq!(fleet_status_from_str("paused"), FleetStatus::Paused);
        assert_eq!(fleet_status_from_str("stopped"), FleetStatus::Stopped);
    }

    #[test]
    fn test_member_status_conversion() {
        assert_eq!(member_status_to_str(FleetMemberStatus::Idle), "idle");
        assert_eq!(member_status_to_str(FleetMemberStatus::Busy), "busy");
        assert_eq!(member_status_to_str(FleetMemberStatus::Paused), "paused");
        assert_eq!(member_status_to_str(FleetMemberStatus::Error), "error");
        assert_eq!(member_status_to_str(FleetMemberStatus::Offline), "offline");
    }
}
