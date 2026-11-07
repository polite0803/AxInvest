// SPDX-License-Identifier: AGPL-3.0-only

import { useDreamStore } from "@/stores/feature/dreamStore";

/**
 * dreamStore 的 Dream 巩固状态读面（DreamStatusIndicator 专用，
 * 组件中立性消键，2026-11 #222 补洞批次）。
 *
 * 活源是 dreamStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * dreamStore（组件中立性），统一从这里取。内部为逐条 selector 订阅
 * （与原组件的订阅粒度一字不差），返回值形态与原 selector 严格一致
 * （totalMemories / totalPatterns 为组件局部名，对应 store 字段
 * totalMemoriesExtracted / totalPatternsDiscovered，仅按原名转发）。
 */
export function useDreamStatus() {
  const status = useDreamStore((s) => s.status);
  const lastResult = useDreamStore((s) => s.lastResult);
  const totalConsolidations = useDreamStore((s) => s.totalConsolidations);
  const totalMemories = useDreamStore((s) => s.totalMemoriesExtracted);
  const totalPatterns = useDreamStore((s) => s.totalPatternsDiscovered);
  return { status, lastResult, totalConsolidations, totalMemories, totalPatterns };
}
