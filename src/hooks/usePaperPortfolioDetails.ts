// SPDX-License-Identifier: AGPL-3.0-only

import { usePaperPortfolioStore } from "@/stores/feature/paperPortfolioStore";

/**
 * 办公（公司模拟）代币面板读模拟组合域的活跃持仓明细读面。
 *
 * 抽成 hook 是为了让 office（TokenPanel）组件不直接依赖 paperPortfolioStore
 * （组件中立性，见 scripts/check-component-neutrality.mjs），统一从这里取。
 */
export function usePaperPortfolioDetails() {
  const activeDetails = usePaperPortfolioStore((s) => s.activeDetails);
  const fetchActiveDetails = usePaperPortfolioStore((s) => s.fetchActiveDetails);
  const loadingList = usePaperPortfolioStore((s) => s.loadingList);
  return { activeDetails, fetchActiveDetails, loadingList };
}
