import { invoke, listen } from "@/lib/invoke";
import type { UnlistenFn } from "@/lib/invoke";
import type {
  AnalysisEvent,
  AnalysisStatus,
  AnalysisSummary,
  KLine,
  StockDecision,
  StockQuote,
  StockSearchResult,
} from "@/types";
import { ANALYST_NAMES } from "@/types";
import i18n from "i18next";
import { create } from "zustand";

interface StockAnalysisState {
  // Search
  searchKeyword: string;
  searchResults: StockSearchResult[];

  // Current analysis
  analysisId: string | null;
  stockCode: string;
  stockName: string;
  analysisDate: string;
  status: AnalysisStatus;

  // Data
  quote: StockQuote | null;
  klineData: KLine[];
  analystReports: Record<string, string>;
  debateRounds: Array<{ round: number; bull: string; bear: string }>;
  riskAssessments: Record<string, string>;
  decision: StockDecision | null;
  error: string | null;

  // History
  history: AnalysisSummary[];

  // 当前管线阶段 (0=数据加载 1=分析 2=辩论 3=风控 4=决策)
  currentStage: number;

  // 实时进度提示文本（给用户展示当前正在做什么）
  progressMessage: string;
  // 整体进度百分比 (0-100)
  progressPct: number;

  // LLM 连接状态
  llmStatus: "live" | "placeholder" | "unknown";

  // Chat 指示器是否已关闭
  chatIndicatorDismissed: boolean;

  // Actions
  searchStock: (keyword: string) => Promise<void>;
  getStockQuote: (code: string) => Promise<void>;
  getStockKline: (code: string, period: string, limit: number) => Promise<void>;
  startAnalysis: (stockCode: string, date: string, providerId: string) => Promise<void>;
  cancelAnalysis: () => Promise<void>;
  fetchHistory: (limit?: number, offset?: number) => Promise<void>;
  loadAnalysis: (analysisId: string) => Promise<void>;
  reset: () => void;
  dismissChatIndicator: () => void;

  // Event listeners
  _unlisten: UnlistenFn | null;
  setupEventListener: () => Promise<void>;
  // 搜索防抖定时器
  _searchTimer: ReturnType<typeof setTimeout> | null;
}

const initialState = {
  searchKeyword: "",
  searchResults: [],
  analysisId: null,
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
    // 防抖 300ms
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

  startAnalysis: async (stockCode: string, date: string, providerId: string) => {
    const { status } = get();
    if (status === "loading" || status === "running") {
      console.warn("[StockAnalysis] Analysis already in progress, ignoring duplicate start");
      return;
    }
    set({
      status: "loading",
      error: null,
      currentStage: 0,
      progressMessage: i18n.t("stockAnalysis.progress.fetchingData"),
      progressPct: 0,
      chatIndicatorDismissed: false,
      analystReports: {},
      debateRounds: [],
      riskAssessments: {},
      decision: null,
    });

    const result = await invoke<{
      analysis_id: string;
      stock_code: string;
      stock_name: string;
      status: string;
    }>("start_stock_analysis", { stockCode, date, providerId });

    set({
      analysisId: result.analysis_id,
      stockCode: result.stock_code,
      stockName: result.stock_name,
      analysisDate: date,
      status: "running",
    });

    // 自动加载行情数据供市场 tab 使用
    get().getStockQuote(result.stock_code);
    get().getStockKline(result.stock_code, "daily", 120);
  },

  cancelAnalysis: async () => {
    const { analysisId } = get();
    if (analysisId) {
      await invoke("cancel_stock_analysis", { analysisId });
    }
    set({ status: "idle" });
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
    // 从 blackboardSnapshot 恢复历史分析报告
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

    const unlisten = await listen<AnalysisEvent>("stock-analysis-event", (event) => {
      const { type, payload } = event.payload;
      switch (type) {
        case "started":
          set({ status: "running", progressMessage: i18n.t("stockAnalysis.progress.started"), progressPct: 5 });
          break;
        case "dataLoaded": {
          const dlPayload = payload as Record<string, unknown>;
          set({
            currentStage: 0,
            progressMessage: i18n.t("stockAnalysis.progress.dataLoaded", {
              kline: dlPayload.klineCount ?? "?",
              news: dlPayload.newsCount ?? "?",
            }),
            progressPct: 10,
          });
          break;
        }
        case "analystProgress": {
          const ap = payload as Record<string, unknown>;
          const expertId = ap.expertId as string;
          const status = ap.status as string;
          const pct = ap.progressPct as number;
          const stage = inferStage(expertId);
          if (stage >= 0) { set({ currentStage: stage }); }
          const name = ANALYST_NAMES[expertId] ?? expertId;
          set({
            progressMessage: i18n.t("stockAnalysis.progress.analystProgress", { name, status }),
            progressPct: pct > 0 ? Math.max(pct, get().progressPct) : get().progressPct,
          });
          break;
        }
        case "analystReport": {
          const { expertId, reportText } = payload as Record<string, string>;
          set((s) => ({
            analystReports: { ...s.analystReports, [expertId]: reportText },
          }));
          const name = ANALYST_NAMES[expertId] ?? expertId;
          set({ progressMessage: i18n.t("stockAnalysis.progress.reportReady", { name }) });
          break;
        }
        case "debateRound": {
          const { round, bullArgument, bearArgument } = payload as Record<string, unknown>;
          set((s) => ({
            debateRounds: [
              ...s.debateRounds,
              {
                round: round as number,
                bull: bullArgument as string,
                bear: bearArgument as string,
              },
            ],
            progressMessage: i18n.t("stockAnalysis.progress.debateRound", { round: round as number, total: 3 }),
          }));
          break;
        }
        case "riskAssessment": {
          const { riskType, report } = payload as Record<string, string>;
          set((s) => ({
            riskAssessments: { ...s.riskAssessments, [riskType]: report },
          }));
          const name = ANALYST_NAMES[riskType] ?? riskType;
          set({ progressMessage: i18n.t("stockAnalysis.progress.riskDone", { name }) });
          break;
        }
        case "investmentPlan": {
          const { plan } = payload as Record<string, string>;
          set((s) => ({
            analystReports: { ...s.analystReports, "investment-plan": plan },
            progressMessage: i18n.t("stockAnalysis.progress.investmentPlan"),
            progressPct: 85,
          }));
          break;
        }
        // NOTE: payload 与 StockDecision 的结构由 serde(rename_all="camelCase") 保证一致
        // 若后端修改 Decision 变体字段，此处需同步更新
        case "decision":
          set({
            decision: payload as unknown as StockDecision,
            status: "completed",
            progressMessage: i18n.t("stockAnalysis.progress.completed"),
            progressPct: 100,
          });
          break;
        case "error": {
          const msg = (payload as Record<string, string>).message;
          set({
            error: msg,
            status: msg.includes("LLM") ? "running" : "error",
            llmStatus: msg.includes("LLM") ? "placeholder" : get().llmStatus,
            progressMessage: msg.includes("LLM")
              ? i18n.t("stockAnalysis.progress.llmFallback")
              : i18n.t("stockAnalysis.progress.error", { msg }),
          });
          break;
        }
      }
    });

    set({ _unlisten: unlisten });
  },
}));

/** 从 expert_id 推断当前管线阶段 */
function inferStage(expertId: string): number {
  if (expertId === "indicators") { return 0; }
  if (ANALYST_STAGE_IDS.has(expertId)) { return 1; }
  if (expertId === "debate") { return 2; }
  if (RISK_STAGE_IDS.has(expertId)) { return 3; }
  if (expertId === "trader" || expertId === "portfolio-manager") { return 4; }
  return -1;
}

const ANALYST_STAGE_IDS = new Set([
  "market-analyst",
  "sentiment-analyst",
  "news-analyst",
  "fundamentals-analyst",
  "policy-analyst",
  "hot-money-tracker",
  "lockup-watcher",
  "value-investor",
]);

const RISK_STAGE_IDS = new Set([
  "aggressive-debator",
  "conservative-debator",
  "neutral-debator",
  "research-manager",
]);
