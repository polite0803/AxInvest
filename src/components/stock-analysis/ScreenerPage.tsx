import { PageErrorBoundary } from "@/components/shared/ErrorBoundary";
import { PageHeader } from "./_shared/PageHeader";
import { DragonTigerPanel } from "./DragonTigerPanel";
import { HotStocksPanel } from "./HotStocksPanel";
import { LimitUpPanel } from "./LimitUpPanel";
import { StockScreenerPanel } from "./StockScreenerPanel";

/**
 * ScreenerPage — 选股中心
 * 覆盖:StockScreener(主选股器)+ HotStocks(热门股)+ LimitUp(涨停板)+ DragonTiger(龙虎榜)
 * ConceptBlocksPanel 需要 stockCode,与选股语义不匹配,不在此处复用
 */
export function ScreenerPage() {
  return (
    <PageErrorBoundary title="Screener">
      <div className="flex h-full flex-col">
        <PageHeader titleKey="screener.title" backTo="/stock-analysis" />
        <div className="flex-1 overflow-auto p-4 space-y-4">
          <StockScreenerPanel />
          <div className="grid grid-cols-1 lg:grid-cols-3 gap-4">
            <HotStocksPanel />
            <LimitUpPanel />
            <DragonTigerPanel />
          </div>
        </div>
      </div>
    </PageErrorBoundary>
  );
}
