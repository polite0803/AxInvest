// SPDX-License-Identifier: AGPL-3.0-only

import { setupAgentEventListeners } from "@/stores/feature/agentStore";
import { setupDreamEventListeners } from "@/stores/feature/dreamStore";
import { setupPlanEventListeners } from "@/stores/feature/planStore";
import { setupTaskShapeApprovalListeners } from "@/stores/feature/taskShapeApprovalStore";
import { useEffect } from "react";

/**
 * 聊天视图挂载期的跨域事件接线（自 ChatView 抽出，组件中立性消键）：
 * 一次性安装 agent / plan / dream / taskShapeApproval 四路 store 事件监听，
 * 卸载时逐一清理。effect 体自 ChatView 逐字搬入（空依赖，行为一致）。
 */
export function useChatEventWiring(): void {
  useEffect(() => {
    const cleanupAgent = setupAgentEventListeners();
    const cleanupPlan = setupPlanEventListeners();
    const cleanupDream = setupDreamEventListeners();
    const cleanupTaskShapeApproval = setupTaskShapeApprovalListeners();
    return () => {
      cleanupAgent();
      cleanupPlan();
      cleanupDream();
      cleanupTaskShapeApproval();
    };
  }, []);
}
