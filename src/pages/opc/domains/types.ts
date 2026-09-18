// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 域包页面共享类型定义
 *
 * ⚠ 契约分层：本文件里的**类型名**是前端标识符（可自由改名），但**字段名**多数镜像
 * 后端 serde 结构（如 `domain_pack_id` / `domain_pack_count`，源自 `#[serde(rename_all)]`），
 * 改名必须与 Rust 侧同批进行，否则前端读不到值且 tsc 不会报错。
 */

import type { ReactNode } from "react";

/** 行业清单 */
export interface DomainManifest {
  id: string;
  name: string;
  icon: string;
  description: string;
  version: number;
  enabled: boolean;
}

/** 工作流用户输入字段（前端渲染表单用） */
export interface WorkflowInputField {
  key: string;
  label: string;
  type: "string" | "number" | "textarea";
  required?: boolean;
  placeholder?: string;
  default?: string;
}

/** 向导步骤类型 */
export type WizardStepType = "form" | "confirm" | "execute" | "result" | "custom";

/** 向导上下文 — 跨步骤共享的状态与操作 */
export interface WizardContext {
  /** 当前所有表单值 */
  values: Record<string, unknown>;
  /** 设置单个字段值 */
  setValue: (key: string, value: unknown) => void;
  /** 批量设置字段值 */
  setValues: (values: Record<string, unknown>) => void;
  /** 当前步骤索引 */
  stepIndex: number;
  /** 工作流元数据 */
  workflow: DomainWorkflow;
  /** 执行工作流 */
  execute: () => Promise<void>;
  /** 执行状态 */
  executing: boolean;
  /** 执行结果 */
  resultStatus: "success" | "failed" | null;
  /** 执行结果消息 */
  resultMessage: string;
  /** 关闭向导 */
  close: () => void;
}

/** 向导步骤定义 */
export interface WizardStep {
  /** 步骤唯一 ID */
  id: string;
  /** 步骤标题（i18n key 或直接文本） */
  title: string;
  /** 步骤标题的 i18n 描述（可选） */
  description?: string;
  /** 步骤类型 */
  type: WizardStepType;
  /** 该步骤关联的输入字段（仅 form 类型使用） */
  fields?: WorkflowInputField[];
  /** 自定义渲染函数（custom 类型使用） */
  render?: (ctx: WizardContext) => ReactNode;
  /** 校验函数，返回是否可进入下一步 */
  validate?: (ctx: WizardContext) => boolean;
  /** 跳过条件函数，返回 true 时跳过此步骤 */
  canSkip?: (ctx: WizardContext) => boolean;
  /** "下一步"按钮文案（i18n key） */
  nextLabel?: string;
  /** "上一步"按钮文案（i18n key） */
  prevLabel?: string;
  /** 是否显示"上一步"按钮 */
  showBack?: boolean;
}

/** 行业工作流 */
export interface DomainWorkflow {
  id: string;
  name: string;
  description: string;
  version: string;
  /** 关联的工作流模板 ID（用于在编辑器中打开，不传则通过 id 查找） */
  template_id?: string;
  /** 用户输入字段（非空时前端渲染表单） */
  inputFields?: WorkflowInputField[];
  /**
   * 自定义向导步骤。未设置时自动生成：
   * - 有 inputFields → form → confirm → execute → result
   * - 无 inputFields → confirm → execute → result
   */
  wizardSteps?: WizardStep[];
}

/** 行业操作项 */
export interface ActionItem {
  key: string;
  icon: ReactNode;
  type: "conversation" | "workflow";
  label?: string;
  /** 工作流模板 ID（type=workflow 时指定要打开的模板，不传则查找关联 workflow 的 template_id） */
  template_id?: string;
}

/** Tab 业务阶段配置 */
export interface DomainTab {
  key: string;
  label: string;
  icon?: ReactNode;
  description?: string;
  actions: ActionItem[];
  workflows: DomainWorkflow[];
}

/** 行业配置（支持 Tab 业务流程） */
export interface DomainConfig {
  // 兼容旧格式（无 Tab）
  actions?: ActionItem[];
  workflows?: DomainWorkflow[];
  // 新格式：Tab 业务流程
  tabs?: DomainTab[];
}

/**
 * KPI 取数状态（后端 `KpiAvailability`）。
 *
 * - `available`：`value` 是真实值（含「统计结果恰为 0」）
 * - `empty`：计算源已接但当前窗口无记录，`value` 仅为占位
 * - `no_data_source`：计算逻辑未接数据源，`value` 无意义
 */
export type KpiAvailability = "available" | "empty" | "no_data_source";

/** KPI 值（对应后端 `opc::analytics::KpiValue`） */
export interface KpiValue {
  key: string;
  /** 展示 ID（缺省等于 key），元数据来自 runtime.yaml */
  id: string;
  /** 展示名称，元数据来自 runtime.yaml */
  name: string;
  value: number;
  unit: string | null;
  target?: number | null;
  timestamp: number;
  availability: KpiAvailability;
  /** 非 available 时的原因说明 */
  note?: string | null;
}

/** 仪表盘展示卡片（后端 `DashboardCard`） */
export interface DashboardCard {
  id: string;
  title: string;
  kpi_key: string;
  display_value: string;
}

/**
 * 风控等级（后端 `opc::analysis::RiskLevel`，serde `snake_case`）。
 *
 * `critical` = 该行业**已声明生效阈值**的受管 KPI 键**全部**违规；一个生效阈值都没
 * 声明时不可能出现 `critical`（分母为 0 ⇒ 最高只到 `high`）。
 */
export type RiskLevel = "critical" | "high" | "medium" | "low";

/** 风控违规明细（后端 `opc::analysis::RiskViolation`） */
export interface RiskViolation {
  /** 规则名，如 `min_word_count` / `max_monthly_expense_ratio` */
  rule: string;
  current: number;
  threshold: number;
  /** 后端生成的说明文本（与 `summary` 同类：数据，不是 UI 文案） */
  message: string;
}

/** 行业仪表盘（对应后端 `DomainDashboard`） */
export interface DomainDashboard {
  domain_pack_id: string;
  kpis: KpiValue[];
  cards: DashboardCard[];
  summary?: string | null;
  /**
   * 风控等级（后端复用 `OpcRiskGate`，与 `opc_execute_analysis` 同一判据）。
   * `null` = **未判定**，不是「无风险」——不得当 `low` 展示。
   */
  risk_level: RiskLevel | null;
  /** 判定命中的违规明细（与 `risk_level` 同源；未判定 ⇒ 空数组） */
  violations: RiskViolation[];
}

/** `opc_get_domain_pack_dashboard` 的响应信封 */
export interface DomainDashboardResponse {
  domainPackId: string;
  dashboard: DomainDashboard;
}

/** 工作流步骤信息 */
export interface WorkflowStepInfo {
  id: string;
  workflow_id: string;
  step_order: number;
  step_type: string;
  name: string;
  description: string;
  avg_duration_ms: number;
  success_rate: number;
  execution_count: number;
}

/** 自动化规则信息 */
export interface AutomationRuleInfo {
  id: string;
  name: string;
  description: string;
  trigger_event: string;
  condition: string;
  action: string;
  enabled: boolean;
  last_triggered: number | null;
  trigger_count: number;
}

/**
 * 行业分析决策（对应后端 `opc::analysis::OpcDomainDecision`）。
 *
 * 字段**以后端结构体为准**：曾声明过 `id` / `days` / `generated_at` 三个后端从未
 * 下发的字段（幽灵字段，全仓无消费点），已删除。
 */
export interface OpcDomainDecision {
  domain_pack_id: string;
  summary: string;
  risk_level: RiskLevel;
  confidence: number;
  decision_type: string;
  kpis: KpiValue[];
  recommendations: string[];
}

/** 工作流执行结果 */
export interface WorkflowExecutionResult {
  workflow_id: string;
  status: "success" | "failed" | "running" | "completed";
  steps_completed: number;
  steps_total: number;
  output?: Record<string, unknown>;
  error?: string;
  duration_ms: number;
  node_results?: Array<{
    id: string;
    name: string;
    status: string;
    duration_ms: number;
    output?: Record<string, unknown>;
  }>;
}

/** 行业学习指标 */
export interface DomainLearningMetrics {
  total_samples: number;
  decision_accuracy: number;
  risk_prediction_accuracy: number;
  avg_feedback_score: number;
  improvement_trend: "improving" | "stable" | "declining";
  reflection_count: number;
  evolution_count: number;
  improvement_count: number;
  avg_improvement_score: number;
  last_reflection_at: number | null;
  last_evolution_at: number | null;
  last_improvement_at: number | null;
}
