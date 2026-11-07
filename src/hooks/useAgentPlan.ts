// SPDX-License-Identifier: AGPL-3.0-only

import { usePlanStore } from "@/stores/feature/planStore";
import type { Conversation } from "@/types";
import { useEffect } from "react";

/**
 * planStore 的聊天消费面接入（自 ChatView 抽出，组件中立性消键）：
 * 会话活动计划订阅、agent 模式下活动计划补拉。
 *
 * 活源是 planStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * planStore（组件中立性），统一从这里取。
 */

/** 指定会话的活动计划（订阅；无计划返回 undefined）。 */
export function useActivePlan(conversationId: string) {
  return usePlanStore((s) => s.activePlans[conversationId]);
}

/**
 * 会话处于 agent 模式且活动计划缺失时补拉（挂载 / 会话切换期 effect，
 * effect 体自 ChatView 逐字搬入：getState 不建立订阅，
 * 依赖保持原样的 [activeConversationId, conversations]；
 * 会话 id 允许 null/undefined（与 ChatView 的 activeConversationId 同型，falsy 即跳过））。
 */
export function useEnsureActivePlanLoaded(
  activeConversationId: string | null | undefined,
  conversations: Conversation[],
): void {
  useEffect(() => {
    if (!activeConversationId) {
      return;
    }
    const conversation = conversations.find(
      (c) => c.id === activeConversationId,
    );
    if (conversation?.mode === "agent") {
      const { activePlans, loadActivePlan } = usePlanStore.getState();
      if (!activePlans[activeConversationId]) {
        void loadActivePlan(activeConversationId);
      }
    }
  }, [activeConversationId, conversations]);
}
