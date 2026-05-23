import { PageErrorBoundary } from "@/components/shared/ErrorBoundary";
import { useStockAnalysisStore } from "@/stores";
import {
  CaretDownOutlined,
  LineChartOutlined,
  SafetyCertificateOutlined,
  SwapOutlined,
  TeamOutlined,
  TrophyOutlined,
} from "@ant-design/icons";
import { Spin, Tabs } from "antd";
import type { CSSProperties, ReactNode } from "react";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useParams, useSearchParams } from "react-router-dom";
import { AnalysisProgress } from "./AnalysisProgress";
import { AnalystReportGrid } from "./AnalystReportGrid";
import { CompareView } from "./CompareView";
import { DebatePanel } from "./DebatePanel";
import { DecisionBanner } from "./DecisionBanner";
import { HistoricalAnalysisPanel } from "./HistoricalAnalysisPanel";
import { KLineChart } from "./KLineChart";
import { PriceAlertPanel } from "./PriceAlertPanel";
import { RiskMatrix } from "./RiskMatrix";
import { StockQuoteCard } from "./StockQuoteCard";
import { StockSearchBar } from "./StockSearchBar";
import { TradePanel } from "./TradePanel";
import { WatchlistPanel } from "./WatchlistPanel";

/** K 线周期映射：store key → { period, limit } */
const PERIOD_MAP: Record<string, { period: string; limit: number }> = {
  "1m": { period: "daily", limit: 22 },
  "3m": { period: "daily", limit: 66 },
  "6m": { period: "daily", limit: 120 },
  "1y": { period: "daily", limit: 250 },
  "weekly": { period: "weekly", limit: 104 },
  "monthly": { period: "monthly", limit: 60 },
};

/** 可折叠侧栏面板包装器 */
function SidebarPanel({ storeKey, icon, title, children }: {
  storeKey: string;
  icon: ReactNode;
  title: string;
  children: ReactNode;
}) {
  const collapsed = useStockAnalysisStore((s) => s.sidebarCollapsed[storeKey] ?? false);
  const toggle = useStockAnalysisStore((s) => s.toggleSidebarPanel);
  const headerStyle: CSSProperties = {
    cursor: "pointer",
    display: "flex",
    justifyContent: "space-between",
    alignItems: "center",
    userSelect: "none",
    padding: "6px 8px",
    borderRadius: 6,
    background: "var(--color-bg-elevated)",
    fontSize: 12,
    fontWeight: 600,
  };
  const arrowStyle: CSSProperties = {
    transition: "transform 0.2s",
    fontSize: 10,
    transform: collapsed ? "rotate(-90deg)" : "rotate(0deg)",
  };
  return (
    <div style={{ display: "flex", flexDirection: "column" }}>
      <div style={headerStyle} onClick={() => toggle(storeKey)}>
        <span>
          {icon} {title}
        </span>
        <CaretDownOutlined style={arrowStyle} />
      </div>
      {!collapsed && children}
    </div>
  );
}

export function StockAnalysisPage() {
  const { t } = useTranslation();
  const { id } = useParams<{ id: string }>();
  const analysisId = useStockAnalysisStore((s) => s.analysisId);
  const setupEventListener = useStockAnalysisStore((s) => s.setupEventListener);
  const loadAnalysis = useStockAnalysisStore((s) => s.loadAnalysis);
  const status = useStockAnalysisStore((s) => s.status);
  const getStockQuote = useStockAnalysisStore((s) => s.getStockQuote);
  const getStockKline = useStockAnalysisStore((s) => s.getStockKline);
  const klinePeriod = useStockAnalysisStore((s) => s.klinePeriod);

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
      const kp = PERIOD_MAP[klinePeriod] ?? PERIOD_MAP["6m"];
      getStockKline(code, kp.period, kp.limit);
    }
  }, [searchParams, getStockQuote, getStockKline, klinePeriod]);

  useEffect(() => {
    if (id) {
      loadAnalysis(id).then(() => {
        const code = useStockAnalysisStore.getState().stockCode;
        if (code) {
          getStockQuote(code);
          const kp = PERIOD_MAP[useStockAnalysisStore.getState().klinePeriod] ?? PERIOD_MAP["6m"];
          getStockKline(code, kp.period, kp.limit);
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
        className="flex flex-col h-full p-1.5 sm:p-2 lg:p-3 gap-2"
        style={{ maxWidth: 1200, margin: "0 auto" }}
      >
        {/* Main layout: search + progress always on top, content + sidebar below */}
        <StockSearchBar />

        {status === "loading" && (
          <div className="flex flex-col items-center justify-center gap-2" style={{ minHeight: 120 }}>
            <Spin size="default" />
            <span style={{ color: "var(--color-text-secondary)", fontSize: 13 }}>
              {t("stockAnalysis.loadingHint")}
            </span>
          </div>
        )}
        {status === "idle" && (
          <div
            className="flex items-center justify-center text-center"
            style={{ minHeight: 200, color: "var(--color-text-secondary)" }}
          >
            <div>
              <p className="text-sm mb-1">{t("stockAnalysis.emptyHint")}</p>
              <p className="text-xs">{t("stockAnalysis.emptyHintDetail")}</p>
            </div>
          </div>
        )}
        {status !== "idle" && (
          <div className="flex flex-col lg:flex-row gap-2" style={{ flex: 1, minHeight: 0 }}>
            {/* Main content */}
            <div className="flex flex-col gap-2 flex-1 min-w-0">
              <AnalysisProgress />
              <Tabs
                items={tabItems}
                defaultActiveKey="market"
                size="small"
                className="stock-tabs"
                tabBarStyle={{ marginBottom: 8 }}
              />
            </div>

            {/* Sidebar — desktop only (<1024px falls through to tablet grid) */}
            <div className="hidden lg:flex lg:flex-col gap-2 shrink-0" style={{ width: 260 }}>
              <SidebarPanel storeKey="trade" icon={<span>💹</span>} title={t("stockAnalysis.tradingTitle")}>
                <TradePanel />
              </SidebarPanel>
              <SidebarPanel storeKey="watchlist" icon={<span>⭐</span>} title={t("stockAnalysis.watchlist")}>
                <WatchlistPanel />
              </SidebarPanel>
              <SidebarPanel storeKey="compare" icon={<span>📊</span>} title={t("stockAnalysis.compare")}>
                <CompareView />
              </SidebarPanel>
              <SidebarPanel storeKey="alerts" icon={<span>🔔</span>} title={t("stockAnalysis.alert.title")}>
                <PriceAlertPanel />
              </SidebarPanel>
              <SidebarPanel storeKey="history" icon={<span>📜</span>} title={t("stockAnalysis.history")}>
                <HistoricalAnalysisPanel analysisId={analysisId ?? ""} />
              </SidebarPanel>
            </div>
          </div>
        )}

        {/* Panels for tablet (<1024px) — shared data, no double-render */}
        <div className="lg:hidden grid grid-cols-1 sm:grid-cols-2 gap-2" style={{ maxWidth: 800 }}>
          <SidebarPanel storeKey="trade" icon={<span>💹</span>} title={t("stockAnalysis.tradingTitle")}>
            <TradePanel />
          </SidebarPanel>
          <SidebarPanel storeKey="watchlist" icon={<span>⭐</span>} title={t("stockAnalysis.watchlist")}>
            <WatchlistPanel />
          </SidebarPanel>
          <SidebarPanel storeKey="compare" icon={<span>📊</span>} title={t("stockAnalysis.compare")}>
            <CompareView />
          </SidebarPanel>
          <SidebarPanel storeKey="alerts" icon={<span>🔔</span>} title={t("stockAnalysis.alert.title")}>
            <PriceAlertPanel />
          </SidebarPanel>
          <SidebarPanel storeKey="history" icon={<span>📜</span>} title={t("stockAnalysis.history")}>
            <HistoricalAnalysisPanel analysisId={analysisId ?? ""} />
          </SidebarPanel>
        </div>
      </div>
    </PageErrorBoundary>
  );
}
