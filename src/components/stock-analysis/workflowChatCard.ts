// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 股票工作流的聊天卡片**数据契约**（消息内容里的 `<!-- workflow-<type>:JSON -->` 标记）。
 *
 * 原与卡片渲染同处 `@/components/chat/WorkflowAgentCard.tsx`，但渲染端在宿主聊天目录里
 * 没有任何分派点（`parseWorkflowCard` 零调用方），而生产端 `stockWorkflowChatBridge` 是
 * 股票域文件 ⇒ 宿主那份只剩数据契约是股票语义。迁出判据（PLAN §93）：卡片渲染只能走
 * schema 卡片或 iframe 富卡片，所以这里**只保留数据**，不再保留 React 实现。
 */
export interface WorkflowCardData {
  type: "progress" | "analyst" | "decision" | "aggregate" | "debate" | "risk";
  phase?: string;
  completed?: number;
  total?: number;
  analystName?: string;
  analystReport?: string;
  action?: string;
  positionPct?: number;
  targetPrice?: number;
  stopLoss?: number;
  reasoning?: string;
  riskLevel?: string;
  confidence?: number;
  round?: number;
  bull?: string;
  bear?: string;
  riskKey?: string;
  riskContent?: string;
  analysts?: Array<
    { nodeId: string; name: string; report?: string; status: "pending" | "running" | "done" | "failed" }
  >;
  debates?: Array<{
    /** 档位 snake_case；null/缺省 = 分档之前的历史单份记录 */
    tier?: string | null;
    round: number;
    bull?: string;
    bear?: string;
    status: "pending" | "running" | "done" | "failed";
  }>;
  risks?: Array<{ key: string; content?: string; status: "pending" | "running" | "done" | "failed" }>;
  extraNodes?: Array<{ key: string; label: string; status: "pending" | "running" | "done" | "failed" }>;
  dataSources?: Array<{
    nodeId: string;
    toolName: string;
    label: string;
    status: "pending" | "fetching" | "success" | "failed";
    error?: string;
    summary?: string;
  }>;
  decision?: WorkflowCardData;
  status?: "running" | "done" | "error";
  error?: string;
  failedSteps?: Array<{ nodeId: string; error?: string }>;
}

/** 把卡片数据拼成一条消息正文：标记在前，`fallback` 是标记解析失败时给人看的纯文本。 */
export function makeWorkflowContent(type: string, data: Record<string, unknown>, fallback: string): string {
  return `<!-- workflow-${type}:${JSON.stringify(data)} -->${fallback}`;
}
