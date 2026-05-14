import { PageErrorBoundary } from "@/components/shared/ErrorBoundary";
import { useStockAnalysisStore } from "@/stores";
import { Spin } from "antd";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useParams } from "react-router-dom";
import { AnalysisProgress } from "./AnalysisProgress";
import { AnalystReportGrid } from "./AnalystReportGrid";
import { DebatePanel } from "./DebatePanel";
import { DecisionBanner } from "./DecisionBanner";
import { KLineChart } from "./KLineChart";
import { RiskMatrix } from "./RiskMatrix";
import { StockQuoteCard } from "./StockQuoteCard";
import { StockSearchBar } from "./StockSearchBar";

export function StockAnalysisPage() {
  const { t } = useTranslation();
  const { id } = useParams<{ id: string }>();
  const setupEventListener = useStockAnalysisStore((s) => s.setupEventListener);
  const loadAnalysis = useStockAnalysisStore((s) => s.loadAnalysis);
  const status = useStockAnalysisStore((s) => s.status);

  useEffect(() => {
    setupEventListener();
  }, [setupEventListener]);

  useEffect(() => {
    if (id) {
      loadAnalysis(id);
    }
  }, [id, loadAnalysis]);

  return (
    <PageErrorBoundary title={t("error.page")}>
      <div className="flex flex-col h-full p-4 gap-4" style={{ maxWidth: 1400, margin: "0 auto", overflow: "auto" }}>
        <h2 className="text-lg font-semibold">{t("stockAnalysis.title")}</h2>
        <StockSearchBar />
        {status === "loading" && (
          <div className="flex items-center justify-center" style={{ minHeight: 200 }}>
            <Spin size="large" />
          </div>
        )}
        {status !== "idle" && (
          <>
            <AnalysisProgress />
            <StockQuoteCard />
            <KLineChart />
            <AnalystReportGrid />
            <DebatePanel />
            <RiskMatrix />
            <DecisionBanner />
          </>
        )}
      </div>
    </PageErrorBoundary>
  );
}
