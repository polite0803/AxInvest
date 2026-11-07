// SPDX-License-Identifier: AGPL-3.0-only

import { useStrategyStore } from "@/stores/feature/quant/strategyStore";

/**
 * 办公（公司模拟）策略室迷你条读量化域的策略清单读面。
 *
 * 抽成 hook 是为了让 office（StrategyRoomMiniBar）组件不直接依赖
 * quant 域的 strategyStore（组件中立性，见 scripts/check-component-neutrality.mjs），
 * 统一从这里取。
 */
export function useOfficeStrategies() {
  const strategies = useStrategyStore((s) => s.strategies);
  const isLoading = useStrategyStore((s) => s.isLoading);
  const error = useStrategyStore((s) => s.error);
  const loadStrategies = useStrategyStore((s) => s.loadStrategies);
  return { strategies, isLoading, error, loadStrategies };
}
