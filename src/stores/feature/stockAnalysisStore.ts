import i18n from "@/i18n";
import { extractContent } from "@/lib/agentOutput";
import { invoke, listen } from "@/lib/invoke";
import type { UnlistenFn } from "@/lib/invoke";
import { detectFutureReferencesForNode } from "@/lib/timeTravel/futureReferenceDetector";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import type {
  AnalysisStatus,
  AnalysisSummary,
  KLine,
  StockConsensus,
  StockDecision,
  StockQuote,
  StockSearchResult,
  TimelineNode,
  TimelinePhase,
} from "@/types";
import { computeStockConsensus, parseAction, parseRiskLevel, StockAction, StockRiskLevel } from "@/types";
import { create } from "zustand";

// ── 工作流结果解析 ──

/** 规范化 decision 对象：兼容 snake_case/camelCase、置信度 0-100、空值保护 */
function normalizeDecision(raw: Record<string, unknown>): StockDecision {
  const action = parseAction(raw.action ?? raw["action"]);
  const positionPct = Number(raw.positionPct ?? raw.position_pct ?? 0);
  const targetPrice = raw.targetPrice != null
    ? Number(raw.targetPrice)
    : (raw.target_price != null ? Number(raw.target_price) : null);
  const stopLoss = raw.stopLoss != null ? Number(raw.stopLoss) : (raw.stop_loss != null ? Number(raw.stop_loss) : null);
  const reasoning = String(raw.reasoning ?? "");
  const riskLevel = parseRiskLevel(raw.riskLevel ?? raw.risk_level);
  const confidence = Math.round(Math.max(0, Math.min(100, Number(raw.confidence ?? 0))));
  return {
    action,
    positionPct: isNaN(positionPct) ? 0 : positionPct,
    targetPrice: targetPrice != null && !isNaN(targetPrice) ? targetPrice : null,
    stopLoss: stopLoss != null && !isNaN(stopLoss) ? stopLoss : null,
    reasoning,
    riskLevel,
    confidence,
  };
}

/** 尝试从文本中解析 JSON decision（兼容 markdown 代码块包裹） */
function tryParseDecision(text: string): StockDecision | null {
  const trimmed = text.trim();
  const candidates = [trimmed];
  const m = trimmed.match(/```(?:json)?\s*([\s\S]*?)\s*```/);
  if (m) { candidates.unshift(m[1].trim()); }
  for (const candidate of candidates) {
    if (!candidate.startsWith("{")) { continue; }
    try {
      const parsed = JSON.parse(candidate);
      if (typeof parsed === "object" && parsed !== null) { return normalizeDecision(parsed); }
    } catch { /* try next */ }
  }
  return null;
}

/** 从工作流 step results 解析结构化状态 */
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
      const parsed = tryParseDecision(output);
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

  history: AnalysisSummary[];

  currentStage: number;
  progressMessage: string;
  progressPct: number;

  llmStatus: "live" | "placeholder" | "unknown";
  chatIndicatorDismissed: boolean;

  // Phase 1: K-line period persistence cross-mount
  klinePeriod: string;
  setKlinePeriod: (period: string) => void;

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
  setAsOfDate: (date: string | null) => void;
  setMode: (mode: "live" | "replay" | "backtest_sweep") => void;
  setViolations: (
    v: Array<{ nodeId: string; snippet: string; ruleHit: string }>,
  ) => void;

  // Actions
  searchStock: (keyword: string) => Promise<void>;
  getStockQuote: (code: string) => Promise<void>;
  getStockKline: (code: string, period: string, limit: number) => Promise<void>;
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

  _unlisten: UnlistenFn | null;
  setupEventListener: () => Promise<void>;
  _searchTimer: ReturnType<typeof setTimeout> | null;
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
  history: [],
  currentStage: 0,
  progressMessage: "",
  progressPct: 0,
  llmStatus: "unknown" as const,
  chatIndicatorDismissed: false,
  klinePeriod: "6m",
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
};

export const useStockAnalysisStore = create<StockAnalysisState>((set, get) => ({
  ...initialState,
  _unlisten: null,
  _searchTimer: null,

  searchStock: async (keyword: string) => {
    set({ searchKeyword: keyword });
    if (keyword.length < 2) {
      set({ searchResults: [] });
      return;
    }
    const { _searchTimer } = get();
    if (_searchTimer) { clearTimeout(_searchTimer); }
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
      // 时间旅行：从 timeAnchorStore 读 as_of_date，透传给后端
      const asOfDate = useTimeAnchorStore.getState().asOfDate;
      const quote = await invoke<StockQuote>("get_stock_quote", { stockCode: code, asOfDate });
      set({ quote, stockCode: code, stockName: quote.name });
    } catch (e) {
      console.error("[StockAnalysis] Failed to get stock quote:", e);
    }
  },

  getStockKline: async (code: string, period: string, limit: number) => {
    try {
      // 时间旅行：K 线按 as_of_date 截断
      const asOfDate = useTimeAnchorStore.getState().asOfDate;
      const klineData = await invoke<KLine[]>("get_stock_kline", {
        stockCode: code,
        period,
        limit,
        asOfDate,
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
      const tmpl: any = await invoke("get_workflow_template", { id: "stock-analysis" });
      const vars: any[] = tmpl?.variables ?? [];
      const v = vars.find((x: any) => x.name === "analysis_dry_run");
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
    });

    // 先注册事件监听，再触发工作流
    try {
      await get().setupEventListener();

      // 数据源健康检查（非阻塞，仅打日志）
      const VENDORS = ["eastmoney", "sina", "tencent", "akshare"];
      for (const v of VENDORS) {
        invoke("check_vendor_health", { vendor: v }).catch((e) => {
          const msg = e instanceof Error ? e.message : typeof e === "string" ? e : JSON.stringify(e);
          console.warn(`[StockAnalysis] Vendor ${v} health check failed: ${msg}`);
        });
      }

      const dryRun = await get().getDryRun();
      // 时间旅行模式：从 useTimeAnchorStore 读 as_of_date，透传给后端
      const asOfDate = useTimeAnchorStore.getState().asOfDate;
      const anchorMode = useTimeAnchorStore.getState().mode;
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

      get().getStockQuote(result.stockCode);
      get().getStockKline(result.stockCode, "daily", 120);
    } catch (e) {
      console.error("[StockAnalysis] Failed to start workflow:", e);
      set({
        status: "error",
        error: typeof e === "string" ? e : (e as Error)?.message ?? i18n.t("stockAnalysis.workflow.startFailed"),
        progressPct: 0,
      });
    }
  },

  cancelAnalysis: async () => {
    const { workflowId } = get();
    if (workflowId) {
      await invoke("cancel_stock_workflow", { workflowId });
    }
    set({ status: "idle", currentStage: 0, progressPct: 0 });
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
            computeStockConsensus(reports, Date.now()),
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
    // Persist to localStorage
    if (typeof window !== "undefined") {
      const next = get().sidebarCollapsed;
      try {
        window.localStorage.setItem("ax_sidebar_collapsed", JSON.stringify(next));
      } catch { /* noop */ }
    }
  },

  reset: () => {
    const { _unlisten } = get();
    if (_unlisten) {
      _unlisten();
    }
    set({ ...initialState, _unlisten: null, llmStatus: "unknown" as const });
  },

  setAsOfDate: (date) => set({ asOfDate: date }),
  setMode: (mode) => set({ mode }),
  setViolations: (violations) => set({ violations }),

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

    // 中间步骤进度事件
    const unlistenStep = await listen<{
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
          // 辩论子节点: 实际 nodeId 为 "bull-researcher" (DAG 引擎单次执行)
          // 兼容未来多轮模式: bull-r1, bull-r2...
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
        }
      }
    });

    // 工作流完成事件（Completed / PartiallyCompleted）
    const unlistenComplete = await listen<{
      workflowId: string;
      results: Record<string, unknown>;
      output?: unknown;
    }>("workflow-completed", (event) => {
      const { results, output } = event.payload;

      // 优先从 portfolio-mgr 节点结果中提取决策（与分析页一致）
      let decision: StockDecision | null = null;
      const pmRaw = results["portfolio-mgr"];
      if (pmRaw) {
        const pmText = typeof pmRaw === "string"
          ? pmRaw
          : (pmRaw as Record<string, unknown>).content ?? JSON.stringify(pmRaw);
        const parsed = tryParseDecision(String(pmText));
        if (parsed) { decision = parsed; }
      }

      // 回退：从 parseWorkflowResults 中获取
      if (!decision) {
        const parsed = parseWorkflowResults(results);
        decision = parsed.decision;
      }

      // 最后回退：尝试 output（最后一个节点输出，通常是 trader）
      if (!decision && output) {
        if (typeof output === "object" && output !== null) {
          decision = normalizeDecision(output as Record<string, unknown>);
        } else if (typeof output === "string") {
          const tryParsed = tryParseDecision(output);
          if (tryParsed) { decision = tryParsed; }
        }
      }

      const parsed = parseWorkflowResults(results);
      set({
        ...parsed,
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
        const consensus = computeStockConsensus(parsed.analystReports);
        get().setStockCodeConsensus(stockCode, consensus);
      }
    });

    const unlistenError = await listen<{
      workflowId: string;
      error: string;
      errorCode?: string;
      results?: Record<string, unknown>;
      output?: StockDecision | null;
    }>("workflow-error", (event) => {
      const msg = event.payload.error;
      const { results, output, errorCode } = event.payload;

      // 即使失败也尝试解析已有的部分结果（优先 portfolio-mgr，与分析页一致）
      let decision: StockDecision | null = null;
      if (results) {
        const pmRaw = results["portfolio-mgr"];
        if (pmRaw) {
          const pmText = typeof pmRaw === "string"
            ? pmRaw
            : (pmRaw as Record<string, unknown>).content ?? JSON.stringify(pmRaw);
          const parsed = tryParseDecision(String(pmText));
          if (parsed) { decision = parsed; }
        }
        if (!decision) {
          const parsed = parseWorkflowResults(results);
          decision = parsed.decision;
        }
      }
      // 最后回退：尝试 output
      if (!decision && output) {
        decision = output;
      }

      // 空决策（全 0、无目标价、无理由）不写入 store，
      // 避免 DecisionBanner 渲染无意义的 0% 仓位 / ¥0 目标价。
      if (
        decision
        && decision.confidence === 0
        && decision.positionPct === 0
        && decision.targetPrice == null
        && decision.stopLoss == null
        && (!decision.reasoning || decision.reasoning.trim() === "")
      ) {
        decision = null;
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
        decision,
      });
    });

    set({
      _unlisten: () => {
        unlistenStep();
        unlistenComplete();
        unlistenError();
      },
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
    return [{ tabKey: "analyze", panelKey: "risk", snippet: "风险评估" }];
  }
  if (nodeId === "value-investor") {
    return [{ tabKey: "analyze", panelKey: "value", snippet: "价值评估" }];
  }
  if (nodeId === "trader") {
    return [{ tabKey: "execute", panelKey: "trade", snippet: "交易计划" }];
  }
  if (nodeId === "portfolio-mgr" || nodeId === "rule-check") {
    return [{ tabKey: "analyze", panelKey: "decision", snippet: "最终决策" }];
  }
  return [];
}
