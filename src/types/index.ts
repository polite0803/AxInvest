// SPDX-License-Identifier: AGPL-3.0-only

// === Paired Types (全局同步字段对) ===
export * from "./paired";

// === Bot 归属层（IPC 冻结契约，见 docs/bot-integration/IPC.md §3） ===
export * from "./bot";

// === Time Travel（时间旅行批次复盘） ===
export * from "./timeTravel";

// === Model Selection System ===
export * from "./modelSelection";

// === IPC 事件名契约（由 schema-gen 从后端 axagent_harness::IpcEventName 生成，勿手改） ===
export type { IpcEventName } from "./generated/events";

// === 后端 DTO 生成物（由 schema-gen 从 axagent-harness 结构体生成，勿手改） ===
// 权威定义在后端：字段名走 serde 的 `rename_all = "camelCase"`，`Option<T>` 走 `T | null`。
// 这里只 re-export —— 既有 `import type { ProviderKey } from "@/types"` 的消费方零改动，
// 但 `src/types/` 里不再保留第二份手写定义（PLAN-shell-plus-plugins §91.3 B8-3 首刀）。
// 单域类型门面不入本 barrel（B8-5 域收敛）：OPC 域类型只经 "@/types/opc" 深路径直取 ——
// 该域随插件化整体归 OPC 面，由 barrel 汇出等于让全局命名空间替单域演进背书，摘一处即少一处耦合面。
// 重新生成：`cd axagent-plugins && cargo run -p schema-gen -- frontend-dto`
import type {
  ConsensusEPS,
  DashboardIndexQuote,
  EarningsEvent,
  IndexQuote,
  KLine,
  ModelCapability,
  ModelType,
  ProviderKey,
  ProviderProxyConfig,
  ProviderType,
  StockQuote,
  StockSearchResult,
  SubWorkflowOrigin,
} from "./generated/dto";
export type {
  ConsensusEPS,
  DashboardIndexQuote,
  EarningsEvent,
  IndexQuote,
  KLine,
  ModelCapability,
  ModelType,
  ProviderKey,
  ProviderProxyConfig,
  ProviderType,
  StockQuote,
  StockSearchResult,
  SubWorkflowOrigin,
};

// === Provider System ===
import type { SkillPermissions } from "@/sdk/types";
import type { RAGPipelineConfig } from "./knowledge";
import type { NullableModelRef } from "./paired";
import type { TaskShapeDecision } from "./taskShape";

export interface ProviderConfig {
  id: string;
  name: string;
  providerType: ProviderType;
  apiHost: string;
  apiPath: string | null;
  enabled: boolean;
  models: Model[];
  keys: ProviderKey[];
  proxyConfig: ProviderProxyConfig | null;
  toolAdaptation: string | null;
  toolAdaptationMarkerPrefix: string | null;
  customHeaders: string | null;
  icon: string | null;
  builtinId: string | null;
  sortOrder: number;
  createdAt: number;
  updatedAt: number;
}

export interface CreateProviderInput {
  name: string;
  providerType: ProviderType;
  apiHost: string;
  apiPath?: string | null;
  enabled: boolean;
}

export interface UpdateProviderInput {
  name?: string;
  providerType?: ProviderType;
  apiHost?: string;
  apiPath?: string | null;
  enabled?: boolean;
  proxyConfig?: ProviderProxyConfig;
  toolAdaptation?: string | null;
  toolAdaptationMarkerPrefix?: string | null;
  customHeaders?: string | null;
  icon?: string | null;
  sortOrder?: number;
}

// === Model System ===
// `ModelCapability` / `ModelType` 见文件顶部的生成物 re-export（权威在 harness，勿手写）。

export interface Model {
  providerId: string;
  modelId: string;
  name: string;
  groupName?: string | null;
  modelType: ModelType;
  capabilities: ModelCapability[];
  maxTokens: number | null;
  enabled: boolean;
  paramOverrides: ModelParamOverrides | null;
  /** Input price per million tokens (USD). From provider sync or user settings. */
  inputPricePerMtok?: number | null;
  /** Output price per million tokens (USD). From provider sync or user settings. */
  outputPricePerMtok?: number | null;
}

export interface ModelParamOverrides {
  temperature?: number;
  maxTokens?: number;
  topP?: number;
  useMaxCompletionTokens?: boolean;
  noSystemRole?: boolean;
  thinkingParamStyle?: string;
  /** 思考预算（token）：0 = 关闭思考；不填 = 不下发（供应商默认）。 */
  thinkingBudget?: number;
  requestDelayMs?: number;
  /**
   * 首字节（HTTP 响应头）等待预算（秒）。这是**硬档总上限**（含同供应商原地重试一次），
   * 软档取其一半；不填 = 用默认（软 45s / 硬 90s）。
   */
  headerTimeoutSecs?: number;
}

// === Conversation & Message ===
export type MessageRole = "system" | "user" | "assistant" | "tool";

export type MessageStatus = "complete" | "partial" | "error" | "cancelled";

export interface ConversationCategory {
  id: string;
  name: string;
  iconType: string | null;
  iconValue: string | null;
  systemPrompt: string | null;

  // 使用 NullableModelRef 保证结构一致性
  defaultModel: NullableModelRef;
  sortOrder: number;
  isCollapsed: boolean;
  createdAt: number;
  updatedAt: number;
}

export interface Conversation {
  id: string;
  title: string;
  modelId: string;
  providerId: string;
  systemPrompt: string | null;
  temperature: number | null;
  maxTokens: number | null;
  topP: number | null;
  searchEnabled: boolean;
  searchProviderId: string | null;
  thinkingBudget: number | null;
  enabledMcpServerIds: string[];
  enabledKnowledgeBaseIds: string[];
  enabledMemoryNamespaceIds: string[];
  enabledWikiIds: string[];
  isPinned: boolean;
  isArchived: boolean;
  contextCompression: boolean;
  categoryId: string | null;
  parentConversationId: string | null;
  mode: "chat" | "agent" | "gateway";
  /** Agent work strategy: "direct" = execute immediately, "plan" = generate plan first, await approval, then execute */
  workStrategy?: "direct" | "plan" | null;
  messageCount: number;
  createdAt: number;
  updatedAt: number;
  scenario?: string | null;
  workspaceDir?: string | null;
  enabledSkillIds: string[];
  /** Agent profile identifier, references AgentProfile.id */
  agentProfileId?: string | null;
  /** Workflow template ID bound to this conversation */
  workflowTemplateId?: string | null;
  /** Bot 归属（null / 缺省 = 未归属；与后端 `conversations.bot_id` 对齐） */
  botId?: string | null;
  /** Session type: "conversation" = free dialog, "workflow" = bound to workflow template */
  sessionType: "conversation" | "workflow";
  /** Workflow execution status: running / completed / failed / cancelled */
  workflowStatus?: string | null;
}

export interface ToolCall {
  id: string;
  type: string;
  function: {
    name: string;
    arguments: string;
  };
}

// ── 无障碍树语义化 DTO（与 harness ui_tree.rs camelCase 对齐）──

/** 语义节点的不透明句柄；rawId 前端不得解析。 */
export interface UiNodeId {
  backend: string;
  rawId: string;
  sessionGen: number;
}

/** 节点屏幕矩形（仅诊断展示，不参与任何语义动作）。 */
export interface UiBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** 节点支持的语义动作。 */
export type UiActionKind =
  | "invoke"
  | "setValue"
  | "focus"
  | "toggle"
  | "select"
  | "expand"
  | "collapse"
  | "scrollIntoView"
  | "scroll";

/** 无障碍树节点。 */
export interface UiNode {
  id: UiNodeId;
  role: string;
  name: string;
  value?: string;
  description?: string;
  states: string[];
  supportedActions: UiActionKind[];
  application: string;
  windowTitle?: string;
  bounds?: UiBounds;
  children: UiNode[];
}

/** 一次查询的树快照（扁平表 + 根 id 列表）。 */
export interface UiTreeSnapshot {
  nodes: UiNode[];
  roots: UiNodeId[];
  capturedAt: number;
}

/** 无障碍树查询条件。 */
export interface UiTreeQuery {
  application?: string;
  windowTitle?: string;
  role?: string;
  nameContains?: string;
  maxDepth?: number;
  includeOffscreen?: boolean;
}

/** 滚动方向（语义动作 Scroll 的参数，无坐标）。 */
export type UiScrollIntent = "up" | "down" | "left" | "right";

/** 纯键盘修饰键（无坐标）。 */
export type UiKeyModifier = "ctrl" | "alt" | "shift" | "meta";

export interface Message {
  id: string;
  conversationId: string;
  role: MessageRole;
  content: string;
  providerId: string | null;
  modelId: string | null;
  tokenCount: number | null;
  promptTokens?: number | null;
  completionTokens?: number | null;
  attachments: Attachment[];
  thinking: string | null;
  toolCallsJson: string | null;
  toolCallId: string | null;
  createdAt: number;
  parentMessageId: string | null;
  versionIndex: number;
  isActive: boolean;
  status: MessageStatus;
  tokensPerSecond?: number | null;
  firstTokenLatencyMs?: number | null;
  /** Structured content blocks (from agent session ContentBlock). */
  blocks?: ContentBlock[];
  /** Additional metadata for extensibility */
  meta?: Record<string, unknown>;
  /** 引用回复：被引用消息的 ID（区别于 parent_message_id） */
  quotedMessageId?: string | null;
  /** 意图澄清状态（仅用户消息有） */
  intentClarification?: IntentClarification | null;
  /** 认知编排决策标签：该消息对应一轮执行的决策信息（ExecutionMode / 路由路径 / 命中工作流 / 专家等） */
  decision?: CognitiveDecisionInfo | null;
  /** P0: 任务形态决策（原则三标尺输出，每条消息显示其分类决策） */
  taskShape?: TaskShapeDecision | null;
}

// ── 认知编排决策标签 ────────────────────────────────
/** 单条消息的认知编排决策信息，由后端 cognitive_query 写入并持久化。 */
export interface CognitiveDecisionInfo {
  /** 执行模式：Workflow / Direct / Delegate / Ask / Plan / Act / ParameterExtract */
  executionMode: string;
  /** 三层路由路径（如 /trade/refund/auto） */
  routePath: string;
  /** 路由置信度 */
  confidence: number;
  /** 命中工作流名称（Workflow 执行模式有值） */
  selectedWorkflowName?: string | null;
  /** 选中的专家/角色画像（Agent 执行模式有值） */
  selectedAgentProfile?: {
    id: string;
    name: string;
    role: string | null;
    expert: string | null;
  } | null;
  /** P1: 任务形态决策（原则三标尺输出，与后端 build_decision_value.taskShape 对齐） */
  taskShape?: TaskShapeDecision | null;
}

// ── Content Block (Part-based message model, short-term) ──────────────
export type ContentBlock =
  | { type: "text"; text: string }
  | { type: "tool_use"; id: string; name: string; input: string }
  | {
    type: "tool_result";
    toolUseId: string;
    toolName: string;
    output: string;
    isError: boolean;
  };

export interface MessagePage {
  messages: Message[];
  hasOlder: boolean;
  oldestMessageId: string | null;
  totalActiveCount: number;
}

export interface ConversationStats {
  totalMessages: number;
  totalUserMessages: number;
  totalAssistantMessages: number;
  totalPromptTokens: number;
  totalCompletionTokens: number;
  totalTokens: number;
  avgTokensPerSecond: number | null;
  avgFirstTokenLatencyMs: number | null;
  avgResponseTimeMs: number | null;
}

export interface DailyUsage {
  date: string;
  messageCount: number;
  totalPromptTokens: number;
  totalCompletionTokens: number;
  totalTokens: number;
  totalCostUsd: number;
}

export interface CostByProvider {
  providerId: string;
  requestCount: number;
  tokenCount: number;
  costUsd: number;
}

export interface Attachment {
  id: string;
  fileType: string;
  fileName: string;
  filePath: string;
  fileSize: number;
  data?: string;
}

export interface AttachmentInput {
  fileName: string;
  fileType: string;
  fileSize: number;
  data: string;
}

export interface ConversationSearchResult {
  conversation: Conversation;
  matchedMessagePreview: string | null;
}

// P2: Cross-session FTS5 search result
export interface SessionSearchResult {
  conversationId: string;
  conversationTitle: string;
  role: string;
  snippet: string;
  rank: number;
}

export interface ConversationSummary {
  id: string;
  conversationId: string;
  summaryText: string;
  compressedUntilMessageId: string | null;
  tokenCount: number | null;
  modelUsed: string | null;
  createdAt: number;
  updatedAt: number;
}

export interface UpdateConversationInput {
  title?: string;
  providerId?: string;
  modelId?: string;
  isPinned?: boolean;
  isArchived?: boolean;
  systemPrompt?: string;
  temperature?: number | null;
  maxTokens?: number | null;
  topP?: number | null;
  searchEnabled?: boolean;
  searchProviderId?: string | null;
  thinkingBudget?: number | null;
  enabledMcpServerIds?: string[];
  enabledKnowledgeBaseIds?: string[];
  enabledMemoryNamespaceIds?: string[];
  enabledWikiIds?: string[];
  contextCompression?: boolean;
  categoryId?: string | null;
  parentConversationId?: string | null;
  mode?: "chat" | "agent" | "gateway";
  workStrategy?: "direct" | "plan" | null;
  scenario?: string | null;
  enabledSkillIds?: string[];
  agentProfileId?: string | null;
  workflowTemplateId?: string | null;
  /** Bot 归属：`null` = 显式解绑，缺键 = 不改 */
  botId?: string | null;
  sessionType?: "conversation" | "workflow";
  workflowStatus?: string | null;
}

// === Gateway System ===
export interface GatewayStatus {
  isRunning: boolean;
  listenAddress: string;
  port: number;
  sslEnabled: boolean;
  startedAt: number | null;
  /** HTTPS listener port; `null` when SSL is disabled or not yet started. */
  httpsPort: number | null;
  /** When `true` the gateway redirects all HTTP traffic to HTTPS. */
  forceSsl: boolean;
}

export interface GatewayKey {
  id: string;
  name: string;
  keyHash: string;
  keyPrefix: string;
  enabled: boolean;
  createdAt: number;
  lastUsedAt: number | null;
  hasEncryptedKey: boolean;
}

export interface CreateGatewayKeyResult {
  gatewayKey: GatewayKey;
  plainKey: string;
}

export interface GatewayMetrics {
  totalRequests: number;
  totalTokens: number;
  totalRequestTokens: number;
  totalResponseTokens: number;
  activeConnections: number;
  todayRequests: number;
  todayTokens: number;
  todayRequestTokens: number;
  todayResponseTokens: number;
  totalCostUsd: number;
  todayCostUsd: number;
}

export interface UsageByKey {
  keyId: string;
  keyName: string;
  requestCount: number;
  tokenCount: number;
  requestTokens: number;
  responseTokens: number;
}

export interface UsageByProvider {
  providerId: string;
  providerName: string;
  requestCount: number;
  tokenCount: number;
  requestTokens: number;
  responseTokens: number;
}

export interface UsageByDay {
  date: string;
  requestCount: number;
  tokenCount: number;
  requestTokens: number;
  responseTokens: number;
}

export interface ConnectedProgram {
  keyId: string;
  keyName: string;
  keyPrefix: string;
  todayRequests: number;
  todayTokens: number;
  todayRequestTokens: number;
  todayResponseTokens: number;
  lastActiveAt: number | null;
  isActive: boolean;
}

export interface GatewayStats {
  totalRequests: number;
  activeConnections: number;
  uptimeSeconds: number;
  requestsPerMinute: number;
}

export interface GatewaySettings {
  listenAddress: string;
  port: number;
  loadBalanceStrategy: "round_robin";
}

// === Settings ===
/** 出站推送渠道的用户配置（判据③）；与后端 harness NotificationChannelConfig 对齐。 */
export interface NotificationChannelConfig {
  id: string;
  enabled: boolean;
  token: string;
  apiUrl: string | null;
  topic: string | null;
}

export interface AppSettings {
  language: string;
  themeMode: string;
  themePreset: string;
  primaryColor: string;
  borderRadius: number;
  autoStart: boolean;
  showOnStart: boolean;
  minimizeToTray: boolean;
  fontSize: number;
  fontWeight: number;
  fontFamily: string;
  codeFontFamily: string;
  bubbleStyle: string;
  codeTheme: string;
  codeThemeLight: string;

  // === 模型选择：使用 NullableModelRef 保证结构一致性 ===
  // 类型系统保证：不可能出现 providerId 存在但 modelId 缺失的状态
  defaultModel: NullableModelRef;
  defaultTemperature: number | null;
  defaultMaxTokens: number | null;
  defaultTopP: number | null;
  defaultContextCount: number | null;

  // 回退模型：主模型失败且未产出内容时自动切换重试
  fallbackModel: NullableModelRef;
  // 图片模型：图像理解 / 图片生成默认模型
  imageModel: NullableModelRef;

  titleSummaryModel: NullableModelRef;
  titleSummaryTemperature: number | null;
  titleSummaryMaxTokens: number | null;
  titleSummaryTopP: number | null;
  titleSummaryContextCount: number | null;
  titleSummaryPrompt: string | null;

  compressionModel: NullableModelRef;
  compressionTemperature: number | null;
  compressionMaxTokens: number | null;
  compressionTopP: number | null;
  compressionPrompt: string | null;

  proxyType: string | null;
  proxyAddress: string | null;
  proxyPort: number | null;
  globalShortcut: string;
  shortcutToggleCurrentWindow: string;
  shortcutToggleAllWindows: string;
  shortcutCloseWindow: string;
  shortcutNewConversation: string;
  shortcutOpenSettings: string;
  shortcutToggleModelSelector: string;
  shortcutFillLastMessage: string;
  shortcutClearContext: string;
  shortcutClearConversationMessages: string;
  shortcutToggleGateway: string;
  shortcutToggleMode: string;
  shortcutShowQuickBar: string;
  gatewayAutoStart: boolean;
  gatewayListenAddress: string;
  gatewayPort: number;
  gatewaySslEnabled: boolean;
  gatewaySslMode: string;
  gatewaySslCertPath: string | null;
  gatewaySslKeyPath: string | null;
  gatewaySslPort: number;
  gatewayForceSsl: boolean;
  // Desktop integration
  alwaysOnTop?: boolean;
  trayEnabled?: boolean;
  globalShortcutsEnabled?: boolean;
  shortcutRegistrationLogsEnabled?: boolean;
  shortcutTriggerToastEnabled?: boolean;
  notificationsEnabled?: boolean;
  startMinimized?: boolean;
  closeToTray?: boolean;
  notifyBackup?: boolean;
  notifyImport?: boolean;
  notifyErrors?: boolean;
  /** 出站推送渠道配置（判据③）；缺省 = 不装配任何出站渠道。 */
  notificationChannels?: NotificationChannelConfig[];
  /** 主行情源（判据③）；null/缺省 = 用默认顺序。 */
  marketDataPrimaryVendor?: string | null;
  // Auto-backup settings
  backupDir?: string | null;
  autoBackupEnabled?: boolean;
  autoBackupIntervalHours?: number;
  autoBackupMaxCount?: number;
  // WebDAV sync settings
  webdavHost?: string | null;
  webdavUsername?: string | null;
  webdavPath?: string | null;
  webdavAcceptInvalidCerts?: boolean;
  webdavSyncEnabled?: boolean;
  webdavSyncIntervalMinutes?: number;
  webdavMaxRemoteBackups?: number;
  webdavIncludeDocuments?: boolean;
  // S3 sync settings
  s3Endpoint?: string | null;
  s3Region?: string | null;
  s3Bucket?: string | null;
  s3AccessKeyId?: string | null;
  s3Root?: string | null;
  s3UsePathStyle?: boolean;
  /** Closed-loop nudge scheduler enabled */
  closedLoopEnabled?: boolean;
  /** Closed-loop nudge interval in minutes (default 5) */
  closedLoopIntervalMinutes?: number;
  lastSelectedConversationId?: string | null;
  /** Custom documents root override (overrides ~/Documents/axagent/) */
  documentsRootOverride?: string | null;
  /** Auto update check interval in minutes (default 60, min 1) */
  updateCheckInterval?: number;
  /** Global system prompt fallback — used when a conversation has no custom system prompt */
  defaultSystemPrompt?: string | null;
  /** Chat minimap / navigation overlay */
  chatMinimapEnabled?: boolean;
  chatMinimapStyle?: "faq" | "sticky";
  /** Agent execution panel — show right-side panel during agent mode */
  agentPanelEnabled?: boolean;
  /** Agent execution panel — use compact (simplified) view by default */
  agentPanelCompact?: boolean;
  /** Onboarding — welcome wizard completed */
  onboardingCompleted?: boolean;
  /** Onboarding — wizard dismissed (user skipped) */
  onboardingWizardDismissed?: boolean;
  /** Onboarding — interactive tutorial completed */
  onboardingTutorialCompleted?: boolean;
  /** Onboarding — selected quick-start preset */
  onboardingSelectedPreset?: string | null;
  /** Multi-model response display mode */
  multiModelDisplayMode?: "tabs" | "side-by-side" | "stacked";
  /** Render user messages as Markdown (like AI messages). Default: false */
  renderUserMarkdown?: boolean;
  /** Default workspace directory for new sessions when not manually set */
  defaultWorkspaceDir?: string | null;
  /** Enable screen perception and vision-based UI control */
  uiAutomationEnabled?: boolean;
  /** Guardian review gate: an LLM pre-screening pass before asking the user to approve sensitive commands */
  guardianReviewEnabled?: boolean;
  /** Enable RL optimizer for tool selection and task strategies */
  rlOptimizerEnabled?: boolean;
  /** Enable LoRA fine-tuning for custom model adaptation */
  loraFinetuneEnabled?: boolean;
  /** Enable proactive nudge suggestions based on context */
  proactiveNudgeEnabled?: boolean;
  /** Enable thought chain visualization for reasoning */
  thoughtChainEnabled?: boolean;
  /** Enable automatic error recovery suggestions */
  errorRecoveryEnabled?: boolean;
  /** Enable Tree of Thoughts multi-path reasoning (expensive) */
  totEnabled?: boolean;
  /**
   * OS 级沙箱模式 — "read-only" | "workspace-write" | "danger-full-access"。
   * 对应后端 SandboxMode（kebab-case）；默认 "danger-full-access" 不启用受限子进程。
   */
  sandboxMode?: "read-only" | "workspace-write" | "danger-full-access";
  /**
   * 审批策略 — "untrusted" | "on-failure" | "on-request" | "never"。
   * 对应后端 ApprovalPolicy（kebab-case）；默认 "on-request"。
   */
  approvalPolicy?: "untrusted" | "on-failure" | "on-request" | "never";
  /** Show the developer tools section (Trace/Benchmark/Fine-Tune/RL) in the sidebar */
  showDeveloperTools?: boolean;
  /** Cloud workspace URI (supports s3://, webdav://, local://) */
  workspaceUri?: string | null;
  /** Cloud backend type: "s3" | "webdav" | null */
  cloudBackend?: string | null;
  /** S3 provider preset key (e.g., "Aws", "TencentCos", "Custom") */
  s3ProviderPreset?: string | null;
  /** S3 secret access key */
  s3SecretAccessKey?: string | null;
  /** WebDAV password */
  webdavPassword?: string | null;
  /** Cloud sync enabled flag */
  cloudSyncEnabled?: boolean;
  /** RAG 高级管线配置（查询增强、重排序、自省式质检） */
  ragPipelineConfig?: RAGPipelineConfig;
  /**
   * 2.7 P1:遥测级别三级开关 — "off" | "minimal" | "full"。
   *
   * - `off`:完全关闭遥测(默认)
   * - `minimal`:仅记录用户行为级事件(Analytics / SessionTrace)
   * - `full`:记录所有遥测事件(含 HTTP 请求细节)
   *
   * 后端 `FilteringSink` 装饰器在运行时通过共享 `Arc<RwLock<TelemetryLevel>>`
   * 引用此设置,`save_settings` 命令保存后立即生效。
   */
  telemetryLevel?: "off" | "minimal" | "full";
  /** Smart Router 智能路由总开关。开启后按任务复杂度自动选择模型 tier。 */
  smartRouterEnabled?: boolean;
  /** tier(budget/balanced/premium) → provider/model 映射表。 */
  smartRouterTierMappings?: Record<string, SmartRouterTierMapping>;
  /** Auto-load downloaded GGUF models into memory when RAG pipeline is active. */
  autoLoadModels?: boolean;
  /** P2-8: ACP (Agent Client Protocol) 服务端 base URL。null 时使用默认值。 */
  acpBaseUrl?: string | null;
}

/** Smart Router tier → provider/model 映射项（对应后端 harness TierModelMapping）。 */
export interface SmartRouterTierMapping {
  /** 目标模型 ID（如 "gpt-4o-mini"） */
  modelId?: string;
  /** 目标 provider ID（如 "openai"） */
  providerId?: string;
  /** 可选的 base URL 覆盖（自建端点 / 代理） */
  baseUrlOverride?: string | null;
}

// === 审批规则（PLAN-codex-parity R2-1）===

/** 规则裁决 — 与后端 `RuleDecision` 的 camelCase 序列化一致。 */
export type ApprovalRuleDecision = "allow" | "prompt" | "forbidden";

/**
 * 已沉淀的审批规则（对应后端 `ApprovalRule`）。
 *
 * 由 Bash 工具在用户批准后自动写入（`program` + `argsPrefix` 为匹配键），
 * 本类型仅用于设置页展示与撤销。
 */
export interface ApprovalRule {
  /** 程序名（小写归一化） */
  program: string;
  /** 参数前缀；空数组表示 program 级规则 */
  argsPrefix: string[];
  /** 规则裁决 */
  decision: ApprovalRuleDecision;
  /** 沉淀来源（会话 id 等，用于追溯） */
  source: string;
}

// === Streaming ===
export interface ChatStreamChunk {
  content: string | null;
  thinking: string | null;
  toolCalls: ToolCall[] | null;
  done: boolean;
  isFinal?: boolean | null;
  usage: TokenUsage | null;
}

export interface ChatStreamEvent {
  conversationId: string;
  messageId: string;
  modelId?: string;
  providerId?: string;
  chunk: ChatStreamChunk;
}

export interface ChatStreamErrorEvent {
  conversationId: string;
  messageId: string;
  error: string;
}

export interface TokenUsage {
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
}

// === Voice ===
export type VoiceSessionState =
  | "Idle"
  | "Connecting"
  | "Connected"
  | "Speaking"
  | "Listening"
  | "Disconnecting"
  | "Error";

// === Intent Clarification ===
export type IntentState =
  | "draft"
  | "clarifying"
  | "needs_confirmation"
  | "submitted"
  | "cancelled";

export interface IntentClarification {
  state: IntentState;
  /** 用户原始输入（语音/文本） */
  originalInput: string;
  /** AI 理解的意图描述 */
  intentSummary?: string;
  /** 澄清问题列表 */
  clarificationQuestions: string[];
  /** 用户对澄清问题的回答 */
  clarificationAnswers: Record<string, string>;
  /** 确认候选方案 */
  confirmationOptions?: string[];
  /** 最终确认的意图 */
  confirmedIntent?: string;
  /** 关联的 DAG 执行 ID */
  workflowExecutionId?: string;
  /** 创建时间戳 */
  createdAt: number;
  /** 更新时间戳 */
  updatedAt: number;
}

export type AudioEncoding = "Pcm16" | "Opus";

export interface AudioFormat {
  sampleRate: number;
  channels: number;
  encoding: AudioEncoding;
}

export interface RealtimeConfig {
  modelId: string;
  voice: string | null;
  audioFormat: AudioFormat;
  sttProviderId?: string | null;
  ttsProviderId?: string | null;
}

// === Gateway Link (Client-side Gateway Connection) ===
export type GatewayLinkType = "openclaw" | "hermes" | "custom";
export type GatewayLinkStatus =
  | "connected"
  | "disconnected"
  | "connecting"
  | "error";

export interface GatewayLink {
  id: string;
  name: string;
  linkType: GatewayLinkType;
  endpoint: string;
  apiKeyId: string | null;
  enabled: boolean;
  status: GatewayLinkStatus;
  errorMessage: string | null;
  autoSyncModels: boolean;
  autoSyncSkills: boolean;
  lastSyncAt: number | null;
  latencyMs: number | null;
  version: string | null;
  createdAt: number;
  updatedAt: number;
}

export interface GatewayLinkModelSync {
  modelId: string;
  providerName: string;
  syncStatus: "synced" | "pending" | "failed" | "not_selected";
  lastSyncAt: number | null;
}

export interface GatewayLinkSkillSync {
  skillName: string;
  skillVersion: string | null;
  syncStatus: "synced" | "pending" | "failed" | "not_selected";
  lastSyncAt: number | null;
}

export interface GatewayLinkPolicy {
  id: string;
  linkId: string;
  routeStrategy: "round_robin" | "least_latency" | "weighted";
  modelFallbackEnabled: boolean;
  globalRpm: number | null;
  perModelRpm: number | null;
  tokenLimitPerMinute: number | null;
  keyRotationStrategy: "sequential" | "random";
  keyFailoverEnabled: boolean;
}

export interface CreateGatewayLinkInput {
  name: string;
  linkType: GatewayLinkType;
  endpoint: string;
  apiKeyId?: string | null;
  apiKey?: string | null;
  autoSyncModels?: boolean;
  autoSyncSkills?: boolean;
}

export interface GatewayLinkActivity {
  id: string;
  linkId: string;
  activityType: string;
  description: string | null;
  createdAt: number;
}

// === UI State ===
export type BuiltinPageKey =
  // ── 能力域聚合入口（对应 domainMeta.ts 的 8 个业务域） ──
  | "general"
  | "finance"
  | "automation"
  | "devops"
  | "data_analysis"
  | "content_creation"
  | "ai_media"
  | "communication"
  // ── 通用功能 ──
  | "chat"
  | "dashboard"
  | "knowledge"
  | "memory"
  | "demand-discovery"
  | "link"
  | "gateway"
  | "files"
  | "terminal"
  | "workflow"
  | "dynamic-ui"
  | "settings"
  | "tasks-center"
  | "marketplace"
  | "wiki"
  | "multi-agent"
  // ── 金融域导航项 ──
  | "finance-investment"
  | "finance-analysis"
  | "finance-accounting"
  // ── 自动化域导航项 ──
  | "automation-operations"
  | "automation-sales"
  | "automation-projects"
  | "automation-consulting"
  | "automation-ecommerce"
  // ── 运维域导航项 ──
  | "devops-software"
  | "devops-security"
  // ── 数据分析域导航项 ──
  | "data-geospatial"
  | "data-ai-research"
  // ── 内容创作域导航项 ──
  | "content-media"
  | "content-design"
  | "content-education"
  // ── AI 媒体域导航项 ──
  | "ai-media-game"
  // ── 旧 key（保留作兼容重定向） ──
  | "invest"
  | "workspace"
  | "stock-analysis"
  | "screener"
  | "watchlist"
  | "portfolio"
  | "paper-portfolio"
  | "screenshot-diagnosis"
  | "trade"
  | "backtest"
  | "compare"
  | "quant"
  | "replay-workbench"
  | "cross-market"
  | "opc"
  | "multi-agent";
export type PageKey = BuiltinPageKey | string;
export type SettingsSection =
  | "providers"
  | "defaultModel"
  | "conversationSettings"
  | "general"
  | "display"
  | "proxy"
  | "shortcuts"
  | "data"
  | "database"
  | "storage"
  | "scheduler"
  | "backup"
  | "about"
  | "searchProviders"
  | "localTools"
  | "mcpServers"
  | "tools"
  | "userProfile"
  | "skillsHub"
  | "dashboardPlugins"
  | "webhooks"
  | "messageChannels"
  | "advanced"
  | "promptTemplates"
  | "acp"
  | "evolution"
  | "persona"
  | "proactiveBehavior"
  | "cloudWorkspace"
  | "plugins"
  | "notificationCenter"
  | "imageGen"
  | "theme"
  | "animations"
  | "cron"
  | "dynamicPages"
  | "readingList"
  | "paperOverview"
  | "knowledgeGraph"
  | "gateway"
  | string;

// === Generated Tool ===
export interface GeneratedToolInfo {
  id: string;
  toolName: string;
  originalName: string;
  originalDescription: string;
  createdAt: number;
}

// === Domain Learning Config ===

/** 行业学习配置视图（从后端 YAML 解析而来） */
export interface DomainLearningConfig {
  version: number;
  capabilityPackId: string;
  capabilityPackName: string;
  reflectionEnabled: boolean;
  evolutionEnabled: boolean;
  codeEvolverEnabled: boolean;
  selfImprovementEnabled: boolean;
  reinforcementLearningEnabled: boolean;
  configPath: string;
}

/** 行业学习配置列表项（用于列表展示） */
export interface DomainLearningConfigSummary {
  version: number;
  capabilityPackId: string;
  capabilityPackName: string;
  reflectionEnabled: boolean;
  evolutionEnabled: boolean;
  codeEvolverEnabled: boolean;
  selfImprovementEnabled: boolean;
  reinforcementLearningEnabled: boolean;
  configPath: string;
}

/** 反思请求参数 */
export interface ReflectOnWorkflowParams {
  capabilityPackId: string;
  workflowId: string;
  workflowResult: Record<string, unknown>;
}

/** 进化请求参数 */
export interface EvolveWorkflowParams {
  capabilityPackId: string;
  workflowId: string;
  reason: string;
}

/** 自我改进请求参数 */
export interface RunSelfImprovementParams {
  capabilityPackId: string;
  target: string;
}

// === Reinforcement Learning ===

/** RL 经验记录 — 单次工作流执行的经验数据 */
export interface RLExperience {
  id: string;
  capabilityPackId: string;
  workflowId: string;
  timestampMs: number;
  qualityScore: number;
  efficiencyScore: number;
  costScore: number;
  innovationScore: number;
  satisfactionScore: number;
  totalReward: number;
  stepCount: number;
  success: boolean;
  metadata: Record<string, unknown>;
}

/** RL 策略优化结果 */
export interface RLPolicyUpdate {
  capabilityPackId: string;
  experiencesUsed: number;
  avgReward: number;
  rewardTrend: "improving" | "declining" | "stable" | string;
  suggestedAdjustments: string[];
  qualityWeightsOptimized?: Array<[string, number]>;
  reflectionThreshold?: number;
  evolutionTriggerAdjusted?: boolean;
}

/** RL 经验池统计 */
export interface ExperiencePoolStats {
  totalExperiences: number;
  capabilityPackCount: number;
  oldestTimestampMs?: number;
  newestTimestampMs?: number;
  avgReward: number;
  successRate: number;
}

/** RL 奖励权重配置 */
export interface RewardWeightConfig {
  quality: number;
  efficiency: number;
  cost: number;
  innovation: number;
  satisfaction: number;
}

/** 强化学习配置 */
export interface ReinforcementLearningConfig {
  enabled: boolean;
  reward_model?: string;
  auto_train_threshold: number;
  learning_rate: number;
  gamma: number;
  epsilon: number;
  reward_weights: RewardWeightConfig;
  optimization_goals: string[];
}

/** RL 经验记录请求参数 */
export interface RecordRLExperienceParams {
  capabilityPackId: string;
  workflowId: string;
  qualityScore: number;
  workflowResult: Record<string, unknown>;
}

/** RL 策略优化请求参数 */
export interface TriggerRLOptimizationParams {
  capabilityPackId: string;
}

/** 自动学习闭环触发结果 */
export interface AutoLearningResult {
  reflection: {
    status: "success" | "skipped" | "failed";
    qualityScore?: number;
    message?: string;
  };
  evolution?: {
    status: "success" | "skipped" | "failed";
    reason?: string;
    message?: string;
  };
  selfImprovement?: {
    status: "success" | "skipped" | "failed";
    target?: string;
    message?: string;
  };
  reinforcementLearning?: {
    status: "success" | "skipped" | "failed";
    experienceRecorded?: boolean;
    poolSize?: number;
    policyOptimized?: boolean;
    message?: string;
  };
  triggeredAt: number;
}

// === Files Module ===
export type FileCategory = "images" | "files" | "backups";

export type FileSortKey = "createdAt" | "size" | "name";

export interface FileRow {
  id: string;
  name: string;
  path: string;
  storagePath?: string;
  size?: number;
  createdAt?: string;
  category?: FileCategory;
  hasThumbnail?: boolean;
  previewUrl?: string;
  missing?: boolean;
}

export interface FilesPageEntry {
  id: string;
  sourceKind: string;
  category: FileCategory;
  displayName: string;
  path: string;
  storagePath?: string | null;
  sizeBytes: number;
  createdAt: string;
  missing: boolean;
  previewUrl?: string | null;
}

// ── Skills ─────────────────────────────────────────────────────────────
export interface Skill {
  name: string;
  description: string;
  author?: string;
  version?: string;
  source: "builtin" | "axagent" | "claude" | "agents" | "project";
  sourcePath: string;
  enabled: boolean;
  hasUpdate: boolean;
  userInvocable: boolean;
  argumentHint?: string;
  whenToUse?: string;
  group?: string;
  manifest?: SkillManifest;
}

export interface SkillDetail {
  info: Skill;
  content: string;
  files: string[];
  manifest?: SkillInstallMeta;
}

export interface SkillInstallMeta {
  sourceKind: string;
  sourceRef?: string;
  branch?: string;
  commit?: string;
  installedAt: string;
  installedVia?: string;
}

/** 技能执行统计（后端 get_skill_execution_stats，源自 trajectory_skills 表聚合字段） */
export interface SkillExecutionStat {
  name: string;
  successRate: number;
  avgExecutionTimeMs: number;
  totalUsages: number;
  successfulUsages: number;
  /** 后端暂无数据源，恒为 null，前端按 0.5 兜底 */
  qualityScore: number | null;
  /** trajectory_skills 表无此列，恒为 null */
  lastUsedAt?: string | null;
}

export interface MarketplaceSkill {
  name: string;
  description: string;
  repo: string;
  stars: number;
  installs: number;
  installed: boolean;
  hasUpdate?: boolean;
  currentVersion?: string;
  latestVersion?: string;
  categories?: string[];
  tags?: string[];
}

export interface SkillUpdateInfo {
  name: string;
  currentCommit: string;
  latestCommit: string;
  sourceRef: string;
  currentVersion?: string;
  latestVersion?: string;
}

export interface SkillProposal {
  taskDescription: string;
  suggestedName: string;
  suggestedContent: string;
  confidence: number;
  triggerEvent: string;
  similarSkills: string[];
}

export interface SkillCreateCheckResult {
  hasSimilar: boolean;
  similarSkills: SkillSimilarInfo[];
  canCreate: boolean;
  message: string;
}

export interface SkillSimilarInfo {
  id: string;
  name: string;
  description: string;
  version: string;
  scenarios: string[];
  successRate: number;
  similarityScore: number;
}

// ── Learning Graph Types ──

export type NodeKind = "skill" | "memory" | "insight" | "entity";

export interface GraphNode {
  id: string;
  label: string;
  kind: NodeKind;
  category: string;
  timestampMs: number;
  useCount: number;
  state: string;
  detail: string | null;
}

export interface GraphEdge {
  source: string;
  target: string;
  weight: number;
  relation: string;
}

export interface CategoryCount {
  category: string;
  count: number;
}

export interface GraphStats {
  totalSkills: number;
  totalMemories: number;
  totalInsights: number;
  totalEntities: number;
  totalEdges: number;
  linkedNodes: number;
  categories: CategoryCount[];
}

export interface LearningGraph {
  nodes: GraphNode[];
  edges: GraphEdge[];
  stats: GraphStats;
}

// ── Skill Manifest ──

/** Skill 清单定义（skill-manifest.json 解析后的统一格式） */
export interface SkillManifest {
  name: string;
  version: string;
  description: string;
  author?: string;
  icon?: string;
  dependencies?: Record<string, string>;
  /** 能力声明列表 */
  capabilities?: SkillCapability[];
  /** 权限白名单 */
  permissions?: SkillPermissions;
  /** 生命周期钩子 */
  lifecycle?: SkillLifecycleHooks;
}

/** 权限白名单 —— 权威定义在技能协议层 `src/sdk/types.ts`（零 import、被 `sdk/sandboxTemplate.ts`
 *  内联，故 sdk 不能反向依赖本文件）。此处仅转出，应用代码照旧 `from "@/types"` 导入。 */
export type { SkillPermissions };

/** 生命周期钩子 */
export interface SkillLifecycleHooks {
  onInstall?: SkillCommandAction[];
  onEnable?: SkillCommandAction[];
  onDisable?: SkillCommandAction[];
  onUninstall?: SkillCommandAction[];
}

// ── Skill Handler ──

export interface SkillHandler {
  mode: "declarative" | "agentic";
  description?: string;
  actions?: SkillCommandAction[];
  promptTemplate?: string;
  contextGatherer?: SkillContextGatherer;
  resultHandler?: SkillResultHandler;
}

// ── Navigation（旧 UI 类型已移除，统一使用 SkillManifest.capabilities） ──

export type NavPosition = "Top" | "Bottom";

// ── SkillCommandAction (声明式 + Agentic) ──

export type SkillCommandAction = DeclarativeAction | AgenticAction;

export interface DeclarativeAction {
  mode: "declarative";
  action: DeclarativeActionType;
}

export type DeclarativeActionType =
  | { type: "invoke"; command: string; args?: Record<string, unknown> }
  | { type: "navigate"; path: string }
  | { type: "emit"; event: string; payload?: unknown }
  | {
    type: "store";
    operation: "get" | "set" | "update";
    storeName: string;
    payload?: unknown;
  }
  | { type: "function"; name: string; args?: unknown[] }
  | { type: "handler"; name: string; args?: Record<string, unknown> }
  | { type: "chain"; actions: DeclarativeActionType[] }
  | { type: "update-schema"; schemaId: string; operation: string; path?: string; newSchema?: unknown };

export interface AgenticAction {
  mode: "agentic";
  prompt: string;
  skillName?: string;
  context?: SkillContextGatherer;
  resultHandler?: SkillResultHandler;
}

export interface SkillResultHandler {
  type: "store" | "emit" | "navigate" | "stream";
  target: string;
}

export interface SkillContextGatherer {
  includeConversation?: boolean;
  includeFiles?: boolean;
  includeSelection?: boolean;
}

export type SkillComponentType = "Sandbox" | "Markdown";

// ── Panels ──

export type UIPanelPosition = "Main" | "Sidebar" | "Header" | "Footer";

export type UIPanelSize = "Small" | "Medium" | "Large" | "FullWidth";

// ── Chat Command Args ──

// ── Skill Capability 类型（Capability-based Manifest） ──────────────

export type SkillCapability =
  | SkillPageCapability
  | SkillPanelCapability
  | SkillToolbarCapability
  | SkillChatCommandCapability
  | SkillStatusBarCapability
  | SkillNavigationCapability
  | SkillSettingsCapability;

export interface SkillPageCapability {
  type: "page";
  id: string;
  title: string;
  componentType: "Sandbox" | "Markdown";
  componentConfig: {
    entry: string;
    props?: Record<string, unknown>;
    layout?: "default" | "fullscreen" | "sidebar";
  };
  icon?: string;
}

export interface SkillPanelCapability {
  type: "panel";
  id: string;
  title: string;
  componentType: "Sandbox" | "Markdown";
  componentConfig: {
    entry: string;
    props?: Record<string, unknown>;
  };
  position: "Main" | "Sidebar" | "Header" | "Footer";
  size?: "Small" | "Medium" | "Large" | "FullWidth";
  collapsible?: boolean;
  defaultCollapsed?: boolean;
}

export interface SkillToolbarCapability {
  type: "toolbar";
  id: string;
  title?: string;
  icon: string;
  tooltip?: string;
  position: "left" | "right";
  priority?: number;
  onClick: SkillCommandAction[];
  menu?: { label: string; actions: SkillCommandAction[] }[];
}

export interface SkillChatCommandCapability {
  type: "chatCommand";
  id: string;
  title: string;
  commandName: string;
  description: string;
  icon?: string;
  mode: "declarative" | "agentic";
  actions?: SkillCommandAction[];
}

export interface SkillStatusBarCapability {
  type: "statusBar";
  id: string;
  title: string;
  alignment: "left" | "right";
  priority?: number;
  text?: string;
  icon?: string;
  dynamicText?: {
    command: string;
    args?: Record<string, unknown>;
    refreshIntervalMs: number;
    template?: string;
  };
  onClick?: SkillCommandAction[];
}

export interface SkillNavigationCapability {
  type: "navigation";
  id: string;
  title: string;
  icon: string;
  pageId: string;
  position?: number;
}

export interface SkillSettingsCapability {
  type: "settings";
  id: string;
  title: string;
  icon?: string;
  settingsGroup: string;
  componentType: "Sandbox" | "Markdown";
  componentConfig: {
    entry: string;
    props?: Record<string, unknown>;
  };
}

// Phase-2 type modules
export * from "./agent";
export * from "./agentProfile";
export * from "./agentRole";
export * from "./approval";
export * from "./artifact";
export * from "./backup";
export * from "./citation";
export * from "./evaluator";
export * from "./evolution";
export * from "./expert";
export * from "./knowledge";
export * from "./llmWiki";
export * from "./localTool";
export * from "./masterKey";
export * from "./mcp";
export * from "./memory";
export * from "./multi-agent";
export * from "./nudge";
// Fleet（多办公室 AI 团队）— 与后端 axagent_harness::fleet 一一对应
export * from "./office";
export * from "./paper";
export * from "./permission";
export * from "./persona";

// === Security Module Types ===
export type { InjectionDetection, InjectionType, SanitizationResult } from "@/lib/security/injectionDetector";

export type {
  PermissionDecision,
  PermissionLevel,
  PermissionPolicy,
  PermissionRequest,
  ResourceType,
} from "@/lib/security/permissionGuard";

export type { OutputRiskLevel, SanitizedOutput } from "@/lib/security/outputSanitizer";
export * from "./capability";
export * from "./narrative";
export * from "./platform";
export * from "./proactive";
export * from "./scheduler";
export * from "./search";
export * from "./style";
export * from "./taskShape";
export * from "./tracer";
export * from "./wiki";
export * from "./workflow";

// ── Workspace / Context Types (merged from workspace.ts) ───────────────
export type ContextSourceType =
  | "attachment"
  | "search"
  | "knowledge"
  | "memory"
  | "wiki"
  | "tool";

export type ContextSource = {
  id: string;
  conversationId: string;
  messageId?: string;
  type: ContextSourceType;
  refId: string;
  title: string;
  enabled: boolean;
  summary?: string;
  /** 多文档协同：限制 RAG 检索范围到这些文档 ID；空数组表示不限制 */
  docIds?: string[];
};

export type ConversationBranch = {
  id: string;
  conversationId: string;
  parentMessageId: string;
  branchLabel: string;
  branchIndex: number;
  comparedMessageIdsJson?: string;
  createdAt: string;
};

export type SearchPolicy = {
  enabled: boolean;
  searchProviderId?: string;
  queryMode: "manual" | "auto";
  resultLimit: number;
};

export type ToolBinding = {
  serverIds: string[];
  defaultTools?: string[];
  approvalMode: "inherit" | "ask" | "allow_safe";
};

export type KnowledgeBinding = {
  knowledgeBaseIds: string[];
  autoAttach: boolean;
};

export type MemoryPolicy = {
  enabled: boolean;
  namespaceId?: string;
  writeBack: boolean;
};

export type ContextToggleState = {
  searchEnabled: boolean;
  searchProviderId?: string;
  enabledKnowledgeBaseIds: string[];
  enabledMcpServerIds: string[];
  enabledWikiIds: string[];
  enabledToolNames?: string[];
  memoryEnabled: boolean;
  memoryNamespaceId?: string;
  memoryWriteBack: boolean;
  disabledContextSourceIds?: string[];
};

export type ConversationWorkspaceSnapshot = {
  searchPolicy: SearchPolicy;
  toolBinding: ToolBinding;
  knowledgeBinding: KnowledgeBinding;
  memoryPolicy: MemoryPolicy;
  toggles: ContextToggleState;
  researchMode: boolean;
  pinnedArtifactIds: string[];
  branches?: ConversationBranch[];
  activeBranchId?: string | null;
};

export type ContextOverrideInput = {
  searchEnabled?: boolean;
  searchProviderId?: string | null;
  enabledKnowledgeBaseIds?: string[];
  enabledMcpServerIds?: string[];
  enabledWikiIds?: string[];
  enabledToolNames?: string[];
  memoryEnabled?: boolean;
  memoryNamespaceId?: string | null;
  memoryWriteBack?: boolean;
  disabledContextSourceIds?: string[];
  researchMode?: boolean;
};

export type CreateConversationInput = {
  title: string;
  providerId: string;
  modelId: string;
  systemPrompt?: string;
  workspaceSnapshot?: ConversationWorkspaceSnapshot;
};

export type WorkspaceUpdateInput = {
  title?: string;
  providerId?: string;
  modelId?: string;
  workspaceSnapshot?: ConversationWorkspaceSnapshot;
  activeBranchId?: string | null;
  activeArtifactId?: string | null;
  researchMode?: boolean;
};

export type SendMessageInput = {
  conversationId: string;
  content: string;
  attachments?: AttachmentInput[];
  contextOverride?: ContextOverrideInput;
};

export type CompareResponsesResult = {
  leftMessage: { id: string; content: string };
  rightMessage: { id: string; content: string };
};

// ── Tool Dependencies ─────────────────────────────────────────────────
export type ToolDependencyStatus =
  | "satisfied"
  | "auto_installable"
  | "manual_installable"
  | "needs_generation";

export interface ToolDependency {
  name: string;
  toolType: string;
  status: ToolDependencyStatus;
  sourceInfo?: string;
  installInstructions?: string;
  configRequirements?: string;
}

// ── Decomposition ─────────────────────────────────────────────────────
export interface DecompositionPreview {
  toolDependencies: ToolDependency[];
  workflowNodes: unknown;
  workflowEdges: unknown;
  originalSource: {
    market: string;
    repo?: string;
    version?: string;
  };
  cacheId: string;
}

// ── Work Engine ───────────────────────────────────────────────────────
export type ExecutionStatus =
  | "running"
  | "paused"
  | "completed"
  | "partially_completed"
  | "failed"
  | "cancelled";

export interface NodeExecutionRecord {
  nodeId: string;
  nodeType: string;
  nodeName: string | null;
  status: string;
  input: unknown;
  output: unknown;
  executionTimeMs: number | null;
  error: string | null;
  startedAt: number;
  completedAt: number | null;
  parentExecutionId: string | null;
  subWorkflowId: string | null;
}

export interface ExecutionStatusResponse {
  executionId: string;
  workflowId: string;
  status: ExecutionStatus;
  currentNodeId: string | null;
  totalTimeMs: number;
  nodeCount: number;
  nodeRecords: NodeExecutionRecord[];
  variables: Record<string, unknown>;
  parentExecutionId: string | null;
}

// `SubWorkflowOrigin`（子执行归属）见文件顶部的生成物 re-export —— 权威定义在
// `axagent-harness::workflow_types`。只有由 SubWorkflow 节点启动的**子执行**发出的事件才带它，
// 顶层执行的事件为 null/缺席（这条语义留在后端 doc 注释里，随生成物一起投影过来）。

/**
 * 一次子执行的实时进度（B-2a）。key = 子执行的 execution_id —— 刻意不用父节点 id 当 key：
 * 重试会就同一父节点启动多个子执行，按子执行 id 分桶才能把每次尝试如实分开，
 * 而不是后一次静默覆盖前一次。
 */
export interface SubWorkflowProgress {
  parentNodeId: string;
  childExecutionId: string;
  /** 子图内节点 id → 状态（running / completed / failed / timeout …） */
  nodeStatuses: Record<string, string>;
}

export interface ExecutionSummary {
  id: string;
  workflowId: string;
  status: string;
  totalTimeMs: number | null;
  /** 失败原因；成功时为 null。此前只进日志，界面上只剩一个 failed 标签 */
  errorDetail: string | null;
  /** Unix 毫秒，与后端 ExecutionSummaryResponse.createdAtMs 对齐 */
  createdAtMs: number;
}

// ── Plan Mode (Agent Work Strategy) ──────────────────────────────────
export type PlanStepStatus =
  | "pending"
  | "approved"
  | "rejected"
  | "running"
  | "completed"
  | "error";

export interface PlanStep {
  id: string;
  title: string;
  description: string;
  status: PlanStepStatus;
  /** Estimated tools that will be used for this step */
  estimatedTools?: string[];
  /** Result summary after completion */
  result?: string | null;
}

export type PlanStatus =
  | "draft"
  | "reviewing"
  | "approved"
  | "executing"
  | "completed"
  | "partial"
  | "cancelled"
  /** DB 出现后端未定义的状态取值时如实返回（修复前被静默报成 "cancelled"） */
  | "unknown";

export interface Plan {
  id: string;
  conversationId: string;
  /** The user message that triggered this plan generation */
  userMessageId: string;
  title: string;
  steps: PlanStep[];
  status: PlanStatus;
  /**
   * 执行授权位（P0-A）：`false`=未授权，`true`=已授权。
   *
   * **这是「能否执行」的唯一判据**，与 `status` 解耦。后端只接受经
   * `plan_authorize` 写入的授权（写入者白名单不含模型路径）。
   * 计划内容被修改、被取消、被恢复时均会自动撤权。
   */
  executionAuthorized: boolean;
  /** 授权时间（毫秒时间戳），未授权为 undefined */
  authorizedAt?: number;
  /** 授权来源（user/system/api/ui/automation），未授权为 undefined */
  authorizedBy?: string;
  isActive: boolean;
  /** The work_strategy that was active when this plan was created, for restoration context */
  createdUnderStrategy?: "direct" | "plan";
  createdAt: number;
  updatedAt: number;
}

export interface PlanGeneratedEvent {
  conversationId: string;
  plan: Plan;
}

export interface PlanStepUpdateEvent {
  conversationId: string;
  planId: string;
  stepId: string;
  status: PlanStepStatus;
  result?: string | null;
}

export interface PlanExecutionCompleteEvent {
  conversationId: string;
  planId: string;
  /** 后端可能回 partial（部分步骤失败）—— 修复前此处类型漏了这个取值 */
  status: "completed" | "partial" | "cancelled";
}

/** 计划执行授权请求（P0-A）—— 授权位的唯一写入口 */
export interface PlanAuthorizeRequest {
  conversationId: string;
  planId: string;
  /** 授权来源，后端按白名单校验：user/system/api/ui/automation（不含模型路径） */
  authorizedBy: string;
  /** true = 批准执行（置授权位）；false = 拒绝（撤权并归档） */
  approved: boolean;
}

/**
 * 执行授权位变更事件（P0-A）。
 *
 * 前端据此刷新计划卡按钮态 —— 修复前前端只能从 `status` 猜「是否已批准」，
 * 而 `approved` 状态在后端从无写入点。
 */
export interface PlanAuthorizationChangedEvent {
  conversationId: string;
  planId: string;
  authorized: boolean;
  authorizedBy?: string;
}

export interface PlanGenerateRequest {
  conversationId: string;
  content: string;
}

export interface PlanExecuteRequest {
  conversationId: string;
  planId: string;
  /** Optional: execute only specific step IDs, otherwise all approved steps */
  stepIds?: string[];
}

export interface PlanModifyStepRequest {
  planId: string;
  stepId: string;
  title?: string;
  description?: string;
  approved?: boolean;
}

export interface PromptTemplate {
  id: string;
  name: string;
  description?: string;
  content: string;
  variablesSchema?: string;
  version: number;
  isActive: boolean;
  abTestEnabled: boolean;
  abTestVariant?: string;
  category?: string;
  tags?: string[];
  author?: string;
  source?: string;
  sourceType?: string;
  format?: string;
  metadataJson?: string;
  usageCount: number;
  isFavorite: boolean;
  createdAt: number;
  updatedAt: number;
}

export interface CreatePromptTemplateInput {
  name: string;
  description?: string;
  content: string;
  variablesSchema?: string;
  category?: string;
  tags?: string[];
  author?: string;
  source?: string;
  sourceType?: string;
  format?: string;
  metadataJson?: string;
}

export interface UpdatePromptTemplateInput {
  name?: string;
  description?: string;
  content?: string;
  variablesSchema?: string;
  isActive?: boolean;
  abTestEnabled?: boolean;
  category?: string;
  tags?: string[];
  author?: string;
  source?: string;
  sourceType?: string;
  format?: string;
  metadataJson?: string;
  isFavorite?: boolean;
}

export interface PromptTemplateVersion {
  id: string;
  templateId: string;
  version: number;
  content: string;
  variablesSchema?: string;
  category?: string;
  tags?: string[];
  author?: string;
  source?: string;
  changelog?: string;
  createdAt: number;
}

export interface ImportPromptTemplateInput {
  name: string;
  description?: string;
  content: string;
  variablesSchema?: string;
  category?: string;
  tags?: string[];
  author?: string;
  source?: string;
  sourceType?: string;
  format?: string;
  metadataJson?: string;
}

export interface ImportPromptResult {
  imported: PromptTemplate[];
  skipped: string[];
  errors: string[];
}

export interface ImportFromUrlInput {
  url: string;
  categoryFilter?: string;
  overwriteExisting?: boolean;
}

export type ExportPromptFormat = "json" | "yaml" | "markdown";

// === Plugin System ===

export interface PluginSummaryDto {
  id: string;
  name: string;
  version: string;
  description: string;
  kind: "builtin" | "bundled" | "external" | "openclaw";
  enabled: boolean;
  tools: string[];
  mcpServers: string[];
  skills: string[];
  /** 安装来源的可读描述；只展示，不参与路径构造或权限判定。 */
  source: string;
  /** manifest 声明的权限名；只展示，判定始终在后端。 */
  permissions: string[];
  /** manifest 声明的页面 id；路径由后端按 (pluginId, pageId) 解析。 */
  pages: string[];
  /** 安装目录完整性：verified / mismatch / unpinned / unknown。 */
  integrityStatus: string;
  /** 安装时固定的期望 SHA-256；本地源或旧安装可能没有。 */
  pinnedHash: string | null;
  /** 插件声明的命名命令（阶段3-① 执行分发的可见面）。 */
  commands: { name: string; description: string }[];
  /**
   * 事件投递计数（B3「丢弃计数」）。
   * `null` = 该插件当前没有挂着的事件订阅；与「全零」不同义（零 = 挂着但一条事件都没来）。
   */
  eventStats: PluginEventStatsDto | null;
}

/**
 * 插件事件投递计数，与 `axagent_plugins::worker::EventDeliveryStats` 对齐。
 * `droppedUnavailable` 是 `droppedInvoke` 的**子集**（脏闸拒单那一类），不参与 `droppedTotal` 求和。
 */
export interface PluginEventStatsDto {
  delivered: number;
  droppedTotal: number;
  droppedSerialize: number;
  droppedInvoke: number;
  droppedUnavailable: number;
  droppedWait: number;
}

export interface PluginManifestDto {
  name: string;
  version: string;
  description: string;
  permissions: string[];
  /** 插件声明的页面 id；路径由后端按 (pluginId, pageId) 解析。 */
  pages: string[];
  defaultEnabled: boolean;
  hooks: Record<string, string[]>;
  tools: { name: string; description: string }[];
  mcpServers: { name: string; command: string }[];
  skills: { name: string; path: string }[];
  capabilities: {
    seam: string;
    capabilityType: string;
    version: string;
    description: string;
  }[];
}

/** `plugin_validate_source` 的返回值：后端签发的预览 token，前端只原样回传。 */
export interface PluginSourceValidationDto extends PluginManifestDto {
  previewToken: string;
}

export interface InstallOutcomeDto {
  pluginId: string;
  version: string;
  installPath: string;
}

/**
 * 仪表盘面板落位区域 —— 后端 `plugins::types::PluginDashboardPanelPosition`，
 * serde `rename_all = "lowercase"`。
 */
export type PluginDashboardPanelPositionDto = "main" | "sidebar" | "header" | "footer";

/** 仪表盘面板尺寸 —— 后端 `PluginDashboardPanelSize`，注意 `fullWidth` 是唯一非全小写值 */
export type PluginDashboardPanelSizeDto = "small" | "medium" | "large" | "fullWidth";

/**
 * 插件贡献的仪表盘面板声明（面板声明的唯一权威结构）。
 *
 * 命名面有一处不对称，勿望文生义：`PluginManifest` 结构体**没有** `rename_all`
 * （只逐字段 `#[serde(rename = "defaultEnabled")]`），故 manifest 上的键是 snake_case
 * `dashboard_panels`；而其列表投影 `DashboardPluginInfo` 带 `rename_all = "camelCase"`，
 * 面板字段是 camelCase。本类型对齐的是**投影**形状。
 *
 * 后端对带 `#[serde(default)]` 的字段无 `skip_serializing_if` ⇒ **恒被序列化**：
 * `props` 空时为 `{}`、`frontendEntry` 缺时为 `null`，所以这里建模为非可选 / 显式可空，而非 `?:`。
 */
export interface PluginDashboardPanelDto {
  id: string;
  title: string;
  /** 渲染期由 ComponentRegistry 按此名解析（P4 插件贡献点）；当前无消费方。 */
  componentName: string;
  position: PluginDashboardPanelPositionDto;
  size: PluginDashboardPanelSizeDto;
  props: Record<string, unknown>;
  frontendEntry: string | null;
}

/** `dashboard_list_plugins` 的返回形状（PluginManager 清单的只读投影）。 */
export interface DashboardPluginInfoDto {
  id: string;
  name: string;
  version: string;
  description: string;
  author: string | null;
  enabled: boolean;
  panels: PluginDashboardPanelDto[];
}

/**
 * 逐页声明的装载形态 —— 后端 `plugins::types::PluginPageLoading`，serde `rename_all = "lowercase"`。
 *
 * 两条通道各自的承接点（`PLAN-shell-plus-plugins.md` §0.12.5 裁定 3：一页只许走一条，
 * **未声明即清单校验错误**，插件装载不出来）：
 * - `bundle`：插件自带的前端产物。`entry` 是相对插件安装目录的资产路径，
 *   由后端解析成绝对路径后经 asset 协议装载（允许根 = `tauri.conf.json` 的 `assetProtocol.scope`）。
 * - `schema`：页面内容是一张动态 UI Schema。`entry` 是 **schema id**（不是文件路径），
 *   走 `DynamicPageViewer` + `dynamicUIStore`（schema 由插件以 `origin="plugin"` 注入）。
 */
export type PluginPageLoadingDto = "bundle" | "schema";

/**
 * 插件贡献的页面声明（`manifest.pages[]`，页面声明的唯一权威结构）。
 *
 * 命名口径与 [`PluginDashboardPanelDto`] 同族：后端 Rust 字段 snake_case +
 * `#[serde(rename_all = "camelCase")]` ⇒ 本类型 camelCase（禁区 13）。
 *
 * ⚠ 寻址不变量：前端只拿 `(pluginId, pageId)` 向后端要路径
 * （`plugin_resolve_page_asset`），**不得**用 `entry` 自行拼接安装目录 ——
 * 与 `PluginPanelHost.tsx` 里面板粒度那条是同一条不变量。
 */
export interface PluginPageDto {
  id: string;
  title: string;
  /** `bundle` ⇒ 安装目录内的相对资产路径；`schema` ⇒ 动态 UI Schema 的 id。 */
  entry: string;
  loading: PluginPageLoadingDto;
  /** 宿主侧栏归属组 key（对齐 `NAV_GROUPS[].key`）；缺省由前端兜底。 */
  navGroup?: string | null;
  /** 宿主侧栏显示名 i18n key；有值时前端 `t(labelKey)`，否则回退 `title`。 */
  labelKey?: string | null;
  /** lucide 图标标识（`TrendingUp` 或 `lucide:TrendingUp`）；缺省前端用 Puzzle。 */
  icon?: string | null;
  /** 组内排序（升序），缺省 0。 */
  navOrder?: number;
  /** 隐藏侧栏/命令面板入口（路由仍注册，可直达）。 */
  hidden?: boolean;
}

/**
 * `plugin_resolve_page_asset` 的返回形状（后端 `plugins::types::PluginPageAssetLocation`）。
 *
 * `absolutePath` 由后端 canonicalize 并校验「在安装目录内 + 在安装根内」后给出，
 * Windows 侧已去掉 `\\?\` verbatim 前缀，可直接交给 `convertFileSrc` 换 asset URL。
 * `entry` 只是原样回带的清单值，供展示与排错，**不参与**路径构造。
 */
export interface PluginPageAssetLocationDto {
  pluginId: string;
  pageId: string;
  entry: string;
  loading: PluginPageLoadingDto;
  absolutePath: string;
}

/**
 * `plugin_list_pages` 的返回形状（后端 `plugins::types::PluginPagesInfo`，任务 #75）。
 *
 * PluginManager 是权威，本结构只是其清单的只读投影：`pages` 来自 `manifest.pages[]`，
 * 每次调用实时扫描；前端**不得**据此另建路由表 / 平行 manifest（禁区 4）。
 * 字段口径与 [`DashboardPluginInfoDto`] 同族：Rust snake_case +
 * `#[serde(rename_all = "camelCase")]` ⇒ 本类型 camelCase（禁区 13）。
 */
export interface PluginPagesInfoDto {
  id: string;
  name: string;
  version: string;
  description: string;
  author: string | null;
  enabled: boolean;
  pages: PluginPageDto[];
}

/**
 * `plugin_list_domain_packs` 的返回形状（后端 `plugins::types::PluginDomainPacksInfo`）。
 *
 * P6.2 的一域一插件边界在宿主侧的只读投影：`domainPacks` 来自插件安装目录内的
 * logical manifest，前端不得据此建立第二份域包清单或按粗域重新聚合。
 */
export interface PluginDomainPacksInfoDto {
  id: string;
  name: string;
  version: string;
  description: string;
  author: string | null;
  enabled: boolean;
  domainPacks: PluginDomainPackDto[];
}

/** 单个 OPC 域包的 logical manifest 投影；字段与插件产物里的 JSON 同源。 */
export interface PluginDomainPackDto {
  id: string;
  pluginId: string;
  transportOwner: string;
  page: Record<string, unknown>;
  capabilities: string[];
  commands: string[];
  calls: string[];
}

/**
 * `plugin_list_task_sources` 的返回形状（后端 `plugins::types::PluginTaskSourcesInfo`，任务 #148）。
 *
 * 与 [`PluginPagesInfoDto`] 同族：真相源是 `manifest.taskSources[]`，每次调用实时扫描，
 * 前端只做只读投影 + 泛型适配器实例化，**不得**另建第二份源清单（禁区 4）。
 * `enabled = false` 的条目由消费侧过滤掉（投影面原样透出启停位，与页面清单同口径）。
 */
export interface PluginTaskSourcesInfoDto {
  id: string;
  name: string;
  version: string;
  description: string;
  author: string | null;
  enabled: boolean;
  taskSources: PluginTaskSourceDto[];
}

/**
 * 插件贡献的任务中心任务源声明（后端 `plugins::types::PluginTaskSource`）。
 *
 * 只声明**数据面**：`command` 是本包 `commands[]` 里已声明的命名命令，其返回体是
 * [`PluginTaskRunRowDto`] 数组。渲染形态不在这里（PLAN §93 裁定：插件不给 React）。
 * `title` 是纯文本、不走宿主 i18n —— 与 `PluginPageDto.title` 同口径。
 *
 * `detailSchema` 是**详情槽**的声明（任务 #149 第 2 步）：插件只交**本插件 `ui.json` 里的 key**
 * （不是动态 UI 库的行 id，拼全 id 由宿主按 `plugin.{pluginId}.{key}` 完成，PLAN §112.4），
 * 宿主用 `PluginSchemaCard` 画出来 ⇒ 主 origin 里仍不出现任何插件 React 代码。
 * 缺省/null 时该源不挂 `renderDetail`，点开一行落中心的空抽屉兜底。
 */
export interface PluginTaskSourceDto {
  id: string;
  title: string;
  /** 取数命令名（同一份清单 `commands[].name`）。 */
  command: string;
  /** antd Tag 色板名；缺省由宿主泛型适配器兜底。 */
  color?: string | null;
  /** 详情槽用的 schema key（后端 `detail_schema`，本插件 `ui.json` 的 key）；缺省/null ⇒ 无详情。 */
  detailSchema?: string | null;
}

/**
 * 卡片可落的位置（后端 `plugins::types::PluginCardSurface`，serde `rename_all = "snake_case"`）。
 *
 * 词汇表**只开两个**（2026-11-07 leader 裁定，不预留 `overview`）：
 * - `chat`：聊天消息侧插槽（`PluginSchemaCardSlot` 读的 `Message.meta.pluginCards[]`）；
 * - `task_card`：任务中心卡片面。
 *
 * 前端注册表（`src/lib/dynamicUI/pluginCardRegistry.ts`）的 `surface` 字段与它逐字同集；
 * 新增取值必须同时在后端枚举、本类型与注册表三处登记并接一个消费点。
 */
export type PluginCardSurfaceDto = "chat" | "task_card";

/**
 * 卡片上的一颗动作按钮（后端 `plugins::types::PluginTaskCardAction`）。
 *
 * `command` 只能是**同一份清单** `commands[]` 里的命名命令 —— 宿主命令名在声明面上写不出来。
 * `mutating` 是权限档位判据：`true` ⇒ 清单 `permissions[]` 必须含 `write`，**装载期**由后端拦
 * （前端判不了：`PluginSummaryDto` 不带 `permissions`）。
 */
export interface PluginTaskCardActionDto {
  id: string;
  /** 按钮文案（纯文本，不走宿主 i18n）。 */
  title: string;
  /** 本包 `commands[].name`。 */
  command: string;
  /** 是否有写语义；缺省 `false` = 只读动作。 */
  mutating?: boolean;
}

/**
 * 插件贡献给宿主的一张卡片（后端 `plugins::types::PluginTaskCard`，任务 #216 前置 ②）。
 *
 * `title` / `actions[].title` 是**插件自带文案**，不占宿主 11 语言 key（与 `PluginPageDto.title` 同口径）。
 * `detailSchema` 是**本插件 `ui.json` 的 key**（不是动态 UI 行 id）：拼 `plugin.{pluginId}.{key}`
 * 归宿主（`src/lib/pluginUiSchema.ts` 的 `pluginSchemaRowId`），跨插件引用因此写不出来。
 */
export interface PluginTaskCardDto {
  /** 卡片标识：与 `pluginId` 拼成注册表键 `${pluginId}:${id}`。 */
  id: string;
  title: string;
  surface: PluginCardSurfaceDto;
  /** 计数 / 取数命令（本包 `commands[].name`）。 */
  summaryCommand: string;
  /** 卡片体 schema key；缺省/null ⇒ 纯计数卡（无卡体，不可点开）。 */
  detailSchema?: string | null;
  /** antd Tag 色板名；缺省/null ⇒ 宿主用默认色。 */
  tone?: string | null;
  /** 渲染顺序（升序）；后端缺省 0。 */
  order: number;
  /** 卡片动作；空表 = 纯展示卡。 */
  actions: PluginTaskCardActionDto[];
}

/**
 * `plugin_list_task_cards` 的返回形状（后端 `plugins::types::PluginTaskCardsInfo`，任务 #216）。
 *
 * 与 [`PluginTaskSourcesInfoDto`] 同族：真相源是 `manifest.taskCards[]`，每次调用实时扫描，
 * 前端只做只读投影（注册 contributions），**不得**另建第二份卡片清单（禁区 4）。
 * `enabled = false` 的条目由消费侧过滤掉（投影面原样透出启停位）。
 */
export interface PluginTaskCardsInfoDto {
  id: string;
  name: string;
  version: string;
  description: string;
  author: string | null;
  enabled: boolean;
  taskCards: PluginTaskCardDto[];
}

/**
 * 插件任务源返回的一行运行记录（后端 `axagent_plugin_proto::PluginTaskRunRow`）。
 *
 * ⚠ `state` 的词汇表与 `TasksCenterPage/model.ts` 的 `RunState` **同源**（七态、精确小写）：
 * 表外值由宿主泛型适配器按「未知」处理，不猜成某个具体状态。
 * `nativeId` 不带源前缀 —— run id 由宿主按 `${source}:${nativeId}` 拼。
 */
export interface PluginTaskRunRowDto {
  nativeId: string;
  name: string;
  state: string;
  trigger?: string | null;
  startedAt?: number | null;
  finishedAt?: number | null;
  durationMs?: number | null;
  deepLink?: string | null;
  raw?: unknown;
}

export interface UpdateOutcomeDto {
  pluginId: string;
  oldVersion: string;
  newVersion: string;
  installPath: string;
}

// === Session Share ===

export interface SharePermissions {
  allowTerminalAccess: boolean;
  allowFileAccess: boolean;
  allowModelAccess: boolean;
  requireApprovalForActions: boolean;
  maxParticipants: number;
}

export interface ShareSessionInfo {
  sessionId: string;
  inviteCode: string;
  conversationId: string;
  permissions: SharePermissions;
  participantCount: number;
  createdAt: number;
}

// === Device Sync Types ===

export type {
  AuditAction,
  AuditLogEntry,
  ChangeLogEntry,
  ChangeOperation,
  ConflictInfo,
  ConflictResolutionStrategy,
  DeviceInfo,
  DevicePermissions,
  DeviceSyncStatus,
  DeviceType,
  EncryptedSyncData,
  EncryptionAlgorithm,
  EncryptionState,
  EntityType,
  KeyDerivation,
  PairingCode,
  PairingRequest,
  PairingResponse,
  PermissionUpdate,
  RealtimePushState,
  SyncDirection,
  SyncEncryptionConfig,
  SyncHistoryEntry,
  SyncPolicy,
  SyncPolicyUpdate,
  SyncResult,
  SyncSignal,
  SyncSignalResponse,
  SyncType,
  TrustLevel,
  VersionVectorEntry,
  WebSocketStatus,
} from "./deviceSync";

export interface ShareParticipant {
  id: string;
  name: string;
  joinedAt: number;
}

// ── Dynamic UI ──
export type {
  ComponentRegistryEntry,
  ConditionalDisplay,
  ConditionalRule,
  CreateDynamicUISchemaParams,
  DataSourceConfig,
  DynamicAction,
  DynamicComponentType,
  DynamicImportance,
  DynamicStatus,
  DynamicUIFormDataRecord,
  DynamicUIPinRecord,
  DynamicUIProps,
  DynamicUISchemaOrigin,
  DynamicUISchemaRecord,
  DynamicUISchemaVersion,
  EventHandler,
  ListVersionsResponse,
  PinDynamicUISchemaParams,
  SaveDynamicUIFormDataParams,
  SchemaValidationError,
  SchemaValidationResult,
  UISchema,
  UpdateDynamicUIPinParams,
  UpdateDynamicUISchemaParams,
} from "./dynamicUI";
export { COMPONENT_REQUIRED_PROPS, TRUSTED_SCHEMA_ORIGINS, VALID_DYNAMIC_COMPONENT_TYPES } from "./dynamicUI";

export interface DashboardStats {
  totalConversations: number;
  totalMessages: number;
  totalPromptTokens: number;
  totalCompletionTokens: number;
  totalTokens: number;
  totalAgentSessions: number;
  completedAgentSessions: number;
  failedAgentSessions: number;
  totalAgentTokens: number;
  totalCostUsd: number;
  totalToolCalls: number;
  /** 今日（本地时区）消息数 */
  todayMessages: number;
  /** 今日（本地时区）输入 token 数 */
  todayPromptTokens: number;
  /** 今日（本地时区）输出 token 数 */
  todayCompletionTokens: number;
  /** 今日（本地时区）总 token 数 */
  todayTokens: number;
}

// === Local Model (llama.cpp) Management ===

export interface LocalModelInfo {
  id: string;
  nEmbd: number | null;
  nCtx: number | null;
  nCtxTrain: number | null;
  nParams: number | null;
  sizeBytes: number | null;
  ftype: string | null;
  nVocab: number | null;
}

export interface LocalModelProps {
  modelPath: string | null;
  modelAlias: string | null;
  modelFtype: string | null;
  nCtx: number | null;
  totalSlots: number | null;
}

export interface LocalModelStatus {
  running: boolean;
  health: string;
  pid: number | null;
  processName: string | null;
  memoryMb: number | null;
  baseUrl: string;
  managed: boolean;
  model: LocalModelInfo | null;
  props: LocalModelProps | null;
}

export interface LocalModelStartConfig {
  serverExe: string;
  modelPath: string;
  host: string;
  port: number;
  alias?: string | null;
  nCtx?: number | null;
  nGpuLayers?: number | null;
  embeddingMode?: boolean;
  extraArgs: string[];
}

export interface EmbedTestResult {
  dimensions: number;
  promptTokens: number | null;
  elapsedMs: number;
  preview: number[];
}

// === Local Model Download ===

export interface LocalFileModel {
  filename: string;
  sizeBytes: number;
  modifiedAt: number | null;
  modelType: string;
  isDownloading: boolean;
  downloadBytes: number;
}

export interface DownloadTaskInfo {
  filename: string;
  downloadedBytes: number;
  totalBytes: number;
  status: "downloading" | "done" | "failed";
  error: string | null;
}

export interface DownloadRequest {
  filename: string;
  hfRepo?: string | null;
  directUrl?: string | null;
}

export interface PresetModelDto {
  filename: string;
  hfRepo: string | null;
  directUrl: string | null;
  displayName: string;
  sizeBytes: number;
  modelType: string;
  isDownloaded: boolean;
}

// === llama.cpp 安装管理 ===

export interface LlamaCppVersionInfo {
  tag: string;
  name: string;
  publishedAt: string;
  downloadUrl: string;
  fileName: string;
  fileSize: number | null;
}

export interface LlamaCppInstallStatus {
  installed: boolean;
  version: string | null;
  installPath: string | null;
  executablePath: string | null;
  isDownloading: boolean;
  downloadProgress: number | null;
  downloadError: string | null;
}

// === 技能学习闭环（Hermes 借鉴） ===

export type {
  ApprovalStatus,
  LearnSkillInput,
  LearnSkillResult,
  MemoryWriteApprovalConfig,
  PendingMemoryWrite,
  PendingOperationType,
  PendingSkillOperation,
  RiskLevel,
  SkillLearningConfig,
} from "./skillLearning";

// === AxInvest 投资域（stock/quant/pipeline） ===
// quant 已从 barrel 拆除（B8-5 系列，38 个导出），12 条消费方 import 一律经 "@/types/quant" 直取
// BacktestResult 与 stock-analysis.ts 同名但语义不同（quant=完整量化回测 vs 单股决策记录）：
// barrel 不再仲裁权威，quant 版走 "@/types/quant"，stock-analysis 版走 "@/types/stock-analysis"
// stock-analysis 已从 barrel 拆除（B8-5 第一批），消费方一律经 "@/types/stock-analysis" 直取
export * from "./backgroundTask";
// pipeline 已从 barrel 拆除（B8-5 第二批），5 个消费方一律经 "@/types/pipeline" 直取
// （PipelineResult / PipelineAnalysisSummary / PipelineSummary / PipelineRun / PipelineStepEvent）
