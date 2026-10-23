// 此文件由 schema-gen 自动生成，请勿手动编辑
// 修改 Rust DTO 后请重新运行: cargo run -p schema-gen -- ipc-types
//
// 本文件包含所有 Tauri IPC 跨进程传输的数据结构定义，
// 用于确保前后端类型一致性。

// 生成文件中的类型别名仅供后端 DTO 同步校验，
// 不在前端运行时直接引用，故禁用 no-unused-vars 规则。
// oxlint-disable no-unused-vars

// ============================================================================
// 基础类型（补充定义）
// ============================================================================

type MessageRole = "system" | "user" | "assistant" | "tool";

type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [key: string]: JsonValue };

// ============================================================================
// Agent 相关 DTO
// ============================================================================

type AgentExecuteRequest = { goal: string, context: string | null, maxSteps: number | null, };

type AgentResult = { output: string, success: boolean, stepsTaken: number, 
/**
 * 执行过程中创建的会话 ID（如果有持久化会话）。
 * 用于 MCP `agent_run` 返回后，调用方可通过 `agent_status` / `agent_cancel` 跟踪。
 */
sessionId: string | null, };

type AgentCapability = { name: string, description: string, };

type AgentInfo = { name: string, description: string, capabilities: Array<AgentCapability>, };

type AgentPlan = { steps: Array<PlanStep>, };

type PlanStep = { description: string, agent: string | null, };


// ============================================================================
// Conversation 相关 DTO
// ============================================================================

type TokenUsage = { inputTokens: number, outputTokens: number, cacheCreationInputTokens: number, cacheReadInputTokens: number, cacheMissInputTokens: number | null, };

type ContentBlock = { "Text": { text: string, } } | { "ToolUse": { id: string, name: string, input: string, } } | { "ToolResult": { tool_use_id: string, tool_name: string, output: string, is_error: boolean, } };

type ConversationMessage = { role: MessageRole, blocks: Array<ContentBlock>, usage: TokenUsage | null, };

type SessionInfo = { sessionId: string, userId: string, title: string | null, createdAt: bigint, updatedAt: bigint, tokenUsage: TokenUsage | null, };


// ============================================================================
// Workflow 相关 DTO
// ============================================================================

type Position = { x: number, y: number, };

type RetryConfig = { enabled: boolean, maxRetries: number, backoffType: BackoffType, baseDelayMs: bigint, maxDelayMs: bigint, };

type BackoffType = "Linear" | "Exponential" | "Fixed";

type CompensationConfig = { strategy: CompensationStrategy, 
/**
 * 需要执行补偿的节点 ID 列表（预留扩展，当前由引擎根据 DAG 自动推导下游）
 */
compensation_nodes: Array<string>, };

type CompensationStrategy = "SkipWithWarning" | "Rollback" | "Escalate";

type NodeKind = "Input" | "Output" | "Tool" | "Agent" | "Condition" | "Loop" | "Container" | "Storage";

type Variable = { name: string, var_type: string, value: JsonValue, description: string | null, is_secret: boolean, };

type WorkflowNodeBase = { id: string, title: string, description: string | null, position: Position, retry: RetryConfig, timeout: bigint | null, enabled: boolean, 
/**
 * 容器父节点 ID。此字段由前端在保存时注入，
 * 用于将子节点（如 Parallel 分支步骤）定位到父容器内。
 */
parentId: string | null, 
/**
 * 节点失败时的补偿/回滚策略。None = 不执行任何补偿。
 */
compensation: CompensationConfig | null, 
/**
 * 节点失败时不中断整个工作流，继续执行后续节点。
 */
continueOnFail: boolean, };


// ============================================================================
// 同步检查元数据
// ============================================================================

// Total DTO types: 18
