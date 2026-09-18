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
  selfImprovingLoop: boolean;
  finalOutputReflection: boolean;
}

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
  selfImprovingLoop: false,
  finalOutputReflection: false,
};

/** 高级引擎参数中的数值项（对应后端 `ReactEngineOverrides` 的数值字段）。 */
export type ReactEngineNumberKey =
  | "timeoutSecs"
  | "maxRetryAttempts"
  | "maxRepeatedCalls"
  | "maxNoProgressIterations"
  | "minQualityThreshold";

/** 高级引擎参数中的开关项。 */
export type ReactEngineFlagKey =
  | "cycleDetectionEnabled"
  | "verificationEnabled"
  | "reflectionEnabled"
  | "checkpointEnabled";

/** 「高级引擎参数」——只含后端 `ReactEngineOverrides` **实际暴露**的 9 个字段。 */
export interface ReactEngineConfig extends Record<ReactEngineNumberKey, number>, Record<ReactEngineFlagKey, boolean> {}

/**
 * 各数值字段的量程，与 `SettingsPanel` 控件的 `min`/`max` 及后端
 * `extract_bounded_u64` 的区间三方一致。
 *
 * 这里是「改坏」的第三道防线（第一道控件、第二道后端）。超时填 0 会让引擎
 * 立即超时，重试填 10 万等于死循环 —— store 内 clamp 拦的是「绕过控件的写入」
 * （如 Programmatic 设置、未来新增的调用方）。
 */
const REACT_ENGINE_BOUNDS: Record<ReactEngineNumberKey, [number, number]> = {
  timeoutSecs: [10, 3600],
  maxRetryAttempts: [0, 10],
  maxRepeatedCalls: [1, 100],
  maxNoProgressIterations: [1, 100],
  minQualityThreshold: [0, 10],
};

/** 与后端 `ReActConfig::default()` / `ReactEngineOverrides` 未配置时的取值对齐。 */
const DEFAULT_REACT_ENGINE: ReactEngineConfig = {
  timeoutSecs: 300,
  maxRetryAttempts: 3,
  maxRepeatedCalls: 3,
  maxNoProgressIterations: 5,
  minQualityThreshold: 5,
  cycleDetectionEnabled: true,
  verificationEnabled: true,
  reflectionEnabled: true,
  checkpointEnabled: false,
};

interface AppConfigState {
  model: ModelTier;
  permissionMode: PermissionMode;
  maxIterations: number;
  features: FeatureFlags;
  reactEngine: ReactEngineConfig;
  loading: boolean;
  error: string | null;

  setModel: (model: ModelTier) => void;
  setPermissionMode: (mode: PermissionMode) => void;
  setMaxIterations: (n: number) => void;
  toggleFeature: (name: keyof FeatureFlags) => void;
  setReactEngineNumber: (key: ReactEngineNumberKey, value: number) => void;
  setReactEngineFlag: (key: ReactEngineFlagKey, value: boolean) => void;
  loadConfig: () => Promise<void>;
  saveConfig: () => Promise<void>;
}

export const useAppConfigStore = create<AppConfigState>((set, get) => ({
  model: "sonnet",
  permissionMode: "workspace-write",
  maxIterations: 50,
  features: { ...DEFAULT_FEATURE_FLAGS },
  reactEngine: { ...DEFAULT_REACT_ENGINE },
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

  setReactEngineNumber: (key, value) => {
    const [min, max] = REACT_ENGINE_BOUNDS[key];
    set((s) => ({
      reactEngine: { ...s.reactEngine, [key]: Math.max(min, Math.min(max, Math.round(value))) },
    }));
  },

  setReactEngineFlag: (key, value) => {
    set((s) => ({ reactEngine: { ...s.reactEngine, [key]: value } }));
  },

  toggleFeature: (name) => {
    set((state) => {
      const newValue = !state.features[name];
      if (name === "proactiveMode") {
        invoke("proactive_set_enabled", { enabled: newValue }).catch(logIpcError("proactive_set_enabled"));
      }
      return {
        features: { ...state.features, [name]: newValue },
      };
    });
    // 缺陷1修复:selfImprovingLoop / finalOutputReflection 切换后:
    // 1. saveConfig 持久化到 DB(供下次启动时 wiring 层读取)
    // 2. set_self_improvement_flags 即时更新后端 SessionManager(无需重启)
    if (name === "selfImprovingLoop" || name === "finalOutputReflection") {
      const { selfImprovingLoop, finalOutputReflection } = get().features;
      void get().saveConfig();
      invoke("set_self_improvement_flags", {
        selfImprovementEnabled: selfImprovingLoop,
        finalOutputReflection: finalOutputReflection,
      }).catch(logIpcError("set_self_improvement_flags"));
    }
  },

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
          reactEngine: data.reactEngine
            ? { ...DEFAULT_REACT_ENGINE, ...data.reactEngine }
            : state.reactEngine,
          loading: false,
        }));
      } else {
        set({ loading: false });
      }
    } catch (e) {
      logIpcError("appConfigStore: loadConfig failed")(e);
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
          reactEngine: state.reactEngine,
        },
      });
    } catch (e) {
      logIpcError("appConfigStore: saveConfig failed")(e);
      set({ error: String(e) });
    }
  },
}));
