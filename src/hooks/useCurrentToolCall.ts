// SPDX-License-Identifier: AGPL-3.0-only

import { useExecutionStore } from "@/stores/feature/executionStore";

/**
 * 当前正在执行（或最近执行）的工具调用追踪。
 *
 * 活源是 executionStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * executionStore（组件中立性），统一从这里取。
 */
export function useCurrentToolCall() {
  return useExecutionStore((s) => s.currentToolCall);
}

/**
 * 指定会话的 Agent 工具调用列表（订阅；自 ChatViewMessages 抽出，组件中立性消键）。
 * 与 useCurrentToolCall 同源（executionStore），chat 组件经此取用。
 */
export function useAgentToolCalls() {
  return useExecutionStore((s) => s.toolCalls);
}
