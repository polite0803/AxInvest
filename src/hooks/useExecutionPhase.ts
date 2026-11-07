// SPDX-License-Identifier: AGPL-3.0-only

import type { ExecutionPhase } from "@/stores/feature/executionPhaseMachine";
import { useExecutionStore } from "@/stores/feature/executionStore";

/**
 * 指定执行 ID 的工作流执行阶段。
 *
 * executionId 为空时返回 undefined，不产生订阅差异。
 *
 * 活源是 executionStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * executionStore（组件中立性），统一从这里取。
 */
export function useExecutionPhase(
  executionId: string | undefined,
): ExecutionPhase | undefined {
  return useExecutionStore((s) => executionId ? s.phases[executionId] : undefined);
}
