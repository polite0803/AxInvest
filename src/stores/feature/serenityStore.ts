import { create } from "zustand";

// ── 类型 ──

export type StepStage = "loading" | "scanning" | "decomposing" | "identifying" | "mapping" | "saving" | "done" | "error";

export interface SerenityCandidate {
  stockCode?: string;
  stock_name?: string;
  stockName?: string;
  stock_code?: string;
  relevance?: string;
  serenityScore?: number;
  serenity_score?: number;
  confidence?: number;
  bottleneckProduct?: string;
  bottleneck_product?: string;
  primaryRisk?: string;
  primary_risk?: string;
}

export interface TrendInfo {
  trendName?: string;
  trend_name?: string;
  bottleneck_candidate?: string;
  confidence?: number;
}

// ── Store ──

interface SerenityState {
  running: boolean;
  stage: StepStage;
  candidates: SerenityCandidate[];
  trends: TrendInfo[];
  error: string | null;

  setRunning: (v: boolean) => void;
  setStage: (s: StepStage) => void;
  setCandidates: (c: SerenityCandidate[]) => void;
  setTrends: (t: TrendInfo[]) => void;
  setError: (e: string | null) => void;
  reset: () => void;
}

const initialState = {
  running: false,
  stage: "done" as StepStage,
  candidates: [] as SerenityCandidate[],
  trends: [] as TrendInfo[],
  error: null as string | null,
};

export const useSerenityStore = create<SerenityState>((set) => ({
  ...initialState,
  setRunning: (v) => set({ running: v }),
  setStage: (s) => set({ stage: s }),
  setCandidates: (c) => set({ candidates: c }),
  setTrends: (t) => set({ trends: t }),
  setError: (e) => set({ error: e }),
  reset: () => set(initialState),
}));
