// SPDX-License-Identifier: AGPL-3.0-only

import { useAgentStore } from "@/stores/feature/agentStore";
import { useExecutionStore } from "@/stores/feature/executionStore";
import { usePlanStore } from "@/stores/feature/planStore";

/**
 * Agent→chat 模式切换时的三联清理动作接入（自 InputArea 抽出，组件中立性消键）：
 * 清 agent 会话态、执行态、活动计划，防止陈旧 UI 状态残留。
 * 三个 action 均为 store 创建期绑定的稳定引用，可安全进入 useCallback 依赖。
 */
export function useAgentModeClearActions() {
  const clearAgentConversation = useAgentStore((s) => s.clearConversation);
  const clearExecutionConversation = useExecutionStore(
    (s) => s.clearConversation,
  );
  const clearActivePlan = usePlanStore((s) => s.clearActivePlan);
  return { clearAgentConversation, clearExecutionConversation, clearActivePlan };
}
