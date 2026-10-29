import { StockAnalysisSettings } from "@/components/settings/StockAnalysisSettings";
import { PageErrorBoundary } from "@/components/shared/ErrorBoundary";
import { useShowAdvancedPanels } from "@/components/stock-workspace/_shared/useSimpleMode";
import { PageTimeAnchor } from "@/components/time-travel/PageTimeAnchor";
import { extractLlmField } from "@/lib/agentOutput";
import { parseDataQualityReport } from "@/lib/dataQualityDiagnosis";
import { investPath } from "@/lib/investRoutes";
import { invoke } from "@/lib/invoke";
import { horizonLabelKey } from "@/lib/stock-analysis-utils";
import { useStockAnalysisStore, useUIStore } from "@/stores";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import { Button, Collapse, Dropdown, Segmented } from "antd";
import {
  ArrowLeftRight,
  Coins,
  FlaskConical,
  LayoutDashboard,
  LineChart,
  RotateCcw,
  Settings,
  Shield,
  Sparkles,
  SplitSquareHorizontal,
  Users,
  X,
} from "lucide-react";
import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useLocation, useNavigate, useSearchParams } from "react-router-dom";
import { useShallow } from "zustand/react/shallow";
import { AnalysisDebugPanel } from "./AnalysisDebugPanel";
import { AnalysisProgress } from "./AnalysisProgress";
import { AnalystReportGrid } from "./AnalystReportGrid";
import { AnnouncementsPanel } from "./AnnouncementsPanel";
import { ClsFlashPanel } from "./ClsFlashPanel";
import { ConceptBlocksPanel } from "./ConceptBlocksPanel";
import { DashboardReportPreview } from "./DashboardReportPreview";
import { DebatePanel } from "./DebatePanel";
import { DecisionBanner } from "./DecisionBanner";
import { ExperimentSidebar } from "./ExperimentSidebar";
import { ExperimentTrail } from "./ExperimentTrail";
import { resolveTimelineJump } from "./timelineJump";
import { useAnalystConsensusByTier } from "./useAnalystConsensusByTier";
import "./dual-view";
import "./stock-analysis.css";
import { AnalysisHistoryButton } from "./AnalysisHistoryButton";
import { DualViewRenderer } from "./dual-view";
import { EventCalendarPanel } from "./EventCalendarPanel";
import { EvidenceCitationPanel } from "./EvidenceCitationPanel";
import { VendorHealthDashboard } from "./VendorHealthDashboard";

/** 决策双视角 Tab 内容：从 store 取 LLM vs 公式决策数据传入 DualViewRenderer */
function DecisionComparisonTabContent() {
  // 窄订阅（2026-10-17）：原为 `useStockAnalysisStore()` 无 selector，等于订阅整个 store。
  // zustand v5 无默认浅比较 ⇒ 工作流运行期间每次 set 都会重渲染本 Tab 内容及其子树
  // （DualViewRenderer 含多档决策卡片）；改成 useShallow 只订阅实际用到的 3 个字段。
  const store = useStockAnalysisStore(
    useShallow((s) => ({
      decision: s.decision,
      llmDecisionJson: s.llmDecisionJson,
      decisionAgreementScore: s.decisionAgreementScore,
    })),
  );
  const dualViewData = {
    decisionAction: store.decision?.action,
    // V76: 持仓状态轴（来自 portfolio-mgr.rhai 的 positionState）
    decisionPositionState: store.decision?.positionState ?? null,
    decisionPositionPct: store.decision?.positionPct,
    confidence: store.decision?.confidence,
    adjustedConfidence: store.decision?.adjustedConfidence ?? null,
    decisionReasoning: store.decision?.reasoning ?? null,
    llmDecisionAction: (extractLlmField(store.llmDecisionJson, "action") as string | null)
      ?? (extractLlmField(store.llmDecisionJson, "stance") as string | null)
      ?? null,
    llmDecisionPositionPct: (extractLlmField(store.llmDecisionJson, "positionPct") as number | null) ?? null,
    llmConfidence: (extractLlmField(store.llmDecisionJson, "confidence") as number | null) ?? null,
    llmDecisionReasoning: (extractLlmField(store.llmDecisionJson, "reasoning") as string | null)
      ?? (extractLlmField(store.llmDecisionJson, "summary") as string | null)
      ?? null,
    decisionAgreementScore: store.decisionAgreementScore,
    agreementBreakdown: store.decision?.agreementBreakdown ?? null,
    // 阶段2：四周期独立决策 —— 决策 Tab 按档并列展示（`DecisionComparisonPanel` 渲染）
    decisionsByHorizon: store.decision?.decisionsByHorizon ?? null,
    // v180：独立公式对照（f7 不读 trader）—— 只读展示，不参与任何生产动作
    f7ReferenceFormula: store.decision?.f7ReferenceFormula ?? null,
    f7Source: store.decision?.f7Source ?? null,
  };
  return <DualViewRenderer id="decision-comparison" data={dualViewData} defaultMode="panel" />;
}

/** 决策仪表盘标签页内容 */
function DashboardTabContent() {
  const dashboardReport = useStockAnalysisStore((s) => s.dashboardReport);
  const dashboardMd = useStockAnalysisStore((s) => s.dashboardMd);
  const { t } = useTranslation();

  if (!dashboardReport) {
    return (
      <div style={{ padding: 24, textAlign: "center", color: "var(--muted)" }}>
        {t("stockAnalysis.dashboard.empty")}
      </div>
    );
  }
  return (
    <div>
      <DashboardReportPreview report={dashboardReport} />
      {dashboardMd && (
        <details style={{ marginTop: 16, padding: "0 16px" }}>
          <summary style={{ cursor: "pointer", color: "var(--muted)" }}>
            {t("stockAnalysis.dashboard.viewMarkdown")}
          </summary>
          <pre style={{ background: "var(--surface-2)", padding: 12, borderRadius: 8, overflow: "auto", fontSize: 12 }}>
            {dashboardMd}
          </pre>
        </details>
      )}
    </div>
  );
}

import { EvolutionDriftPanel } from "./EvolutionDriftPanel";
import { IndustryRankingPanel } from "./IndustryRankingPanel";
import { InvestDashboard } from "./InvestDashboard";
import { KLineChart } from "./KLineChart";
import { MarketMainlinePanel } from "./MarketMainlinePanel";
import { NorthBoundPanel } from "./NorthBoundPanel";
import { OptionPcrPanel } from "./OptionPcrPanel";
import { ReflectionPanel } from "./ReflectionPanel";
import { RiskMatrix } from "./RiskMatrix";
import { SectorHeatmapPanel } from "./SectorHeatmapPanel";
import { SimulationTabContent } from "./SimulationTabContent";
import { StockAnalysisPageContext } from "./StockAnalysisPageContext";
import { StockAnalysisSettingsModal } from "./StockAnalysisSettingsModal";
import { StockQuoteCard } from "./StockQuoteCard";
import { StockSearchBar } from "./StockSearchBar";
import { ValueAssessmentPanel } from "./ValueAssessmentPanel";

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

export function StockAnalysisPage({ embeddedInWorkspace }: { embeddedInWorkspace?: boolean }) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  // 唯一的进入路径：`/invest?tab=workspace&view=analysis&analysisId=xxx`
  // （InvestHub 工作区）；`analysisId` 来自 searchParams。
  //
  // 历史上这里还有一条 `routeId ?? …` 的兜底（旧路由 `/stock-analysis/:id` 的
  // `useParams()`）。该路由已于 2026-10-28 从 ContentArea 删除 ⇒ `routeId` 恒为
  // `undefined`，属于**永不成立的分支**；留着它会让读者以为还有第二条入口，
  // 也会让「记录详情从哪进」的排查多绕一圈。已删除（2026-10-29）。
  const id = searchParams.get("analysisId");
  const loadAnalysis = useStockAnalysisStore((s) => s.loadAnalysis);
  const status = useStockAnalysisStore((s) => s.status);
  const [showDebug, setShowDebug] = useState(false);
  const error = useStockAnalysisStore((s) => s.error);
  const failedNodes = useStockAnalysisStore((s) => s.failedNodes);
  const failedNodeErrors = useStockAnalysisStore((s) => s.failedNodeErrors);
  const stockCode = useStockAnalysisStore((s) => s.stockCode);
  const startAnalysis = useStockAnalysisStore((s) => s.startAnalysis);
  const getStockQuote = useStockAnalysisStore((s) => s.getStockQuote);
  const getStockKline = useStockAnalysisStore((s) => s.getStockKline);
  const klinePeriod = useStockAnalysisStore((s) => s.klinePeriod);
  const analysisId = useStockAnalysisStore((s) => s.analysisId);

  const deviceLayout = useUIStore((s) => s.deviceLayout);
  const isMobile = deviceLayout === "mobile" || deviceLayout === "tablet";

  // 时间旅行: 监听全局 TimeAnchor,用于 sa-header 的 L2 视觉信号(回放半透紫色遮罩)
  const timeAnchorMode = useTimeAnchorStore((s) => s.mode);
  const isReplay = timeAnchorMode === "replay" || timeAnchorMode === "backtest_sweep";

  // 在工作区壳层内使用时：
  //   - 跳过  sa-header（外层已有自己的标题栏和 PageTimeAnchor）
  //   - 跳过  InvestDashboard idle 态（外层控制何时开始分析）
  //   - 保留  sa-tabs、搜索栏、进度条、内容面板
  const isInWorkspace = embeddedInWorkspace ?? useLocation().pathname.startsWith("/workspace");
  // 次级信息（共识 / 实验轨迹 / 证据审计 / 调试）跟随工作区的简洁↔专业密度；
  // 判据（含「没有开关就恒展开」）单一收在 useShowAdvancedPanels。
  const showAdvancedPanels = useShowAdvancedPanels(isInWorkspace);
  const [activeTab, setActiveTab] = useState("market");
  const [sheetOpen, setSheetOpen] = useState(false);
  const [sheetTab, setSheetTab] = useState("trade");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsDefaultTab, setSettingsDefaultTab] = useState<string | undefined>(undefined);
  const [marketStatus, setMarketStatus] = useState("");
  const [expandedFailedNode, setExpandedFailedNode] = useState<string | null>(null);
  // 记录已按 URL 预填过的股票代码：工作区壳层每次切视图都会 setSearchParams(?view=…)，
  // searchParams 引用随之变化会重跑下面的预填 effect，无此守卫会重复拉行情 / K 线。
  const prefilledCodeRef = useRef<string | null>(null);

  const openDataSourceSettings = useCallback(() => {
    setSettingsDefaultTab("data");
    setSettingsOpen(true);
  }, []);

  useEffect(() => {
    let cancelled = false;
    invoke<{ status: string }>("get_market_status").then((r) => {
      if (!cancelled) { setMarketStatus(r.status); }
    }).catch((e) => {
      if (!cancelled) { console.warn("[StockAnalysis] get_market_status failed:", e); }
    });
    return () => {
      cancelled = true;
    };
  }, []);

  // 注意：setupEventListener 改为仅在 startAnalysis 内调用。
  // 之前 L76-78 在页面挂载时也调用，startAnalysis 已 await setupEventListener，
  // 存在竞态——两个 listen 会同时进行，后一个 set({_unlisten}) 覆盖前一次，
  // 前一次的 3 个监听句柄变成孤儿（永远不会 unlisten）。
  // 现在挂载时不再注册，只在用户点击"开始分析"时由 startAnalysis 负责注册。

  // ── URL 股票代码/名称 → 分析页预填 ──
  // 两个参数名都指向同一语义（当前股票代码），必须同时接受：
  //   - ?code=      → 等价参数名（历史上由旧 `/stock-analysis?code=` 入口写入）；
  //                   旧路由已于 2026-10-28 删除，`useStockJump` 现直接产出 stockCode
  //   - ?stockCode= → InvestHub 内跳转（useStockJump 统一写入）直接改写 query
  // 此前只认 code，而 InvestHub 内跳转只写 stockCode，导致「能切到分析页但搜索栏/行情不预填」。
  // ?stockName= 是可选增强：由 useStockJump 写入，有它就不必等行情回来才知道名称。
  useEffect(() => {
    let cancelled = false;
    const code = searchParams.get("code") ?? searchParams.get("stockCode");
    if (code) {
      if (prefilledCodeRef.current === code) { return; }
      prefilledCodeRef.current = code;
      // 立即把 code（有名称时是「名称 (代码)」）写入 searchKeyword，让 StockSearchBar 输入框显示
      const urlName = searchParams.get("stockName");
      useStockAnalysisStore.setState({
        searchKeyword: urlName ? `${urlName} (${code})` : code,
      });
      getStockQuote(code).then(() => {
        if (cancelled) { return; }
        // getStockQuote 完成后，stockName 已写入 store，把 searchKeyword 更新为 "名称(code)" 格式
        const { stockCode, stockName } = useStockAnalysisStore.getState();
        if (stockCode) {
          useStockAnalysisStore.setState({
            searchKeyword: stockName ? `${stockName} (${stockCode})` : stockCode,
          });
        }
      });
      const kp = PERIOD_MAP[klinePeriod] ?? PERIOD_MAP["6m"];
      getStockKline(code, kp.period, kp.limit).then(() => {
        if (cancelled) { return; }
      });
    }
    return () => {
      cancelled = true;
    };
  }, [searchParams, getStockQuote, getStockKline, klinePeriod]);

  useEffect(() => {
    if (!id) { return; }
    // ── 同步预填：避免 loadAnalysis 异步期间(或失败时)占位卡显示"搜索股票"按钮 ──
    // 用户从 AnalysisHistoryButton 进入时,history 缓存必有该条,
    // 立即把 stockCode / stockName / analysisId 写进 store,
    // DecisionBanner 占位卡就能直接渲染"重跑分析"按钮,
    // 而不是默认空 stockCode 走"搜索股票"分支。
    const state = useStockAnalysisStore.getState();
    const cached = state.history.find((h) => h.id === id);
    useStockAnalysisStore.setState({
      analysisId: id,
      ...(cached
        ? { stockCode: cached.stockCode, stockName: cached.stockName }
        : {}),
    });

    let cancelled = false;
    loadAnalysis(id).then(() => {
      if (cancelled) { return; }
      const code = useStockAnalysisStore.getState().stockCode;
      if (code) {
        getStockQuote(code);
        const kp = PERIOD_MAP[useStockAnalysisStore.getState().klinePeriod] ?? PERIOD_MAP["6m"];
        getStockKline(code, kp.period, kp.limit);
      }
    }).catch((e) => {
      if (cancelled) { return; }
      // 修复:loadAnalysis 失败时把错误暴露到 store,占位卡渲染"重试"按钮
      // 而不是永远停在"搜索股票"分支(用户认为这违反"history 已有 stockCode"的预期)
      const msg = e instanceof Error ? e.message : String(e);
      console.error("[StockAnalysis] loadAnalysis failed:", msg);
      useStockAnalysisStore.setState({ error: msg });
    });
    return () => {
      cancelled = true;
    };
  }, [id, loadAnalysis, getStockQuote, getStockKline]);

  // 离开分析页时重置时间锚点为 live，避免影响其他页面（如选股/荐股）
  useEffect(() => {
    return () => {
      const ta = useTimeAnchorStore.getState();
      if (ta.mode !== "live") {
        ta.enterLive(false);
      }
    };
  }, []);

  // Decision Timeline 证据芯片 → 切换主 tab / 打开 sheet panel / 滚到决策 banner
  // (useRightPanel 派发的 timeline-jump 事件)
  // 格式: timelineJump = "<tabKey>:<panelKey>"
  //   tabKey  = "market" | "analyze" | "execute"   (抽象层，由 evidence 表定义)
  //   panelKey = evidence 指向的"具体面板"，可能落在：
  //     - 主区 tab（market/analysts/debate/value/risk/reflection/evolution）
  //     - 侧栏 sheet panel（concepts/announcements/...）
  //     - 决策 hero（decision，顶部 DecisionBanner）
  //     - 交易页（trade，跳转路由）
  useEffect(() => {
    const handle = () => {
      const raw = searchParams.get("timelineJump");
      if (!raw) { return; }
      const [tabKey, panelKey] = raw.split(":");
      const plan = resolveTimelineJump(tabKey, panelKey);
      if (plan.activeTab) { setActiveTab(plan.activeTab); }
      if (plan.sheetTab) {
        setSheetTab(plan.sheetTab);
        if (!sheetOpen) { setSheetOpen(true); }
      }
      if (plan.navigateTo) { navigate(plan.navigateTo); }
      if (plan.scrollTo) {
        const el = document.getElementById(plan.scrollTo);
        el?.scrollIntoView({ behavior: "smooth", block: "start" });
      }
    };
    window.addEventListener("timeline-jump", handle);
    return () => window.removeEventListener("timeline-jump", handle);
  }, [searchParams, navigate, sheetOpen]);

  const tabs = useMemo(() => [
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
    {
      key: "value",
      label: t("stockAnalysis.tab.value"),
      icon: <Coins size={14} />,
      children: <ValueAssessmentPanel />,
    },
    { key: "risk", label: t("stockAnalysis.tab.risk"), icon: <Shield size={14} />, children: <RiskMatrix /> },
    {
      key: "decision-comparison",
      label: t("stockAnalysis.tab.decision"),
      icon: <SplitSquareHorizontal size={14} />,
      children: <DecisionComparisonTabContent />,
    },
    {
      key: "dashboard",
      label: t("stockAnalysis.tab.dashboard"),
      icon: <LayoutDashboard size={14} />,
      children: <DashboardTabContent />,
    },
    {
      // 模拟仿真：三个仿真面板（市场微观结构 / 压力测试 / 策略沙盒）。
      // 原挂在回测页（BacktestPage）的 tab 上，2026-09-14 迁入分析页 ——
      // 仿真回答的是「这仓位建得起吗、最坏能坏到哪」，属单股分析动作，不是回测验证。
      //
      // 位置：**决策 → 仪表盘之后**（同日两次调整，此前先误排 `risk` 之后、再误插决策与仪表盘之间）。
      // 迁入时仅按「归类」（属分析动作）定位，未判定它在叙事链中的位置；而三处
      // **相互独立**的语义依据都把它定义在决策之后：
      //   ① 图上位置 —— `commands/sim-verify.rhai:1`「sim-verify 节点体（**图上位于决策之后**）」；
      //   ② 后端命名 —— `stock_workflow/sim_hook.rs::spawn_simulation_after_decision`
      //      （调用点 `decision.rs` 决策落库后、`core.rs` 主链同点）；
      //   ③ 界面文案 —— `simulation.autoTitle` =「工作流自动仿真（**决策之后**）」。
      // 本 tab 的 label 恰是「决策」，用户读 tab 序列即读步骤顺序 ⇒ 排在决策之前会与真实
      // 执行时序相反（真实时序：决策落库 → 挂钩触发仿真）。
      //
      // 为何最终落在**仪表盘之后**而不是紧跟决策：仿真验证是对**整份分析结论**的独立复核，
      // 排在汇总性的仪表盘之后 —— 叙事为「看完结论 → 再做独立验证」；同时不打断
      // 「决策 → 仪表盘」这条最紧的相邻关系（决策产出即仪表盘汇总的内容）。
      key: "simulation",
      label: t("stockAnalysis.tab.simulation"),
      icon: <FlaskConical size={14} />,
      children: <SimulationTabContent />,
    },
    // Decision tab removed — now rendered as full-width hero at top
    {
      key: "reflection",
      label: t("stockAnalysis.tab.reflection"),
      icon: <RotateCcw size={14} />,
      children: <ReflectionPanel />,
    },
    {
      key: "evolution",
      label: t("stockAnalysis.tab.evolution"),
      icon: <Sparkles size={14} />,
      children: <EvolutionDriftPanel />,
    },
  ], [t]);

  // [Phase 2] 决策一致性胶囊 "切换 tab" 事件（tabs 定义之后，确保引用有效性）
  useEffect(() => {
    const handleSwitchTab = (e: Event) => {
      const tab = (e as CustomEvent).detail;
      if (typeof tab === "string" && tabs.some((t) => t.key === tab)) {
        setActiveTab(tab);
      }
    };
    window.addEventListener("switch-tab", handleSwitchTab);
    return () => window.removeEventListener("switch-tab", handleSwitchTab);
  }, [tabs]);

  const allSheetPanels: SheetPanel[] = [
    { key: "sectors", label: t("stockAnalysis.settings.sheet.sectors"), element: <SectorHeatmapPanel /> },
    { key: "north", label: t("stockAnalysis.settings.sheet.north"), element: <NorthBoundPanel /> },
    { key: "events", label: t("stockAnalysis.settings.sheet.events"), element: <EventCalendarPanel /> },
    {
      key: "announcements",
      label: t("stockAnalysis.announcements"),
      element: <AnnouncementsPanel stockCode={stockCode} />,
    },
    { key: "concepts", label: t("stockAnalysis.conceptBlocks"), element: <ConceptBlocksPanel stockCode={stockCode} /> },
    { key: "optionpcr", label: t("stockAnalysis.optionPcr"), element: <OptionPcrPanel stockCode={stockCode} /> },
    { key: "industry", label: t("stockAnalysis.industryRanking"), element: <IndustryRankingPanel /> },
    { key: "mainlines", label: t("backtest.mainlines.title", { days: 7 }), element: <MarketMainlinePanel /> },
    { key: "flash", label: t("stockAnalysis.clsFlash"), element: <ClsFlashPanel /> },
  ];
  // 桌面全部显示，移动端只直接显示 3 个核心面板，其余通过"更多"下拉菜单访问
  // 注: 大盘指数和持仓表已提升为主区域常驻面板，不再放在底部 Sheet
  const mobileCoreKeys = [
    "sectors",
    "north",
    "events",
  ];
  const sheetPanels = isMobile ? allSheetPanels.filter((p) => mobileCoreKeys.includes(p.key)) : allSheetPanels;

  const activePanel = allSheetPanels.find((p) => p.key === sheetTab);
  const activeContent = tabs.find((t) => t.key === activeTab);

  return (
    <PageErrorBoundary title={t("error.page")}>
      <StockAnalysisPageContext.Provider value={{ openDataSourceSettings }}>
        <div className="sa-layout" style={isInWorkspace ? { height: "auto", overflow: "visible" } : undefined}>
          {!isInWorkspace && (
            <div className="sa-header">
              <button type="button" className="sa-header-back" onClick={() => navigate("/")}>
                ? {t("nav.chat")}
              </button>
              <h2 className="sa-header-title">{t("stockAnalysis.title")}</h2>
              {marketStatus && <span className="sa-header-meta">{marketStatus}</span>}
              <PageTimeAnchor />
              <button
                type="button"
                className="sa-header-back"
                onClick={() => navigate(investPath({ view: "trade" }))}
                title={t("nav.trade")}
              >
                <ArrowLeftRight size={14} /> {t("nav.trade")}
              </button>
              <button
                type="button"
                className="sa-header-back"
                onClick={() => setSettingsOpen(!settingsOpen)}
                title={t("stockAnalysis.settings.title")}
                style={settingsOpen && !isMobile
                  ? { background: "var(--accent-bg)", color: "var(--accent)" }
                  : undefined}
              >
                {settingsOpen && !isMobile ? <X size={16} /> : <Settings size={16} />}
              </button>
            </div>
          )}
          {isReplay && (
            // L2 视觉信号: 页面态细条 — 顶部 1px 紫色细线 + 极淡紫色背景,让用户一眼看到当前是回放模式
            <div
              data-testid="sa-replay-stripe"
              style={{
                height: 2,
                background: "linear-gradient(90deg, var(--purple) 0%, var(--purple) 50%, var(--purple) 100%)",
                opacity: 0.85,
              }}
            />
          )}

          <div
            className="sa-body"
            style={{
              ...(isInWorkspace ? { overflow: "visible", display: "block" } : {}),
              ...(isReplay
                ? {
                  backgroundImage:
                    "linear-gradient(180deg, rgba(124,58,237,0.04) 0%, rgba(124,58,237,0.01) 30%, transparent 100%)",
                }
                : {}),
            }}
          >
            <StockSearchBar />
            <div style={{ margin: "4px 16px 0 16px", display: "flex", gap: 8, alignItems: "center" }}>
              <AnalysisHistoryButton />
              {
                /* 工作区模式下 sa-header 隐藏（Shell 未提供替代），设置入口在此补充（2026-08-01）。
                  打开后变 X 图标（与独立模式齿轮 toggle 行为一致，避免"打开了找不到关闭"）。 */
              }
              {isInWorkspace && (
                <button
                  type="button"
                  className="text-xs px-2 py-0.5 rounded cursor-pointer hover:opacity-80 transition-opacity"
                  style={{
                    background: settingsOpen && !isMobile
                      ? "var(--accent-bg, rgba(99,102,241,0.18))"
                      : "rgba(255,255,255,0.06)",
                    border: settingsOpen && !isMobile
                      ? "1px solid var(--accent, #6366f1)"
                      : "1px solid rgba(255,255,255,0.14)",
                    color: settingsOpen && !isMobile ? "var(--accent, #a5b4fc)" : "inherit",
                  }}
                  onClick={() => setSettingsOpen(!settingsOpen)}
                  title={settingsOpen ? t("common.close") : t("stockAnalysis.settings.title")}
                >
                  {settingsOpen && !isMobile
                    ? (
                      <>
                        <X size={12} /> {t("common.close")}
                      </>
                    )
                    : (
                      <>
                        <Settings size={12} /> {t("stockAnalysis.settings.title")}
                      </>
                    )}
                </button>
              )}
              {/* Debug 面板属于专业工具 —— 简洁密度下不占位 */}
              {showAdvancedPanels && (
                <button
                  data-testid="sa-debug-toggle"
                  className="text-xs px-2 py-0.5 rounded cursor-pointer hover:opacity-80 transition-opacity"
                  style={{
                    background: showDebug ? "var(--purple)" : "rgba(124,58,237,0.12)",
                    color: showDebug ? "#fff" : "var(--purple)",
                    border: "1px solid rgba(124,58,237,0.3)",
                    fontWeight: 500,
                  }}
                  onClick={() => setShowDebug(!showDebug)}
                  title={t("stockAnalysis.debugPanel.toggleTitle")}
                >
                  🔍 {showDebug ? t("stockAnalysis.debugPanel.hide") : t("stockAnalysis.debugPanel.show")}
                </button>
              )}
            </div>

            {/* Decision hero (full width, at top) — outside flex row to avoid layout shift */}
            {status === "completed" && (
              <div style={{ margin: "0 16px 12px 16px" }}>
                <DecisionBanner embeddedInWorkspace={isInWorkspace} />
                {/* 次级信息（共识 / 实验轨迹 / 证据审计）—— 专业密度才展开 */}
                {showAdvancedPanels && (
                  <div data-testid="sa-advanced-summary">
                    <AnalystConsensusBar />
                    <ExperimentTrail />
                    <EvidenceCitationPanel analysisId={analysisId ?? ""} />
                  </div>
                )}
              </div>
            )}

            <div
              className="sa-body-inner"
              style={isInWorkspace ? { overflow: "visible", display: "block" } : undefined}
            >
              <div className="sa-main" style={isInWorkspace ? { overflow: "visible", display: "block" } : undefined}>
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
                        <StockAnalysisSettings key={settingsDefaultTab ?? "default"} defaultTab={settingsDefaultTab} />
                      </div>
                    </div>
                  )
                  : (
                    <>
                      {showDebug && <AnalysisDebugPanel />}

                      {status === "loading" && (
                        <div className="sa-loading">
                          <AnalysisProgress />
                          <div className="flex justify-center mt-2">
                            <button
                              className="text-xs px-3 py-1 rounded cursor-pointer transition-colors"
                              style={{
                                background: "rgba(239,68,68,0.15)",
                                color: "var(--red)",
                                border: "1px solid rgba(239,68,68,0.3)",
                              }}
                              onClick={() => {
                                if (window.confirm(t("stockAnalysis.stopAnalysisConfirm"))) {
                                  useStockAnalysisStore.getState().cancelAnalysis();
                                }
                              }}
                            >
                              ⏹ {t("stockAnalysis.stopAnalysis")}
                            </button>
                          </div>
                        </div>
                      )}

                      {status === "idle" && !isInWorkspace && <InvestDashboard />}

                      {status === "error" && (
                        <div
                          style={{
                            padding: 16,
                            margin: 16,
                            border: "1px solid var(--red)",
                            borderRadius: 8,
                            background: "var(--surface)",
                          }}
                        >
                          <h3 style={{ margin: "0 0 8px 0", color: "var(--red)" }}>
                            {failedNodes.length > 0
                              ? t("stockAnalysis.workflow.partialFailed", { count: failedNodes.length })
                              : t("stockAnalysis.workflow.startFailed")}
                          </h3>
                          <p style={{ margin: "0 0 12px 0", color: "var(--muted)", whiteSpace: "pre-wrap" }}>
                            {error ?? t("common.unknownError")}
                          </p>
                          {failedNodes.length > 0 && (
                            <div style={{ marginBottom: 12 }}>
                              <div style={{ fontSize: 12, fontWeight: 600, color: "var(--red)", marginBottom: 4 }}>
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
                                        background: "var(--red-bg)",
                                        color: "var(--red)",
                                        cursor: "pointer",
                                        display: "inline-block",
                                      }}
                                    >
                                      {id} {expandedFailedNode === id ? "▼" : "▶"}
                                    </span>
                                    {expandedFailedNode === id && (
                                      <div
                                        style={{
                                          marginTop: 4,
                                          padding: 8,
                                          background: "var(--surface)",
                                          borderRadius: 6,
                                          border: "1px solid var(--red)",
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

                      {
                        /* 「没有结论地消失」—— 与 error 分开渲染（2026-10-17）。
                        为什么不能并进上面那个 error 面板：`error` 是「跑过了并失败」，
                        面板要展示 failedNodes 与失败原因；本态是进程消失、**连原因都没留下**，
                        挂失败节点清单只会伪造一个不存在的诊断。
                        本面板的关键职责是**给出重新发起的入口** —— 此前这种记录被
                        `loadAnalysis` 映射成 running，页面显示进度条且「开始分析」被锁死，
                        用户既看不到真相、也发不出新分析。 */
                      }
                      {status === "interrupted" && (
                        <div
                          style={{
                            padding: 16,
                            margin: 16,
                            border: "1px solid var(--amber)",
                            borderRadius: 8,
                            background: "var(--surface)",
                          }}
                        >
                          <h3 style={{ margin: "0 0 8px 0", color: "var(--amber)" }}>
                            {t("stockAnalysis.interrupted.title")}
                          </h3>
                          <p style={{ margin: "0 0 12px 0", color: "var(--muted)", whiteSpace: "pre-wrap" }}>
                            {t("stockAnalysis.interrupted.hint")}
                          </p>
                          <Button
                            type="primary"
                            disabled={!stockCode}
                            onClick={() => stockCode && startAnalysis(stockCode)}
                          >
                            {t("stockAnalysis.startAnalysis")}
                          </Button>
                        </div>
                      )}

                      {(status === "running" || status === "completed") && (
                        <>
                          {status === "running" && (
                            <div
                              style={{
                                padding: "12px 16px",
                                margin: "12px 16px 0 16px",
                                border: "1px solid var(--border, #e0e0e0)",
                                borderRadius: 8,
                                background: "var(--surface)",
                              }}
                            >
                              <div className="flex items-start gap-3">
                                <div className="flex-1">
                                  <AnalysisProgress />
                                </div>
                                <button
                                  className="text-xs px-2.5 py-1 rounded cursor-pointer transition-colors shrink-0 mt-1"
                                  style={{
                                    background: "rgba(239,68,68,0.15)",
                                    color: "var(--red)",
                                    border: "1px solid rgba(239,68,68,0.3)",
                                  }}
                                  onClick={() => {
                                    if (window.confirm(t("stockAnalysis.stopAnalysisConfirm"))) {
                                      useStockAnalysisStore.getState().cancelAnalysis();
                                    }
                                  }}
                                >
                                  ⏹ {t("stockAnalysis.stopAnalysis")}
                                </button>
                              </div>
                            </div>
                          )}
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

              {/* 侧边栏 — 仅独立模式显示，工作区模式下由 ContextSidebar 承载 */}
              {!isInWorkspace && (
                <div className="sa-sidebar">
                  {status === "completed" && (
                    <>
                      <ExperimentSidebar />
                      {/* Execute shortcut */}
                      <div
                        onClick={() => navigate(investPath({ view: "trade" }))}
                        style={{
                          marginTop: 8,
                          padding: "8px 0",
                          textAlign: "center",
                          fontSize: 12,
                          border: "0.5px solid var(--border-light)",
                          borderRadius: 6,
                          cursor: "pointer",
                          color: "var(--color-text-secondary)",
                        }}
                      >
                        {t("stockAnalysis.experiment.executeTrade")}
                      </div>
                    </>
                  )}
                  {status !== "completed" && (
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
                  )}
                </div>
              )}

              {
                /* 数据源健康仪表盘 — 诊断向内容，专业密度才显示（未选股时仍可见） */
              }
              {!stockCode && showAdvancedPanels && (
                <div className="p-2">
                  <VendorHealthDashboard />
                </div>
              )}
            </div>
          </div>

          {/* Experiment trail (Phase 10) */}
          {/* 底部滑出面板 — 仅独立模式显示，工作区模式下由 ContextSidebar 承载 */}
          {!isInWorkspace && (
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
                    <button type="button" className="sa-sheet-tab">{t("stockAnalysis.settings.sheet.more")} ?</button>
                  </Dropdown>
                )}
              </div>

              <div className="sa-sheet-body">
                {activePanel?.element}
              </div>
            </div>
          )}

          {!isInWorkspace && (
            <button
              type="button"
              className="sa-sheet-toggle"
              onClick={() => setSheetOpen(!sheetOpen)}
            >
              {sheetOpen ? " ?" : "+"}
            </button>
          )}
        </div>
        {isMobile && (
          <StockAnalysisSettingsModal
            open={settingsOpen}
            onClose={() => setSettingsOpen(false)}
            defaultTab={settingsDefaultTab}
          />
        )}
      </StockAnalysisPageContext.Provider>
    </PageErrorBoundary>
  );
}

/** 分析师共识条 — 在 Decision Hero 下方显示**当前持有期**的 bull/bear/neutral 分布 */
function AnalystConsensusBar() {
  const { t } = useTranslation();
  const analystReports = useStockAnalysisStore((s) => s.analystReports);
  const decisionTimeHorizon = useStockAnalysisStore((s) => s.decision?.timeHorizon);
  const dataQualitySummary = useStockAnalysisStore((s) => s.dataQualitySummary);
  const dataQualityReport = useMemo(
    () => parseDataQualityReport(dataQualitySummary),
    [dataQualitySummary],
  );

  // 逐档共识：每档有自己的一组分析师（4/5/7/7），不合计成一个池。
  // 计票与分析师 tab 分歧卡共用 `tallyAnalystSentiment`（空产出不计票的裁定住在那里）；
  // 「结论方向与多空评分矛盾」的票由后端 data-quality 判、这里只读它扣除谁。
  const tierConsensus = useAnalystConsensusByTier(
    analystReports,
    decisionTimeHorizon,
    dataQualityReport,
  );
  const { bullish: bull, bearish: bear, neutral, total } = tierConsensus.tally;
  // 一票不剩**且不是被扣光的**才隐藏整条：全被扣光时必须留下那句原因，
  // 否则「7 位分析师结论全部自相矛盾」会显示成「这里没有共识条」。
  if (total === 0 && tierConsensus.excludedCount === 0) { return null; }

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 12,
        padding: "6px 12px",
        marginTop: 8,
        border: "0.5px solid var(--border-light)",
        borderRadius: 6,
        fontSize: 12,
      }}
    >
      <span style={{ fontWeight: 500, color: "var(--color-text-secondary)", whiteSpace: "nowrap" }}>
        {t("stockAnalysis.analystConsensus")}
      </span>
      {tierConsensus.options.length > 1 && (
        <Segmented
          size="small"
          value={tierConsensus.activeTier ?? undefined}
          onChange={(v) => tierConsensus.setActiveTier(String(v))}
          options={tierConsensus.options.map((o) => ({
            value: o.tier,
            label: `${t(horizonLabelKey(o.tier) ?? o.tier)} ${o.tally.total}`,
          }))}
        />
      )}
      {bull > 0 && (
        <span style={{ color: "var(--red)" }}>
          {bull} {t("stockAnalysis.bullish")}
        </span>
      )}
      {bear > 0 && (
        <span style={{ color: "var(--green)" }}>
          {bear} {t("stockAnalysis.bearish")}
        </span>
      )}
      {neutral > 0 && (
        <span style={{ color: "var(--muted)" }}>
          {neutral} {t("stockAnalysis.neutral")}
        </span>
      )}
      {tierConsensus.excludedCount > 0 && (
        /* 扣票必须可见：少了的原因不写出来，剩下的票数就会被读成「分析师总数」。 */
        <span style={{ color: "var(--amber)" }}>
          {t("stockAnalysis.analystConsensusExcluded", { count: tierConsensus.excludedCount })}
        </span>
      )}
    </div>
  );
}
