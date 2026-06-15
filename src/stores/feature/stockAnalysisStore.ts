import i18n from "@/i18n";
import { extractContent, extractDecision, normalizeDecision, tryParseDecision } from "@/lib/agentOutput";
import { invoke, listen } from "@/lib/invoke";
import type { UnlistenFn } from "@/lib/invoke";
import { detectFutureReferencesForNode } from "@/lib/timeTravel/futureReferenceDetector";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import type {
  AnalysisStatus,
  AnalysisSummary,
  EarningsEvent,
  KLine,
  StockConsensus,
  StockDecision,
  StockQuote,
  StockSearchResult,
  TimelineNode,
  TimelinePhase,
} from "@/types/stock-analysis";
import { computeStockConsensus, StockAction, StockRiskLevel } from "@/types/stock-analysis";
import { create } from "zustand";

// ── 模块级缓存 ──
let lastEarningsFetch: { stockCode: string; ts: number } | null = null;
const EARNINGS_CACHE_TTL_MS = 10 * 60 * 1000; // 10 分钟

// ── 工作流结果解析 ──
function parseWorkflowResults(results: Record<string, unknown>) {
  const analystReports: Record<string, string> = {};
  const debateRounds: Array<{ round: number; bull: string; bear: string }> = [];
  const riskAssessments: Record<string, string> = {};
  // 修复 Bug #2: 补充 value-investor / rule-check / data-quality / raw-data
  // 四个节点的解析分支。这四个节点类型在前端 store 已有对应字段
  // (valueAssessments / ruleCheckResults / dataQualitySummary / rawData)，
  // 但 parseWorkflowResults 没把它们填进去，导致 workflow-completed 后
  // 对应字段一直为空。
  const valueAssessments: Record<string, string> = {};
  const ruleCheckResults: Record<string, string> = {};
  let dataQualitySummary = "";
  const rawData: Record<string, string> = {};
  let decision: StockDecision | null = null;

  for (const [stepId, raw] of Object.entries(results)) {
    const output = extractContent(raw);

    if (stepId.startsWith("a-") && !stepId.includes("bull") && !stepId.includes("bear")) {
      analystReports[stepId.slice(2)] = output;
    } else if (stepId === "bull-researcher" || (stepId.startsWith("bull-r") && stepId !== "bull-researcher")) {
      // 辩论子节点: 实际 nodeId 为 "bull-researcher" (DAG 引擎单次执行)
      // 兼容未来多轮模式: bull-r1, bull-r2...
      const round = stepId === "bull-researcher" ? 1 : parseInt(stepId.slice(6), 10);
      const bearKey = stepId === "bull-researcher" ? "bear-researcher" : `bear-r${round}`;
      debateRounds.push({ round, bull: output, bear: extractContent(results[bearKey] ?? "") });
    } else if (stepId === "bear-researcher" || (stepId.startsWith("bear-r") && stepId !== "bear-researcher")) {
      continue;
    } else if (stepId.startsWith("risk-") || stepId === "research-mgr") {
      riskAssessments[stepId] = output;
    } else if (stepId === "trader") {
      analystReports["investment-plan"] = output;
    } else if (stepId === "portfolio-mgr") {
      // 不要构造全 0 假决策——解析失败就保持 null，
      // 让调用方决定如何处理缺失决策。
      const parsed = extractDecision(raw);
      if (parsed) { decision = parsed; }
    } else if (stepId === "value-investor") {
      // 巴菲特框架评估（与 risk-evaluator 并行，在辩论之后运行）
      valueAssessments[stepId] = output;
    } else if (stepId === "rule-check") {
      ruleCheckResults[stepId] = output;
    } else if (stepId === "data-quality") {
      // 整个工作流只产出一条 data-quality 报告，直接覆盖即可
      dataQualitySummary = output;
    } else if (stepId === "raw-data") {
      rawData[stepId] = output;
    }
  }

  debateRounds.sort((a, b) => a.round - b.round);
  return {
    analystReports,
    debateRounds,
    riskAssessments,
    valueAssessments,
    ruleCheckResults,
    dataQualitySummary,
    rawData,
    decision,
  };
}

// ── Store ──

/** R1 复盘→进化：dashboard 中单条 (strategy, period) 的统计行 */
export interface EvolutionStrategyStatRow {
  strategyId: string;
  period: string;
  oldWeight: number;
  newWeight: number;
  deltaPct: number;
  winRate: number;
  sampleSize: number;
  rationale: string;
}

/** R1 复盘→进化：recent changes 中的一条历史调整 */
export interface EvolutionRecentChangeRow {
  id: number;
  strategyId: string;
  period: string;
  oldWeight: number;
  newWeight: number;
  deltaPct: number;
  trigger: string;
  appliedAt: number;
  rationale: string;
}

/** R1 复盘→进化：strategy 维度聚合卡片 */
export interface EvolutionStrategySummaryRow {
  strategyId: string;
  avgWeight: number;
  avgWinRate: number;
  totalSamples: number;
  trend: "up" | "down" | "stable";
}

/** R1 复盘→进化：dashboard 完整结构 */
export interface EvolutionDriftDashboard {
  currentWeights: Record<string, number>;
  lastRecalcAt: number;
  stats: EvolutionStrategyStatRow[];
  recentChanges: EvolutionRecentChangeRow[];
  strategySummary: EvolutionStrategySummaryRow[];
}

/** getDryRun 模块级缓存 (60s TTL) */
let dryRunCache: { value: boolean; ts: number } | null = null;

// ── R2 组合监控类型 ──

/** R2 压测单条结果 */
export interface PortfolioStressResult {
  scenario: string;
  label: string;
  portfolioPnl: number;
  portfolioPnlPct: number;
  topHit?: { stockCode: string; stockName: string; pnlPct: number };
  note: string;
}

export interface PortfolioStressBundle {
  m10?: PortfolioStressResult;
  m20?: PortfolioStressResult;
  blackSwan?: PortfolioStressResult;
}

export interface PortfolioPositionRow {
  stockCode: string;
  stockName: string;
  totalShares: number;
  avgCost: number;
  currentPrice?: number;
  marketValue?: number;
  unrealizedPnl?: number;
  unrealizedPnlPct?: number;
  totalRealizedPnl: number;
  sectorName?: string;
}

export interface PortfolioDashboard {
  isHistorical: boolean;
  asOfDate?: string;
  totalMarketValue: number;
  totalPnl: number;
  totalPnlPct: number;
  cashPct: number;
  maxDrawdownPct: number;
  beta?: number;
  sharpe30d?: number;
  correlationAvg?: number;
  topConcentrationPct: number;
  sectorExposure: Record<string, number>;
  concentrationWarning?: string;
  riskLevel: string;
  diversificationScore: number;
  stressTest: PortfolioStressBundle;
  positions: PortfolioPositionRow[];
  snapshotAt: number;
}

export interface PortfolioCorrelationCell {
  codeA: string;
  codeB: string;
  correlation: number;
}

export interface PositionLimitsCheck {
  ok: boolean;
  reason?: string;
  maxSingleStockPct: number;
  maxTotalPositions: number;
  maxSectorExposurePct: number;
  newPositionValue: number;
}
const DRY_RUN_TTL_MS = 60_000;

interface StockAnalysisState {
  searchKeyword: string;
  searchResults: StockSearchResult[];

  analysisId: string | null;
  workflowId: string | null;
  stockCode: string;
  stockName: string;
  analysisDate: string;
  status: AnalysisStatus;

  quote: StockQuote | null;
  klineData: KLine[];
  analystReports: Record<string, string>;
  debateRounds: Array<{ round: number; bull: string; bear: string }>;
  riskAssessments: Record<string, string>;
  // 决策后处理阶段新增字段（修复 #7）
  valueAssessments: Record<string, string>;
  ruleCheckResults: Record<string, string>;
  dataQualitySummary: string;
  rawData: Record<string, string>;
  decision: StockDecision | null;
  error: string | null;
  errorCode: string | null;
  failedNodes: string[];
  failedNodeErrors: Record<string, string>;
  /** 数据源降级/空数据警告（不阻断流程） */
  dataWarnings: string[];

  history: AnalysisSummary[];

  currentStage: number;
  progressMessage: string;
  progressPct: number;

  llmStatus: "live" | "placeholder" | "unknown";
  chatIndicatorDismissed: boolean;

  // Phase 1: K-line period persistence cross-mount
  klinePeriod: string;
  setKlinePeriod: (period: string) => void;

  // R3-A: K-line adjustment type (复权)
  klineAdj: "auto" | "none" | "forward" | "backward";
  setKlineAdj: (adj: "auto" | "none" | "forward" | "backward") => void;

  // Phase 1: Auto-refresh toggle
  autoRefresh: boolean;
  setAutoRefresh: (enabled: boolean) => void;

  // Phase 4: K-line indicator line toggles
  klineIndicators: { ma5: boolean; ma10: boolean; ma20: boolean };
  toggleIndicator: (key: "ma5" | "ma10" | "ma20") => void;

  // Phase 4: Sidebar panel collapse state (persisted to localStorage)
  sidebarCollapsed: Record<string, boolean>;
  toggleSidebarPanel: (key: string) => void;

  watchlistVersion: number;
  bumpWatchlistVersion: () => void;

  // Phase 8: Decision Timeline
  timeline: TimelineNode[];
  pushTimelineNode: (node: TimelineNode) => void;
  updateTimelineNode: (id: string, patch: Partial<TimelineNode>) => void;
  clearTimeline: () => void;
  // Phase 8: Right panel highlight (0.4s 闪烁)
  highlightedPanel: string | null;
  setHighlightedPanel: (key: string | null) => void;

  // 荐股 ↔ 分析师交叉验证：每只股票在最近一次工作流完成时
  // 缓存的分析师共识，用于 RecommendationPanel 中提示"推荐与共识是否一致"。
  stockCodeConsensus: Record<string, StockConsensus>;
  setStockCodeConsensus: (stockCode: string, consensus: StockConsensus) => void;

  // Phase 9: Time-travel snapshot metadata
  // - `asOfDate`: 当前 analysis 的 as_of_date（live 时为 null）
  // - `mode`: 模式标签（"live" / "replay" / "backtest_sweep"）
  // - `violations`: 3 阶段 LLM 未来引用检测发现的违规列表
  asOfDate: string | null;
  mode: "live" | "replay" | "backtest_sweep";
  violations: Array<{ nodeId: string; snippet: string; ruleHit: string }>;

  // Phase 10: Experiment mode (What-If integrated)
  decisionMode: "view" | "experiment" | "execute";
  setDecisionMode: (mode: "view" | "experiment" | "execute") => void;
  experiments: ExperimentRecord[];
  pushExperiment: (record: ExperimentRecord) => void;
  clearExperiments: () => void;
  setAsOfDate: (date: string | null) => void;
  setMode: (mode: "live" | "replay" | "backtest_sweep") => void;
  setViolations: (
    v: Array<{ nodeId: string; snippet: string; ruleHit: string }>,
  ) => void;

  // Actions
  searchStock: (keyword: string, skipDebounce?: boolean) => Promise<void>;
  getStockQuote: (code: string) => Promise<void>;
  getStockKline: (
    code: string,
    period: string,
    limit: number,
    adj?: "auto" | "none" | "forward" | "backward",
  ) => Promise<void>;
  startAnalysis: (stockCode: string) => Promise<void>;
  cancelAnalysis: () => Promise<void>;
  getDryRun: () => Promise<boolean>;
  fetchHistory: (limit?: number, offset?: number) => Promise<void>;
  loadAnalysis: (analysisId: string) => Promise<void>;
  reset: () => void;
  dismissChatIndicator: () => void;

  // R1 复盘→进化
  evolutionDashboard: EvolutionDriftDashboard | null;
  evolutionRecalculating: boolean;
  evolutionLastError: string | null;
  fetchEvolutionDashboard: (asOfDate?: string | null) => Promise<void>;
  recalcEvolutionNow: (asOfDate?: string | null) => Promise<void>;
  loadRecoStrategyWeights: () => Promise<Record<string, number>>;

  // R2 组合监控
  portfolioDashboard: PortfolioDashboard | null;
  portfolioCorrelations: PortfolioCorrelationCell[];
  portfolioRefreshing: boolean;
  portfolioLastError: string | null;
  fetchPortfolioDashboard: (asOfDate?: string | null) => Promise<void>;
  refreshPortfolioMetrics: (asOfDate?: string | null) => Promise<void>;
  fetchPortfolioCorrelations: (asOfDate?: string | null) => Promise<void>;
  checkPositionLimits: (
    stockCode: string,
    proposedShares: number,
    proposedPrice: number,
  ) => Promise<PositionLimitsCheck | null>;

  // R3-B: 财报披露事件 — K 线叠加图标用
  earningsEvents: EarningsEvent[];
  earningsLoading: boolean;
  showEarningsOnChart: boolean;
  setShowEarningsOnChart: (show: boolean) => void;
  fetchEarningsEvents: (stockCode: string) => Promise<void>;

  // P2-6: RealtimeMonitor T+0 自动重跑配置
  t0Config: {
    enabled: boolean;
    changePctThreshold: number | null;
    turnoverRateThreshold: number | null;
    minIntervalMinutes: number;
  };
  t0Loading: boolean;
  setT0Config: (
    cfg: Partial<{
      enabled: boolean;
      changePctThreshold: number | null;
      turnoverRateThreshold: number | null;
      minIntervalMinutes: number;
    }>,
  ) => Promise<void>;
  fetchT0Config: () => Promise<void>;

  _unlisten: UnlistenFn | null;
  setupEventListener: () => Promise<void>;
  _searchTimer: ReturnType<typeof setTimeout> | null;
}

export interface ExperimentRecord {
  id: string;
  step: number;
  params: Record<string, number | string>;
  configOverrides: Record<string, number>;
  decisionBefore: Partial<StockDecision>;
  decisionAfter: Partial<StockDecision>;
  accepted: boolean;
  createdAt: number;
}

const initialState = {
  searchKeyword: "",
  searchResults: [],
  analysisId: null,
  workflowId: null,
  stockCode: "",
  stockName: "",
  analysisDate: "",
  status: "idle" as AnalysisStatus,
  quote: null,
  klineData: [],
  analystReports: {},
  debateRounds: [],
  riskAssessments: {},
  valueAssessments: {},
  ruleCheckResults: {},
  dataQualitySummary: "",
  rawData: {},
  decision: null,
  error: null,
  errorCode: null,
  failedNodes: [],
  failedNodeErrors: {},
  dataWarnings: [],
  history: [],
  currentStage: 0,
  progressMessage: "",
  progressPct: 0,
  llmStatus: "unknown" as const,
  chatIndicatorDismissed: false,
  klinePeriod: "6m",
  klineAdj: "auto" as const,
  autoRefresh: false,
  klineIndicators: { ma5: true, ma10: true, ma20: true },
  sidebarCollapsed: {},
  watchlistVersion: 0,
  timeline: [],
  highlightedPanel: null,
  stockCodeConsensus: {},
  asOfDate: null,
  mode: "live" as const,
  violations: [],
  evolutionDashboard: null,
  evolutionRecalculating: false,
  evolutionLastError: null,
  portfolioDashboard: null,
  portfolioCorrelations: [],
  portfolioRefreshing: false,
  portfolioLastError: null,
  earningsEvents: [],
  earningsLoading: false,
  showEarningsOnChart: true,
  t0Config: {
    enabled: false,
    changePctThreshold: 3.0,
    turnoverRateThreshold: 8.0,
    minIntervalMinutes: 30,
  },
  t0Loading: false,
  decisionMode: "view" as "view" | "experiment" | "execute",
  experiments: [],
};

export const useStockAnalysisStore = create<StockAnalysisState>((set, get) => ({
  ...initialState,
  _unlisten: null,
  _searchTimer: null,

  searchStock: async (keyword: string, skipDebounce?: boolean) => {
    set({ searchKeyword: keyword });
    if (keyword.length < 2) {
      set({ searchResults: [] });
      return;
    }
    const { _searchTimer } = get();
    if (_searchTimer) { clearTimeout(_searchTimer); }
    if (skipDebounce) {
      // Enter/点击搜索直接执行，跳过防抖
      try {
        const results = await invoke<StockSearchResult[]>("search_stock", { keyword });
        set({ searchResults: results });
      } catch {
        set({ searchResults: [] });
      }
      return;
    }
    const timer = setTimeout(async () => {
      try {
        const results = await invoke<StockSearchResult[]>("search_stock", { keyword });
        set({ searchResults: results });
      } catch {
        set({ searchResults: [] });
      }
    }, 300);
    set({ _searchTimer: timer });
  },

  getStockQuote: async (code: string) => {
    try {
      // 时间旅行：从 timeAnchorStore 读 as_of_date，透传给后端（仅 replay/backtest_sweep 模式）
      const asOfDate = (() => {
        const state = useTimeAnchorStore.getState();
        return state.mode === "replay" || state.mode === "backtest_sweep" ? state.asOfDate : null;
      })();
      const quote = await invoke<StockQuote>("get_stock_quote", { stockCode: code, asOfDate });
      set({ quote, stockCode: code, stockName: quote.name });
      // R3-B: 财报事件缓存 10 分钟，避免每次报价刷新都拉取
      const now = Date.now();
      if (
        !lastEarningsFetch || lastEarningsFetch.stockCode !== code
        || (now - lastEarningsFetch.ts) > EARNINGS_CACHE_TTL_MS
      ) {
        get().fetchEarningsEvents(code);
        lastEarningsFetch = { stockCode: code, ts: now };
      }
    } catch (e) {
      console.error("[StockAnalysis] Failed to get stock quote:", e);
    }
  },

  getStockKline: async (
    code: string,
    period: string,
    limit: number,
    adj?: "auto" | "none" | "forward" | "backward",
  ) => {
    try {
      // 时间旅行：K 线按 as_of_date 截断（仅 replay/backtest_sweep 模式）
      const asOfDate = (() => {
        const state = useTimeAnchorStore.getState();
        return state.mode === "replay" || state.mode === "backtest_sweep" ? state.asOfDate : null;
      })();
      const klineData = await invoke<KLine[]>("get_stock_kline", {
        stockCode: code,
        period,
        limit,
        asOfDate,
        adj: adj ?? get().klineAdj,
      });
      set({ klineData });
    } catch (e) {
      console.error("[StockAnalysis] Failed to get kline:", e);
    }
  },

  /** 读取 analysis_dry_run 模板变量 (60s 模块级缓存) */
  getDryRun: async () => {
    const now = Date.now();
    if (dryRunCache && now - dryRunCache.ts < DRY_RUN_TTL_MS) {
      return dryRunCache.value;
    }
    try {
      const tmpl = await invoke<Record<string, unknown>>("get_workflow_template", { id: "stock-analysis" });
      const vars = (tmpl?.variables ?? []) as Record<string, unknown>[];
      const v = vars.find((x: Record<string, unknown>) => x.name === "analysis_dry_run");
      const value = !!v?.value;
      dryRunCache = { value, ts: now };
      return value;
    } catch {
      return false;
    }
  },

  startAnalysis: async (stockCode: string) => {
    const { status } = get();
    if (status === "loading" || status === "running") {
      console.warn("[StockAnalysis] Analysis already in progress, ignoring duplicate start");
      return;
    }

    const { _unlisten } = get();
    if (_unlisten) { _unlisten(); }

    set({
      status: "loading",
      error: null,
      errorCode: null,
      failedNodes: [],
      failedNodeErrors: {},
      dataWarnings: [],
      currentStage: 0,
      workflowId: null,
      progressMessage: i18n.t("stockAnalysis.progress.fetchingData"),
      progressPct: 0,
      chatIndicatorDismissed: false,
      analystReports: {},
      debateRounds: [],
      riskAssessments: {},
      valueAssessments: {},
      ruleCheckResults: {},
      dataQualitySummary: "",
      rawData: {},
      decision: null,
      _unlisten: null,
      timeline: [],
      // 清理跨分析轮次缓存，防止上轮数据污染
      stockCodeConsensus: {},
    });

    // 先拉取工作流模板（getDryRun），再注册事件监听
    // 顺序不可颠倒：getDryRun 可能失败，此时不应注册监听器
    try {
      const dryRun = await get().getDryRun();

      await get().setupEventListener();

      // 数据源健康检查（非阻塞，仅打日志）
      const VENDORS = ["eastmoney", "sina", "tencent", "akshare"];
      for (const v of VENDORS) {
        invoke("check_vendor_health", { vendor: v }).catch((e) => {
          const msg = e instanceof Error ? e.message : typeof e === "string" ? e : JSON.stringify(e);
          console.warn(`[StockAnalysis] Vendor ${v} health check failed: ${msg}`);
        });
      }

      // 时间旅行模式：从 useTimeAnchorStore 读 as_of_date，透传给后端
      // 只在 replay / backtest_sweep 模式传日期，live 模式传 null 以避免 persist 残留
      const anchorMode = useTimeAnchorStore.getState().mode;
      const rawAsOfDate = useTimeAnchorStore.getState().asOfDate;
      const asOfDate = anchorMode === "replay" || anchorMode === "backtest_sweep" ? rawAsOfDate : null;
      set({
        asOfDate,
        mode: anchorMode === "backtest_sweep" ? "backtest_sweep" : anchorMode === "replay" ? "replay" : "live",
      });
      const result = await invoke<{
        analysisId: string;
        workflowId: string;
        stockCode: string;
        stockName: string;
      }>("run_stock_workflow", { stockCode, dryRun, asOfDate });

      set({
        analysisId: result.analysisId,
        workflowId: result.workflowId,
        stockCode: result.stockCode,
        stockName: result.stockName,
        status: "running",
        progressMessage: i18n.t("stockAnalysis.progress.started"),
        progressPct: 5,
      });

      // 异步拉取报价和 K 线
      // 如果用户已通过 StockSearchBar 选中了同一股票，数据已就绪，跳过重复请求
      const preloadedQuote = get().quote;
      if (!preloadedQuote || preloadedQuote.code !== result.stockCode) {
        get().getStockQuote(result.stockCode).catch(() => set({ quote: null }));
      }
      const preloadedKline = get().klineData;
      if (preloadedKline.length === 0) {
        get().getStockKline(result.stockCode, "daily", 120).catch(() => set({ klineData: [] }));
      }
    } catch (e) {
      console.error("[StockAnalysis] Failed to start workflow:", e);
      // 清理可能已注册的监听器（getDryRun 之后 setupEventListener 可能已经成功）
      const { _unlisten } = get();
      if (_unlisten) { _unlisten(); }
      set({
        status: "error",
        error: typeof e === "string" ? e : (e as Error)?.message ?? i18n.t("stockAnalysis.workflow.startFailed"),
        _unlisten: null,
        progressPct: 0,
      });
    }
  },

  cancelAnalysis: async () => {
    const { workflowId, _unlisten } = get();
    if (workflowId) {
      await invoke("cancel_stock_workflow", { workflowId });
    }
    if (_unlisten) { _unlisten(); }
    // 使用 "cancelled" 状态保留已收集的部分数据（面板仍显示，带"已取消"标记）
    set({ status: "cancelled" as AnalysisStatus, _unlisten: null, currentStage: 0, progressPct: 0, workflowId: null });
  },

  fetchHistory: async (limit = 20, offset = 0) => {
    const history = await invoke<AnalysisSummary[]>("list_stock_analyses", { limit, offset });
    set({ history });
  },

  loadAnalysis: async (analysisId: string) => {
    const record = await invoke<
      AnalysisSummary & {
        decisionJson: string | null;
        blackboardSnapshot: string | null;
      }
    >("get_stock_analysis", { analysisId });
    set({ analysisId: record.id, stockCode: record.stockCode, stockName: record.stockName, status: "completed" });
    if (record.decisionJson) {
      try {
        const raw = JSON.parse(record.decisionJson);
        set({ decision: normalizeDecision(raw) });
      } catch (e) {
        console.error("[StockAnalysis] Failed to parse decision JSON:", e);
      }
    }
    if (record.blackboardSnapshot) {
      try {
        const snap: Record<string, string> = JSON.parse(record.blackboardSnapshot);
        const reports: Record<string, string> = {};
        const debates: Array<{ round: number; bull: string; bear: string }> = [];
        const risks: Record<string, string> = {};
        const values: Record<string, string> = {};
        const ruleChecks: Record<string, string> = {};
        const raws: Record<string, string> = {};
        let dataQuality = "";
        for (const [key, value] of Object.entries(snap)) {
          if (key.startsWith("report.")) {
            reports[key.slice(7)] = value;
          } else if (key.startsWith("debate.bull.round_")) {
            const round = parseInt(key.slice("debate.bull.round_".length));
            const bearKey = `debate.bear.round_${round}`;
            debates.push({ round, bull: value, bear: snap[bearKey] ?? "" });
          } else if (key.startsWith("risk.")) {
            risks[key.slice(5)] = value;
          } else if (key.startsWith("value.")) {
            values[key.slice(6)] = value;
          } else if (key.startsWith("rule_check.")) {
            ruleChecks[key.slice("rule_check.".length)] = value;
          } else if (key === "data_quality_summary") {
            dataQuality = value;
          } else if (key.startsWith("raw.")) {
            raws[key.slice(4)] = value;
          }
        }
        set({
          analystReports: reports,
          debateRounds: debates,
          riskAssessments: risks,
          valueAssessments: values,
          ruleCheckResults: ruleChecks,
          dataQualitySummary: dataQuality,
          rawData: raws,
        });

        // 历史分析回放：也缓存一次共识，让 RecommendationPanel 能用
        if (record.stockCode && Object.keys(reports).length > 0) {
          get().setStockCodeConsensus(
            record.stockCode,
            computeStockConsensus(reports, Date.now(), get().decision?.timeHorizon),
          );
        }
      } catch (e) {
        console.error("[StockAnalysis] Failed to restore blackboard snapshot:", e);
      }
    }
  },

  dismissChatIndicator: () => {
    set({ chatIndicatorDismissed: true });
  },

  // R1 复盘→进化
  fetchEvolutionDashboard: async (asOfDate?: string | null) => {
    try {
      const data = await invoke<EvolutionDriftDashboard>("get_evolution_drift_dashboard", {
        asOfDate: asOfDate ?? null,
      });
      set({ evolutionDashboard: data, evolutionLastError: null });
    } catch (e) {
      set({ evolutionLastError: e instanceof Error ? e.message : String(e) });
      console.error("[EvolutionDrift] fetch dashboard failed:", e);
    }
  },

  recalcEvolutionNow: async (asOfDate?: string | null) => {
    set({ evolutionRecalculating: true, evolutionLastError: null });
    try {
      await invoke<{ written: number; currentWeights: Array<[string, string, number]> }>(
        "manual_recalc_strategy_weights",
        { asOfDate: asOfDate ?? null },
      );
      // 重算后立即拉一次新 dashboard
      await get().fetchEvolutionDashboard(asOfDate ?? null);
    } catch (e) {
      set({ evolutionLastError: e instanceof Error ? e.message : String(e) });
      console.error("[EvolutionDrift] manual recalc failed:", e);
    } finally {
      set({ evolutionRecalculating: false });
    }
  },

  loadRecoStrategyWeights: async () => {
    try {
      const data = await invoke<Record<string, number>>("get_reco_strategy_weights");
      return data;
    } catch (e) {
      console.warn("[EvolutionDrift] load reco strategy weights failed:", e);
      return {};
    }
  },

  // R2 组合监控
  fetchPortfolioDashboard: async (asOfDate?: string | null) => {
    try {
      const data = await invoke<PortfolioDashboard>("get_portfolio_dashboard", {
        asOfDate: asOfDate ?? null,
      });
      set({ portfolioDashboard: data, portfolioLastError: null });
    } catch (e) {
      set({ portfolioLastError: e instanceof Error ? e.message : String(e) });
      console.error("[PortfolioMonitor] fetch dashboard failed:", e);
    }
  },

  refreshPortfolioMetrics: async (asOfDate?: string | null) => {
    set({ portfolioRefreshing: true, portfolioLastError: null });
    try {
      await invoke<{
        metricsId: string;
        positionsSnapshotted: number;
        correlationPairsWritten: number;
        asOfDate: string | null;
      }>("refresh_portfolio_metrics", { asOfDate: asOfDate ?? null });
      await Promise.all([
        get().fetchPortfolioDashboard(asOfDate ?? null),
        get().fetchPortfolioCorrelations(asOfDate ?? null),
      ]);
    } catch (e) {
      set({ portfolioLastError: e instanceof Error ? e.message : String(e) });
      console.error("[PortfolioMonitor] refresh failed:", e);
    } finally {
      set({ portfolioRefreshing: false });
    }
  },

  fetchPortfolioCorrelations: async (asOfDate?: string | null) => {
    try {
      const data = await invoke<PortfolioCorrelationCell[]>(
        "get_portfolio_correlations",
        { asOfDate: asOfDate ?? null },
      );
      set({ portfolioCorrelations: data });
    } catch (e) {
      console.error("[PortfolioMonitor] fetch correlations failed:", e);
    }
  },

  checkPositionLimits: async (
    stockCode: string,
    proposedShares: number,
    proposedPrice: number,
  ) => {
    try {
      return await invoke<PositionLimitsCheck>("check_position_limits", {
        stockCode,
        proposedShares,
        proposedPrice,
      });
    } catch (e) {
      console.error("[PortfolioMonitor] check position limits failed:", e);
      return null;
    }
  },

  bumpWatchlistVersion: () => {
    set((s) => ({ watchlistVersion: s.watchlistVersion + 1 }));
  },

  pushTimelineNode: (node) => {
    set((s) => {
      // 同 id 视为 update（去重），避免重复推送
      const idx = s.timeline.findIndex((n) => n.id === node.id);
      if (idx >= 0) {
        const next = s.timeline.slice();
        next[idx] = { ...next[idx], ...node };
        return { timeline: next };
      }
      return { timeline: [...s.timeline, node] };
    });
  },

  updateTimelineNode: (id, patch) => {
    set((s) => {
      const idx = s.timeline.findIndex((n) => n.id === id);
      if (idx < 0) { return {}; }
      const next = s.timeline.slice();
      next[idx] = { ...next[idx], ...patch };
      return { timeline: next };
    });
  },

  clearTimeline: () => {
    set({ timeline: [] });
  },

  setHighlightedPanel: (key) => {
    set({ highlightedPanel: key });
  },

  setStockCodeConsensus: (stockCode, consensus) => {
    if (!stockCode) { return; }
    set((s) => ({
      stockCodeConsensus: { ...s.stockCodeConsensus, [stockCode]: consensus },
    }));
  },

  setKlinePeriod: (period: string) => {
    set({ klinePeriod: period });
  },

  setKlineAdj: (adj) => {
    set({ klineAdj: adj });
  },

  setAutoRefresh: (enabled: boolean) => {
    set({ autoRefresh: enabled });
  },

  toggleIndicator: (key) => {
    set((s) => ({
      klineIndicators: { ...s.klineIndicators, [key]: !s.klineIndicators[key] },
    }));
  },

  toggleSidebarPanel: (key) => {
    set((s) => ({
      sidebarCollapsed: { ...s.sidebarCollapsed, [key]: !s.sidebarCollapsed[key] },
    }));
    // Persist to localStorage（延迟写入，避免高频 toggle 阻塞主线程）
    if (typeof window !== "undefined") {
      // 使用 requestIdleCallback 或 setTimeout 延迟写入
      const schedule = window.requestIdleCallback || ((cb: IdleRequestCallback) => setTimeout(cb, 200));
      schedule(() => {
        try {
          window.localStorage.setItem("ax_sidebar_collapsed", JSON.stringify(get().sidebarCollapsed));
        } catch { /* noop */ }
      });
    }
  },

  reset: () => {
    const { _unlisten, _searchTimer } = get();
    if (_unlisten) {
      _unlisten();
    }
    if (_searchTimer) {
      clearTimeout(_searchTimer);
    }
    set({ ...initialState, _searchTimer: null, _unlisten: null, llmStatus: "unknown" as const });
  },

  setAsOfDate: (date) => set({ asOfDate: date }),
  setMode: (mode) => set({ mode }),
  setViolations: (violations) => set({ violations }),
  setShowEarningsOnChart: (show) => set({ showEarningsOnChart: show }),

  // Phase 10: Experiment mode actions
  setDecisionMode: (mode) => set({ decisionMode: mode }),
  pushExperiment: (record) => set((s) => ({ experiments: [...s.experiments, record] })),
  clearExperiments: () => set({ experiments: [] }),

  /**
   * R3-B: 拉取财报披露事件列表,在 K 线图上叠加图标。
   * 拉取失败时清空列表(避免显示陈旧数据),但不影响 K 线本身渲染。
   */
  fetchEarningsEvents: async (stockCode: string) => {
    if (!stockCode) {
      set({ earningsEvents: [] });
      return;
    }
    // 切换股票:清旧数据再拉新数据,避免上一只股票的事件短暂闪烁在 UI 上
    set({ earningsLoading: true });
    try {
      const events = await invoke<EarningsEvent[]>("get_earnings_calendar", { stockCode });
      set({ earningsEvents: Array.isArray(events) ? events : [], earningsLoading: false });
    } catch (e) {
      console.error("[StockAnalysis] Failed to fetch earnings events:", e);
      set({ earningsEvents: [], earningsLoading: false });
    }
  },

  /**
   * P2-6: 拉取 T+0 自动重跑配置
   */
  fetchT0Config: async () => {
    try {
      const cfg = await invoke<{
        enabled: boolean;
        changePctThreshold: number | null;
        turnoverRateThreshold: number | null;
        minIntervalMinutes: number;
      }>("get_t0_config");
      set({
        t0Config: {
          enabled: !!cfg.enabled,
          changePctThreshold: cfg.changePctThreshold,
          turnoverRateThreshold: cfg.turnoverRateThreshold,
          minIntervalMinutes: cfg.minIntervalMinutes,
        },
      });
    } catch (e) {
      console.warn("[StockAnalysis] Failed to fetch t0 config:", e);
    }
  },

  /**
   * P2-6: 更新 T+0 配置 (partial merge + 立即同步到后端)
   */
  setT0Config: async (patch) => {
    const cur = get().t0Config;
    const next = { ...cur, ...patch };
    set({ t0Config: next, t0Loading: true });
    try {
      await invoke("set_t0_config", { config: next });
    } catch (e) {
      console.error("[StockAnalysis] Failed to set t0 config:", e);
    } finally {
      set({ t0Loading: false });
    }
  },

  setupEventListener: async () => {
    const existing = get()._unlisten;
    if (existing) { return; }

    // Restore sidebar collapse state from localStorage (one-time)
    if (typeof window !== "undefined") {
      try {
        const saved = window.localStorage.getItem("ax_sidebar_collapsed");
        if (saved) { set({ sidebarCollapsed: JSON.parse(saved) }); }
      } catch { /* noop */ }
    }

    // 使用数组收集所有已注册的 unlisten 函数，即使中途失败也能统一清理
    const unlisteners: Array<() => void> = [];
    const unlistenAll = () => {
      for (const u of unlisteners) {
        try {
          u();
        } catch { /* noop */ }
      }
    };

    // 手动 try-catch 包装每个 listen，一个失败不影响其他的
    try {
      const u1 = await listen<{
        workflowId: string;
        nodeId: string;
        status: string;
        totalNodes: number;
        completedNodes: number;
        output?: unknown;
        error?: string;
      }>("workflow-step-done", (event) => {
        const { nodeId, status, totalNodes, completedNodes, output, error } = event.payload;
        const stage = inferStage(nodeId);
        if (stage >= 0) { set({ currentStage: stage }); }
        const pct = totalNodes > 0
          ? Math.round((completedNodes / totalNodes) * 100)
          : get().progressPct;
        set({
          progressPct: Math.max(pct, get().progressPct),
          progressMessage: status === "completed"
            ? i18n.t("stockAnalysis.progress.stepDone", { name: nodeId })
            : status === "failed"
            ? i18n.t("stockAnalysis.progress.stepRetrying", { name: nodeId })
            : i18n.t("stockAnalysis.progress.stepRunning", { name: nodeId }),
          failedNodes: status === "failed"
            ? [...get().failedNodes, nodeId]
            : get().failedNodes,
          failedNodeErrors: status === "failed" && error
            ? { ...get().failedNodeErrors, [nodeId]: error }
            : get().failedNodeErrors,
        });

        // ── 数据源降级检测：新闻/情绪/公告工具返回空数据时发出警告 ──
        if (
          status === "completed"
          && (nodeId === "t-news-data" || nodeId === "t-sentiment-data" || nodeId === "t-catalyst-data")
        ) {
          const outputValue = event.payload.output;
          const isEmpty = outputValue == null
            || (typeof outputValue === "string" && (outputValue === "[]" || outputValue.trim() === ""))
            || (Array.isArray(outputValue) && outputValue.length === 0);
          if (isEmpty) {
            const label = nodeId === "t-news-data"
              ? "新闻"
              : nodeId === "t-sentiment-data"
              ? "舆情"
              : "公告";
            const warnings = get().dataWarnings;
            const msg = `⚠️ ${label}数据获取为空，相关分析师将基于有限数据分析`;
            if (!warnings.includes(msg)) {
              set({ dataWarnings: [...warnings, msg] });
            }
          }
        }

        // 失败节点也写入 timeline，状态为 "failed"，便于侧栏脊柱高亮红色
        if (status === "failed") {
          const phase = inferTimelinePhase(nodeId);
          if (phase) {
            get().pushTimelineNode({
              id: nodeId,
              phase,
              agentId: nodeId,
              agentName: agentDisplayName(nodeId),
              title: agentDisplayName(nodeId),
              summary: error ?? "",
              confidence: 0,
              status: "failed",
              evidenceRefs: inferEvidenceRefs(nodeId),
              startedAt: Date.now(),
              finishedAt: Date.now(),
            });
          }
        }

        if (status === "completed" && output != null) {
          const text = extractContent(output);
          const s = get();
          // 同步推送 timeline 节点(去重:同 id 的后续 push 视为 update)
          const phase = inferTimelinePhase(nodeId);
          if (phase) {
            s.pushTimelineNode({
              id: nodeId,
              phase,
              agentId: nodeId,
              agentName: agentDisplayName(nodeId),
              title: agentDisplayName(nodeId),
              summary: text.slice(0, 200),
              confidence: 0.5,
              status: "done",
              evidenceRefs: inferEvidenceRefs(nodeId),
              startedAt: Date.now(),
              finishedAt: Date.now(),
            });
          }
          // ── spec §6.2: 3 阶段 LLM 未来引用检测 ──
          // 仅在 as-of 模式下激活;live 模式(asOfDate=null)不做检测
          const asOf = s.asOfDate;
          if (asOf) {
            const newViolations = detectFutureReferencesForNode(nodeId, text, asOf);
            if (newViolations.length > 0) {
              set({
                violations: [...s.violations, ...newViolations],
              });
            }
          }
          if (nodeId.startsWith("a-") && !nodeId.includes("bull") && !nodeId.includes("bear")) {
            set({ analystReports: { ...s.analystReports, [nodeId.slice(2)]: text } });
          } else if (nodeId === "bull-researcher" || (nodeId.startsWith("bull-r") && nodeId !== "bull-researcher")) {
            const round = nodeId === "bull-researcher" ? 1 : parseInt(nodeId.slice(6), 10);
            const debates = [...s.debateRounds];
            const idx = debates.findIndex((d) => d.round === round);
            if (idx >= 0) {
              debates[idx] = { ...debates[idx], bull: text };
            } else {
              debates.push({ round, bull: text, bear: "" });
            }
            debates.sort((a, b) => a.round - b.round);
            set({ debateRounds: debates });
          } else if (nodeId === "bear-researcher" || (nodeId.startsWith("bear-r") && nodeId !== "bear-researcher")) {
            const round = nodeId === "bear-researcher" ? 1 : parseInt(nodeId.slice(6), 10);
            const debates = [...s.debateRounds];
            const idx = debates.findIndex((d) => d.round === round);
            if (idx >= 0) {
              debates[idx] = { ...debates[idx], bear: text };
            } else {
              debates.push({ round, bull: "", bear: text });
            }
            debates.sort((a, b) => a.round - b.round);
            set({ debateRounds: debates });
          } else if (nodeId.startsWith("risk-") || nodeId === "research-mgr") {
            set({ riskAssessments: { ...s.riskAssessments, [nodeId]: text } });
          } else if (nodeId === "trader") {
            set({ analystReports: { ...s.analystReports, "investment-plan": text } });
          } else if (nodeId === "portfolio-mgr") {
            const parsed = tryParseDecision(text);
            set({
              decision: parsed ?? {
                action: StockAction.HOLD,
                positionPct: 0,
                targetPrice: null,
                stopLoss: null,
                reasoning: text,
                riskLevel: StockRiskLevel.MID,
                confidence: 0,
              },
            });
          } else if (nodeId === "value-investor") {
            set({ valueAssessments: { ...s.valueAssessments, [nodeId]: text } });
          } else if (nodeId === "data-quality") {
            set({ dataQualitySummary: text });
          } else if (nodeId === "raw-data") {
            set({ rawData: { ...s.rawData, [nodeId]: text } });
          } else if (nodeId === "rule-check") {
            set({ ruleCheckResults: { ...s.ruleCheckResults, [nodeId]: text } });
          }
        }
      });
      unlisteners.push(u1);
    } catch (e) {
      console.error("[StockAnalysis] Failed to listen workflow-step-done:", e);
    }

    try {
      const u2 = await listen<{
        workflowId: string;
        results: Record<string, unknown>;
        output?: unknown;
      }>("workflow-completed", (event) => {
        const { results, output } = event.payload;

        // 优先从 portfolio-mgr 节点结果中提取决策（与分析页一致）
        let decision: StockDecision | null = null;
        const pmRaw = results["portfolio-mgr"];
        if (pmRaw) {
          decision = extractDecision(pmRaw);
        }

        // 回退：从 output 中提取 decision
        if (!decision && output !== undefined) {
          decision = extractDecision(output);
        }

        // 回退：从 parseWorkflowResults 中获取
        if (!decision) {
          const parsed = parseWorkflowResults(results);
          decision = parsed.decision;
        }

        const parsed = parseWorkflowResults(results);
        // 增量合并 workflow-step-done 已填充的数据，避免覆盖实时进度
        const s = get();
        set({
          analystReports: { ...s.analystReports, ...parsed.analystReports },
          debateRounds: parsed.debateRounds.length > 0 ? parsed.debateRounds : s.debateRounds,
          riskAssessments: { ...s.riskAssessments, ...parsed.riskAssessments },
          valueAssessments: { ...s.valueAssessments, ...parsed.valueAssessments },
          ruleCheckResults: { ...s.ruleCheckResults, ...parsed.ruleCheckResults },
          dataQualitySummary: parsed.dataQualitySummary || s.dataQualitySummary,
          rawData: { ...s.rawData, ...parsed.rawData },
          decision,
          status: "completed",
          progressMessage: i18n.t("stockAnalysis.progress.completed"),
          progressPct: 100,
          currentStage: 4,
        });

        // 荐股 ↔ 分析师交叉验证：把本次的分析师投票结果缓存到 stockCodeConsensus
        // RecommendationPanel 会读取这个缓存来提示用户"推荐与共识是否一致"。
        const stockCode = get().stockCode;
        if (stockCode && parsed.analystReports && Object.keys(parsed.analystReports).length > 0) {
          const consensus = computeStockConsensus(parsed.analystReports, undefined, get().decision?.timeHorizon);
          get().setStockCodeConsensus(stockCode, consensus);
        }
      });
      unlisteners.push(u2);
    } catch (e) {
      console.error("[StockAnalysis] Failed to listen workflow-completed:", e);
    }

    try {
      const u3 = await listen<{
        workflowId: string;
        error: string;
        errorCode?: string;
        results?: Record<string, unknown>;
        output?: unknown;
      }>("workflow-error", (event) => {
        const msg = event.payload.error;
        const { results, errorCode, output } = event.payload;

        // 即使失败也尝试解析已有的部分结果
        if (results) {
          const parsed = parseWorkflowResults(results);
          set({
            analystReports: parsed.analystReports,
            debateRounds: parsed.debateRounds,
            riskAssessments: parsed.riskAssessments,
            valueAssessments: parsed.valueAssessments,
            ruleCheckResults: parsed.ruleCheckResults,
            dataQualitySummary: parsed.dataQualitySummary,
            rawData: parsed.rawData,
            decision: parsed.decision,
          });
        } else if (output) {
          // 兜底：从 output 中解析决策（少数异常路径无 results 但含 output）
          const parsed = extractDecision(output);
          if (parsed) { set({ decision: parsed }); }
        }

        // 修复 #9: 优先用结构化 errorCode，回退到 msg.includes("LLM") 字符串判断
        const effectiveErrorCode = errorCode ?? (msg.includes("LLM") ? "LLM_FALLBACK" : "GENERIC_ERROR");
        const isLlmError = effectiveErrorCode.startsWith("LLM_");
        set({
          error: msg,
          errorCode: effectiveErrorCode,
          status: isLlmError ? "running" : "error",
          llmStatus: isLlmError ? "placeholder" : get().llmStatus,
          progressMessage: isLlmError
            ? i18n.t("stockAnalysis.progress.llmFallback")
            : msg,
          progressPct: 100,
          currentStage: 4,
        });
      });
      unlisteners.push(u3);
    } catch (e) {
      console.error("[StockAnalysis] Failed to listen workflow-error:", e);
    }

    try {
      const u4 = await listen<{
        stockCode: string;
        reason: string;
        currentPrice: number;
        changePct: number;
        turnoverRate: number;
        timestamp: number;
      }>("stock-monitor-t0-rerun-requested", (event) => {
        const { stockCode, reason } = event.payload;
        // 防抖: 当前正在跑 workflow 就不重入
        const cur = get();
        if (cur.status === "running" || cur.status === "loading") {
          console.warn(`[t0] skip ${stockCode}: workflow 已在运行中`);
          return;
        }
        console.info(
          `[t0] 收到 T+0 重跑请求: stock=${stockCode} reason=${reason}`,
        );
        // 直接调 store 内的 startAnalysis (它会拉 quote/kline 再触发 workflow)
        get().startAnalysis(stockCode);
      });
      unlisteners.push(u4);
    } catch (e) {
      console.error("[StockAnalysis] Failed to listen stock-monitor-t0-rerun-requested:", e);
    }

    set({
      _unlisten: unlistenAll,
    });
  },
}));

/** 从节点 ID 推断当前管线阶段 */
function inferStage(nodeId: string): number {
  // 触发器节点（工作流入口）→ 阶段 0（数据准备）
  if (nodeId === "trigger") { return 0; }
  // 工具节点 t-* （t-fundamentals-data / t-news-data / t-policy-data /
  // t-research-data / t-scoring / t-valuation / t-risk）→ 阶段 1
  // 这些节点给 a-* 分析师提供数据，与分析师同属"数据采集与分析"阶段。
  if (nodeId.startsWith("t-")) { return 1; }
  // a-* 分析师节点 → 阶段 1
  if (nodeId.startsWith("a-")) { return 1; }
  // 装饰节点 p-analysts（分析师容器）→ 阶段 1
  if (nodeId === "p-analysts") { return 1; }
  // 辩论相关节点 → 阶段 2
  // - debate-bull-bear：装饰容器（容器本身立即成功，但前端希望进度能反映
  //   用户已进入辩论阶段）
  // - bull-r{1,2,3} / bear-r{1,2,3}：实际辩论节点（后端统一使用 bull-rN 命名）
  // - bull-researcher / bear-researcher：早期版本使用的别名（已不再生成，
  //   但保留兼容以便历史快照/外部测试用例不丢阶段号）
  if (nodeId === "debate-bull-bear") { return 2; }
  if (
    nodeId === "bull-researcher" || nodeId === "bear-researcher" || nodeId.startsWith("bull-r")
    || nodeId.startsWith("bear-r")
  ) { return 2; }
  // 风险评估阶段节点 → 阶段 3
  // - value-investor：巴菲特框架（与 risk-evaluator 并行运行）
  // - risk-agg / risk-con / risk-neu：激进/保守/中性风险评估
  // - research-mgr：研究主管
  // - p-risk-assess：装饰容器
  if (
    nodeId === "value-investor" || nodeId.startsWith("risk-") || nodeId === "research-mgr"
    || nodeId === "p-risk-assess"
  ) { return 3; }
  // 决策阶段节点 → 阶段 4
  if (nodeId === "trader" || nodeId === "portfolio-mgr") { return 4; }
  // 决策后处理节点 → 阶段 4（最大阶段，进度 100%）
  if (nodeId === "agg-risk" || nodeId === "cls-risk-level" || nodeId === "v-validate" || nodeId === "notify-result") {
    return 4;
  }
  // P3 (real-nodes) 决策辅助节点：
  // - data-quality：数据质量检查（v-validate 之后启动）→ 阶段 3
  // - raw-data：12 个 t-* 工具节点原始数据聚合（t-risk 之后启动）→ 阶段 3
  // - rule-check：硬性规则检查（portfolio-mgr 之后启动）→ 阶段 4
  if (nodeId === "data-quality" || nodeId === "raw-data") { return 3; }
  if (nodeId === "rule-check") { return 4; }
  return -1;
}

// 暴露给单元测试使用
export { inferStage };

// ── Decision Timeline helpers（Phase 8）──

/** 从节点 ID 推断时间线 4 阶段之一；非业务节点返回 null 不进 timeline */
function inferTimelinePhase(nodeId: string): TimelinePhase | null {
  // scan: 工具节点（数据采集）
  if (nodeId.startsWith("t-")) { return "scan"; }
  // diagnose: 分析师节点
  if (nodeId.startsWith("a-")) { return "diagnose"; }
  // debate: bull/bear 辩论（含早期别名）
  if (
    nodeId === "bull-researcher" || nodeId === "bear-researcher" || nodeId.startsWith("bull-r")
    || nodeId.startsWith("bear-r")
  ) { return "debate"; }
  // decide: 决策与决策后处理
  if (
    nodeId === "trader" || nodeId === "portfolio-mgr" || nodeId === "rule-check"
    || nodeId === "value-investor" || nodeId === "research-mgr" || nodeId.startsWith("risk-")
  ) { return "decide"; }
  return null;
}

/** Agent 显示名：去前缀 + 首字母大写。a-tech-analyst → Tech Analyst */
function agentDisplayName(nodeId: string): string {
  const stripped = nodeId
    .replace(/^a-/, "")
    .replace(/^t-/, "")
    .replace(/^bull-/, "")
    .replace(/^bear-/, "")
    .replace(/-/g, " ");
  return stripped.replace(/\b\w/g, (c) => c.toUpperCase());
}

/** 节点 → 证据引用：根据 nodeId 推断其结果会落在哪个侧栏 sheet panel */
function inferEvidenceRefs(
  nodeId: string,
): Array<{ tabKey: "market" | "analyze" | "execute"; panelKey: string; snippet: string }> {
  // 工具节点 → 行情/概念面板
  if (nodeId === "t-fundamentals-data" || nodeId === "t-valuation") {
    return [{ tabKey: "market", panelKey: "concepts", snippet: "基本面/估值数据" }];
  }
  if (nodeId === "t-news-data" || nodeId === "t-policy-data") {
    return [{ tabKey: "market", panelKey: "announcements", snippet: "新闻/政策公告" }];
  }
  if (nodeId === "t-research-data") {
    return [{ tabKey: "market", panelKey: "industry", snippet: "行业排名" }];
  }
  if (nodeId === "t-scoring") {
    return [{ tabKey: "market", panelKey: "screener", snippet: "推荐评分" }];
  }
  if (nodeId === "t-risk") {
    return [{ tabKey: "market", panelKey: "north", snippet: "北向资金/风险" }];
  }
  // 分析师节点 → 报告
  if (nodeId.startsWith("a-")) {
    return [{ tabKey: "analyze", panelKey: "analysts", snippet: "分析师报告" }];
  }
  // 辩论 → 辩论
  if (nodeId.startsWith("bull-") || nodeId.startsWith("bear-")) {
    return [{ tabKey: "analyze", panelKey: "debate", snippet: "多空辩论" }];
  }
  // 风险/研究/价值/规则
  if (nodeId.startsWith("risk-") || nodeId === "research-mgr") {
    return [{ tabKey: "analyze", panelKey: "risk", snippet: "Risk Assessment" }];
  }
  if (nodeId === "value-investor") {
    return [{ tabKey: "analyze", panelKey: "value", snippet: "Valuation" }];
  }
  if (nodeId === "trader") {
    return [{ tabKey: "execute", panelKey: "trade", snippet: "Trading Plan" }];
  }
  if (nodeId === "portfolio-mgr" || nodeId === "rule-check") {
    return [{ tabKey: "analyze", panelKey: "decision", snippet: "Final Decision" }];
  }
  return [];
}
