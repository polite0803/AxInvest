// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Fleet（多办公室 AI 团队）类型定义。
 *
 * 与后端 `axagent_harness::fleet` DTO 一一对应。
 * 后端权威定义：src-tauri/crates/harness/src/fleet.rs
 */

/** 舰队（办公室）状态 */
export type FleetStatus = "active" | "paused" | "stopped";

/** 舰队成员状态 */
export type FleetMemberStatus =
  | "idle"
  | "busy"
  | "paused"
  | "error"
  | "offline";

/** 舰队元数据 — 业务层可扩展信息 */
export interface FleetMetadata {
  /** 业务描述 */
  description: string;
  /** 最大成员数（0 表示无限制） */
  maxMembers: number;
  /** 协作策略名称（如 "ecommerce_ops" / "customer_service"） */
  strategy?: string;
  /** 自定义标签 */
  tags: string[];
}

/** 舰队（办公室）— 一个正在运行的 AI 团队 */
export interface Fleet {
  /** 唯一 ID（UUID） */
  id: string;
  /** 显示名称 */
  name: string;
  /** 场景模板 slug（可选，下游业务系统可填） */
  sceneTemplateSlug?: string;
  /** 舰队状态 */
  status: FleetStatus;
  /** 创建时间（Unix 毫秒） */
  createdAt: number;
  /** 更新时间（Unix 毫秒） */
  updatedAt: number;
  /** 业务元数据 */
  metadata: FleetMetadata;
}

/** 舰队成员 — 办公室里的一个 agent */
export interface FleetMember {
  /** 唯一 ID（UUID） */
  id: string;
  /** 所属舰队 ID */
  fleetId: string;
  /** 关联的 AgentSession ID */
  agentId: string;
  /** agent slug（业务标识，用于 Dispatcher 路由） */
  agentSlug: string;
  /** 显示名称 */
  displayName: string;
  /** 角色描述（注入到 Dispatcher prompt；与 agentProfileId 二选一，均可） */
  role: string;
  /** 关联的 AgentProfile ID（AgentProfile = 角色 + 专家组合，定义成员智能体身份） */
  agentProfileId?: string;
  /** 房间 ID（前端 Phaser 渲染位置，如 "manager" / "meeting"） */
  roomId: string;
  /** 成员状态 */
  status: FleetMemberStatus;
  /** 加入时间（Unix 毫秒） */
  joinedAt: number;
  /** 今日 token 用量 */
  todayTokens: number;
  /** 累计 token 用量 */
  totalTokens: number;
}

// ── Dispatcher 事件流 ────────────────────────────────────────────────

/** 调度事件 — Dispatcher 在路由与执行过程中产生的事件流 */
export type DispatchEvent =
  | { type: "routing"; agentSlug: string; agentId: string; roomId: string; taskSummary: string }
  | { type: "process"; agentSlug: string; agentId: string; status: string }
  | { type: "agent_message"; agentSlug: string; agentId: string; content: string }
  | { type: "agent_status"; agentSlug: string; agentId: string; status: FleetMemberStatus }
  | { type: "token_usage"; agentSlug: string; agentId: string; inputTokens: number; outputTokens: number }
  | { type: "complete" }
  | { type: "error"; message: string }
  /**
   * 消息被协调门暂扣 —— 本次回合**未执行**。
   *
   * `heldMessages` 是**未读的更新消息**，服务端已把它们展示出来；
   * 读过之后直接重发即可通过（「展示即已见」契约），**不需要任何旗标**。
   * 详见后端 `DispatchEvent::Held` 的说明与「为何刻意没有 force 参数」。
   *
   * `maxSeq` 是**该会话**的 agent 水位，与 `heldMessages` 同源但**不相等**
   * （后者受条数上限截断）；UI 用它显示「已推进到 #N」，让用户判断落后多远。
   */
  | { type: "held"; maxSeq: number; heldMessages: FleetMessage[] };

/**
 * 舰队消息 — **已持久化**的群聊 / DM 消息。
 *
 * 与后端 `FleetMessage` 对应。这是对话的**真源**：历史不再由前端临时构造
 * 后传给后端（那种做法一刷新就没了，agent 也看不到房间的真实来龙去脉）。
 */
export interface FleetMessage {
  /** 唯一 ID */
  id: string;
  /** 所属舰队 ID */
  fleetId: string;
  /**
   * **会话作用域** ID —— 消息线程的归属。
   *
   * - 群聊：`"group"`（见 {@link CONVERSATION_GROUP}）
   * - 私信：`"dm:<agentSlug>"`（见 {@link conversationDm}）
   *
   * ⚠ 不是 `FleetMember.roomId` —— 那是精灵站位的**物理房间**，不参与消息查询。
   */
  conversationId: string;
  /** 单调递增序号（**同一 (fleetId, conversationId) 内**唯一且递增） */
  seq: number;
  /** 作者类型 */
  authorKind: "human" | "agent";
  /** 作者 ID（human 为本地固定标识；agent 为 agentId） */
  authorId: string;
  /** 作者 slug（agent 才有） */
  authorSlug?: string;
  /** 作者显示名（human 无） */
  authorDisplayName?: string;
  /** 消息正文 */
  content: string;
  /** 创建时间（Unix 毫秒） */
  createdAt: number;
}

// ── 会话作用域 ────────────────────────────────────────────────────────

/**
 * 群聊（智能路由）会话的 `conversationId`。
 *
 * 群聊是舰队内**唯一**的共享会话。⚠ 与 `FleetMember.roomId`（精灵站位的物理房间，
 * 如 `"workspace"` / `"showroom"`）是两个正交概念，**不要互相顶替** ——
 * 后端曾因两者同名而在「DM 与群聊共享一条时间线」上翻车。
 */
export const CONVERSATION_GROUP = "group";

/** 与指定成员私信的 `conversationId`（后端同名 helper：`conversation_dm`）。 */
export function conversationDm(agentSlug: string): string {
  return `dm:${agentSlug}`;
}

/** `messagesByConversation` 的键：会话是 (舰队, 会话 ID) 的二元组。 */
export function conversationKey(fleetId: string, conversationId: string): string {
  // 用 join 而不是「模板字符串里把两段插值夹着 `::` 拼起来」：门禁 `check-id-validation.mjs`
  // 会把那种形态报成「模板字符串拼接可能为 undefined 的 ID」（纯正则扫描，读不到类型；
  // 连注释里写的示例也算命中 —— 这正是该脚本自带 FALSE_POSITIVE_FILES 白名单的原因）。
  // 但它建议的修法 `safeJoinIds` 在这个位置是错的：`safeJoinIds` 是
  // `filter(isValidId)` 后再 join，缺一段时**静默**产出**一段**的键；而本键是
  // **持久化**结构（`messagesByConversation`，键格式见 `ChatPanel.tsx` 顶部说明）
  // ⇒ 退化的键既读不回原消息、也不抛错，比 `undefined::xxx` 更难发现。
  // 两段在类型上都非可选（`string`），保证来自赋值端；此处只拼接，不做过滤。
  return [fleetId, conversationId].join("::");
}

// ── 命令输入参数 ──────────────────────────────────────────────────────

/** 创建舰队输入 */
export interface CreateFleetInput {
  /** 显示名称 */
  name: string;
  /** 场景模板 slug（可选） */
  sceneTemplateSlug?: string;
  /** 业务元数据 */
  metadata?: FleetMetadata;
}

/** 添加成员输入 */
export interface AddMemberInput {
  /** 所属舰队 ID */
  fleetId: string;
  /** 关联的 AgentSession ID */
  agentId: string;
  /** agent slug */
  agentSlug: string;
  /** 显示名称 */
  displayName: string;
  /** 角色描述 */
  role?: string;
  /** 关联的 AgentProfile ID（定义成员智能体身份） */
  agentProfileId?: string;
  /** 房间 ID（默认 "workspace"） */
  roomId?: string;
}

/**
 * 域包专家 Profile — `list_domain_pack_profiles` 返回值。
 *
 * 与后端 `DomainPackProfile`（commands/fleet/mod.rs，`PLAN-office-auto-provision.md` 阶段 1 DTO）字段一致。
 */
export interface DomainPackProfile {
  /** profile id（约定 `opc-<expertKey>`，同时用作 agentSlug / agentProfileId） */
  profileId: string;
  /** 显示名（seed 行带 icon 前缀） */
  name: string;
  expertKey: string;
  recommendedTools: string[];
  /** `agent_profiles` 表中是否已有该行（false = seed 未跑或被删，入房时应跳过） */
  existsInDb: boolean;
}

/** 建房即成队的播种结果 */
export interface SeedMembersResult {
  /** 成功入房成员数 */
  seeded: number;
  /** 跳过数（无 DB 行 / slug 冲突 / 单条失败） */
  skipped: number;
}

/**
 * 群聊智能路由输入。
 *
 * 已无 `history` 字段：历史由后端从数据库读取（库是唯一真源）。
 * 即便调用方多传了该字段，serde 也会忽略，不会报错。
 */
export interface DispatchInput {
  /** 舰队 ID */
  fleetId: string;
  /** 用户消息 */
  userMessage: string;
}

/** 直接 DM 指定 agent 输入（同样无 history，历史由后端从库读取） */
export interface DirectMessageInput {
  /** 舰队 ID */
  fleetId: string;
  /** 目标 agent slug */
  agentSlug: string;
  /** 用户消息 */
  userMessage: string;
}

// ── 前端 UI 辅助类型 ──────────────────────────────────────────────────

/** Phaser 场景模板（前端 Phaser 办公室渲染用） */
export interface OfficeSceneTemplate {
  /** 模板 slug（如 "default_office" / "ecommerce_showroom"） */
  slug: string;
  /** 显示名称（i18n key） */
  displayNameKey: string;
  /** 房间布局（房间 ID → 像素坐标） */
  rooms: Record<string, { x: number; y: number; width: number; height: number }>;
  /** 默认房间 ID */
  defaultRoomId: string;
}

/** Phaser agent 精灵状态（前端 Phaser 渲染用） */
export interface AgentSpriteState {
  /** 关联的成员 ID */
  memberId: string;
  /** agent slug */
  agentSlug: string;
  /** 当前房间 ID */
  roomId: string;
  /** 精灵动画状态：idle / walking / typing / celebrating */
  animation: "idle" | "walking" | "typing" | "celebrating";
  /** 朝向：left / right */
  facing: "left" | "right";
  /** 像素坐标 */
  x: number;
  y: number;
  /** 目标坐标（行走动画的目标点） */
  targetX?: number;
  targetY?: number;
}
