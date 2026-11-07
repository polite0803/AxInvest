// SPDX-License-Identifier: AGPL-3.0-only

import { useNudgeStore } from "@/stores/feature/nudgeStore";

/**
 * nudgeStore 的洞察/提醒读面（EvolutionSidebar / InsightPanel / NudgePanel
 * 三个 chat 组件共用，组件中立性消键，2026-11 #222 补洞批次）。
 *
 * 活源是 nudgeStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * nudgeStore（组件中立性），统一从这里取。各 hook 内部为逐条 selector 订阅
 * （与原组件的订阅粒度一字不差），返回值形态与原 selector 严格一致。
 */

/** 侧栏徽标/轮询面（EvolutionSidebar 专用；insightCount 对应 s.insights.length，按原名转发）。 */
export function useNudgeSidebarSummary() {
  const insightCount = useNudgeStore((s) => s.insights.length);
  const pendingNudges = useNudgeStore((s) => s.pendingNudges);
  const closedLoopNudges = useNudgeStore((s) => s.closedLoopNudges);
  const fetchInsights = useNudgeStore((s) => s.fetchInsights);
  const fetchClosedLoopNudges = useNudgeStore((s) => s.fetchClosedLoopNudges);
  return { insightCount, pendingNudges, closedLoopNudges, fetchInsights, fetchClosedLoopNudges };
}

/** 洞察列表面（InsightPanel 专用）。 */
export function useNudgeInsightFeed() {
  const insights = useNudgeStore((s) => s.insights);
  const fetchInsights = useNudgeStore((s) => s.fetchInsights);
  const memoryFlush = useNudgeStore((s) => s.memoryFlush);
  return { insights, fetchInsights, memoryFlush };
}

/** 提醒面板全读面（NudgePanel 专用）。 */
export function useNudgePanelData() {
  const pendingNudges = useNudgeStore((s) => s.pendingNudges);
  const closedLoopNudges = useNudgeStore((s) => s.closedLoopNudges);
  const stats = useNudgeStore((s) => s.stats);
  const fetchPendingNudges = useNudgeStore((s) => s.fetchPendingNudges);
  const fetchClosedLoopNudges = useNudgeStore((s) => s.fetchClosedLoopNudges);
  const fetchStats = useNudgeStore((s) => s.fetchStats);
  const dismissNudge = useNudgeStore((s) => s.dismissNudge);
  const snoozeNudge = useNudgeStore((s) => s.snoozeNudge);
  const executeNudge = useNudgeStore((s) => s.executeNudge);
  const acknowledgeClosedLoopNudge = useNudgeStore(
    (s) => s.acknowledgeClosedLoopNudge,
  );
  return {
    pendingNudges,
    closedLoopNudges,
    stats,
    fetchPendingNudges,
    fetchClosedLoopNudges,
    fetchStats,
    dismissNudge,
    snoozeNudge,
    executeNudge,
    acknowledgeClosedLoopNudge,
  };
}
