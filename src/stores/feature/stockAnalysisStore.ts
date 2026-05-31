import { invoke, listen } from "@/lib/invoke";
import type { UnlistenFn } from "@/lib/invoke";
import type { AnalysisStatus, AnalysisSummary, KLine, StockDecision, StockQuote, StockSearchResult } from "@/types";
import i18n from "i18next";
import { create } from "zustand";

// ── 工作流结果解析 ──

/** AgentExecutor 输出的 JSON 结构 */
interface AgentResult {
  role?: string;
  model?: string;
  content?: string;
  thinking?: string;
  usage?: { input_tokens?: number; output_tokens?: number };
  node_id?: string;
}

/** 从 AgentExecutor 输出中提取纯文本内容 */
function extractContent(value: unknown): string {
  if (typeof value === "string") { return value; }
  if (value && typeof value === "object") {
    const r = value as AgentResult;
    return r.content ?? JSON.stringify(value);
  }
  return String(value ?? "");
}

/** 从工作流 step results 解析结构化状态 */
function parseWorkflowResults(results: Record<string, unknown>) {
  const analystReports: Record<string, string> = {};
  const debateRounds: Array<{ round: number; bull: string; bear: string }> = [];
  const riskAssessments: Record<string, string> = {};
  let decision: StockDecision | null = null;

  for (const [stepId, raw] of Object.entries(results)) {
    const output = extractContent(raw);

    if (stepId.startsWith("a-") && !stepId.includes("bull") && !stepId.includes("bear")) {
      analystReports[stepId.slice(2)] = output;
    } else if (stepId.startsWith("bull-r")) {
      const round = parseInt(stepId.slice(6), 10);
      const bearKey = `bear-r${round}`;
      debateRounds.push({ round, bull: output, bear: extractContent(results[bearKey] ?? "") });
    } else if (stepId.startsWith("bear-r")) {
      continue; // 已在 bull 处理时配对
    } else if (stepId.startsWith("risk-") || stepId === "research-mgr") {
      riskAssessments[stepId] = output;
    } else if (stepId === "trader") {
      analystReports["investment-plan"] = output;
    } else if (stepId === "portfolio-mgr") {
      try {
        decision = JSON.parse(output) as StockDecision;
      } catch {
        decision = {
          action: "HOLD",
          positionPct: 0,
          targetPrice: null,
          stopLoss: null,
          reasoning: output,
          riskLevel: i18n.t("stockAnalysis.riskUnknown"),
          confidence: 0,
        };
      }
    }
  }

  debateRounds.sort((a, b) => a.round - b.round);
  return { analystReports, debateRounds, riskAssessments, decision };
}

// ── Store ──

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
  decision: StockDecision | null;
  error: string | null;

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
  decision: null,
  error: null,
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
      const quote = await invoke<StockQuote>("get_stock_quote", { stockCode: code });
      set({ quote, stockCode: code, stockName: quote.name });
    } catch (e) {
      console.error("[StockAnalysis] Failed to get stock quote:", e);
    }
  },

  getStockKline: async (code: string, period: string, limit: number) => {
    try {
      const klineData = await invoke<KLine[]>("get_stock_kline", {
        stockCode: code,
        period,
        limit,
      });
      set({ klineData });
    } catch (e) {
      console.error("[StockAnalysis] Failed to get kline:", e);
    }
  },

  /** 读取 analysis_dry_run 模板变量 */
  getDryRun: async () => {
    try {
      const tmpl: any = await invoke("get_workflow_template", { id: "stock-analysis" });
      const vars: any[] = tmpl?.variables ?? [];
      const v = vars.find((x: any) => x.name === "analysis_dry_run");
      return !!v?.value;
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

    // 先注册事件监听再启动工作流，防止竞态丢失事件
    const { _unlisten } = get();
    if (_unlisten) { _unlisten(); }

    set({
      status: "loading",
      error: null,
      currentStage: 0,
      workflowId: null,
      progressMessage: i18n.t("stockAnalysis.progress.fetchingData"),
      progressPct: 0,
      chatIndicatorDismissed: false,
      analystReports: {},
      debateRounds: [],
      riskAssessments: {},
      decision: null,
      _unlisten: null,
    });

    // 先注册事件监听，再触发工作流
    await get().setupEventListener();

    const dryRun = await get().getDryRun();
    const result = await invoke<{
      analysisId: string;
      workflowId: string;
      stockCode: string;
      stockName: string;
    }>("run_stock_workflow", { stockCode, dryRun });

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
        set({ decision: JSON.parse(record.decisionJson) });
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
        for (const [key, value] of Object.entries(snap)) {
          if (key.startsWith("report.")) {
            reports[key.slice(7)] = value;
          } else if (key.startsWith("debate.bull.round_")) {
            const round = parseInt(key.slice("debate.bull.round_".length));
            const bearKey = `debate.bear.round_${round}`;
            debates.push({ round: round - 1, bull: value, bear: snap[bearKey] ?? "" });
          } else if (key.startsWith("risk.")) {
            risks[key.slice(5)] = value;
          }
        }
        set({ analystReports: reports, debateRounds: debates, riskAssessments: risks });
      } catch (e) {
        console.error("[StockAnalysis] Failed to restore blackboard snapshot:", e);
      }
    }
  },

  dismissChatIndicator: () => {
    set({ chatIndicatorDismissed: true });
  },

  bumpWatchlistVersion: () => {
    set((s) => ({ watchlistVersion: s.watchlistVersion + 1 }));
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
    }>("workflow-step-done", (event) => {
      const { nodeId, status, totalNodes, completedNodes } = event.payload;
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
      });
    });

    // 工作流完成事件（Completed / PartiallyCompleted）
    const unlistenComplete = await listen<{
      workflowId: string;
      results: Record<string, unknown>;
      output?: StockDecision | null;
    }>("workflow-completed", (event) => {
      const { results, output } = event.payload;
      const parsed = parseWorkflowResults(results);
      const decision = output ?? parsed.decision;
      set({
        ...parsed,
        decision,
        status: "completed",
        progressMessage: i18n.t("stockAnalysis.progress.completed"),
        progressPct: 100,
        currentStage: 4,
      });
    });

    const unlistenError = await listen<{
      workflowId: string;
      error: string;
      results?: Record<string, unknown>;
      output?: StockDecision | null;
    }>("workflow-error", (event) => {
      const msg = event.payload.error;
      // 即使失败也尝试解析已有的部分结果
      const { results, output } = event.payload;
      if (results) {
        const parsed = parseWorkflowResults(results);
        set({ ...parsed, decision: output ?? parsed.decision });
      }
      set({
        error: msg,
        status: msg.includes("LLM") ? "running" : "error",
        llmStatus: msg.includes("LLM") ? "placeholder" : get().llmStatus,
        progressMessage: msg.includes("LLM")
          ? i18n.t("stockAnalysis.progress.llmFallback")
          : msg,
        progressPct: 100,
        currentStage: 4,
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
  if (nodeId.startsWith("a-")) { return 1; }
  if (nodeId.startsWith("bull-r") || nodeId.startsWith("bear-r")) { return 2; }
  if (nodeId.startsWith("risk-") || nodeId === "research-mgr") { return 3; }
  if (nodeId === "trader" || nodeId === "portfolio-mgr") { return 4; }
  return -1;
}
