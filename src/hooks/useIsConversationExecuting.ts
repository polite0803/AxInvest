// SPDX-License-Identifier: AGPL-3.0-only

import { useAgentStore } from "@/stores/feature/agentStore";

/**
 * 指定会话的 Agent 是否正在执行（agentStore.isExecuting[conversationId]）。
 *
 * 活源是 agentStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * agentStore（组件中立性），统一从这里取。
 */
export function useIsConversationExecuting(conversationId: string): boolean {
  return !!useAgentStore((s) => s.isExecuting[conversationId]);
}
