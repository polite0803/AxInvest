// SPDX-License-Identifier: AGPL-3.0-only

import { useExecutionStore } from "@/stores/feature/executionStore";
import type { AgentPoolItem } from "@/types";

const EMPTY_POOL: never[] = [];

/**
 * 指定会话的 Agent 池条目列表。
 *
 * 活源是 executionStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * executionStore（组件中立性），统一从这里取。
 */
export function useAgentPool(conversationId: string): AgentPoolItem[] {
  return useExecutionStore((s) => s.agentPool[conversationId] || EMPTY_POOL);
}

/**
 * 新增/更新 Agent 池条目的写操作（store action 引用，引用稳定）。
 */
export function useUpsertPoolItem() {
  return useExecutionStore((s) => s.upsertPoolItem);
}
