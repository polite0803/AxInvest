// SPDX-License-Identifier: AGPL-3.0-only
// Phase 4: NL2Workflow — 自然语言驱动工作流类型定义

/** 节点类型枚举 */
export type NodeType = "trigger" | "action" | "condition" | "loop" | "parallel" | "subflow" | "output";

/** 单个节点定义 */
export interface WorkflowNode {
  id: string;
  type: NodeType;
  label: string;
  description?: string;
  config: Record<string, unknown>;
  position: { x: number; y: number };
  inputs?: string[];
  outputs?: string[];
}

/** 边 */
export interface WorkflowEdge {
  id: string;
  source: string;
  target: string;
  label?: string;
  condition?: string;
}

/** 完整工作流定义 */
export interface WorkflowDefinition {
  id: string;
  name: string;
  description: string;
  version: number;
  nodes: WorkflowNode[];
  edges: WorkflowEdge[];
  variables: Record<string, unknown>;
  createdAt: number;
  updatedAt: number;
  status: "draft" | "active" | "archived";
}

/** 自然语言解析请求 */
export interface NLParseRequest {
  prompt: string;
  context?: string;
  constraints?: string[];
  /** 当前画布节点（供上下文感知生成 / 合并模式使用）。结构为编辑器节点类型，此处泛化为 unknown 避免跨类型依赖 */
  currentNodes?: unknown[];
  /** 当前画布边 */
  currentEdges?: unknown[];
}

/** 自然语言解析结果 */
export interface NLParseResult {
  workflow: WorkflowDefinition;
  confidence: number;
  suggestions: string[];
  alternatives?: WorkflowDefinition[];
}

/** 工作流模板 */
export interface WorkflowTemplate {
  id: string;
  name: string;
  description: string;
  category: "data-processing" | "notification" | "content-generation" | "monitoring" | "integration";
  nodeCount: number;
  tags: string[];
  workflow: WorkflowDefinition;
  isBuiltIn: boolean;
  createdAt: number;
  updatedAt: number;
}

/** 工作流执行状态 */
export type WorkflowExecutionStatus = "idle" | "running" | "paused" | "completed" | "failed";

/** 节点执行状态 */
export interface NodeExecutionState {
  nodeId: string;
  status: "waiting" | "running" | "success" | "failed";
  startedAt?: number;
  finishedAt?: number;
  output?: unknown;
  error?: string;
}

/** 工作流执行记录 */
export interface WorkflowExecution {
  id: string;
  workflowId: string;
  status: WorkflowExecutionStatus;
  startedAt: number;
  finishedAt?: number;
  nodeStates: NodeExecutionState[];
  inputs: Record<string, unknown>;
  outputs?: Record<string, unknown>;
  logs: ExecutionLogEntry[];
}

/** 执行日志条目 */
export interface ExecutionLogEntry {
  timestamp: number;
  nodeId: string;
  nodeName: string;
  level: "info" | "warn" | "error";
  message: string;
}

/** 工作流版本记录 */
export interface WorkflowVersion {
  version: number;
  updatedAt: number;
  summary: string;
  status: "draft" | "active" | "archived";
  snapshot: WorkflowDefinition;
}

/** 版本对比结果 */
export interface VersionDiff {
  addedNodes: WorkflowNode[];
  removedNodes: WorkflowNode[];
  modifiedNodes: { before: WorkflowNode; after: WorkflowNode }[];
  addedEdges: WorkflowEdge[];
  removedEdges: WorkflowEdge[];
  modifiedEdges: { before: WorkflowEdge; after: WorkflowEdge }[];
}

/** 工作流筛选 */
export interface WorkflowFilter {
  search?: string;
  status?: "all" | "draft" | "active" | "archived";
  category?: WorkflowTemplate["category"] | "all";
}

// ============================================================
// Phase 4: NL2Skill — 自然语言→技能定义
// ============================================================

/** NL2Skill 解析请求 */
export interface NL2SkillRequest {
  prompt: string;
  context?: string;
  skillType?: "chat" | "tool" | "workflow" | "automation";
}

/** 技能定义 */
export interface SkillDefinition {
  id: string;
  name: string;
  description: string;
  type: "chat" | "tool" | "workflow" | "automation";
  triggers: string[];
  promptTemplate: string;
  parameters: SkillParameter[];
  tools: string[];
  icon?: string;
  tags?: string[];
}

export interface SkillParameter {
  name: string;
  type: "string" | "number" | "boolean" | "enum" | "object";
  description: string;
  required: boolean;
  default?: unknown;
  options?: string[];
}

/** NL2Skill 解析阶段 */
export interface NL2SkillPhase {
  phase: string;
  status: "pending" | "running" | "done";
  detail: string;
}

/** NL2Skill 解析结果 */
export interface NL2SkillResult {
  skill: SkillDefinition;
  confidence: number;
  phases: NL2SkillPhase[];
  suggestions: string[];
  alternatives?: SkillDefinition[];
}

// ============================================================
// Phase 4: NL2UI — 自然语言→动态 UI Schema
// ============================================================

/** NL2UI 解析请求 */
export interface NL2UIRequest {
  prompt: string;
  context?: string;
  uiType?: "form" | "dashboard" | "settings" | "report" | "custom";
}

/** NL2UI 解析阶段 */
export interface NL2UIPhase {
  phase: string;
  status: "pending" | "running" | "done";
  detail: string;
}

/** NL2UI 解析结果 */
export interface NL2UIResult {
  schema: UISchema;
  confidence: number;
  phases: NL2UIPhase[];
  suggestions: string[];
  alternatives?: { schema: UISchema; description: string }[];
}

// ============================================================
// 工作流运行时工具（workflow_tools 表 DTO，与后端 camelCase 对齐）
// ============================================================

/** 工具类型 */
export type WorkflowToolType = "rhai_script" | "workflow_dag" | "llm_function";

/** 工具状态 */
export type WorkflowToolStatus = "pending" | "active" | "disabled";

/** 工作流运行时工具 —— 动态发现/生成工具的持久化表示 */
export interface WorkflowTool {
  id: string;
  workflowId: string;
  toolName: string;
  toolType: WorkflowToolType;
  description?: string;
  code?: string;
  inputSchema?: string;
  source: string;
  status: WorkflowToolStatus;
  usageCount: number;
  successRate: number;
  createdAt: number;
  updatedAt: number;
}

/** 新增/覆盖工具入参 */
export interface WorkflowToolInput {
  workflowId: string;
  toolName: string;
  toolType: WorkflowToolType;
  description?: string;
  code?: string;
  inputSchema?: string;
  source?: string;
  status?: WorkflowToolStatus;
}

// UISchema 类型从 @/types/dynamicUI 导入，此处仅做 forward-declaration
import type { UISchema } from "./dynamicUI";

/**
 * AI 语义检查给出的「工具升级建议」（后端 `workflow_tool_semantic_check` 的响应元素）。
 *
 * 2026-11-07（任务 #193，组件中立性 A 类）由
 * `src/components/workflow/types/workflow.types.ts` 下沉至此：唯一消费方是
 * `src/components/settings/ToolSemanticCheck.tsx`，属跨域共享 DTO，
 * 不该让 settings 反向依赖 `components/workflow` 的类型模块。
 * 原位置保留 `export type { ToolUpgradeSuggestion } from "@/types"` 的薄 re-export。
 */
export interface ToolUpgradeSuggestion {
  name: string;
  description: string;
  inputSchema: Record<string, unknown> | null;
  outputSchema: Record<string, unknown> | null;
  reasoning: string;
}

// ============================================================
// 工作流模板共享 DTO（组件中立性 A 档，与上方 ToolUpgradeSuggestion 同族）
// 由 src/components/workflow/types/workflow.types.ts 与 stores/feature/workflowEditorStore.ts
// 下沉：settings / stock-analysis 跨域消费这些类型，不该反向依赖 components/workflow
// 的类型模块或 workflowEditor store。原位置保留薄 re-export 供工作流内部存量消费方。
// ============================================================

// 字段类型仍引用编辑器域内类型（type-only，编译后擦除，无运行时耦合）。
// WorkflowNode/WorkflowEdge 必须用别名：本文件上方已有 NL2Workflow 的简化版同名类型，
// 二者语义不同（编辑器版含 retry/timeout/enabled 等），不可混用。
import type {
  DiagnosticFix,
  EditAssetOperation,
  ErrorConfig,
  JsonSchema,
  RhaiToolDef,
  TriggerConfig,
  ValidationSpec,
  WorkflowEdge as EditorWorkflowEdge,
  WorkflowNode as EditorWorkflowNode,
} from "@/components/workflow/types/workflow.types";

export interface Variable {
  name: string;
  varType: string;
  value: unknown;
  description?: string;
  isSecret: boolean;
}

export interface WorkflowTemplateInput {
  name: string;
  description?: string;
  icon: string;
  tags: string[];
  triggerConfig?: TriggerConfig;
  nodes: EditorWorkflowNode[];
  edges: EditorWorkflowEdge[];
  inputSchema?: JsonSchema;
  outputSchema?: JsonSchema;
  variables: Variable[];
  errorConfig?: ErrorConfig;
  toolDefs?: RhaiToolDef[];
  /** L2 集群 ID（三层路由第二层，命令层可从 tags 推导） */
  clusterId?: string;
  /** 三层路由路径（命令层可从 tags 推导，格式 /{domain}/{cluster}/{capability}） */
  routePath?: string;
}

export interface WorkflowTemplateResponse {
  id: string;
  name: string;
  description?: string;
  icon: string;
  tags: string[];
  version: number;
  isPreset: boolean;
  isEditable: boolean;
  isPublic: boolean;
  /** 是否为系统模板（认知编排器等），由后端按 isPreset + cognitiveRouter 标签权威判定 */
  isSystem: boolean;
  triggerConfig?: TriggerConfig;
  nodes: EditorWorkflowNode[];
  edges: EditorWorkflowEdge[];
  inputSchema?: JsonSchema;
  outputSchema?: JsonSchema;
  variables: Variable[];
  errorConfig?: ErrorConfig;
  toolDefs?: RhaiToolDef[];
  missionHash?: string;
  /** L2 集群 ID（三层路由第二层） */
  clusterId?: string;
  /** 三层路由路径（格式 /{domain}/{cluster}/{capability}） */
  routePath?: string;
  createdAt: number;
  updatedAt: number;
}

/**
 * AI 聊天面板产出的工作流变更动作。
 * 与后端 workflow_ai_chat_stream 系统 prompt 中的 :::action 块对应，
 * 前端以 discriminated union 解析，applyAiChatAction 按 action_type 分发。
 */
export type AiChatAction =
  | { actionType: "generate_workflow"; data: { nodes: EditorWorkflowNode[]; edges: EditorWorkflowEdge[] } }
  | { actionType: "add_node"; data: { node: EditorWorkflowNode; position?: { x: number; y: number } } }
  | { actionType: "add_nodes"; data: { nodes: EditorWorkflowNode[] } }
  | { actionType: "update_node"; data: { nodeId: string; changes: Partial<EditorWorkflowNode> } }
  | { actionType: "modify_node"; data: { nodeId: string; changes: Record<string, unknown> } }
  | { actionType: "delete_node"; data: { nodeId: string } }
  | { actionType: "delete_nodes"; data: { nodeIds: string[] } }
  | { actionType: "add_edge"; data: { edge: EditorWorkflowEdge } }
  | { actionType: "update_edge"; data: { edgeId: string; changes: Partial<EditorWorkflowEdge> } }
  | { actionType: "delete_edge"; data: { edgeId: string } }
  | { actionType: "optimize_prompt"; data: { nodeId: string; optimizedPrompt: string } }
  // ── v2.0 基础设施类 action(与后端 workflow_ai_protocol::ChatAction 对齐)──
  | { actionType: "update_variable"; data: { templateId: string; name: string; value: unknown } }
  | { actionType: "rollback_to_version"; data: { templateId: string; version: number } }
  | {
    actionType: "update_input_mapping";
    data: {
      nodeId: string;
      mappings: Array<{ target: string; source: string }>;
    };
  }
  | {
    actionType: "edit_asset_file";
    data: {
      path: string;
      operation: EditAssetOperation;
      anchorLine: number;
      /** insert_after / replace 时必填;delete 时可省 */
      code?: string;
      description: string;
    };
  }
  | {
    actionType: "apply_diff_with_validation";
    data: {
      actions: AiChatAction[];
      validation: ValidationSpec;
      rollbackOnFailure?: boolean;
    };
  };

/** AiChatAction 的 actionType 联合类型（用于 switch 穷尽性检查） */
export type AiChatActionType = AiChatAction["actionType"];

/**
 * V2 协议 LLM diagnose 报告原始 schema(后端 `llm_diagnose_workflow` 返回):
 * 4 档 severity + 顶层 fixes[] + autoApply 标志
 *
 * 导出供 WorkflowExecutor 等跨上下文场景复用(禁止重复定义)。
 */
export interface LlmDiagnoseV2 {
  summary: string;
  issues: Array<{
    severity: string;
    category: string;
    nodeId: string | null;
    title: string;
    detail: string;
    suggestion: string;
    fix?: DiagnosticFix;
  }>;
  suggestions: string[];
  fixes?: DiagnosticFix[];
  autoApply?: boolean;
}
