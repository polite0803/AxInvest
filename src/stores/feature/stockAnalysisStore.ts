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

  // LLM 连接状态
  llmStatus: "live" | "placeholder" | "unknown";

  // Actions
  searchStock: (keyword: string) => Promise<void>;
  getStockQuote: (code: string) => Promise<void>;
  getStockKline: (code: string, period: string, limit: number) => Promise<void>;
  startAnalysis: (stockCode: string, date: string, providerId: string) => Promise<void>;
  cancelAnalysis: () => Promise<void>;
  fetchHistory: (limit?: number, offset?: number) => Promise<void>;
  loadAnalysis: (analysisId: string) => Promise<void>;
  reset: () => void;

  // Event listeners
  _unlisten: UnlistenFn | null;
  setupEventListener: () => Promise<void>;
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
  llmStatus: "unknown" as const,
};

export const useStockAnalysisStore = create<StockAnalysisState>((set, get) => ({
  ...initialState,
  _unlisten: null,

  searchStock: async (keyword: string) => {
    set({ searchKeyword: keyword });
    if (keyword.length < 2) {
      set({ searchResults: [] });
      return;
    }
    try {
      const results = await invoke<StockSearchResult[]>("search_stock", { keyword });
      set({ searchResults: results });
    } catch {
      set({ searchResults: [] });
    }
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
    const record = await invoke<AnalysisSummary & { decisionJson: string | null }>(
      "get_stock_analysis",
      { analysisId },
    );
    set({ analysisId: record.id, stockCode: record.stockCode, stockName: record.stockName });
    if (record.decisionJson) {
      try {
        set({ decision: JSON.parse(record.decisionJson) });
      } catch (e) {
        console.error("[StockAnalysis] Failed to parse decision JSON:", e);
      }
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

    const unlisten = await listen<AnalysisEvent>("stock-analysis-event", (event) => {
      const { type, payload } = event.payload;
      switch (type) {
        case "Started":
          set({ status: "running" });
          break;
        case "DataLoaded":
          break;
        case "AnalystProgress":
          break;
        case "AnalystReport": {
          const { expertId, reportText } = payload as Record<string, string>;
          set((s) => ({
            analystReports: { ...s.analystReports, [expertId]: reportText },
          }));
          break;
        }
        case "DebateRound": {
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
          }));
          break;
        }
        case "RiskAssessment": {
          const { riskType, report } = payload as Record<string, string>;
          set((s) => ({
            riskAssessments: { ...s.riskAssessments, [riskType]: report },
          }));
          break;
        }
        case "InvestmentPlan": {
          const { plan } = payload as Record<string, string>;
          set((s) => ({
            analystReports: { ...s.analystReports, "investment-plan": plan },
          }));
          break;
        }
        // NOTE: payload 与 StockDecision 的结构由 serde(rename_all="camelCase") 保证一致
        // 若后端修改 Decision 变体字段，此处需同步更新
        case "Decision":
          set({ decision: payload as unknown as StockDecision, status: "completed" });
          break;
        case "Error": {
          const msg = (payload as Record<string, string>).message;
          set({
            error: msg,
            status: msg.includes("LLM") ? "running" : "error",
            llmStatus: msg.includes("LLM") ? "placeholder" : get().llmStatus,
          });
          break;
        }
      }
    });

    set({ _unlisten: unlisten });
  },
}));
