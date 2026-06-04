import { StockAnalysisSettings } from "@/components/settings/StockAnalysisSettings";
import { PageErrorBoundary } from "@/components/shared/ErrorBoundary";
import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore, useUIStore } from "@/stores";
import { Button, Collapse, Dropdown } from "antd";
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
import { ExecutionReplayPanel } from "./ExecutionReplayPanel";
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
  const loadAnalysis = useStockAnalysisStore((s) => s.loadAnalysis);
  const status = useStockAnalysisStore((s) => s.status);
  const error = useStockAnalysisStore((s) => s.error);
  const failedNodes = useStockAnalysisStore((s) => s.failedNodes);
  const failedNodeErrors = useStockAnalysisStore((s) => s.failedNodeErrors);
  const stockCode = useStockAnalysisStore((s) => s.stockCode);
  const startAnalysis = useStockAnalysisStore((s) => s.startAnalysis);
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
  const [expandedFailedNode, setExpandedFailedNode] = useState<string | null>(null);

  useEffect(() => {
    invoke<{ status: string }>("get_market_status").then((r) => setMarketStatus(r.status)).catch(() => {});
  }, []);

  // 注意：setupEventListener 改为仅在 startAnalysis 内调用。
  // 之前 L76-78 在页面挂载时也调用，与 startAnalysis 的 await setupEventListener
  // 存在竞态——两组 listen 会同时进行，后一次 set({_unlisten}) 覆盖前一次，
  // 前一次的 3 个监听句柄变成孤儿（永远不会被 unlisten）。
  // 现在挂载时不再注册，只在用户点击"开始分析"时由 startAnalysis 负责注册。

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
    { key: "screener", label: t("stockAnalysis.settings.sheet.screener"), element: <StockScreenerPanel /> },
    { key: "limitup", label: t("stockAnalysis.settings.sheet.limitUp"), element: <LimitUpPanel /> },
    { key: "dragontiger", label: t("stockAnalysis.settings.sheet.dragonTiger"), element: <DragonTigerPanel /> },
    { key: "sectors", label: t("stockAnalysis.settings.sheet.sectors"), element: <SectorHeatmapPanel /> },
    { key: "north", label: t("stockAnalysis.settings.sheet.north"), element: <NorthBoundPanel /> },
    { key: "watchlist", label: t("stockAnalysis.watchlist"), element: <WatchlistPanel /> },
    { key: "trade", label: t("stockAnalysis.tradingTitle"), element: <TradePanel /> },
    { key: "alerts", label: t("stockAnalysis.alert.title"), element: <PriceAlertPanel /> },
    { key: "compare", label: t("stockAnalysis.compare"), element: <CompareView /> },
    {
      key: "history",
      label: t("stockAnalysis.history"),
      element: <HistoricalAnalysisPanel analysisId={analysisId ?? ""} />,
    },
    { key: "review", label: t("stockAnalysis.settings.sheet.review"), element: <DailyReviewPanel /> },
    { key: "events", label: t("stockAnalysis.settings.sheet.events"), element: <EventCalendarPanel /> },
    { key: "replay", label: t("workEngine.executionHistory"), element: <ExecutionReplayPanel /> },
  ];
  // 桌面全部显示，移动端前7个核心面板 + 其余通过"更多"下拉菜单访问
  // 移动端只直接显示 7 个核心面板（满足股市日常扫盘需求），其余 6 个面板
  // （alerts, compare, history, review, events, replay）通过"更多"下拉菜单访问。
  // 总面板数 = 7 + 6 = 13。
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

                    {status === "error" && (
                      <div
                        style={{
                          padding: 16,
                          margin: 16,
                          border: "1px solid var(--sa-red)",
                          borderRadius: 8,
                          background: "var(--surface)",
                        }}
                      >
                        <h3 style={{ margin: "0 0 8px 0", color: "var(--sa-red)" }}>
                          {failedNodes.length > 0
                            ? t("stockAnalysis.workflow.partialFailed", { count: failedNodes.length })
                            : t("stockAnalysis.workflow.startFailed")}
                        </h3>
                        <p style={{ margin: "0 0 12px 0", color: "var(--muted)", whiteSpace: "pre-wrap" }}>
                          {error ?? t("common.unknownError")}
                        </p>
                        {failedNodes.length > 0 && (
                          <div style={{ marginBottom: 12 }}>
                            <div style={{ fontSize: 12, fontWeight: 600, color: "var(--sa-red)", marginBottom: 4 }}>
                              {t("stockAnalysis.workflow.failedSteps")}
                            </div>
                            <div style={{ display: "flex", flexWrap: "wrap", gap: 4 }}>
                              {failedNodes.map((id) => (
                                <div key={id}>
                                  <span
                                    onClick={() =>
                                      setExpandedFailedNode(
                                        expandedFailedNode === id ? null : id,
                                      )}
                                    style={{
                                      fontSize: 11,
                                      padding: "2px 6px",
                                      borderRadius: 4,
                                      background: "var(--sa-red-bg)",
                                      color: "var(--sa-red)",
                                      cursor: "pointer",
                                      display: "inline-block",
                                    }}
                                  >
                                    {id} {expandedFailedNode === id ? "▲" : "▼"}
                                  </span>
                                  {expandedFailedNode === id && (
                                    <div
                                      style={{
                                        marginTop: 4,
                                        padding: 8,
                                        background: "var(--surface)",
                                        borderRadius: 6,
                                        border: "1px solid var(--sa-red)",
                                        fontSize: 11,
                                        color: "var(--muted)",
                                        whiteSpace: "pre-wrap",
                                        lineHeight: 1.5,
                                      }}
                                    >
                                      {failedNodeErrors[id] || error || t("common.unknownError")}
                                    </div>
                                  )}
                                </div>
                              ))}
                            </div>
                          </div>
                        )}
                        <Button
                          type="primary"
                          disabled={!stockCode}
                          onClick={() => stockCode && startAnalysis(stockCode)}
                        >
                          {t("common.retry")}
                        </Button>
                      </div>
                    )}

                    {status !== "loading" && status !== "idle" && status !== "error" && (
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
                <button type="button" className="sa-sheet-tab">{t("stockAnalysis.settings.sheet.more")} ▾</button>
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
