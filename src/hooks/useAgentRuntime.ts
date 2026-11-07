// SPDX-License-Identifier: AGPL-3.0-only

import { useAgentStore } from "@/stores/feature/agentStore";
import { useEffect } from "react";

/**
 * agentStore 的 chat 域运行时读面接入（组件中立性消键，2026-11 #222 补洞批次）：
 * AgentStatsPanel / AskUserCard / PermissionCard / PermissionModal /
 * ChatView / ChatViewMessages 六个 chat 组件原先直接
 * `import { useAgentStore } from "@/stores"`，统一改从这里取。
 *
 * 活源是 agentStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * agentStore（组件中立性），统一从这里取。各 hook 内部为逐条 selector
 * 订阅（与原组件的订阅粒度一字不差），返回值形态与原 selector 严格一致。
 */

/**
 * 当前流式消息的查询统计（token 用量 / 成本）。streamingMessageId 为空时返回 null，
 * 与原 AgentStatsPanel 的 selector 逐字一致。
 */
export function useAgentQueryStats(streamingMessageId: string | null | undefined) {
  return useAgentStore((s) => streamingMessageId ? (s.queryStats[streamingMessageId] ?? null) : null);
}

/**
 * Agent 暂停/恢复控制面：暂停、恢复两个动作引用 + 指定会话的暂停态查询方法。
 * （AgentStatsPanel 专用组装；action 引用稳定，订阅不会引发额外重渲。）
 */
export function useAgentPauseResume() {
  const pauseAgent = useAgentStore((s) => s.pauseAgent);
  const resumeAgent = useAgentStore((s) => s.resumeAgent);
  const isAgentPaused = useAgentStore((s) => s.isAgentPaused);
  return { pauseAgent, resumeAgent, isAgentPaused };
}

/** 应答 Agent 主动提问的动作引用（AskUserCard 专用）。 */
export function useRespondAskUser() {
  return useAgentStore((s) => s.respondAskUser);
}

/** 批准工具调用的动作引用（PermissionCard / PermissionModal 专用）。 */
export function useApproveToolUse() {
  return useAgentStore((s) => s.approveToolUse);
}

/** 待审批权限请求数组（PermissionModal 专用）。 */
export function usePendingPermissions() {
  return useAgentStore((s) => s.pendingPermissions);
}

/** 待应答的 Agent 主动提问（ChatViewMessages 专用）。 */
export function useAgentPendingAskUser() {
  return useAgentStore((s) => s.pendingAskUser);
}

/**
 * 指定会话的 Agent 状态文案（ChatView 专用）。会话为空时返回 undefined，
 * 与原 ChatView 的 selector 逐字一致（null / undefined 同走 falsy 分支）。
 */
export function useAgentStatusMessage(conversationId: string | null | undefined) {
  return useAgentStore((s) => (conversationId ? s.agentStatus[conversationId] : undefined));
}

/**
 * Agent 模式会话的工具历史补拉（ChatView 专用，effect 体逐字搬入）。
 */
export function useAgentToolHistoryLoad(
  conversationId: string | null | undefined,
  mode: string | undefined,
): void {
  useEffect(() => {
    if (mode === "agent" && conversationId) {
      useAgentStore.getState().loadToolHistory(conversationId);
    }
  }, [conversationId, mode]);
}
