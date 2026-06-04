import i18n from "@/i18n";
import { extractContent } from "@/lib/agentOutput";
import { invoke, listen } from "@/lib/invoke";
import type { UnlistenFn } from "@/lib/invoke";
import type { AnalysisStatus, AnalysisSummary, KLine, StockDecision, StockQuote, StockSearchResult } from "@/types";
import { parseAction, parseRiskLevel, StockAction, StockRiskLevel } from "@/types";
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
      const parsed = tryParseDecision(output);
      decision = parsed ?? {
        action: StockAction.HOLD,
        positionPct: 0,
        targetPrice: null,
        stopLoss: null,
        reasoning: output,
        riskLevel: StockRiskLevel.MID,
        confidence: 0,
      };
    }
  }

  debateRounds.sort((a, b) => a.round - b.round);
  return { analystReports, debateRounds, riskAssessments, decision };
}

// ── Store ──

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
    });

    // 先注册事件监听，再触发工作流
    try {
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

      if (status === "completed" && output != null) {
        const text = extractContent(output);
        const s = get();
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
      const parsed = parseWorkflowResults(results);
      let decision: StockDecision | null = parsed.decision;
      if (output && typeof output === "object") {
        decision = normalizeDecision(output as Record<string, unknown>);
      } else if (typeof output === "string") {
        const tryParsed = tryParseDecision(output);
        if (tryParsed) { decision = tryParsed; }
      }
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
      errorCode?: string;
      results?: Record<string, unknown>;
      output?: StockDecision | null;
    }>("workflow-error", (event) => {
      const msg = event.payload.error;
      // 即使失败也尝试解析已有的部分结果
      const { results, output, errorCode } = event.payload;
      if (results) {
        const parsed = parseWorkflowResults(results);
        set({ ...parsed, decision: output ?? parsed.decision });
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
  if (
    nodeId === "bull-researcher" || nodeId === "bear-researcher" || nodeId.startsWith("bull-r")
    || nodeId.startsWith("bear-r")
  ) { return 2; }
  if (nodeId.startsWith("risk-") || nodeId === "research-mgr") { return 3; }
  if (nodeId === "trader" || nodeId === "portfolio-mgr") { return 4; }
  if (nodeId === "agg-risk" || nodeId === "cls-risk-level" || nodeId === "v-validate" || nodeId === "notify-result") {
    return 4; // 决策后处理阶段
  }
  return -1;
}

// 暴露给单元测试使用
export { inferStage };
