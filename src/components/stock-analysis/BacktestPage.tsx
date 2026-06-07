import { PageErrorBoundary } from "@/components/shared/ErrorBoundary";
import { useStockAnalysisStore } from "@/stores/feature/stockAnalysisStore";
import { PageHeader } from "./_shared/PageHeader";
import { BacktestPanel } from "./BacktestPanel";
import { HistoricalAnalysisPanel } from "./HistoricalAnalysisPanel";

/**
 * BacktestPage — 回测验证
 * 覆盖:BacktestPanel(全量回测统计)+ HistoricalAnalysisPanel(历史分析列表 + 单次回测)
 * analysisId 可选:BacktestPage 默认无聚焦分析,展示历史列表;有 store 当前 analysis 时,会显示其 blackboard snapshot
 */
export function BacktestPage() {
  const analysisId = useStockAnalysisStore((s) => s.analysisId);
  return (
    <PageErrorBoundary title="Backtest">
      <div className="flex h-full flex-col">
        <PageHeader titleKey="backtest.title" backTo="/stock-analysis" />
        <div className="flex-1 overflow-auto p-4 space-y-4">
          <BacktestPanel />
          <HistoricalAnalysisPanel analysisId={analysisId ?? ""} />
        </div>
      </div>
    </PageErrorBoundary>
  );
}
