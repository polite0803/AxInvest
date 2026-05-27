import { StockAnalysisSettings } from "@/components/settings/StockAnalysisSettings";
import { PageErrorBoundary } from "@/components/shared/ErrorBoundary";
import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore, useUIStore } from "@/stores";
import { Collapse, Dropdown } from "antd";
import { ArrowLeftRight, LineChart, Settings, Shield, TrendingUp, Users, X } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate, useParams, useSearchParams } from "react-router-dom";
import { AnalysisProgress } from "./AnalysisProgress";
import { AnalystReportGrid } from "./AnalystReportGrid";
import { CompareView } from "./CompareView";
import { DailyReviewPanel } from "./DailyReviewPanel";
import { DebatePanel } from "./DebatePanel";
import { DecisionBanner } from "./DecisionBanner";
import { DragonTigerPanel } from "./DragonTigerPanel";
import { EventCalendarPanel } from "./EventCalendarPanel";
import { HistoricalAnalysisPanel } from "./HistoricalAnalysisPanel";
import { KLineChart } from "./KLineChart";
import { LimitUpPanel } from "./LimitUpPanel";
import { NorthBoundPanel } from "./NorthBoundPanel";
import { PriceAlertPanel } from "./PriceAlertPanel";
import { RiskMatrix } from "./RiskMatrix";
import { SectorHeatmapPanel } from "./SectorHeatmapPanel";
import { StockAnalysisSettingsModal } from "./StockAnalysisSettingsModal";
import { StockQuoteCard } from "./StockQuoteCard";
import { StockScreenerPanel } from "./StockScreenerPanel";
import { StockSearchBar } from "./StockSearchBar";
import { TradePanel } from "./TradePanel";
import { WatchlistPanel } from "./WatchlistPanel";

const PERIOD_MAP: Record<string, { period: string; limit: number }> = {
  "1m": { period: "daily", limit: 22 },
  "3m": { period: "daily", limit: 66 },
  "6m": { period: "daily", limit: 120 },
  "1y": { period: "daily", limit: 250 },
  "weekly": { period: "weekly", limit: 104 },
  "monthly": { period: "monthly", limit: 60 },
};

interface SheetPanel {
  key: string;
  label: string;
  element: ReactNode;
}

export function StockAnalysisPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { id } = useParams<{ id: string }>();
  const analysisId = useStockAnalysisStore((s) => s.analysisId);
  const setupEventListener = useStockAnalysisStore((s) => s.setupEventListener);
  const loadAnalysis = useStockAnalysisStore((s) => s.loadAnalysis);
  const status = useStockAnalysisStore((s) => s.status);
  const getStockQuote = useStockAnalysisStore((s) => s.getStockQuote);
  const getStockKline = useStockAnalysisStore((s) => s.getStockKline);
  const klinePeriod = useStockAnalysisStore((s) => s.klinePeriod);

  const deviceLayout = useUIStore((s) => s.deviceLayout);
  const isMobile = deviceLayout === "mobile" || deviceLayout === "tablet";

  const [searchParams] = useSearchParams();
  const [activeTab, setActiveTab] = useState("market");
  const [sheetOpen, setSheetOpen] = useState(false);
  const [sheetTab, setSheetTab] = useState("trade");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [marketStatus, setMarketStatus] = useState("");

  useEffect(() => {
    invoke<{ status: string }>("get_market_status").then((r) => setMarketStatus(r.status)).catch(() => {});
  }, []);

  useEffect(() => {
    setupEventListener();
  }, [setupEventListener]);

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

  const tabs = [
    {
      key: "market",
      label: t("stockAnalysis.tab.market"),
      icon: <LineChart size={14} />,
      children: (
        <>
          <StockQuoteCard />
          <KLineChart />
        </>
      ),
    },
    {
      key: "analysts",
      label: t("stockAnalysis.tab.analysts"),
      icon: <Users size={14} />,
      children: <AnalystReportGrid />,
    },
    {
      key: "debate",
      label: t("stockAnalysis.tab.debate"),
      icon: <ArrowLeftRight size={14} />,
      children: <DebatePanel />,
    },
    { key: "risk", label: t("stockAnalysis.tab.risk"), icon: <Shield size={14} />, children: <RiskMatrix /> },
    {
      key: "decision",
      label: t("stockAnalysis.tab.decision"),
      icon: <TrendingUp size={14} />,
      children: <DecisionBanner />,
    },
  ];

  const allSheetPanels: SheetPanel[] = [
    { key: "screener", label: "荐股", element: <StockScreenerPanel /> },
    { key: "limitup", label: "涨停", element: <LimitUpPanel /> },
    { key: "dragontiger", label: "龙虎", element: <DragonTigerPanel /> },
    { key: "sectors", label: "板块", element: <SectorHeatmapPanel /> },
    { key: "north", label: "北向", element: <NorthBoundPanel /> },
    { key: "watchlist", label: t("stockAnalysis.watchlist"), element: <WatchlistPanel /> },
    { key: "trade", label: t("stockAnalysis.tradingTitle"), element: <TradePanel /> },
    { key: "alerts", label: t("stockAnalysis.alert.title"), element: <PriceAlertPanel /> },
    { key: "compare", label: t("stockAnalysis.compare"), element: <CompareView /> },
    {
      key: "history",
      label: t("stockAnalysis.history"),
      element: <HistoricalAnalysisPanel analysisId={analysisId ?? ""} />,
    },
    { key: "review", label: "复盘", element: <DailyReviewPanel /> },
    { key: "events", label: "日历", element: <EventCalendarPanel /> },
  ];
  // 桌面全部显示，移动端前7个核心面板 + 其余通过 tag 切换
  const mobileCoreKeys = ["screener", "limitup", "dragontiger", "sectors", "north", "watchlist", "trade"];
  const sheetPanels = isMobile ? allSheetPanels.filter((p) => mobileCoreKeys.includes(p.key)) : allSheetPanels;

  const activePanel = allSheetPanels.find((p) => p.key === sheetTab);
  const activeContent = tabs.find((t) => t.key === activeTab);

  return (
    <PageErrorBoundary title={t("error.page")}>
      <div className="sa-layout">
        <div className="sa-header">
          <button type="button" className="sa-header-back" onClick={() => navigate("/")}>
            ← {t("nav.chat")}
          </button>
          <h2 className="sa-header-title">{t("stockAnalysis.title")}</h2>
          <span className="sa-header-meta">{marketStatus || t("stockAnalysis.subtitle")}</span>
          <button
            type="button"
            className="sa-header-back"
            onClick={() => setSettingsOpen(!settingsOpen)}
            title={t("stockAnalysis.settings.title")}
            style={settingsOpen && !isMobile ? { background: "var(--accent-bg)", color: "var(--accent)" } : undefined}
          >
            {settingsOpen && !isMobile ? <X size={16} /> : <Settings size={16} />}
          </button>
        </div>

        <div className="sa-body">
          <StockSearchBar />

          <div className="sa-body-inner">
            <div className="sa-main">
              {settingsOpen && !isMobile
                ? (
                  <div className="sa-settings-inline">
                    <div className="sa-settings-header">
                      <span className="sa-settings-title">{t("stockAnalysis.settings.title")}</span>
                      <button type="button" className="sa-header-back" onClick={() => setSettingsOpen(false)}>
                        <X size={14} /> {t("common.close")}
                      </button>
                    </div>
                    <div className="sa-settings-body">
                      <StockAnalysisSettings />
                    </div>
                  </div>
                )
                : (
                  <>
                    {status === "loading" && (
                      <div className="sa-loading">
                        <div className="sa-spinner" />
                        <span style={{ fontSize: 13 }}>{t("stockAnalysis.loadingHint")}</span>
                      </div>
                    )}

                    {status === "idle" && (
                      <div className="sa-empty">
                        <div>
                          <p className="sa-empty-title">{t("stockAnalysis.emptyHint")}</p>
                          <p className="sa-empty-desc">{t("stockAnalysis.emptyHintDetail")}</p>
                        </div>
                      </div>
                    )}

                    {status !== "loading" && status !== "idle" && (
                      <>
                        <AnalysisProgress />

                        <div className="sa-tabs">
                          {tabs.map((tab) => (
                            <button
                              key={tab.key}
                              type="button"
                              className={`sa-tab${tab.key === activeTab ? " active" : ""}`}
                              onClick={() => setActiveTab(tab.key)}
                            >
                              {tab.icon}
                              {tab.label}
                            </button>
                          ))}
                        </div>

                        {activeContent?.children}
                      </>
                    )}
                  </>
                )}
            </div>

            <div className="sa-sidebar">
              <Collapse
                size="small"
                ghost
                defaultActiveKey={["screener"]}
                items={sheetPanels.map((p) => ({
                  key: p.key,
                  label: <span className="text-xs font-medium">{p.label}</span>,
                  children: <div className="sa-panel-body">{p.element}</div>,
                }))}
              />
            </div>
          </div>
        </div>

        {/* 底部滑出面板 — 平板/移动端 */}
        <div className={`sa-bottom-sheet${sheetOpen ? " open" : ""}`}>
          <div className="sa-sheet-handle" onClick={() => setSheetOpen(!sheetOpen)}>
            <div className="sa-sheet-handle-bar" />
          </div>

          <div className="sa-sheet-tabs">
            {sheetPanels.map((p) => (
              <button
                key={p.key}
                type="button"
                className={`sa-sheet-tab${sheetTab === p.key ? " active" : ""}`}
                onClick={() => {
                  setSheetTab(p.key);
                  if (!sheetOpen) { setSheetOpen(true); }
                }}
              >
                {p.label}
              </button>
            ))}
            {isMobile && (
              <Dropdown
                menu={{
                  items: allSheetPanels.filter((p) => !mobileCoreKeys.includes(p.key)).map((p) => ({
                    key: p.key,
                    label: p.label,
                    onClick: () => {
                      setSheetTab(p.key);
                      if (!sheetOpen) { setSheetOpen(true); }
                    },
                  })),
                }}
                trigger={["click"]}
              >
                <button type="button" className="sa-sheet-tab">更多 ▾</button>
              </Dropdown>
            )}
          </div>

          <div className="sa-sheet-body">
            {activePanel?.element}
          </div>
        </div>

        <button
          type="button"
          className="sa-sheet-toggle"
          onClick={() => setSheetOpen(!sheetOpen)}
        >
          {sheetOpen ? "✕" : "+"}
        </button>
      </div>
      {isMobile && <StockAnalysisSettingsModal open={settingsOpen} onClose={() => setSettingsOpen(false)} />}
    </PageErrorBoundary>
  );
}
