// SPDX-License-Identifier: AGPL-3.0-only

import { setEnabledPlugins } from "@/lib/domainPageRegistry";
import { refreshCardContributions } from "@/lib/dynamicUI/pluginCardRegistry";
import { translateBackendError } from "@/lib/errorI18n";
import { invoke, isTauri, listen } from "@/lib/invoke";
import { invalidatePluginCommandCache } from "@/lib/pluginCommandResolve";
import type {
  DynamicAction,
  InstallOutcomeDto,
  PluginSourceValidationDto,
  PluginSummaryDto,
  UpdateOutcomeDto,
} from "@/types";
import { create } from "zustand";

interface PluginState {
  plugins: PluginSummaryDto[];
  loading: boolean;
  error: string | null;
  installing: string | null;
  validating: boolean;

  loadPlugins: () => Promise<void>;
  validateSource: (
    source: string,
    expectedSha256?: string,
  ) => Promise<PluginSourceValidationDto | null>;
  installPlugin: (
    source: string,
    expectedSha256?: string,
  ) => Promise<InstallOutcomeDto | null>;
  enablePlugin: (pluginId: string) => Promise<boolean>;
  disablePlugin: (pluginId: string) => Promise<boolean>;
  uninstallPlugin: (pluginId: string) => Promise<boolean>;
  updatePlugin: (
    pluginId: string,
    expectedSha256?: string,
  ) => Promise<UpdateOutcomeDto | null>;
  runUiAction: (pluginId: string, action: DynamicAction) => Promise<unknown | null>;
}

/** 后端 `plugin-state-changed` 事件负载；事件只是失效信号，真相仍是 `plugin_list` 投影。 */
export interface PluginStateChangedPayload {
  pluginId: string;
  state: "enabled" | "disabled" | "uninstalled" | "installed" | "updated";
}

let pluginStateUnlisten: (() => void) | null = null;

/**
 * 初始化插件状态事件监听：后端在 enable/disable/uninstall/install/update 成功后广播，
 * 这里统一失效命令反查缓存、卡片注册表并重拉插件列表。启停入口（store action / 设置页）
 * 因此不需要各自记得刷新；`bindPluginRouteStore` 只订阅 store，随列表引用变化刷新路由。
 *
 * 幂等：多次调用不会重复注册（HMR 友好），形态对齐 `initGatewayStatusListener`。
 */
export function initPluginStateListener(): () => void {
  if (pluginStateUnlisten) { return () => {}; }
  const pending = listen<PluginStateChangedPayload>("plugin-state-changed", () => {
    invalidatePluginCommandCache();
    void refreshCardContributions();
    void usePluginStore.getState().loadPlugins();
  });
  // 占位避免并发初始化（listen 是 async）。
  pluginStateUnlisten = () => {};
  void pending.then((fn) => {
    pluginStateUnlisten = fn;
  });
  return () => {
    pluginStateUnlisten?.();
    pluginStateUnlisten = null;
  };
}

export const usePluginStore = create<PluginState>((set, get) => ({
  plugins: [],
  loading: false,
  error: null,
  installing: null,
  validating: false,

  loadPlugins: async () => {
    set({ loading: true, error: null });
    try {
      const plugins = await invoke<PluginSummaryDto[]>("plugin_list");
      set({ plugins, loading: false });
      // 域页的 requiresPlugin 门槛只认这份启用投影；失败时传 null 让门槛整体失效，
      // 不能把一次后端抖动放大成「所有依赖插件的页面消失」。
      // 浏览器模式没有插件安装根（browserMock 的 plugin_list 恒为 mock 列表），
      // 那里启用门槛会让本地开发/e2e 的股票页凭空消失 ⇒ 只在 Tauri 真机生效。
      setEnabledPlugins(
        isTauri() ? plugins.filter((p) => p.enabled).flatMap((p) => [p.name, p.id]) : null,
      );
    } catch (e) {
      const errorMsg = translateBackendError(e);
      set({ error: errorMsg, loading: false });
      setEnabledPlugins(null);
      if (import.meta.env.DEV) {
        console.error("Failed to load plugins:", e);
      }
    }
  },

  validateSource: async (source, expectedSha256) => {
    set({ validating: true, error: null });
    try {
      const manifest = await invoke<PluginSourceValidationDto>("plugin_validate_source", {
        source,
        expectedSha256: expectedSha256?.trim() || undefined,
      });
      set({ validating: false });
      return manifest;
    } catch (e) {
      const errorMsg = translateBackendError(e);
      set({ error: errorMsg, validating: false });
      if (import.meta.env.DEV) {
        console.error("Failed to validate plugin source:", e);
      }
      return null;
    }
  },

  installPlugin: async (source, expectedSha256) => {
    set({ installing: source, error: null });
    try {
      const outcome = await invoke<InstallOutcomeDto>("plugin_install", {
        source,
        expectedSha256: expectedSha256?.trim() || undefined,
      });
      set((s) => ({
        installing: null,
        plugins: [
          ...s.plugins,
          {
            id: outcome.pluginId,
            name: outcome.pluginId.split("@")[0] || outcome.pluginId,
            version: outcome.version,
            description: "",
            kind: source.startsWith("openclaw:") ? "openclaw" as const : "external" as const,
            enabled: true,
            tools: [],
            mcpServers: [],
            skills: [],
            source,
            permissions: [],
            pages: [],
            // 刚装上还没走一遍后端投影：不能冒充 verified。
            integrityStatus: "unknown",
            pinnedHash: null,
            commands: [],
            // 刚装上、还没走一遍后端投影 ⇒ 计数未知（null），别拿 0 冒充「一条都没丢」。
            eventStats: null,
          },
        ],
      }));
      await get().loadPlugins();
      return outcome;
    } catch (e) {
      const errorMsg = translateBackendError(e);
      set({ error: errorMsg, installing: null });
      if (import.meta.env.DEV) {
        console.error("Failed to install plugin:", e);
      }
      return null;
    }
  },

  enablePlugin: async (pluginId) => {
    try {
      await invoke("plugin_enable", { pluginId });
      set((s) => ({
        plugins: s.plugins.map((p) => p.id === pluginId ? { ...p, enabled: true } : p),
        error: null,
      }));
      // 本地补丁只覆盖 enabled；commands/pages/eventStats 等投影字段须重拉取回真相。
      // 事件到达时也会重拉，这里保留是 emit 缺失时的兜底（两次都是幂等读取）。
      await get().loadPlugins();
      return true;
    } catch (e) {
      const errorMsg = translateBackendError(e);
      set({ error: errorMsg });
      if (import.meta.env.DEV) {
        console.error("Failed to enable plugin:", e);
      }
      return false;
    }
  },

  disablePlugin: async (pluginId) => {
    try {
      await invoke("plugin_disable", { pluginId });
      set((s) => ({
        plugins: s.plugins.map((p) => p.id === pluginId ? { ...p, enabled: false } : p),
        error: null,
      }));
      await get().loadPlugins();
      return true;
    } catch (e) {
      const errorMsg = translateBackendError(e);
      set({ error: errorMsg });
      if (import.meta.env.DEV) {
        console.error("Failed to disable plugin:", e);
      }
      return false;
    }
  },

  uninstallPlugin: async (pluginId) => {
    try {
      await invoke("plugin_uninstall", { pluginId });
      set((s) => ({
        plugins: s.plugins.filter((p) => p.id !== pluginId),
        error: null,
      }));
      await get().loadPlugins();
      return true;
    } catch (e) {
      const errorMsg = translateBackendError(e);
      set({ error: errorMsg });
      if (import.meta.env.DEV) {
        console.error("Failed to uninstall plugin:", e);
      }
      return false;
    }
  },

  updatePlugin: async (pluginId, expectedSha256) => {
    try {
      const outcome = await invoke<UpdateOutcomeDto>("plugin_update", {
        pluginId,
        expectedSha256: expectedSha256?.trim() || undefined,
      });
      set((s) => ({
        plugins: s.plugins.map((p) => p.id === pluginId ? { ...p, version: outcome.newVersion } : p),
        error: null,
      }));
      return outcome;
    } catch (e) {
      const errorMsg = translateBackendError(e);
      set({ error: errorMsg });
      if (import.meta.env.DEV) {
        console.error("Failed to update plugin:", e);
      }
      return null;
    }
  },

  /**
   * 把动态 UI 里触发的 action 回流传给**插件自己的** worker 进程（`plugin_ui_action`）。
   *
   * 仅在 Schema 来自插件（`origin === "plugin"`，`ownerId` 即 pluginId）时调用。
   * 插件未声明 `worker` 或未启用时后端返回 `PLUGIN_UI_ACTION_UNAVAILABLE` —— 这是
   * 常态而非异常，故失败只记 `error` 并返回 `null`，不抛给调用方（否则一次点击
   * 就会把渲染树的 error boundary 打掉）。
   */
  runUiAction: async (pluginId, action) => {
    try {
      const result = await invoke<unknown>("plugin_ui_action", { pluginId, action });
      set({ error: null });
      return result;
    } catch (e) {
      set({ error: translateBackendError(e) });
      if (import.meta.env.DEV) {
        console.error("Failed to run plugin UI action:", e);
      }
      return null;
    }
  },
}));
