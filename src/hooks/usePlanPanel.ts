// SPDX-License-Identifier: AGPL-3.0-only

import { usePlanStore } from "@/stores/feature/planStore";

/**
 * planStore 的 chat 域计划读面（ExecutionTimeline / PlanCard / PlanHistoryPanel
 * 三个 chat 组件共用，组件中立性消键，2026-11 #222 补洞批次）。
 *
 * 活源是 planStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * planStore（组件中立性），统一从这里取。各 hook 内部为逐条 selector 订阅
 * （与原组件的订阅粒度一字不差），返回值形态与原 selector 严格一致。
 */

/** 指定会话的当前活跃计划（ExecutionTimeline / PlanHistoryPanel 专用）。 */
export function useActivePlan(conversationId: string) {
  return usePlanStore((s) => s.activePlans[conversationId]);
}

/**
 * 计划审批动作面（PlanCard 专用）：批准/拒绝/改步/恢复/取消动作引用 +
 * 指定会话的加载态。
 */
export function usePlanActions(conversationId: string) {
  const approvePlan = usePlanStore((s) => s.approvePlan);
  const rejectPlan = usePlanStore((s) => s.rejectPlan);
  const modifyStep = usePlanStore((s) => s.modifyStep);
  const resumePlan = usePlanStore((s) => s.resumePlan);
  const cancelPlan = usePlanStore((s) => s.cancelPlan);
  const loading = usePlanStore((s) => s.loading[conversationId]);
  return { approvePlan, rejectPlan, modifyStep, resumePlan, cancelPlan, loading };
}

/** 空历史稳定引用：原 PlanHistoryPanel 的模块级兜底逐字搬入，避免空数组字面量每次渲染换引用。 */
const _EMPTY: never[] = [];

/**
 * 计划历史面（PlanHistoryPanel 专用）：活跃计划 + 历史清单（空时复用模块级
 * _EMPTY 稳定引用，原 selector 兜底逐字搬入）+ 拉取/恢复动作引用。
 */
export function usePlanHistoryPanel(conversationId: string) {
  const activePlan = usePlanStore((s) => s.activePlans[conversationId]);
  const history = usePlanStore((s) => s.planHistory[conversationId] || _EMPTY);
  const loadPlanHistory = usePlanStore((s) => s.loadPlanHistory);
  const resumePlan = usePlanStore((s) => s.resumePlan);
  return { activePlan, history, loadPlanHistory, resumePlan };
}
