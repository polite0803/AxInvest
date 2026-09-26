// SPDX-License-Identifier: AGPL-3.0-only

//! Fleet（多办公室 AI 团队）命令模块。
//!
//! ## 命令清单
//!
//! ### 舰队 CRUD
//! - `fleet_list` — 列出所有舰队（可选状态过滤）
//! - `fleet_get` — 获取舰队详情
//! - `fleet_create` — 创建舰队
//! - `fleet_update_status` — 更新舰队状态（active/paused/stopped）
//! - `fleet_delete` — 删除舰队（级联删除成员）
//!
//! ### 成员管理
//! - `fleet_list_members` — 列出舰队所有成员
//! - `fleet_add_member` — 添加成员
//! - `fleet_get_member` — 获取单个成员
//! - `fleet_update_member_status` — 更新成员状态
//! - `fleet_remove_member` — 移除成员
//! - `fleet_reset_daily_tokens` — 重置舰队所有成员今日 token
//!
//! ### Dispatcher 智能路由（真实执行）
//! - `fleet_dispatch` — 群聊智能路由：真实 LLM 意图分类 → 路由到成员 → 真实 Agent 回合执行
//!   （通过 `Channel<DispatchEvent>` 流式回传事件）
//! - `fleet_direct_message` — 直接 DM 指定 agent（绕过 LLM 路由，仍真实执行）
//! - `fleet_list_messages` — 读取**指定会话**的持久化消息（群聊 / 某条 DM）
//!
//! ## 会话（conversation）
//!
//! 消息按**会话**隔离，不按成员房间：群聊 = [`CONVERSATION_GROUP`]（`"group"`），
//! 私信 = `conversation_dm(slug)`（`"dm:<slug>"`）。群聊的路由 prompt 与新鲜度门
//! 只在群聊会话内取值 —— 否则别人私聊一句就会把群聊判为「已被推进」而误暂扣。
//!
//! ## 错误处理
//!
//! 所有命令返回 `Result<T, ErrorResponse>`，错误码见 `error_code::fleet`，
//! 前端按 `error.${code}` 走 i18n 翻译（`@/lib/errorI18n.ts`）。

use crate::AppState;
use crate::commands::error::{ErrorCategory, ErrorResponse};
use crate::commands::error_code::fleet as fleet_err;
use axagent_agent_macro::agent_command;
use axagent_harness::fleet::{
    AUTHOR_KIND_AGENT, AUTHOR_KIND_HUMAN, CONVERSATION_GROUP, DispatchEvent, Fleet, FleetMember,
    FleetMemberStatus, FleetMessage, FleetMetadata, FleetStatus, conversation_dm,
};
use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::{info, warn};

pub mod executor;
use executor::execute_fleet_turn;

// ── 舰队 CRUD ────────────────────────────────────────────────────────

/// 列出所有舰队（可选状态过滤）
#[agent_command(domain = fleet, safety = Safe, call_mode = StateInput, description = "列出所有舰队")]
#[tauri::command]
pub async fn fleet_list(
    app_state: State<'_, AppState>,
    status_filter: Option<FleetStatus>,
) -> Result<Vec<Fleet>, ErrorResponse> {
    app_state
        .fleet_repository
        .list_fleets(status_filter)
        .await
        .map_err(|e| ErrorResponse::from_error(e, ErrorCategory::General))
}

/// 获取舰队详情
#[agent_command(domain = fleet, safety = Safe, call_mode = StateInput, description = "获取舰队详情")]
#[tauri::command]
pub async fn fleet_get(
    app_state: State<'_, AppState>,
    fleet_id: String,
) -> Result<Option<Fleet>, ErrorResponse> {
    app_state
        .fleet_repository
        .get_fleet(&fleet_id)
        .await
        .map_err(|e| ErrorResponse::from_error(e, ErrorCategory::General))
}

/// 创建舰队的输入参数
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFleetInput {
    /// 显示名称
    pub name: String,
    /// 场景模板 slug（可选）
    pub scene_template_slug: Option<String>,
    /// 业务元数据
    #[serde(default)]
    pub metadata: FleetMetadata,
}

/// 创建舰队
#[agent_command(domain = fleet, safety = Caution, call_mode = StateInput, description = "创建舰队")]
#[tauri::command]
pub async fn fleet_create(
    app_state: State<'_, AppState>,
    input: CreateFleetInput,
) -> Result<Fleet, ErrorResponse> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(
            ErrorResponse::new(fleet_err::NAME_REQUIRED).with_category(ErrorCategory::Validation)
        );
    }
    let now = chrono::Utc::now().timestamp_millis();
    let fleet = Fleet {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        scene_template_slug: input.scene_template_slug,
        status: FleetStatus::Active,
        created_at: now,
        updated_at: now,
        metadata: input.metadata,
    };
    app_state
        .fleet_repository
        .create_fleet(fleet)
        .await
        .map_err(|e| ErrorResponse::from_error(e, ErrorCategory::General))
}

/// 更新舰队状态
#[agent_command(domain = fleet, safety = Caution, call_mode = StateInput, description = "更新舰队状态")]
#[tauri::command]
pub async fn fleet_update_status(
    app_state: State<'_, AppState>,
    fleet_id: String,
    status: FleetStatus,
) -> Result<(), ErrorResponse> {
    app_state
        .fleet_repository
        .update_fleet_status(&fleet_id, status)
        .await
        .map_err(|e| ErrorResponse::from_error(e, ErrorCategory::General))
}

/// 删除舰队（级联删除成员）
#[agent_command(domain = fleet, safety = Dangerous, call_mode = StateInput, description = "删除舰队")]
#[tauri::command]
pub async fn fleet_delete(
    app_state: State<'_, AppState>,
    fleet_id: String,
) -> Result<(), ErrorResponse> {
    app_state
        .fleet_repository
        .delete_fleet(&fleet_id)
        .await
        .map_err(|e| ErrorResponse::from_error(e, ErrorCategory::General))
}

// ── 成员管理 ─────────────────────────────────────────────────────────

/// 列出舰队所有成员
#[agent_command(domain = fleet, safety = Safe, call_mode = StateInput, description = "列出舰队所有成员")]
#[tauri::command]
pub async fn fleet_list_members(
    app_state: State<'_, AppState>,
    fleet_id: String,
) -> Result<Vec<FleetMember>, ErrorResponse> {
    app_state
        .fleet_repository
        .list_members(&fleet_id)
        .await
        .map_err(|e| ErrorResponse::from_error(e, ErrorCategory::General))
}

/// 添加成员的输入参数
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddMemberInput {
    /// 所属舰队 ID
    pub fleet_id: String,
    /// 关联的 AgentSession ID（会话键 = conversation_id，由 get_or_create_session 懒创建）
    pub agent_id: String,
    /// agent slug（业务标识，用于 Dispatcher 路由）
    pub agent_slug: String,
    /// 显示名称
    pub display_name: String,
    /// 角色描述（注入到 Dispatcher prompt；与 agent_profile_id 二选一，均可）
    #[serde(default)]
    pub role: String,
    /// 关联的 AgentProfile ID（AgentProfile = 角色 + 专家组合，定义成员智能体身份）
    #[serde(default)]
    pub agent_profile_id: Option<String>,
    /// **物理房间** ID —— 像素办公室里精灵站位（如 "manager" / "meeting"），
    /// 由前端 Phaser 渲染消费，**不参与消息查询**。
    /// 消息的会话归属是 `FleetMessage.conversation_id`，与它无关。
    #[serde(default = "default_member_room")]
    pub room_id: String,
}

/// 成员物理房间的缺省值（前端 Phaser 的默认站位）。
///
/// ⚠ 与 [`CONVERSATION_GROUP`] 是两个正交概念：曾共用同一个函数当默认值，
/// 于是「群聊会话 id」被写死成 `"workspace"`（一个房间名），语义就此含混。
fn default_member_room() -> String {
    "workspace".to_string()
}

/// 添加成员到舰队
#[agent_command(domain = fleet, safety = Caution, call_mode = StateInput, description = "添加成员到舰队")]
#[tauri::command]
pub async fn fleet_add_member(
    app_state: State<'_, AppState>,
    input: AddMemberInput,
) -> Result<FleetMember, ErrorResponse> {
    if input.agent_slug.trim().is_empty() {
        return Err(ErrorResponse::new(fleet_err::NAME_REQUIRED)
            .with_category(ErrorCategory::Validation)
            .with_detail("agent_slug 不能为空".to_string()));
    }
    // 同舰队内 slug 唯一性校验：slug 是 Dispatcher 路由与前端事件回写的键，
    // 重复会导致 DM 错配 / 事件状态回写到错误成员（精灵动画失真）。
    let slug = input.agent_slug.trim().to_string();
    let existing = app_state
        .fleet_repository
        .list_members(&input.fleet_id)
        .await
        .map_err(|e| ErrorResponse::from_error(e, ErrorCategory::General))?;
    if existing.iter().any(|m| m.agent_slug == slug) {
        return Err(ErrorResponse::new(fleet_err::SLUG_EXISTS)
            .with_category(ErrorCategory::Validation)
            .with_param("slug", slug.clone()));
    }
    let member = FleetMember {
        id: uuid::Uuid::new_v4().to_string(),
        fleet_id: input.fleet_id,
        agent_id: input.agent_id,
        agent_slug: slug,
        display_name: input.display_name,
        role: input.role,
        agent_profile_id: input.agent_profile_id,
        room_id: input.room_id,
        status: FleetMemberStatus::Idle,
        joined_at: chrono::Utc::now().timestamp_millis(),
        today_tokens: 0,
        total_tokens: 0,
    };
    app_state
        .fleet_repository
        .add_member(member)
        .await
        .map_err(|e| ErrorResponse::from_error(e, ErrorCategory::General))
}

/// 获取单个成员
#[agent_command(domain = fleet, safety = Safe, call_mode = StateInput, description = "获取单个成员详情")]
#[tauri::command]
pub async fn fleet_get_member(
    app_state: State<'_, AppState>,
    member_id: String,
) -> Result<Option<FleetMember>, ErrorResponse> {
    app_state
        .fleet_repository
        .get_member(&member_id)
        .await
        .map_err(|e| ErrorResponse::from_error(e, ErrorCategory::General))
}

/// 更新成员状态
#[agent_command(domain = fleet, safety = Caution, call_mode = StateInput, description = "更新成员状态")]
#[tauri::command]
pub async fn fleet_update_member_status(
    app_state: State<'_, AppState>,
    member_id: String,
    status: FleetMemberStatus,
) -> Result<(), ErrorResponse> {
    app_state
        .fleet_repository
        .update_member_status(&member_id, status)
        .await
        .map_err(|e| ErrorResponse::from_error(e, ErrorCategory::General))
}

/// 移除成员
#[agent_command(domain = fleet, safety = Dangerous, call_mode = StateInput, description = "移除成员")]
#[tauri::command]
pub async fn fleet_remove_member(
    app_state: State<'_, AppState>,
    member_id: String,
) -> Result<(), ErrorResponse> {
    app_state
        .fleet_repository
        .remove_member(&member_id)
        .await
        .map_err(|e| ErrorResponse::from_error(e, ErrorCategory::General))
}

/// 重置舰队所有成员今日 token（每日定时任务调用）
#[agent_command(domain = fleet, safety = Caution, call_mode = StateInput, description = "重置舰队所有成员今日token")]
#[tauri::command]
pub async fn fleet_reset_daily_tokens(
    app_state: State<'_, AppState>,
    fleet_id: String,
) -> Result<(), ErrorResponse> {
    app_state
        .fleet_repository
        .reset_daily_tokens(&fleet_id)
        .await
        .map_err(|e| ErrorResponse::from_error(e, ErrorCategory::General))
}

// ── Dispatcher 智能路由（真实执行）───────────────────────────────────

/// 群聊智能路由的输入
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchInput {
    /// 舰队 ID
    pub fleet_id: String,
    /// 用户消息
    pub user_message: String,
}

/// 群聊智能路由 — 真实 LLM 意图分类 → 路由到成员 → 真实 Agent 回合执行。
///
/// 事件通过 `Channel<DispatchEvent>` 流式回传：
/// `Routing → AgentStatus(busy) → [Process/AgentMessage/TokenUsage]* → AgentStatus(idle) → Complete`。
#[agent_command(domain = fleet, safety = Caution, call_mode = StateInput, description = "群聊智能路由分派执行")]
#[tauri::command]
pub async fn fleet_dispatch(
    app_state: State<'_, AppState>,
    input: DispatchInput,
    on_event: tauri::ipc::Channel<DispatchEvent>,
) -> Result<(), ErrorResponse> {
    // 1. 加载成员并过滤可路由成员（Idle / Busy）
    let members = app_state.fleet_repository.list_members(&input.fleet_id).await.map_err(|e| {
        ErrorResponse::from_error_with_code(fleet_err::NOT_FOUND, e, ErrorCategory::General)
    })?;
    if members.is_empty() {
        return Err(
            ErrorResponse::new(fleet_err::NO_MEMBERS).with_category(ErrorCategory::Validation)
        );
    }
    let routable: Vec<FleetMember> = members
        .into_iter()
        .filter(|m| matches!(m.status, FleetMemberStatus::Idle | FleetMemberStatus::Busy))
        .collect();
    if routable.is_empty() {
        return Err(ErrorResponse::new(fleet_err::ALL_MEMBERS_UNAVAILABLE));
    }

    // 2. 记录**群聊会话**的水位基线（协调门的比对基准）
    //
    //    取「进入本命令那一刻」群聊会话的 **agent** 水位。之后的路由 LLM 往返与
    //    回合执行都是真实的时间窗口，期间若有另一个 dispatch 写入了回复，
    //    本次决策即已过期。只统计 agent 消息 —— 本命令随后写入的用户消息（human）
    //    不该被算作「过期」。
    //
    //    ⚠ 必须限定在群聊会话内：若取舰队全局水位，任何一条 DM 的 agent 回复
    //    都会把水位顶高，于是每次群聊都被判为「已被推进」而误暂扣。
    let baseline_seq = app_state
        .fleet_repository
        .max_seq(&input.fleet_id, CONVERSATION_GROUP, true)
        .await
        .map_err(|e| {
            ErrorResponse::from_error_with_code(fleet_err::NOT_FOUND, e, ErrorCategory::General)
        })?;

    // 3. 读取群聊会话的真实历史（用于路由 prompt）
    //
    //    历史不再由调用方传入（`DispatchInput` 已无 `history` 字段）：
    //    库是唯一真源，否则前端一刷新「对话记忆」就没了。
    //    也只读群聊 —— DM 是用户与某个成员的私密往返，不该进群聊的路由上下文。
    //
    //    ⚠ 此处**尚未**写入本轮的用户消息 —— 落库推迟到协调门放行之后。
    //    否则被 HELD 时用户会重发，历史里就留下两条一模一样的用户消息。
    //    HELD 的语义是「本轮不做」，那本轮的输入自然也不该进历史。
    let history = load_conversation_history(&app_state, &input.fleet_id, CONVERSATION_GROUP).await;

    // 4. 真实 LLM 意图路由（wiring 层注入的 FleetIntentLlm）；失败/未命中时兜底到第一个可路由成员
    let system_prompt = build_fleet_system_prompt(&routable);
    let user_prompt = build_fleet_user_prompt(&input.user_message, &history);
    let mut fell_back = false;
    let target_slug = match app_state.fleet_intent_llm.route(&system_prompt, &user_prompt).await {
        Ok(resp) => {
            match parse_route_response(&resp).and_then(|slug| resolve_target_slug(&routable, &slug))
            {
                Some(slug) => slug,
                None => {
                    fell_back = true;
                    warn!("[fleet] LLM 路由未命中任何成员，兜底到首个成员");
                    routable[0].agent_slug.clone()
                },
            }
        },
        Err(e) => {
            fell_back = true;
            warn!("[fleet] LLM 路由失败，兜底到首个成员: {e}");
            routable[0].agent_slug.clone()
        },
    };

    let target = routable
        .into_iter()
        .find(|m| m.agent_slug == target_slug)
        .expect("target_slug 来自 routable，必然存在");

    // 兜底时明确告知实际路由到的成员（避免静默错配，前端据此提示用户）
    if fell_back {
        let notice = DispatchEvent::Process {
            agent_slug: target.agent_slug.clone(),
            agent_id: target.agent_id.clone(),
            status: format!("意图路由未命中，本次任务转派给成员「{}」", target.display_name),
        };
        send_event(&on_event, notice);
    }

    // 5. 路由决策事件
    send_event(
        &on_event,
        DispatchEvent::Routing {
            agent_slug: target.agent_slug.clone(),
            agent_id: target.agent_id.clone(),
            room_id: target.room_id.clone(),
            task_summary: input.user_message.clone(),
        },
    );

    // 6. ★ 协调门（新鲜度 preflight）—— 执行**前**复核群聊会话是否已被推进
    //
    //    放在这里而不是入口是刻意的：入口处会话必然与基线一致（基线正取自入口），
    //    只有经过路由的 LLM 往返之后才可能出现「我已经晚了」。
    if let Some((max_seq, held_messages)) =
        preflight_held(&app_state, &input.fleet_id, CONVERSATION_GROUP, baseline_seq).await
    {
        // HELD 是**暂扣**而非错误：内联未读消息即为「展示」，
        // 调用方读过之后直接重发即可通过（见 `DispatchEvent::Held` 的契约）。
        send_event(&on_event, DispatchEvent::Held { max_seq, held_messages });
        send_event(&on_event, DispatchEvent::Complete);
        return Ok(());
    }

    // 7. 用户消息落库 —— 此刻已确定本轮会真正执行
    //
    //    放在闸门之后是关键：HELD 的语义是「本轮不做」，那本轮的输入也不该进历史，
    //    否则用户重发时会看到两条一模一样的用户消息。
    append_human_message(&app_state, &input.fleet_id, CONVERSATION_GROUP, &input.user_message)
        .await?;

    // 8. 真实执行成员回合（事件流式转发；错误事件已由 executor 推送，仍需 Complete 收尾）
    let emit = |evt: DispatchEvent| {
        let _ = on_event.send(evt);
    };
    let result = execute_fleet_turn(
        &app_state,
        &input.fleet_id,
        CONVERSATION_GROUP,
        &target,
        &input.user_message,
        &emit,
    )
    .await;

    // 9. 流结束
    send_event(&on_event, DispatchEvent::Complete);

    result.map(|_| ())
}

/// 直接 DM 指定 agent（绕过 LLM 路由，仍真实执行）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectMessageInput {
    /// 舰队 ID
    pub fleet_id: String,
    /// 目标 agent slug
    pub agent_slug: String,
    /// 用户消息
    pub user_message: String,
}

#[agent_command(domain = fleet, safety = Caution, call_mode = StateInput, description = "直接发送消息给指定agent")]
#[tauri::command]
pub async fn fleet_direct_message(
    app_state: State<'_, AppState>,
    input: DirectMessageInput,
    on_event: tauri::ipc::Channel<DispatchEvent>,
) -> Result<(), ErrorResponse> {
    // 1. 定位目标成员
    let members = app_state.fleet_repository.list_members(&input.fleet_id).await.map_err(|e| {
        ErrorResponse::from_error_with_code(fleet_err::NOT_FOUND, e, ErrorCategory::General)
    })?;
    let target =
        members.into_iter().find(|m| m.agent_slug == input.agent_slug).ok_or_else(|| {
            ErrorResponse::new(fleet_err::TARGET_NOT_IN_FLEET)
                .with_category(ErrorCategory::Validation)
                .with_param("slug", input.agent_slug.clone())
        })?;

    // 2. 用户消息落库（DM 同样进对话记录）
    //
    //    ⚠ 会话 = `dm:<slug>`，**不是** `target.room_id`。后者是精灵站位，
    //    用它当会话键会让「与 A 的私信」和「A 在群聊里的回复」落进同一线程
    //    （A 的两次发言房间都是它的站位），于是 DM 混进群聊面板与路由 prompt。
    //
    //    ⚠ 此处**刻意不做**新鲜度 preflight：DM 是「点名找这个人」，目标唯一且
    //    用户明确，不存在「抢答」语义，被暂扣只会让人困惑。
    //    cumora 也把 2 人 DM 列在新鲜度门的豁免名单里（其 COORDINATION.md：
    //    「2 人 DM（并行打字是正常的）」）—— 那道门只属于群聊路径。
    let conversation_id = conversation_dm(&target.agent_slug);
    append_human_message(&app_state, &input.fleet_id, &conversation_id, &input.user_message)
        .await?;

    // 3. 路由决策事件
    send_event(
        &on_event,
        DispatchEvent::Routing {
            agent_slug: target.agent_slug.clone(),
            agent_id: target.agent_id.clone(),
            room_id: target.room_id.clone(),
            task_summary: input.user_message.clone(),
        },
    );

    // 4. 真实执行成员回合（错误事件已由 executor 推送，仍需 Complete 收尾）
    let emit = |evt: DispatchEvent| {
        let _ = on_event.send(evt);
    };
    let result = execute_fleet_turn(
        &app_state,
        &input.fleet_id,
        &conversation_id,
        &target,
        &input.user_message,
        &emit,
    )
    .await;

    // 5. 流结束
    send_event(&on_event, DispatchEvent::Complete);

    result.map(|_| ())
}

/// 列出**指定会话**的持久化消息（按 `seq` 升序）。
///
/// - `conversation_id`：群聊传 `"group"`，私信传 `"dm:<slug>"`。
///   缺省为群聊会话（前端群聊面板不传也能工作）。
/// - `after_seq`：只返回 `seq > after_seq` 的消息（增量拉取 / 断线续传）
/// - `limit`：条数上限，缺省 [`CONVERSATION_HISTORY_LIMIT`]
#[agent_command(domain = fleet, safety = Safe, call_mode = StateInput, description = "列出会话消息历史")]
#[tauri::command]
pub async fn fleet_list_messages(
    app_state: State<'_, AppState>,
    fleet_id: String,
    conversation_id: Option<String>,
    after_seq: Option<i64>,
    limit: Option<u32>,
) -> Result<Vec<FleetMessage>, ErrorResponse> {
    let conversation_id = conversation_id.unwrap_or_else(|| CONVERSATION_GROUP.to_string());
    app_state
        .fleet_repository
        .list_messages(
            &fleet_id,
            &conversation_id,
            after_seq,
            limit.unwrap_or(CONVERSATION_HISTORY_LIMIT),
        )
        .await
        .map_err(|e| ErrorResponse::from_error(e, ErrorCategory::General))
}

// ── 内部工具 ─────────────────────────────────────────────────────────

/// 发送事件到 Channel（忽略前端已销毁的静默失败）
fn send_event(channel: &tauri::ipc::Channel<DispatchEvent>, event: DispatchEvent) {
    if let Err(e) = channel.send(event) {
        warn!("[fleet] 事件推送失败（前端可能已关闭）: {e}");
    }
}

// ── 消息持久化辅助 ───────────────────────────────────────────────────

/// 单次注入 prompt / **单次**返回给前端的会话历史最大条数。
///
/// 太大既费 token、又稀释当前意图；太小则 agent 看不到必要的来龙去脉。
///
/// ⚠ 这是**单次往返**的上限，不是前端累积的上限：前端按 `seq` 合并多次结果，
/// 本地时间线会随会话推进而增长（消息只追加不删除，故累积是安全的）。
const CONVERSATION_HISTORY_LIMIT: u32 = 30;

/// 人类作者的固定本地标识。
///
/// AxInvest 是单用户桌面应用，「我是谁」暂时不构成需要建模的维度；
/// 保留 `author_id` 是为了让协调门的「作者 ≠ 当前成员」判据在将来引入
/// 多人类成员时无需改表结构。
const LOCAL_USER_ID: &str = "local-user";

/// 写入一条人类消息，返回带已分配 `seq` 的记录。
async fn append_human_message(
    app_state: &AppState,
    fleet_id: &str,
    conversation_id: &str,
    content: &str,
) -> Result<FleetMessage, ErrorResponse> {
    let message = FleetMessage {
        id: uuid::Uuid::new_v4().to_string(),
        fleet_id: fleet_id.to_string(),
        conversation_id: conversation_id.to_string(),
        // seq 由 DAO 在写入时分配，此处仅为占位
        seq: 0,
        author_kind: AUTHOR_KIND_HUMAN.to_string(),
        author_id: LOCAL_USER_ID.to_string(),
        author_slug: None,
        author_display_name: None,
        content: content.to_string(),
        created_at: chrono::Utc::now().timestamp_millis(),
    };
    app_state
        .fleet_repository
        .append_message(message)
        .await
        .map_err(|e| ErrorResponse::from_error(e, ErrorCategory::General))
}

/// 写入一条 agent 消息，返回带已分配 `seq` 的记录。
///
/// `conversation_id` 必须由调用方传入**本轮所属的会话**（群聊 or 对应 DM），
/// 不能用 `member.room_id` 顶替 —— 那是精灵站位，同一成员在群聊与私信里
/// 站位相同，用它当会话键会把两个线程混成一个。
pub(crate) async fn append_agent_message(
    app_state: &AppState,
    fleet_id: &str,
    conversation_id: &str,
    member: &FleetMember,
    content: &str,
) -> Result<FleetMessage, ErrorResponse> {
    let message = FleetMessage {
        id: uuid::Uuid::new_v4().to_string(),
        fleet_id: fleet_id.to_string(),
        conversation_id: conversation_id.to_string(),
        seq: 0,
        author_kind: AUTHOR_KIND_AGENT.to_string(),
        author_id: member.agent_id.clone(),
        author_slug: Some(member.agent_slug.clone()),
        author_display_name: Some(member.display_name.clone()),
        content: content.to_string(),
        created_at: chrono::Utc::now().timestamp_millis(),
    };
    app_state
        .fleet_repository
        .append_message(message)
        .await
        .map_err(|e| ErrorResponse::from_error(e, ErrorCategory::General))
}

/// 读取某会话最近的对话历史（按 `seq` 升序）。
///
/// 读库失败时返回空历史并**出声** —— 与 `preflight_held` 同为 fail-open：
/// 让 agent 在缺历史的情况下作答，好过让用户完全说不了话。
/// 但绝不静默：静默的降级会让「agent 突然失忆」变成无从诊断的悬案。
async fn load_conversation_history(
    app_state: &AppState,
    fleet_id: &str,
    conversation_id: &str,
) -> Vec<FleetMessage> {
    match app_state
        .fleet_repository
        .list_messages(fleet_id, conversation_id, None, CONVERSATION_HISTORY_LIMIT)
        .await
    {
        Ok(messages) => messages,
        Err(e) => {
            warn!("[fleet] 读取会话历史失败，本轮以空历史继续（fail-open）: {e}");
            Vec::new()
        },
    }
}

/// 协调门 —— **新鲜度 preflight**。
///
/// ## 判据
///
/// 不是「会话有没有新消息」，而是「**本次读取（`baseline_seq`）之后，会话有没有
/// 被其他 agent 推进**」。
///
/// 这个区别决定成败：若用「有没有比自己见过的更新」，首次调用必然命中
/// （会话本来就有历史），于是**每次都要被暂扣一轮**——cumora 的 compose-anchor
/// 门就是因为这个被退役的（其 COORDINATION.md §5a：在繁忙房间总是首次尝试即 HOLD，
/// 每次回复多花 1-2 次大模型往返）。
///
/// 两侧都只看 **agent** 水位（`only_agents = true`），且都限定在
/// `conversation_id` 指定的会话内 —— 正常串行下 `current_seq == baseline_seq`，
/// 恒放行；DM 的写入也不会污染群聊的判据。
///
/// ## 返回
///
/// `Some((max_seq, held_messages))` = 应当暂扣，并把未读消息内联返回（即「展示」）。
/// `None` = 放行。
///
/// ## fail-open
///
/// 查询失败一律**放行**：宁可漏过一次竞态检测，也不要把用户的正常对话卡死
/// （与 cumora 对 hold-token 的处理一致 —— Redis 报错即不阻塞工作）。但必须出声。
async fn preflight_held(
    app_state: &AppState,
    fleet_id: &str,
    conversation_id: &str,
    baseline_seq: i64,
) -> Option<(i64, Vec<FleetMessage>)> {
    let current_seq =
        match app_state.fleet_repository.max_seq(fleet_id, conversation_id, true).await {
            Ok(seq) => seq,
            Err(e) => {
                warn!("[fleet] 新鲜度门查询会话水位失败，本次放行（fail-open）: {e}");
                return None;
            },
        };
    if current_seq <= baseline_seq {
        return None;
    }

    // 把「基线之后新增的消息」内联返回 —— 这一步就是契约里的「展示」
    match app_state
        .fleet_repository
        .list_messages(fleet_id, conversation_id, Some(baseline_seq), CONVERSATION_HISTORY_LIMIT)
        .await
    {
        Ok(held_messages) => {
            info!(
                "[fleet] 新鲜度门暂扣本轮：会话「{conversation_id}」agent 基线 seq={baseline_seq}，\
                 水位已到 {current_seq}，内联展示 {} 条新增消息",
                held_messages.len()
            );
            Some((current_seq, held_messages))
        },
        Err(e) => {
            warn!("[fleet] 新鲜度门读取未读消息失败，本次放行（fail-open）: {e}");
            None
        },
    }
}

/// 构造路由系统提示词（成员列表 + 路由规则）
fn build_fleet_system_prompt(members: &[FleetMember]) -> String {
    let member_list: Vec<String> = members
        .iter()
        .map(|m| {
            format!(
                "- slug: \"{}\", 角色: \"{}\", 房间: \"{}\", 状态: {:?}",
                m.agent_slug, m.role, m.room_id, m.status
            )
        })
        .collect();

    format!(
        "你是一个智能调度员,负责将用户消息路由到最合适的 AI agent。\n\n\
         ## 可用成员\n{}\n\n\
         ## 路由规则\n\
         1. 仔细分析用户消息的意图\n\
         2. 根据成员的角色描述选择最合适的一个\n\
         3. 仅返回 JSON,不要任何额外文本\n\n\
         ## 返回格式\n\
         {{\"agent_slug\": \"<成员 slug>\", \"reason\": \"<选择原因,简短>\"}}",
        member_list.join("\n")
    )
}

/// 构造路由用户提示词（用户消息 + 真实历史）
///
/// 历史来自数据库（`FleetMessage`），不再由调用方传入。
/// 也不再需要按 role 过滤 —— `author_kind` 已保证每条都是真实发生过的对话消息。
fn build_fleet_user_prompt(user_message: &str, history: &[FleetMessage]) -> String {
    if history.is_empty() {
        return format!("用户消息:\n{user_message}");
    }

    let history_text: Vec<String> = history
        .iter()
        .map(|h| {
            // 人类作者没有 slug，用 "user" 呈现；agent 用其 slug
            let speaker = h.author_slug.as_deref().unwrap_or("user");
            format!("[{speaker}]: {}", h.content)
        })
        .collect();

    format!("历史对话:\n{}\n\n用户消息:\n{user_message}", history_text.join("\n"))
}

/// 解析 LLM 返回的 JSON，提取 agent_slug（兼容 markdown 包裹）
fn parse_route_response(response: &str) -> Option<String> {
    let trimmed = response.trim();
    let json_str = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|s| s.strip_suffix("```"))
        .map(|s| s.trim())
        .unwrap_or(trimmed);

    let parsed: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let slug = parsed.get("agent_slug")?.as_str()?.to_string();
    if slug.is_empty() { None } else { Some(slug) }
}

/// 将 LLM 返回的目标标识解析到实际成员 slug：精确 → 归一化 → 子串/显示名匹配。
///
/// LLM 自由文本可能返回带引号/大小写变体/显示名，直接 `==` 比对经常落空，
/// 此处逐级容错后再兜底，减少"静默路由到第一个成员"的错配概率。
fn resolve_target_slug(routable: &[FleetMember], raw_slug: &str) -> Option<String> {
    // 1. 精确匹配
    if let Some(m) = routable.iter().find(|m| m.agent_slug == raw_slug) {
        return Some(m.agent_slug.clone());
    }
    // 2. 归一化匹配（去空白/引号，大小写不敏感）
    let norm = |s: &str| s.trim().trim_matches('"').trim_matches('\'').to_lowercase();
    let normalized = norm(raw_slug);
    if let Some(m) = routable.iter().find(|m| norm(&m.agent_slug) == normalized) {
        return Some(m.agent_slug.clone());
    }
    // 3. 子串 / 显示名匹配（LLM 可能返回 display_name 或别名）
    routable
        .iter()
        .find(|m| {
            norm(&m.agent_slug).contains(&normalized)
                || normalized.contains(&norm(&m.agent_slug))
                || norm(&m.display_name) == normalized
        })
        .map(|m| m.agent_slug.clone())
}

// ── 域包建房即成队查询（PLAN-office-auto-provision.md 阶段 1）────────

/// 域包专家 Profile（`list_domain_pack_profiles` 返回，创建房间弹窗「自动配齐成员」消费）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainPackProfile {
    pub profile_id: String,
    pub name: String,
    pub expert_key: String,
    pub recommended_tools: Vec<String>,
    /// `agent_profiles` 表中是否已有该行（false = seed 未跑或被删，消费方应跳过并提示）
    pub exists_in_db: bool,
    /// manifest `office.seed_members[].room` 显式指定的 Phaser 房间；None = 前端轮转分配
    pub room: Option<String>,
}

/// 列出域包专家的种子化 Profile。
///
/// 权威源：manifest 的 `office.seed_members` 段（数据驱动，优先）；缺省回退
/// Rust 名册（`capability_pack_agents::domain_pack_roster`），profile id 约定 `opc-<expert_key>`。
/// 未知或无 seed 的域包（如 content_media）返回空列表而非报错——前端只对行业场景模板调用，
/// 「配不出队」是事实不是异常。
#[agent_command(domain = fleet, safety = Safe, call_mode = StateInput, description = "列出域包专家 Profile")]
#[tauri::command]
pub async fn list_domain_pack_profiles(
    app_state: State<'_, AppState>,
    domain_pack_id: String,
) -> Result<Vec<DomainPackProfile>, ErrorResponse> {
    use crate::commands::opc_setup::capability_pack_agents::{
        domain_pack_profile_id, domain_pack_roster,
    };
    use axagent_analysis_engine::opc::capability_pack;
    use axagent_entities::agent_profiles;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    // manifest office 段（目录名容忍 kebab→snake 变体，见 opc_capability_pack_runtime.rs 先例）
    let base = capability_pack::resolve_capability_packs_dir(Some(&app_state.app_data_dir));
    let direct = base.join(&domain_pack_id);
    let pack_dir = if direct.is_dir() {
        direct
    } else {
        base.join(domain_pack_id.replace('-', "_"))
    };
    let declared =
        capability_pack::read_manifest(&pack_dir).and_then(|m| m.office).map(|o| o.seed_members);

    let roster = domain_pack_roster(&domain_pack_id);
    let members = resolve_seed_members(roster.map(|(e, _)| e).unwrap_or(&[]), declared);
    if members.is_empty() {
        warn!("[fleet] list_domain_pack_profiles: 域包 {domain_pack_id} 无专家名册，返回空");
        return Ok(vec![]);
    }
    let ids: Vec<String> = members.iter().map(|(k, _)| domain_pack_profile_id(k)).collect();
    let db = app_state.harness.db();
    let rows = agent_profiles::Entity::find()
        .filter(agent_profiles::Column::Id.is_in(ids))
        .all(db)
        .await
        .map_err(|e| ErrorResponse::from_error(e, ErrorCategory::General))?;
    Ok(members
        .iter()
        .map(|(key, room)| {
            let pid = domain_pack_profile_id(key);
            let row = rows.iter().find(|r| r.id == pid);
            let fallback_name = roster
                .map(|(e, _)| {
                    e.iter().find(|(k, _, _)| k == key).map(|(_, n, _)| *n).unwrap_or(key.as_str())
                })
                .unwrap_or(key.as_str());
            DomainPackProfile {
                name: row.map(|r| r.name.clone()).unwrap_or_else(|| fallback_name.to_string()),
                recommended_tools: roster
                    .and_then(|(_, t)| t.iter().find(|(k, _)| *k == key).map(|(_, v)| v))
                    .map(|t| t.iter().map(|s| s.to_string()).collect())
                    .unwrap_or_default(),
                expert_key: key.clone(),
                exists_in_db: row.is_some(),
                room: room.clone(),
                profile_id: pid,
            }
        })
        .collect())
}

/// 播种名单裁决：manifest `office.seed_members` 非空 ⇒ 整表覆盖；否则回退 Rust 名册推导。
fn resolve_seed_members(
    roster_experts: &[(&str, &str, &str)],
    declared: Option<Vec<axagent_analysis_engine::opc::capability_pack::CapabilityPackSeedMember>>,
) -> Vec<(String, Option<String>)> {
    if let Some(members) = declared.filter(|v| !v.is_empty()) {
        return members.into_iter().map(|m| (m.expert, m.room)).collect();
    }
    roster_experts.iter().map(|(k, _, _)| (k.to_string(), None)).collect()
}

// ── 域包办公室场景模板数据驱动（PLAN-office-auto-provision.md 阶段 3）────

/// `{domain_pack_dir}/office_scene.yaml` 的镜像 —— 与前端 `OfficeSceneTemplate` 同形
/// （camelCase，禁区 13）。前端启动时经 `registerSceneTemplate` 注入，TS 内置模板同名优先。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfficeSceneTemplateDto {
    pub slug: String,
    pub display_name_key: String,
    pub canvas_width: u32,
    pub canvas_height: u32,
    pub default_room_id: String,
    pub rooms: Vec<OfficeSceneRoomDto>,
    #[serde(default)]
    pub furniture: Option<std::collections::HashMap<String, Vec<OfficeSceneFurnitureDto>>>,
    #[serde(default)]
    pub decorations: Option<std::collections::HashMap<String, Vec<OfficeSceneFurnitureDto>>>,
}

/// 房间矩形（key = roomId 的家具/装饰映射使用 `id` 对齐）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfficeSceneRoomDto {
    pub id: String,
    pub name_key: String,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub color: u32,
}

/// 家具 / 墙面装饰（两种 kind 联合由前端类型与门禁脚本约束，此处统一形状）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfficeSceneFurnitureDto {
    pub kind: String,
    pub x: u32,
    pub y: u32,
    #[serde(default)]
    pub w: Option<u32>,
    #[serde(default)]
    pub h: Option<u32>,
    #[serde(default)]
    pub color: Option<u32>,
}

/// 列出全部域包办公室场景模板（`office_scene.yaml` 数据驱动，零代码新增域包场景）。
///
/// 解析失败/缺文件的目录 warn 后跳过——「存在即必须可解析」的判据由
/// `scripts/check-office-scene-align.mjs` 硬拦，运行时面不重复设防。
#[agent_command(domain = fleet, safety = Safe, call_mode = StateOnly, description = "列出域包办公室场景模板")]
#[tauri::command]
pub async fn list_office_scene_templates(
    app_state: State<'_, AppState>,
) -> Result<Vec<OfficeSceneTemplateDto>, ErrorResponse> {
    use axagent_analysis_engine::opc::capability_pack;

    let base = capability_pack::resolve_capability_packs_dir(Some(&app_state.app_data_dir));
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&base) else {
        warn!("[fleet] list_office_scene_templates: 域包目录不存在: {}", base.display());
        return Ok(out);
    };
    for entry in entries.flatten() {
        let file = entry.path().join("office_scene.yaml");
        if !file.is_file() {
            continue;
        }
        // 注意：此处解析失败仅 warn 后跳过、不作为命令错误返回前端，
        // 故不走 ErrorResponse 错误码通道（见函数文档）；直接 match 保留原始错误用于日志，
        // 避免触发 check-rust-raw-map-err.mjs 的裸转换门禁。
        let raw = match std::fs::read_to_string(&file) {
            Ok(raw) => raw,
            Err(e) => {
                warn!("[fleet] 办公室场景模板读取失败（{}）: {e}", file.display());
                continue;
            },
        };
        match serde_yaml::from_str::<OfficeSceneTemplateDto>(&raw) {
            Ok(tpl) => out.push(tpl),
            Err(e) => warn!("[fleet] 办公室场景模板解析失败（{}）: {e}", file.display()),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod office_provision_tests {
    use super::{OfficeSceneTemplateDto, resolve_seed_members};
    use axagent_analysis_engine::opc::capability_pack::{
        CapabilityPackManifest, CapabilityPackSeedMember,
    };

    #[test]
    fn manifest_office_section_parses_and_defaults() {
        let m: CapabilityPackManifest = serde_yaml::from_str(
            "id: design\nname: 设计\noffice:\n  seed_members:\n    - expert: design-ux\n      room: studio\n",
        )
        .unwrap();
        let office = m.office.expect("office 段应解析");
        assert_eq!(office.seed_members.len(), 1);
        assert_eq!(office.seed_members[0].expert, "design-ux");
        assert_eq!(office.seed_members[0].room.as_deref(), Some("studio"));

        // 无 office 段的存量 manifest 不受影响
        let m2: CapabilityPackManifest =
            serde_yaml::from_str("id: accounting\nname: 会计\n").unwrap();
        assert!(m2.office.is_none());
    }

    #[test]
    fn resolve_seed_members_override_and_fallback() {
        let roster: &[(&str, &str, &str)] = &[("a-x", "A", ""), ("b-y", "B", "")];
        let declared =
            vec![CapabilityPackSeedMember { expert: "b-y".into(), room: Some("meeting".into()) }];
        // 非空声明 ⇒ 整表覆盖（含显式 room）
        assert_eq!(
            resolve_seed_members(roster, Some(declared)),
            vec![("b-y".to_string(), Some("meeting".to_string()))]
        );
        // 空声明 ⇒ 回退名册（room 全 None）
        let fb = resolve_seed_members(roster, Some(vec![]));
        assert_eq!(fb.len(), 2);
        assert!(fb.iter().all(|(_, room)| room.is_none()));
        // 无名册无声明 ⇒ 空（命令层据此 warn + 返回空）
        assert!(resolve_seed_members(&[], None).is_empty());
    }

    #[test]
    fn office_scene_yaml_dto_camel_case_roundtrip() {
        let yaml = r#"
slug: design
displayNameKey: design
canvasWidth: 800
canvasHeight: 500
defaultRoomId: studio
rooms:
  - id: studio
    nameKey: studio
    x: 40
    y: 60
    width: 360
    height: 220
    color: 16711680
furniture:
  studio:
    - kind: desk
      x: 80
      y: 130
"#;
        let tpl: OfficeSceneTemplateDto = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(tpl.default_room_id, "studio");
        assert_eq!(tpl.rooms.len(), 1);
        assert_eq!(tpl.furniture.as_ref().unwrap()["studio"].len(), 1);
        assert!(tpl.decorations.is_none());
    }
}
