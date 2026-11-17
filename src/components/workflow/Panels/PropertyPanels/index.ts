// SPDX-License-Identifier: AGPL-3.0-only

import { AgentPropertyPanel } from "./AgentPropertyPanel";
export { AgentPropertyPanel };
import { AggregatorPropertyPanel } from "./AggregatorPropertyPanel";
export { AggregatorPropertyPanel };
import { ApprovalPropertyPanel } from "./ApprovalPropertyPanel";
export { ApprovalPropertyPanel };
import { BasePropertyPanel } from "./BasePropertyPanel";
export { BasePropertyPanel };
import { CodePropertyPanel } from "./CodePropertyPanel";
export { CodePropertyPanel };
import { ConditionPropertyPanel } from "./ConditionPropertyPanel";
export { ConditionPropertyPanel };
import { DatabaseQueryPropertyPanel } from "./DatabaseQueryPropertyPanel";
export { DatabaseQueryPropertyPanel };
import { DataTransformerPropertyPanel } from "./DataTransformerPropertyPanel";
export { DataTransformerPropertyPanel };
import { DebatePropertyPanel } from "./DebatePropertyPanel";
export { DebatePropertyPanel };
import { DelayPropertyPanel } from "./DelayPropertyPanel";
export { DelayPropertyPanel };
import { DocumentParserPropertyPanel } from "./DocumentParserPropertyPanel";
export { DocumentParserPropertyPanel };
import { EmailPropertyPanel } from "./EmailPropertyPanel";
export { EmailPropertyPanel };
import { EndPropertyPanel } from "./EndPropertyPanel";
export { EndPropertyPanel };
import { FileOperationPropertyPanel } from "./FileOperationPropertyPanel";
export { FileOperationPropertyPanel };
import { HttpRequestPropertyPanel } from "./HttpRequestPropertyPanel";
export { HttpRequestPropertyPanel };
import { LlmClassifierPropertyPanel } from "./LlmClassifierPropertyPanel";
export { LlmClassifierPropertyPanel };
import { LLMPropertyPanel } from "./LLMPropertyPanel";
export { LLMPropertyPanel };
import { LoggingPropertyPanel } from "./LoggingPropertyPanel";
export { LoggingPropertyPanel };
import { LoopPropertyPanel } from "./LoopPropertyPanel";
export { LoopPropertyPanel };
import { MergePropertyPanel } from "./MergePropertyPanel";
export { MergePropertyPanel };
import { MultiAgentPropertyPanel } from "./MultiAgentPropertyPanel";
export { MultiAgentPropertyPanel };
import { NotificationPropertyPanel } from "./NotificationPropertyPanel";
export { NotificationPropertyPanel };
import { ParallelPropertyPanel } from "./ParallelPropertyPanel";
export { ParallelPropertyPanel };
import { StoragePropertyPanel } from "./StoragePropertyPanel";
export { StoragePropertyPanel };
import { SubWorkflowPropertyPanel } from "./SubWorkflowPropertyPanel";
export { SubWorkflowPropertyPanel };
import { SwarmPropertyPanel } from "./SwarmPropertyPanel";
export { SwarmPropertyPanel };
import { SwitchPropertyPanel } from "./SwitchPropertyPanel";
export { SwitchPropertyPanel };
import { ToolPropertyPanel } from "./ToolPropertyPanel";
export { ToolPropertyPanel };
import { TriggerPropertyPanel } from "./TriggerPropertyPanel";
export { TriggerPropertyPanel };
import { ValidationPropertyPanel } from "./ValidationPropertyPanel";
export { ValidationPropertyPanel };
import { VectorRetrievePropertyPanel } from "./VectorRetrievePropertyPanel";
export { VectorRetrievePropertyPanel };
import { WebhookSendPropertyPanel } from "./WebhookSendPropertyPanel";
export { WebhookSendPropertyPanel };
import { WorkflowRefPropertyPanel } from "./WorkflowRefPropertyPanel";
export { WorkflowRefPropertyPanel };

// ── 节点类型 → 属性面板注册表（P6.0-① 阶段5）────────────────────────────
//
// 原 RightPanel 的 32 臂 switch 改为按本表查找；新增节点类型只需在插件侧
// `registerNodePanel(type, component)`，主壳零改。面板本体 props 已统一。

import type { ComponentType } from "react";
import type { WorkflowNode } from "../../types";

/** 所有属性面板共用的 props（各面板独立 interface 与此结构同形）。 */
export interface NodePanelProps {
  node: WorkflowNode;
  onUpdate: (updates: Partial<WorkflowNode>) => void;
  onDelete: () => void;
}

/** 节点类型 → 属性面板组件。 */
export const NODE_PANEL_COMPONENTS: Record<string, ComponentType<NodePanelProps>> = {
  trigger: TriggerPropertyPanel,
  agent: AgentPropertyPanel,
  multiAgent: MultiAgentPropertyPanel,
  llm: LLMPropertyPanel,
  condition: ConditionPropertyPanel,
  parallel: ParallelPropertyPanel,
  loop: LoopPropertyPanel,
  merge: MergePropertyPanel,
  delay: DelayPropertyPanel,
  tool: ToolPropertyPanel,
  code: CodePropertyPanel,
  subWorkflow: SubWorkflowPropertyPanel,
  documentParser: DocumentParserPropertyPanel,
  vectorRetrieve: VectorRetrievePropertyPanel,
  validation: ValidationPropertyPanel,
  end: EndPropertyPanel,
  debate: DebatePropertyPanel,
  httpRequest: HttpRequestPropertyPanel,
  switch: SwitchPropertyPanel,
  databaseQuery: DatabaseQueryPropertyPanel,
  storage: StoragePropertyPanel,
  notification: NotificationPropertyPanel,
  approval: ApprovalPropertyPanel,
  fileOperation: FileOperationPropertyPanel,
  dataTransformer: DataTransformerPropertyPanel,
  webhookSend: WebhookSendPropertyPanel,
  logging: LoggingPropertyPanel,
  llmClassifier: LlmClassifierPropertyPanel,
  aggregator: AggregatorPropertyPanel,
  email: EmailPropertyPanel,
  swarm: SwarmPropertyPanel,
  workflowRef: WorkflowRefPropertyPanel,
};

/** 注册自定义节点类型的属性面板；同名重复注册不覆盖（保留先注册者）。 */
export function registerNodePanel(type: string, component: ComponentType<NodePanelProps>): boolean {
  if (NODE_PANEL_COMPONENTS[type]) { return false; }
  NODE_PANEL_COMPONENTS[type] = component;
  return true;
}
