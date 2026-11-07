// SPDX-License-Identifier: AGPL-3.0-only

import { useExecutionStore } from "@/stores/feature/executionStore";

/**
 * 指定 key（会话/执行 ID）的 Agent 状态消息。
 *
 * key 为空时返回 undefined，不产生订阅差异。
 *
 * 活源是 executionStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * executionStore（组件中立性），统一从这里取。
 */
export function useAgentStatus(key: string | undefined): string | undefined {
  return useExecutionStore((s) => (key ? s.agentStatus[key] : undefined));
}
