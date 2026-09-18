// SPDX-License-Identifier: AGPL-3.0-only

//! Fleet 抽象契约 — 多办公室（AI 团队）协作的统一接口。
//!
//! ## 设计动机
//!
//! AxAgent 已有的多 Agent 能力分散在三个实现：
//! - **`runtime/swarm/Team`**：跨进程团队（纯内存，无持久化，无生命周期管理）
//! - **`trajectory/SubAgentRegistry`**：SubAgent 层级树（JSON 文件持久化）
//! - **`agent/AgentSession.team_id`**：单 Agent 会话的团队归属字段（无强约束）
//!
//! 三者各自为政，缺少统一的「舰队」一等公民抽象，导致：
//! 1. 无法跨 crate 查询「某舰队下所有成员状态」
//! 2. Team 仅内存态，重启丢失
//! 3. 缺少舰队级生命周期（暂停/恢复/停止）
//! 4. 缺少对话级智能路由（Dispatcher）
//!
//! 本模块在 harness 层定义：
//! 1. **共享 DTO** — `Fleet` / `FleetMember` / `FleetMessage` / `FleetStatus` / `FleetMemberStatus`
//! 2. **`FleetRepository` trait** — 舰队、成员、**消息**的持久化与查询接口
//! 3. **`DispatchEvent`** — 群聊智能路由的事件流契约
//! 4. **`FleetIntentLlm` trait** — 意图分类的 LLM 能力注入点
//!
//! ## 实现方
//!
//! - `axagent_trajectory::SeaOrmFleetRepository` → 实现 `FleetRepository` trait（SeaORM 持久化，SQLite + PostgreSQL 双兼容）
//! - `commands/fleet/executor.rs::ProviderFleetIntentLlm` → 实现 `FleetIntentLlm` trait（真实 LLM 意图分类）
//! - wiring 层在 `init/state.rs` 注入到 `AppState`
//!
//! ## 历史
//!
//! 本模块曾定义 `IntentDispatcher` trait（P0 计划中的「统一 Dispatcher 抽象」），
//! 由 `axagent_agent::LlmDispatcher` 实现。**该抽象从未接线**：全仓零构造点、
//! 零调用点，命令层直接走 `AppState.fleet_intent_llm` + `execute_fleet_turn`。
//! 2026-09-15 已删除该 trait 与实现（含仅服务它的 `DispatchChatMessage`），
//! 以免它带着**已废弃的内存历史形态**继续误导后续改动。

use serde::{Deserialize, Serialize};

// ============================================================================
// 共享 DTO
// ============================================================================

/// 舰队（办公室）状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FleetStatus {
    /// 活跃 — 成员可接收任务
    #[default]
    Active,
    /// 暂停 — 整个舰队停止接收新任务，运行中任务继续
    Paused,
    /// 停止 — 舰队已停止，所有成员离线
    Stopped,
}

/// 舰队成员状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FleetMemberStatus {
    /// 空闲 — 可接收任务
    #[default]
    Idle,
    /// 忙碌 — 正在执行任务
    Busy,
    /// 暂停 — 用户手动暂停，不接收新任务
    Paused,
    /// 错误 — 上次任务失败
    Error,
    /// 离线 — 成员已离开舰队
    Offline,
}

/// 舰队元数据 — 业务层可扩展信息
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetMetadata {
    /// 业务描述
    pub description: String,
    /// 最大成员数（0 表示无限制）
    pub max_members: u32,
    /// 协作策略名称（由下游业务系统填充，如 "ecommerce_ops" / "customer_service"）
    pub strategy: Option<String>,
    /// 自定义标签
    pub tags: Vec<String>,
}

/// 舰队（办公室）— 一个正在运行的 AI 团队
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fleet {
    /// 唯一 ID（UUID）
    pub id: String,
    /// 显示名称
    pub name: String,
    /// 场景模板 slug（可选，下游业务系统可填）
    pub scene_template_slug: Option<String>,
    /// 舰队状态
    pub status: FleetStatus,
    /// 创建时间（Unix 毫秒）
    pub created_at: i64,
    /// 更新时间（Unix 毫秒）
    pub updated_at: i64,
    /// 业务元数据
    pub metadata: FleetMetadata,
}

/// 舰队成员 — 办公室里的一个 agent
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetMember {
    /// 唯一 ID（UUID）
    pub id: String,
    /// 所属舰队 ID
    pub fleet_id: String,
    /// 关联的 AgentSession ID（由 SessionManager 创建）
    pub agent_id: String,
    /// agent slug（业务标识，用于 Dispatcher 路由）
    pub agent_slug: String,
    /// 显示名称
    pub display_name: String,
    /// 角色描述（注入到 Dispatcher prompt；与 agent_profile_id 二选一，均可）
    pub role: String,
    /// 关联的 AgentProfile ID（AgentProfile = 角色 + 专家组合，定义成员智能体身份）
    pub agent_profile_id: Option<String>,
    /// 房间 ID（前端 Phaser 渲染位置，如 "manager" / "meeting"）
    pub room_id: String,
    /// 成员状态
    pub status: FleetMemberStatus,
    /// 加入时间（Unix 毫秒）
    pub joined_at: i64,
    /// 今日 token 用量（实时累计，由 Dispatcher 事件更新）
    pub today_tokens: u64,
    /// 累计 token 用量
    pub total_tokens: u64,
}

// ============================================================================
// FleetRepository trait
// ============================================================================

/// 舰队持久化与查询的统一接口。
///
/// ## 异步设计
///
/// 所有方法都是 `async fn`，实现方使用 `tokio::sync::RwLock` 提供内部可变性。
/// SeaORM 实现直接走数据库连接池，无需外层锁。
///
/// ## 错误处理
///
/// 返回 `Result<T, String>`，实现方把内部错误转换为 `String` 返回，
/// 不传播 panic，符合 harness 错误隔离约定（与 `SharedBlackboard` 一致）。
#[async_trait::async_trait]
pub trait FleetRepository: Send + Sync {
    /// 创建舰队
    async fn create_fleet(&self, fleet: Fleet) -> Result<Fleet, String>;

    /// 列出所有舰队（可选状态过滤）
    async fn list_fleets(&self, status_filter: Option<FleetStatus>) -> Result<Vec<Fleet>, String>;

    /// 获取舰队详情
    async fn get_fleet(&self, fleet_id: &str) -> Result<Option<Fleet>, String>;

    /// 更新舰队状态
    async fn update_fleet_status(&self, fleet_id: &str, status: FleetStatus) -> Result<(), String>;

    /// 删除舰队（同时删除所有成员）
    async fn delete_fleet(&self, fleet_id: &str) -> Result<(), String>;

    /// 列出舰队下所有成员
    async fn list_members(&self, fleet_id: &str) -> Result<Vec<FleetMember>, String>;

    /// 添加成员到舰队
    async fn add_member(&self, member: FleetMember) -> Result<FleetMember, String>;

    /// 获取单个成员
    async fn get_member(&self, member_id: &str) -> Result<Option<FleetMember>, String>;

    /// 更新成员状态
    async fn update_member_status(
        &self,
        member_id: &str,
        status: FleetMemberStatus,
    ) -> Result<(), String>;

    /// 累加成员 token 用量（today_tokens + total_tokens 同时累加）
    async fn add_member_tokens(&self, member_id: &str, tokens: u64) -> Result<(), String>;

    /// 重置成员今日 token（每日定时任务调用）
    async fn reset_daily_tokens(&self, fleet_id: &str) -> Result<(), String>;

    /// 移除成员
    async fn remove_member(&self, member_id: &str) -> Result<(), String>;

    // ── 消息持久化（协调门的地基） ──────────────────────────────────────

    /// 追加一条消息，返回**带已分配 `seq`** 的记录。
    ///
    /// ## seq 分配与并发
    ///
    /// 实现方以 `MAX(seq)+1` 分配（⚠ 读与写是两条独立语句、**不在同一事务**，
    /// PG 默认 READ COMMITTED 下并发请求可能读到同一个 MAX），并依赖
    /// `UNIQUE(fleet_id, conversation_id, seq)` 兜底并发：插入冲突时**重试**
    /// （建议 ≤3 次），不要把冲突当业务错误抛出。
    /// 入参 `message.seq` 被忽略（由实现方覆写）。
    ///
    /// `seq` 的**作用域是会话**，不是舰队：群聊与每条 DM 各有独立递增序列。
    async fn append_message(&self, message: FleetMessage) -> Result<FleetMessage, String>;

    /// 按 `seq` 升序列出**指定会话**的消息。
    ///
    /// - `conversation_id` 必须显式传入：不隔离会话会让 DM 混进群聊上下文
    ///   （包括混进路由 prompt），这是改名前的实际缺陷；
    /// - `after_seq = Some(n)` 只返回 `seq > n`（增量展示 / 协调门比对）；
    /// - `limit` 为条数上限：**先取最新的 N 条，再按 seq 升序返回**
    ///   （保证拿到的是最近上下文，而不是最早的历史）。
    async fn list_messages(
        &self,
        fleet_id: &str,
        conversation_id: &str,
        after_seq: Option<i64>,
        limit: u32,
    ) -> Result<Vec<FleetMessage>, String>;

    /// **指定会话**当前水位（最大 `seq`；无消息时为 0）。
    ///
    /// `only_agents = true` 只统计 agent 作者的消息 —— 协调门判断
    /// 「有没有比我读时更新的**同伴**发言」用的就是这一档。
    ///
    /// ⚠ 水位必须按会话统计：若跨会话取全局最大，别的 DM 一发言就会把群聊
    /// 误判为「已被推进」而暂扣本轮。
    async fn max_seq(
        &self,
        fleet_id: &str,
        conversation_id: &str,
        only_agents: bool,
    ) -> Result<i64, String>;
}

// ============================================================================
// DispatchEvent + FleetIntentLlm
// ============================================================================

/// 调度事件 — Dispatcher 在路由与执行过程中产生的事件流
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DispatchEvent {
    /// 路由决策 — 调度员决定路由到某个 agent
    #[serde(rename_all = "camelCase")]
    Routing {
        /// 目标 agent slug
        agent_slug: String,
        /// 目标 agent ID
        agent_id: String,
        /// 目标房间 ID（前端据此移动精灵）
        room_id: String,
        /// 任务摘要（注入到 agent 的 prompt）
        task_summary: String,
    },
    /// Agent 处理中的中间状态
    #[serde(rename_all = "camelCase")]
    Process {
        /// agent slug
        agent_slug: String,
        /// agent ID
        agent_id: String,
        /// 状态描述
        status: String,
    },
    /// Agent 回复消息
    #[serde(rename_all = "camelCase")]
    AgentMessage {
        /// agent slug
        agent_slug: String,
        /// agent ID
        agent_id: String,
        /// 回复内容
        content: String,
    },
    /// Agent 状态变更
    #[serde(rename_all = "camelCase")]
    AgentStatus {
        /// agent slug
        agent_slug: String,
        /// agent ID
        agent_id: String,
        /// 新状态
        status: FleetMemberStatus,
    },
    /// Token 用量上报
    #[serde(rename_all = "camelCase")]
    TokenUsage {
        /// agent slug
        agent_slug: String,
        /// agent ID
        agent_id: String,
        /// 输入 token 数
        input_tokens: u64,
        /// 输出 token 数
        output_tokens: u64,
    },
    /// 消息被协调门暂扣 —— 本次回合**未执行**
    ///
    /// ## 契约（「展示即已见」，shown ⇒ seen）
    ///
    /// 服务端只在「本次读取之后、执行之前，**会话**被其他 agent 推进过」时返回本事件，
    /// 并把**未读的更新消息内联在 `held_messages` 里** —— 这一步就是「展示」。
    /// 调用方读过之后**直接重发即可通过**，不需要任何旗标仪式。
    ///
    /// ## 为什么本仓没有 `force` / `send_anyway` 参数
    ///
    /// 这是刻意设计。cumora 的实证教训：`--send-anyway` 本是免费无条件旁路，
    /// 智能体为省一次往返开始**抢占式**传它，协调门就此静默消失
    /// （其 anti-pattern #10「不要发布没有代价的覆盖标志 —— 软门会侵蚀」）。
    ///
    /// 本仓的等价保障来自结构而非纪律：**会话水位基线只在服务端把消息展示出去时
    /// 才被取用**，客户端无法自行推进它；且重发时基线会重新取值 —— 若会话在此期间
    /// 又被推进，会**再次 HELD**（同一回合的旧确认也跳不过从未被展示的消息）。
    #[serde(rename_all = "camelCase")]
    Held {
        /// 发现的最新水位 —— **该会话**的 agent 水位
        /// （`max_seq(fleet_id, conversation_id, only_agents = true)`）。
        ///
        /// ⚠ **不等于** `held_messages` 中最大的 seq：后者受服务端条数上限
        /// （`CONVERSATION_HISTORY_LIMIT`）约束，未读超过上限时只保留最新 N 条被截断。
        ///
        /// 前端消费端：HELD 提示条会显示本值（「已推进到 #N」），让用户判断自己
        /// 落后多远。之所以不需要客户端**回报**它：本仓的基线每次都在
        /// **服务端入口重新取值**，而非依赖客户端确认。
        max_seq: i64,
        /// 未读的更新消息（服务器已展示 ⇒ 重发即可通过）。
        ///
        /// 与 `max_seq` 同源，故**只含本会话的消息**。理论上可能混有
        /// `author_kind = human` 的条目（例如另一个窗口对本会话并发 dispatch 时
        /// 写入的用户消息）—— 这是刻意的：暂扣期间让用户看到会话全貌，
        /// 好过只给他看 agent 那一半。
        held_messages: Vec<FleetMessage>,
    },
    /// 流结束
    Complete,
    /// 错误
    Error {
        /// 错误消息
        message: String,
    },
}

/// 舰队消息 — **已持久化**的群聊 / 私信消息。
///
/// ## 为什么不是「调用方临时传一个 history」
///
/// 协调门的判据形如「有没有比水位 X 更新的**非自己**消息」，这要求每条消息都能
/// 回答「**谁写的**」和「**写的先后**」。以前那种「调用方每次传一组
/// role + content」的临时历史两样都给不出，且前端一刷新记忆就没了。
///
/// 因此本类型是库里的真源：`author_kind` / `author_id` 定身份，
/// `seq` 定顺序，`conversation_id` 定**会话作用域**（群聊 or 某个 DM）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetMessage {
    /// 唯一 ID
    pub id: String,
    /// 所属舰队 ID
    pub fleet_id: String,
    /// **会话作用域** ID（群聊 = [`CONVERSATION_GROUP`]，私信 = [`conversation_dm`]）
    ///
    /// ⚠ 不是 `FleetMember.room_id`：那是像素办公室里精灵站的物理房间，
    /// 由前端渲染消费，**从不参与消息查询**。两者曾同名，导致 DM 与群聊
    /// 共享一条时间线而无人察觉。
    pub conversation_id: String,
    /// 单调递增序号（同一舰队内唯一且递增）
    pub seq: i64,
    /// 作者类型：human / agent
    pub author_kind: String,
    /// 作者 ID（human 用本地固定标识；agent 用 agent_id）
    pub author_id: String,
    /// 作者 slug（agent 才有）
    pub author_slug: Option<String>,
    /// 作者显示名（human 为 None）
    pub author_display_name: Option<String>,
    /// 消息正文
    pub content: String,
    /// 创建时间（Unix 毫秒）
    pub created_at: i64,
}

/// `FleetMessage.author_kind` 的取值 — 人类作者
pub const AUTHOR_KIND_HUMAN: &str = "human";
/// `FleetMessage.author_kind` 的取值 — agent 作者
pub const AUTHOR_KIND_AGENT: &str = "agent";

/// 群聊（智能路由）会话的 `conversation_id`。
///
/// 群聊是舰队内**唯一**的共享会话：所有成员的公开发言与用户的群聊输入都落在这里。
///
/// ⚠ 不要与 `FleetMember.room_id` 混用 —— 那是像素办公室里精灵站的物理房间
/// （`"workspace"` / `"showroom"` …），**从不参与消息查询**。
pub const CONVERSATION_GROUP: &str = "group";

/// 与指定成员私信（DM）的 `conversation_id`。
///
/// 用 slug 而非 member_id 作键：slug 是路由与前端事件的稳定业务标识，
/// 且同一 agent 在舰队内 slug 唯一（`fleet_add_member` 强制校验）。
/// 这样「一次 DM」就是一个确定的会话，无需额外建表记录会话关系。
pub fn conversation_dm(agent_slug: &str) -> String {
    format!("dm:{agent_slug}")
}

/// Fleet 意图分类 LLM 调用 trait —— 路由能力的**唯一**注入点。
///
/// ## 设计动机
///
/// `axagent_agent` 是 consumer crate，按铁律只能依赖 `axagent-harness`，
/// 不能直接依赖 `axagent-providers`。因此 LLM 调用能力通过本 trait 注入：
/// wiring 层实现此 trait，在 `init/state.rs` 构造后放入 `AppState.fleet_intent_llm`，
/// 由 `commands::fleet::fleet_dispatch` 直接调用（不经中间 Dispatcher 层）。
///
/// ## 输出约定
///
/// `route()` 返回 LLM 原始文本响应，期望是 JSON：
/// ```json
/// {"agent_slug": "copywriter", "reason": "用户要求写产品文案"}
/// ```
/// 解析失败时由调用方（`commands::fleet`）兜底为第一个可用成员。
#[async_trait::async_trait]
pub trait FleetIntentLlm: Send + Sync {
    /// 调用 LLM 做意图分类
    ///
    /// - `system_prompt`: 系统提示词（含成员列表与路由规则）
    /// - `user_prompt`: 用户消息（可选含历史摘要）
    /// - 返回：LLM 原始响应文本
    async fn route(&self, system_prompt: &str, user_prompt: &str) -> Result<String, String>;
}

/// `FleetIntentLlm` 的空实现 — 用于测试 / 离线模式（始终返回空字符串）
pub struct NoopFleetIntentLlm;

#[async_trait::async_trait]
impl FleetIntentLlm for NoopFleetIntentLlm {
    async fn route(&self, _system_prompt: &str, _user_prompt: &str) -> Result<String, String> {
        Ok(String::new())
    }
}

// ============================================================================
// Noop 实现（用于测试 / 离线模式）
// ============================================================================

/// 空实现 — 返回空结果，不执行任何操作
pub struct NoopFleetRepository;

#[async_trait::async_trait]
impl FleetRepository for NoopFleetRepository {
    async fn create_fleet(&self, fleet: Fleet) -> Result<Fleet, String> {
        Ok(fleet)
    }
    async fn list_fleets(&self, _status_filter: Option<FleetStatus>) -> Result<Vec<Fleet>, String> {
        Ok(Vec::new())
    }
    async fn get_fleet(&self, _fleet_id: &str) -> Result<Option<Fleet>, String> {
        Ok(None)
    }
    async fn update_fleet_status(
        &self,
        _fleet_id: &str,
        _status: FleetStatus,
    ) -> Result<(), String> {
        Ok(())
    }
    async fn delete_fleet(&self, _fleet_id: &str) -> Result<(), String> {
        Ok(())
    }
    async fn list_members(&self, _fleet_id: &str) -> Result<Vec<FleetMember>, String> {
        Ok(Vec::new())
    }
    async fn add_member(&self, member: FleetMember) -> Result<FleetMember, String> {
        Ok(member)
    }
    async fn get_member(&self, _member_id: &str) -> Result<Option<FleetMember>, String> {
        Ok(None)
    }
    async fn update_member_status(
        &self,
        _member_id: &str,
        _status: FleetMemberStatus,
    ) -> Result<(), String> {
        Ok(())
    }
    async fn add_member_tokens(&self, _member_id: &str, _tokens: u64) -> Result<(), String> {
        Ok(())
    }
    async fn reset_daily_tokens(&self, _fleet_id: &str) -> Result<(), String> {
        Ok(())
    }
    async fn remove_member(&self, _member_id: &str) -> Result<(), String> {
        Ok(())
    }
    async fn append_message(&self, mut message: FleetMessage) -> Result<FleetMessage, String> {
        // Noop 不落库（无 seq 可分配），但形态与真实实现保持一致：
        // seq=0 ⇒ `max_seq` 恒 0 ⇒ 协调门在离线/测试模式下**恒定放行**，
        // 不会把「没有持久化」伪装成「检测到竞态」。
        message.seq = 0;
        Ok(message)
    }
    async fn list_messages(
        &self,
        _fleet_id: &str,
        _conversation_id: &str,
        _after_seq: Option<i64>,
        _limit: u32,
    ) -> Result<Vec<FleetMessage>, String> {
        Ok(Vec::new())
    }
    async fn max_seq(
        &self,
        _fleet_id: &str,
        _conversation_id: &str,
        _only_agents: bool,
    ) -> Result<i64, String> {
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fleet_status_serde() {
        let status = FleetStatus::Active;
        let json = serde_json::to_string(&status).expect("测试：JSON序列化应成功");
        assert_eq!(json, "\"active\"");
        let de: FleetStatus = serde_json::from_str(&json).expect("测试：JSON反序列化应成功");
        assert_eq!(de, FleetStatus::Active);
    }

    #[test]
    fn test_member_status_serde() {
        let status = FleetMemberStatus::Busy;
        let json = serde_json::to_string(&status).expect("测试：JSON序列化应成功");
        assert_eq!(json, "\"busy\"");
    }

    #[test]
    fn test_dispatch_event_tagged_enum() {
        let event = DispatchEvent::Routing {
            agent_slug: "copywriter".to_string(),
            agent_id: "agt_001".to_string(),
            room_id: "showroom".to_string(),
            task_summary: "写产品文案".to_string(),
        };
        let json = serde_json::to_string(&event).expect("测试：JSON序列化应成功");
        assert!(json.contains("\"type\":\"routing\""));
        assert!(json.contains("\"agentSlug\":\"copywriter\""));
    }

    #[test]
    fn test_noop_repository() {
        let noop = NoopFleetRepository;
        let result = futures::executor::block_on(noop.list_fleets(None)).expect("测试应成功");
        assert!(result.is_empty());
    }

    #[test]
    fn test_fleet_metadata_default() {
        let meta = FleetMetadata::default();
        assert_eq!(meta.max_members, 0);
        assert!(meta.strategy.is_none());
        assert!(meta.tags.is_empty());
    }
}
