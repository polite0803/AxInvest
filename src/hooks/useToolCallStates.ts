// SPDX-License-Identifier: AGPL-3.0-only

import { useExecutionStore } from "@/stores/feature/executionStore";

/**
 * 全量工具调用状态表（key 为 toolUseId）。
 *
 * 供需要跨条目遍历的场景（如时间线合并渲染）；只查单个条目的组件
 * 请用 useToolCallState 以缩小订阅面。
 *
 * 活源是 executionStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * executionStore（组件中立性），统一从这里取。
 */
export function useToolCallStates() {
  return useExecutionStore((s) => s.toolCalls);
}

/**
 * 单个工具调用状态（key 为 toolUseId），只订阅该条目切片。
 */
export function useToolCallState(toolUseId: string) {
  return useExecutionStore((s) => s.toolCalls[toolUseId]);
}
