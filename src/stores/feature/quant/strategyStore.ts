// =====================================================================
// Quant: strategyStore
// =====================================================================
//
// 持有：
//  - 策略元数据列表（DB 已注册 + 内置 5 项默认）
//  - Rhai 编辑器草稿（与 store 解耦，组件级 useState 即可）
//  - 注册 / 刷新 actions
//
// 经 `invokePluginCommand` 调用插件 `stock_quant_strategies_list` /
// `stock_quant_strategy_register_rhai`（#336 迁移）。
// =====================================================================

import { create } from "zustand";

import { invokePluginCommand } from "@/lib/pluginCommand";
import type { RegisterRhaiRequest, StrategyMeta } from "@/types/quant";

interface StrategyState {
  strategies: StrategyMeta[];
  isLoading: boolean;
  isRegistering: boolean;
  error: string | null;
  lastFetchedAt: number | null;

  loadStrategies: (force?: boolean) => Promise<void>;
  registerRhai: (request: RegisterRhaiRequest) => Promise<StrategyMeta>;
  getStrategyById: (id: string) => StrategyMeta | undefined;
  reset: () => void;
}

const INITIAL: Omit<StrategyState, "loadStrategies" | "registerRhai" | "getStrategyById" | "reset"> = {
  strategies: [],
  isLoading: false,
  isRegistering: false,
  error: null,
  lastFetchedAt: null,
};

export const useStrategyStore = create<StrategyState>((set, get) => ({
  ...INITIAL,

  loadStrategies: async (force = false) => {
    const state = get();
    if (state.isLoading) { return; }
    if (!force && state.strategies.length > 0) { return; }
    set({ isLoading: true, error: null });
    try {
      const result = await invokePluginCommand<StrategyMeta[]>("stock_quant_strategies_list");
      set({
        strategies: result,
        isLoading: false,
        lastFetchedAt: Date.now(),
      });
    } catch (e) {
      set({
        isLoading: false,
        error: e instanceof Error ? e.message : String(e),
      });
    }
  },

  registerRhai: async (request: RegisterRhaiRequest) => {
    set({ isRegistering: true, error: null });
    try {
      // #336：插件命令入参平铺（无 request 包裹），与清单 inputSchema 对齐。
      const newStrategy = await invokePluginCommand<StrategyMeta>(
        "stock_quant_strategy_register_rhai",
        { ...request },
      );
      // 追加 / 替换到列表
      const list = get().strategies;
      const idx = list.findIndex((s) => s.id === newStrategy.id);
      if (idx >= 0) {
        const next = [...list];
        next[idx] = newStrategy;
        set({ strategies: next, isRegistering: false });
      } else {
        set({
          strategies: [newStrategy, ...list],
          isRegistering: false,
        });
      }
      return newStrategy;
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      set({ isRegistering: false, error: msg });
      throw new Error(msg, { cause: e });
    }
  },

  getStrategyById: (id: string) => get().strategies.find((s) => s.id === id),

  reset: () => set(INITIAL),
}));
