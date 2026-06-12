// SPDX-License-Identifier: AGPL-3.0-only

import { invoke, logIpcError } from "@/lib/invoke";
import type { SystemPermissionMode } from "@/types";
import { create } from "zustand";

export interface FeatureFlags {
  forkSubagent: boolean;
  coordinatorMode: boolean;
  proactiveMode: boolean;
  swarmMode: boolean;
  toolConcurrency: boolean;
  verificationAgent: boolean;
  dreamTask: boolean;
}

/** 时间旅行模式配置 — spec §10
 *  控制 Replay/Backtest 模式是否可用、as-of 范围、LLM prompt 严格度等。 */
export interface TimeTravelConfig {
  /** 总开关：false 时 AppHeader ModeSwitch 与 StockAnalysisPage 全部隐藏,降级为纯 live */
  enabled: boolean;
  /** as-of 日期距今天最大天数；超过的日期被 DatePicker disabled 拒绝 */
  maxAsOfAgeDays: number;
  /** 允许选未来日期(spec 默认 false，强制闭世界；调试 / 沙盒环境可开) */
  allowFuture: boolean;
  /** Backtest 默认持仓天数,影响后续 PnL 模拟 */
  defaultHoldingDays: number;
  /** 严格 prompt 模式: as-of 注入 LLM system prompt,违规将被打回(本轮 T6 已实现) */
  promptStrictMode: boolean;
  /** 严格 LLM judge: 用 LLM 二次扫描节点输出找未来引用(本轮 T9 已实现) */
  promptStrictLLMJudge: boolean;
  /** Tour 提示是否被用户关闭(用于首次进入回放模式的引导) */
  tourDismissed: boolean;
}

const DEFAULT_TIME_TRAVEL_CONFIG: TimeTravelConfig = {
  enabled: true,
  maxAsOfAgeDays: 365,
  allowFuture: false,
  defaultHoldingDays: 5,
  promptStrictMode: true,
  promptStrictLLMJudge: false,
  tourDismissed: false,
};

export type ModelTier = "opus" | "sonnet" | "haiku";
export type PermissionMode = SystemPermissionMode;

const DEFAULT_FEATURE_FLAGS: FeatureFlags = {
  forkSubagent: false,
  coordinatorMode: false,
  proactiveMode: true,
  swarmMode: false,
  toolConcurrency: true,
  verificationAgent: false,
  dreamTask: true,
};

interface AppConfigState {
  model: ModelTier;
  permissionMode: PermissionMode;
  maxIterations: number;
  features: FeatureFlags;
  timeTravel: TimeTravelConfig;
  loading: boolean;
  error: string | null;

  setModel: (model: ModelTier) => void;
  setPermissionMode: (mode: PermissionMode) => void;
  setMaxIterations: (n: number) => void;
  toggleFeature: (name: keyof FeatureFlags) => void;
  /** 局部更新 timeTravel 字段；key/value 单项写入以避免竞态 */
  updateTimeTravel: <K extends keyof TimeTravelConfig>(key: K, value: TimeTravelConfig[K]) => void;
  loadConfig: () => Promise<void>;
  saveConfig: () => Promise<void>;
}

export const useAppConfigStore = create<AppConfigState>((set, get) => ({
  model: "sonnet",
  permissionMode: "workspace-write",
  maxIterations: 50,
  features: { ...DEFAULT_FEATURE_FLAGS },
  timeTravel: { ...DEFAULT_TIME_TRAVEL_CONFIG },
  loading: false,
  error: null,

  setModel: (model) => {
    set({ model });
  },

  setPermissionMode: (mode) => {
    set({ permissionMode: mode });
  },

  setMaxIterations: (n) => {
    set({ maxIterations: Math.max(1, Math.min(100, Math.round(n))) });
  },

  toggleFeature: (name) =>
    set((state) => {
      const newValue = !state.features[name];
      if (name === "proactiveMode") {
        invoke("proactive_set_enabled", { enabled: newValue }).catch(logIpcError("proactive_set_enabled"));
      }
      return {
        features: { ...state.features, [name]: newValue },
      };
    }),

  updateTimeTravel: (key, value) =>
    set((state) => ({
      timeTravel: { ...state.timeTravel, [key]: value },
    })),

  loadConfig: async () => {
    set({ loading: true, error: null });
    try {
      const data = await invoke<Partial<AppConfigState>>("get_app_config");
      if (data) {
        set((state) => ({
          model: (data.model as ModelTier) ?? state.model,
          permissionMode: (data.permissionMode as PermissionMode) ?? state.permissionMode,
          maxIterations: data.maxIterations ?? state.maxIterations,
          features: data.features
            ? { ...DEFAULT_FEATURE_FLAGS, ...data.features }
            : state.features,
          timeTravel: data.timeTravel
            ? { ...DEFAULT_TIME_TRAVEL_CONFIG, ...data.timeTravel }
            : state.timeTravel,
          loading: false,
        }));
      } else {
        set({ loading: false });
      }
    } catch (e) {
      logIpcError("appConfigStore: 加载配置失败")(e);
      set({ loading: false, error: String(e) });
    }
  },

  saveConfig: async () => {
    const state = get();
    try {
      await invoke("save_app_config", {
        config: {
          model: state.model,
          permissionMode: state.permissionMode,
          maxIterations: state.maxIterations,
          features: state.features,
          timeTravel: state.timeTravel,
        },
      });
    } catch (e) {
      logIpcError("appConfigStore: 保存配置失败")(e);
      set({ error: String(e) });
    }
  },
}));
