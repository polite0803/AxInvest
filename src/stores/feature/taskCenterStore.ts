// SPDX-License-Identifier: AGPL-3.0-only

import { aggregateRuns } from "@/pages/TasksCenterPage/adapters/registry";
import type { RunState, RunTrigger, SourceFetchError, SourceId, UnifiedRun } from "@/pages/TasksCenterPage/model";
import { create } from "zustand";

export type RunTimeRange = "24h" | "7d" | "all";

export interface RunFilters {
  /** 空数组 = 全部源 */
  sources: SourceId[];
  /** 空数组 = 全部状态 */
  states: RunState[];
  /** 空数组 = 全部触发方式 */
  triggers: RunTrigger[];
  search: string;
  timeRange: RunTimeRange;
}

const DEFAULT_FILTERS: RunFilters = {
  sources: [],
  states: [],
  triggers: [],
  search: "",
  timeRange: "all",
};

const RANGE_MS: Record<Exclude<RunTimeRange, "all">, number> = {
  "24h": 24 * 60 * 60 * 1000,
  "7d": 7 * 24 * 60 * 60 * 1000,
};

interface TaskCenterState {
  runs: UnifiedRun[];
  sourceErrors: SourceFetchError[];
  loading: boolean;
  refreshing: boolean;
  loadedAt: number | null;
  filters: RunFilters;
  /** 当前打开详情抽屉的 run（列表刷新后按 id 同步最新值） */
  selectedRun: UnifiedRun | null;

  loadRuns: (silent?: boolean) => Promise<void>;
  setFilters: (patch: Partial<RunFilters>) => void;
  resetFilters: () => void;
  selectRun: (run: UnifiedRun | null) => void;
  /** 事件回调：增量插入或更新一条 run。 */
  upsertRun: (run: UnifiedRun) => void;
}

function buildQuery(filters: RunFilters) {
  return {
    sources: filters.sources.length > 0 ? filters.sources : undefined,
    states: filters.states.length > 0 ? filters.states : undefined,
    triggers: filters.triggers.length > 0 ? filters.triggers : undefined,
    search: filters.search.trim() || undefined,
    sinceMs: filters.timeRange === "all" ? undefined : Date.now() - RANGE_MS[filters.timeRange],
  };
}

export const useTaskCenterStore = create<TaskCenterState>((set, get) => ({
  runs: [],
  sourceErrors: [],
  loading: false,
  refreshing: false,
  loadedAt: null,
  filters: DEFAULT_FILTERS,
  selectedRun: null,

  loadRuns: async (silent = false) => {
    if (silent) {
      set({ refreshing: true });
    } else {
      set({ loading: true });
    }
    try {
      const { runs, errors } = await aggregateRuns(buildQuery(get().filters));
      const selected = get().selectedRun;
      const nextSelected = selected === null
        ? null
        : runs.find((r) => r.id === selected.id) ?? selected;
      set({
        runs,
        sourceErrors: errors,
        loadedAt: Date.now(),
        selectedRun: nextSelected,
      });
    } finally {
      set({ loading: false, refreshing: false });
    }
  },

  setFilters: (patch) => set((s) => ({ filters: { ...s.filters, ...patch } })),

  resetFilters: () => set({ filters: DEFAULT_FILTERS }),

  selectRun: (run) => set({ selectedRun: run }),

  upsertRun: (run) =>
    set((s) => {
      const idx = s.runs.findIndex((r) => r.id === run.id);
      const runs = idx === -1
        ? [run, ...s.runs]
        : s.runs.map((r) => (r.id === run.id ? run : r));
      return {
        runs,
        selectedRun: s.selectedRun?.id === run.id ? run : s.selectedRun,
      };
    }),
}));
