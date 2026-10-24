import { PageErrorBoundary } from "@/components/shared/ErrorBoundary";
import { useStockAnalysisStore } from "@/stores/feature/stockAnalysisStore";
import { Tabs } from "antd";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useSearchParams } from "react-router-dom";
import { PageHeader } from "./_shared/PageHeader";
import { SectionHeading } from "./_shared/SectionHeading";
import { BacktestChart } from "./BacktestChart";
import { BacktestPanel } from "./BacktestPanel";
import { HistoricalAnalysisPanel } from "./HistoricalAnalysisPanel";
import { MoverRecallPanel } from "./MoverRecallPanel";
import { PnLHistogram, SectorAllocationDonut } from "./PortfolioCharts";
import { QuickBacktestPanel } from "./QuickBacktestPanel";
import { RecoSignalTimeline } from "./RecoSignalTimeline";
import { RecoStrategyMatrix } from "./RecoStrategyMatrix";
import { SerenityValidationPanel } from "./SerenityValidationPanel";
import { UniverseSamplePanel } from "./UniverseSamplePanel";
import { WhatIfBacktest } from "./WhatIfBacktest";

/**
 * BacktestPage — 回测验证
 * 覆盖:BacktestPanel(全量回测统计)+ HistoricalAnalysisPanel(历史分析列表 + 单次回测)
 * + WhatIfBacktest(参数修改回测)
 * analysisId 可选:BacktestPage 默认无聚焦分析,展示历史列表;有 store 当前 analysis 时,会显示其 blackboard snapshot
 *
 * 模拟仿真三个面板（市场模拟 / 蒙特卡洛 / 量化仿真）已于 2026-09-14 迁出，
 * 现挂在股票分析页的「模拟仿真」标签下（见 `SimulationTabContent.tsx`）——
 * 它们属单股分析动作，不属于回测验证。
 */
export function BacktestPage() {
  const { t } = useTranslation();
  const analysisId = useStockAnalysisStore((s) => s.analysisId);
  const [selectedStrategy, setSelectedStrategy] = useState<string | null>(null);
  const [params] = useSearchParams();
  // D-5：任务中心「妖股漏检核查」历史行带 `?moverRun=<runId>` 过来 ⇒ 必须直接落在策略页签，
  // 否则落地页是「快速回测」，用户还得自己找 —— 那正是这次反馈要消除的找不到。
  const [tab, setTab] = useState<string>(params.get("moverRun") ? "strategy" : "quick");
  return (
    <PageErrorBoundary title={t("stockAnalysis.page.backtest")}>
      <div className="flex h-full flex-col">
        <PageHeader titleKey="stockAnalysis.backtest.title" backTo="/stock-analysis" />
        <div className="flex-1 overflow-auto p-4">
          <Tabs
            activeKey={tab}
            onChange={setTab}
            size="small"
            items={[
              {
                key: "quick",
                label: t("stockAnalysis.backtest.quickBacktest"),
                children: <QuickBacktestPanel />,
              },
              {
                key: "analysis",
                label: t("stockAnalysis.backtest.tabAnalysis"),
                children: (
                  <div className="space-y-4">
                    <BacktestPanel />
                    <WhatIfBacktest />
                    <HistoricalAnalysisPanel analysisId={analysisId ?? ""} />
                  </div>
                ),
              },
              {
                key: "strategy",
                label: t("stockAnalysis.backtest.tabStrategy"),
                children: (
                  /* 五段业务串成一条阅读线：数据基础 → 策略表现 → 信号追踪 →
                     漏检核查 → 趋势智选验证。每段一个编号标题 + 自己的卡，
                     不再把采集按钮与漏检明细塞进矩阵卡内部（2026-10-24 重排）。 */
                  <div className="space-y-4">
                    <SectionHeading
                      step={1}
                      title={t("stockAnalysis.backtest.sectionData")}
                      description={t("stockAnalysis.backtest.sectionDataDesc")}
                    />
                    <UniverseSamplePanel />
                    <SectionHeading
                      step={2}
                      title={t("stockAnalysis.backtest.sectionMatrix")}
                      description={t("stockAnalysis.backtest.sectionMatrixDesc")}
                    />
                    <RecoStrategyMatrix onSelectStrategy={setSelectedStrategy} />
                    <SectionHeading
                      step={3}
                      title={t("stockAnalysis.backtest.sectionSignals")}
                      description={t("stockAnalysis.backtest.sectionSignalsDesc")}
                    />
                    <RecoSignalTimeline strategyId={selectedStrategy ?? ""} />
                    <SectionHeading
                      step={4}
                      title={t("stockAnalysis.backtest.sectionMisses")}
                      description={t("stockAnalysis.backtest.sectionMissesDesc")}
                    />
                    <MoverRecallPanel />
                    <SectionHeading
                      step={5}
                      title={t("stockAnalysis.backtest.sectionSerenity")}
                      description={t("stockAnalysis.backtest.sectionSerenityDesc")}
                    />
                    <SerenityValidationPanel />
                  </div>
                ),
              },
              {
                key: "charts",
                label: t("stockAnalysis.backtest.tabCharts"),
                children: (
                  <div className="space-y-4">
                    <BacktestChart
                      equityCurve={[]}
                      metrics={{
                        strategyName: "",
                        totalReturn: 0,
                        annualizedReturn: 0,
                        sharpe: 0,
                        maxDrawdown: 0,
                        winRate: 0,
                        totalTrades: 0,
                      }}
                    />
                    <SectorAllocationDonut data={[]} />
                    <PnLHistogram data={[]} />
                  </div>
                ),
              },
            ]}
          />
        </div>
      </div>
    </PageErrorBoundary>
  );
}
