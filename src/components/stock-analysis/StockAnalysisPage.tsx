import { PageErrorBoundary } from "@/components/shared/ErrorBoundary";
import { useStockAnalysisStore } from "@/stores";
import {
  LineChartOutlined,
  SafetyCertificateOutlined,
  SwapOutlined,
  TeamOutlined,
  TrophyOutlined,
} from "@ant-design/icons";
import { Spin, Tabs } from "antd";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useParams } from "react-router-dom";
import { AnalysisProgress } from "./AnalysisProgress";
import { AnalystReportGrid } from "./AnalystReportGrid";
import { CompareView } from "./CompareView";
import { DebatePanel } from "./DebatePanel";
import { DecisionBanner } from "./DecisionBanner";
import { KLineChart } from "./KLineChart";
import { PriceAlertPanel } from "./PriceAlertPanel";
import { RiskMatrix } from "./RiskMatrix";
import { StockQuoteCard } from "./StockQuoteCard";
import { StockSearchBar } from "./StockSearchBar";
import { WatchlistPanel } from "./WatchlistPanel";

export function StockAnalysisPage() {
  const { t } = useTranslation();
  const { id } = useParams<{ id: string }>();
  const setupEventListener = useStockAnalysisStore((s) => s.setupEventListener);
  const loadAnalysis = useStockAnalysisStore((s) => s.loadAnalysis);
  const reset = useStockAnalysisStore((s) => s.reset);
  const status = useStockAnalysisStore((s) => s.status);

  useEffect(() => {
    setupEventListener();
    return () => {
      reset();
    };
  }, [setupEventListener, reset]);

  useEffect(() => {
    if (id) {
      loadAnalysis(id);
    }
  }, [id, loadAnalysis]);

  const tabItems = [
    {
      key: "market",
      label: (
        <span>
          <LineChartOutlined /> {t("stockAnalysis.tab.market")}
        </span>
      ),
      children: (
        <>
          <StockQuoteCard />
          <KLineChart />
        </>
      ),
    },
    {
      key: "analysts",
      label: (
        <span>
          <TeamOutlined /> {t("stockAnalysis.tab.analysts")}
        </span>
      ),
      children: <AnalystReportGrid />,
    },
    {
      key: "debate",
      label: (
        <span>
          <SwapOutlined /> {t("stockAnalysis.tab.debate")}
        </span>
      ),
      children: <DebatePanel />,
    },
    {
      key: "risk",
      label: (
        <span>
          <SafetyCertificateOutlined /> {t("stockAnalysis.tab.risk")}
        </span>
      ),
      children: <RiskMatrix />,
    },
    {
      key: "decision",
      label: (
        <span>
          <TrophyOutlined /> {t("stockAnalysis.tab.decision")}
        </span>
      ),
      children: <DecisionBanner />,
    },
  ];

  return (
    <PageErrorBoundary title={t("error.page")}>
      <div className="flex flex-col h-full p-4 gap-4" style={{ maxWidth: 1400, margin: "0 auto", overflow: "auto" }}>
        <h2 className="text-lg font-semibold">{t("stockAnalysis.title")}</h2>
        <div className="flex gap-4" style={{ flex: 1, overflow: "hidden" }}>
          <div className="flex flex-col gap-4" style={{ flex: 1, minWidth: 0 }}>
            <StockSearchBar />
            {status === "loading" && (
              <div className="flex items-center justify-center" style={{ minHeight: 200 }}>
                <Spin size="large" />
              </div>
            )}
            {status === "idle" && (
              <div
                className="flex items-center justify-center text-center"
                style={{ minHeight: 300, color: "var(--color-text-secondary)" }}
              >
                <div>
                  <p className="text-lg mb-2">{t("stockAnalysis.emptyHint")}</p>
                  <p className="text-xs">{t("stockAnalysis.emptyHintDetail")}</p>
                </div>
              </div>
            )}
            {status !== "idle" && (
              <>
                <AnalysisProgress />
                <Tabs items={tabItems} defaultActiveKey="market" />
              </>
            )}
          </div>
          <div className="flex-shrink-0 flex flex-col gap-2" style={{ width: 260 }}>
            <WatchlistPanel />
            <PriceAlertPanel />
            <CompareView />
          </div>
        </div>
      </div>
    </PageErrorBoundary>
  );
}
