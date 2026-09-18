// SPDX-License-Identifier: AGPL-3.0-only

import { create } from "zustand";

import {
  getLearningConfig,
  getRLStats,
  listLearningConfigs,
  recordRLExperience,
  triggerAutoLearning,
  triggerRLOptimization,
} from "@/lib/opcLearning";
import type {
  AutoLearningResult,
  DomainLearningConfig,
  DomainLearningConfigSummary,
  ExperiencePoolStats,
  RLPolicyUpdate,
} from "@/types";

interface DomainLearningState {
  /** 所有行业学习配置缓存 */
  configs: Map<string, DomainLearningConfig>;
  /** 配置列表摘要 */
  summaries: DomainLearningConfigSummary[];
  /** 加载状态 */
  loading: boolean;
  /** 错误信息 */
  error: string | null;
  /** 最后加载时间 */
  lastLoadedAt: number | null;

  /** RL 经验池统计（按行业 ID 缓存） */
  rlStats: Map<string, ExperiencePoolStats>;
  /** 全局 RL 统计（跨行业汇总） */
  rlGlobalStats: ExperiencePoolStats | null;
  /** RL 策略优化结果（按行业 ID 缓存） */
  rlPolicyUpdates: Map<string, RLPolicyUpdate>;
  /** 自动学习闭环触发历史 */
  autoLearningHistory: AutoLearningResult[];
  /** RL 操作加载状态 */
  rlLoading: boolean;

  /** 加载指定行业的学习配置 */
  loadConfig: (domainPackId: string) => Promise<DomainLearningConfig | null>;
  /** 加载所有行业学习配置列表 */
  loadAllConfigs: () => Promise<void>;
  /** 从缓存获取配置 */
  getConfig: (domainPackId: string) => DomainLearningConfig | undefined;
  /** 清除缓存 */
  clearCache: () => void;

  /** 获取指定行业的 RL 经验池统计 */
  loadRLStats: (domainPackId?: string) => Promise<ExperiencePoolStats | null>;
  /** 记录 RL 经验 */
  recordExperience: (params: {
    domainPackId: string;
    workflowId: string;
    qualityScore: number;
    workflowResult: Record<string, unknown>;
  }) => Promise<boolean>;
  /** 触发 RL 策略优化 */
  triggerOptimization: (domainPackId: string) => Promise<RLPolicyUpdate | null>;
  /** 触发自动学习闭环 */
  triggerAutoLearning: (params: {
    domainPackId: string;
    workflowId: string;
    workflowResult: Record<string, unknown>;
  }) => Promise<AutoLearningResult | null>;
  /** 获取最近的自动学习结果 */
  getLatestAutoLearning: () => AutoLearningResult | undefined;
}

const CACHE_TTL_MS = 5 * 60 * 1000;

export const useDomainLearningStore = create<DomainLearningState>(
  (set, get) => ({
    configs: new Map(),
    summaries: [],
    loading: false,
    error: null,
    lastLoadedAt: null,

    rlStats: new Map(),
    rlGlobalStats: null,
    rlPolicyUpdates: new Map(),
    autoLearningHistory: [],
    rlLoading: false,

    loadConfig: async (domainPackId: string) => {
      const state = get();
      const cached = state.configs.get(domainPackId);
      if (cached && state.lastLoadedAt && Date.now() - state.lastLoadedAt < CACHE_TTL_MS) {
        return cached;
      }

      set({ loading: true, error: null });
      try {
        const config = await getLearningConfig(domainPackId);
        const newConfigs = new Map(state.configs);
        newConfigs.set(domainPackId, config);
        set({ configs: newConfigs, loading: false, lastLoadedAt: Date.now() });
        return config;
      } catch (e) {
        set({ error: String(e), loading: false });
        return null;
      }
    },

    loadAllConfigs: async () => {
      set({ loading: true, error: null });
      try {
        const summaries = await listLearningConfigs();
        set({ summaries, loading: false, lastLoadedAt: Date.now() });
      } catch (e) {
        set({ error: String(e), loading: false });
      }
    },

    getConfig: (domainPackId: string) => {
      return get().configs.get(domainPackId);
    },

    clearCache: () => {
      set({
        configs: new Map(),
        summaries: [],
        lastLoadedAt: null,
        rlStats: new Map(),
        rlGlobalStats: null,
        rlPolicyUpdates: new Map(),
      });
    },

    loadRLStats: async (domainPackId?: string) => {
      set({ rlLoading: true });
      try {
        const stats = await getRLStats(domainPackId);
        if (domainPackId) {
          const newStats = new Map(get().rlStats);
          newStats.set(domainPackId, stats);
          set({ rlStats: newStats, rlLoading: false });
        } else {
          set({ rlGlobalStats: stats, rlLoading: false });
        }
        return stats;
      } catch (e) {
        set({ error: String(e), rlLoading: false });
        return null;
      }
    },

    recordExperience: async (params: {
      domainPackId: string;
      workflowId: string;
      qualityScore: number;
      workflowResult: Record<string, unknown>;
    }) => {
      set({ rlLoading: true });
      try {
        const result = await recordRLExperience(params);
        set({ rlLoading: false });
        return result.success;
      } catch (e) {
        set({ error: String(e), rlLoading: false });
        return false;
      }
    },

    triggerOptimization: async (domainPackId: string) => {
      set({ rlLoading: true });
      try {
        const update = await triggerRLOptimization({ domainPackId });
        const newUpdates = new Map(get().rlPolicyUpdates);
        newUpdates.set(domainPackId, update);
        set({ rlPolicyUpdates: newUpdates, rlLoading: false });
        return update;
      } catch (e) {
        set({ error: String(e), rlLoading: false });
        return null;
      }
    },

    triggerAutoLearning: async (params) => {
      set({ rlLoading: true });
      try {
        const result = await triggerAutoLearning(params);
        const history = [result, ...get().autoLearningHistory].slice(0, 50);
        set({ autoLearningHistory: history, rlLoading: false });
        return result;
      } catch (e) {
        set({ error: String(e), rlLoading: false });
        return null;
      }
    },

    getLatestAutoLearning: () => {
      return get().autoLearningHistory[0];
    },
  }),
);
