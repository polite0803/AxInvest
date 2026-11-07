// SPDX-License-Identifier: AGPL-3.0-only

import { useStockAnalysisStore } from "@/stores/feature/stockAnalysisStore";

/**
 * 办公（公司模拟）面板读股票分析域的行情读面,按场景拆三档,服务同域多面板:
 *
 * - useOfficeStockQuote —— 交易室迷你条（TradingRoomMiniBar）:报价 + 自动刷新 + 代码搜索全读面。
 * - useOfficeStockContext —— 聊天/私信/轨迹面板（ChatPanel / DirectMessagePanel / TrajectoryPanel）
 *   共用的「当前股票上下文」薄读面（代码/名称/报价/分析状态/时间线）。
 * - useOfficePortfolioRisk —— 风险室迷你条（RiskRoomMiniBar）:组合仪表盘读面。
 *
 * 抽成 hook 是为了让 office 组件不直接依赖 stockAnalysisStore
 * （组件中立性，见 scripts/check-component-neutrality.mjs），统一从这里取。
 */

/** 交易室迷你条:报价 + 自动刷新 + 代码搜索（10 面,与原组件 selector 一致）。 */
export function useOfficeStockQuote() {
  const stockCode = useStockAnalysisStore((s) => s.stockCode);
  const stockName = useStockAnalysisStore((s) => s.stockName);
  const quote = useStockAnalysisStore((s) => s.quote);
  const quoteLoading = useStockAnalysisStore((s) => s.quoteLoading);
  const quoteError = useStockAnalysisStore((s) => s.quoteError);
  const autoRefresh = useStockAnalysisStore((s) => s.autoRefresh);
  const setAutoRefresh = useStockAnalysisStore((s) => s.setAutoRefresh);
  const getStockQuote = useStockAnalysisStore((s) => s.getStockQuote);
  const searchStock = useStockAnalysisStore((s) => s.searchStock);
  const searchResults = useStockAnalysisStore((s) => s.searchResults);
  return {
    stockCode,
    stockName,
    quote,
    quoteLoading,
    quoteError,
    autoRefresh,
    setAutoRefresh,
    getStockQuote,
    searchStock,
    searchResults,
  };
}

/** 聊天/私信/轨迹面板共用的当前股票上下文（代码/名称/报价/分析状态/时间线）。 */
export function useOfficeStockContext() {
  const stockCode = useStockAnalysisStore((s) => s.stockCode);
  const stockName = useStockAnalysisStore((s) => s.stockName);
  const quote = useStockAnalysisStore((s) => s.quote);
  const analysisStatus = useStockAnalysisStore((s) => s.status);
  const timeline = useStockAnalysisStore((s) => s.timeline);
  return { stockCode, stockName, quote, analysisStatus, timeline };
}

/** 风险室迷你条:组合压力测试仪表盘读面。 */
export function useOfficePortfolioRisk() {
  const portfolioDashboard = useStockAnalysisStore((s) => s.portfolioDashboard);
  const portfolioRefreshing = useStockAnalysisStore((s) => s.portfolioRefreshing);
  const portfolioLastError = useStockAnalysisStore((s) => s.portfolioLastError);
  const fetchPortfolioDashboard = useStockAnalysisStore((s) => s.fetchPortfolioDashboard);
  return { portfolioDashboard, portfolioRefreshing, portfolioLastError, fetchPortfolioDashboard };
}
