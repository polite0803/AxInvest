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
import { useParams, useSearchParams } from "react-router-dom";
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
import { TradePanel } from "./TradePanel";
import { WatchlistPanel } from "./WatchlistPanel";

export function StockAnalysisPage() {
  const { t } = useTranslation();
  const { id } = useParams<{ id: string }>();
  const setupEventListener = useStockAnalysisStore((s) => s.setupEventListener);
  const loadAnalysis = useStockAnalysisStore((s) => s.loadAnalysis);
  const status = useStockAnalysisStore((s) => s.status);
  const getStockQuote = useStockAnalysisStore((s) => s.getStockQuote);
  const getStockKline = useStockAnalysisStore((s) => s.getStockKline);

  const [searchParams] = useSearchParams();

  useEffect(() => {
    setupEventListener();
    // 不 reset: 离开页面再回来时保留分析状态，事件监听器常驻
  }, [setupEventListener]);

  // 从 URL ?code= 参数自动加载行情（对话页触发时使用）
  useEffect(() => {
    const code = searchParams.get("code");
    if (code) {
      getStockQuote(code);
      getStockKline(code, "daily", 120);
    }
  }, [searchParams, getStockQuote, getStockKline]);

  useEffect(() => {
    if (id) {
      loadAnalysis(id).then(() => {
        const code = useStockAnalysisStore.getState().stockCode;
        if (code) {
          getStockQuote(code);
          getStockKline(code, "daily", 120);
        }
      });
    }
  }, [id, loadAnalysis, getStockQuote, getStockKline]);

  const tabItems = [
    {
      key: "market",
      label: (
        <span>
          <LineChartOutlined /> <span className="hidden sm:inline">{t("stockAnalysis.tab.market")}</span>
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
          <TeamOutlined /> <span className="hidden sm:inline">{t("stockAnalysis.tab.analysts")}</span>
        </span>
      ),
      children: <AnalystReportGrid />,
    },
    {
      key: "debate",
      label: (
        <span>
          <SwapOutlined /> <span className="hidden sm:inline">{t("stockAnalysis.tab.debate")}</span>
        </span>
      ),
      children: <DebatePanel />,
    },
    {
      key: "risk",
      label: (
        <span>
          <SafetyCertificateOutlined /> <span className="hidden sm:inline">{t("stockAnalysis.tab.risk")}</span>
        </span>
      ),
      children: <RiskMatrix />,
    },
    {
      key: "decision",
      label: (
        <span>
          <TrophyOutlined /> <span className="hidden sm:inline">{t("stockAnalysis.tab.decision")}</span>
        </span>
      ),
      children: <DecisionBanner />,
    },
  ];

  return (
    <PageErrorBoundary title={t("error.page")}>
      <div
        className="flex flex-col h-full p-2 sm:p-3 lg:p-4 gap-3 lg:gap-4 overflow-auto"
        style={{ maxWidth: 1400, margin: "0 auto" }}
      >
        <h2 className="text-base sm:text-lg font-semibold">{t("stockAnalysis.title")}</h2>

        {/* Main layout: stacked on mobile, side-by-side on desktop */}
        <div className="flex flex-col lg:flex-row gap-3 lg:gap-4" style={{ flex: 1, minHeight: 0 }}>
          {/* Main content area */}
          <div className="flex flex-col gap-3 lg:gap-4 flex-1 min-w-0">
            <StockSearchBar />

            {status === "loading" && (
              <div className="flex flex-col items-center justify-center gap-3" style={{ minHeight: 200 }}>
                <Spin size="large" />
                <span style={{ color: "var(--color-text-secondary)" }}>
                  {t("stockAnalysis.loadingHint")}
                </span>
              </div>
            )}
            {status === "idle" && (
              <div
                className="flex items-center justify-center text-center"
                style={{ minHeight: 300, color: "var(--color-text-secondary)" }}
              >
                <div>
                  <p className="text-base sm:text-lg mb-2">{t("stockAnalysis.emptyHint")}</p>
                  <p className="text-xs">{t("stockAnalysis.emptyHintDetail")}</p>
                </div>
              </div>
            )}
            {status !== "idle" && (
              <>
                <AnalysisProgress />
                <Tabs items={tabItems} defaultActiveKey="market" size="small" />
              </>
            )}
          </div>

          {/* Sidebar: hidden on mobile, visible on lg+ */}
          <div className="hidden lg:flex lg:flex-col gap-2 shrink-0" style={{ width: 260 }}>
            <TradePanel />
            <WatchlistPanel />
            <PriceAlertPanel />
            <CompareView />
          </div>
        </div>

        {/* Collapsible panels for tablet (md) */}
        <div className="lg:hidden grid grid-cols-1 sm:grid-cols-2 gap-2">
          <TradePanel />
          <WatchlistPanel />
        </div>
      </div>
    </PageErrorBoundary>
  );
}
