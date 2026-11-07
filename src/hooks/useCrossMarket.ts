// SPDX-License-Identifier: AGPL-3.0-only

import { useCrossMarketStore } from "@/stores/feature/crossMarketStore";

/**
 * 跨市场面板全读面（国际行情/K 线 + 基准指数 + 外汇 + 分区加载态与错误）。
 *
 * 抽成 hook 是为了让 stock-analysis（CrossMarketDashboard）组件不直接依赖
 * crossMarketStore（组件中立性，见 scripts/check-component-neutrality.mjs），
 * 统一从这里取。返回形态与原组件解构完全一致，不改任何逻辑。
 */
export function useCrossMarket() {
  const intlQuotes = useCrossMarketStore((s) => s.intlQuotes);
  const intlKlines = useCrossMarketStore((s) => s.intlKlines);
  const benchmarkKlines = useCrossMarketStore((s) => s.benchmarkKlines);
  const forexKlines = useCrossMarketStore((s) => s.forexKlines);
  const loadingQuote = useCrossMarketStore((s) => s.loadingQuote);
  const loadingKline = useCrossMarketStore((s) => s.loadingKline);
  const loadingBenchmark = useCrossMarketStore((s) => s.loadingBenchmark);
  const loadingForex = useCrossMarketStore((s) => s.loadingForex);
  const errors = useCrossMarketStore((s) => s.errors);
  const fetchIntlQuote = useCrossMarketStore((s) => s.fetchIntlQuote);
  const fetchIntlKline = useCrossMarketStore((s) => s.fetchIntlKline);
  const fetchBenchmarkKline = useCrossMarketStore((s) => s.fetchBenchmarkKline);
  const fetchForexKline = useCrossMarketStore((s) => s.fetchForexKline);
  return {
    intlQuotes,
    intlKlines,
    benchmarkKlines,
    forexKlines,
    loadingQuote,
    loadingKline,
    loadingBenchmark,
    loadingForex,
    errors,
    fetchIntlQuote,
    fetchIntlKline,
    fetchBenchmarkKline,
    fetchForexKline,
  };
}

// 测试负控钳制通道:CrossMarketDashboard.failure.test.tsx 必须拿**真实 store** 的
// setState/getState 验证「上游取数失败 → 错误按区归位（errors.benchmark / errors.forex 互不覆盖）」
// 的不变式 —— 若 mock 掉 useCrossMarket,store 的归位逻辑就脱测,负控变假绿。
// 此 re-export 仅供测试钳制使用,组件一律走 useCrossMarket()。
export { useCrossMarketStore };
