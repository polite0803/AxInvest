// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 插件域页导航注册表。
 *
 * 与 `domainPageRegistry` 的分工：那张表登记主壳内编译期域页；本表登记**运行期插件**
 * 贡献的 iframe 页面，数据源只能是 `plugin_list_pages` + `plugin_list_domain_packs`。
 * 每次刷新都整体替换快照，插件被禁用/卸载后对应路由与侧栏入口自然消失。
 */

import { invoke } from "@/lib/invoke";
import { type PluginDomainPageRoute, resolvePluginDomainPageRoutes } from "@/lib/pluginDomainPageRoutes";
import { usePluginStore } from "@/stores";
import type { PluginDomainPacksInfoDto, PluginPagesInfoDto } from "@/types";
import type { ComponentType, LazyExoticComponent } from "react";
import { lazy, useSyncExternalStore } from "react";

export interface PluginPageRouteEntry extends PluginDomainPageRoute {
  readonly Page: LazyExoticComponent<ComponentType>;
}

let snapshot: readonly PluginPageRouteEntry[] = [];
let loadPromise: Promise<readonly PluginPageRouteEntry[]> | null = null;
const listeners = new Set<() => void>();

function publish(next: readonly PluginPageRouteEntry[]): void {
  snapshot = next;
  for (const listener of listeners) { listener(); }
}

/**
 * 页面组件延迟加载到 `PluginPageHost`，路由只需拿到稳定的 lazy 引用。
 *
 * 同一 plugin/page 复用同一个 lazy 组件，避免每次刷新生成新的 Suspense 类型导致卸载重建。
 */
const pageComponents = new Map<string, LazyExoticComponent<ComponentType>>();
function pageComponent(key: string): LazyExoticComponent<ComponentType> {
  const cached = pageComponents.get(key);
  if (cached) { return cached; }
  const component = lazy(async () => {
    const mod = await import("@/components/plugin/PluginPageRouteHost");
    return { default: mod.PluginPageRouteHost };
  });
  pageComponents.set(key, component);
  return component;
}

/** 重新读取后端两个实时投影；并发调用共享同一个 in-flight Promise。 */
export async function refreshPluginPageRoutes(): Promise<readonly PluginPageRouteEntry[]> {
  if (loadPromise) { return loadPromise; }
  loadPromise = (async () => {
    try {
      const [pages, packs] = await Promise.all([
        invoke<PluginPagesInfoDto[]>("plugin_list_pages"),
        invoke<PluginDomainPacksInfoDto[]>("plugin_list_domain_packs"),
      ]);
      const resolved = resolvePluginDomainPageRoutes(
        Array.isArray(pages) ? pages : [],
        Array.isArray(packs) ? packs : [],
      ).map((route) => {
        const entry = { ...route, Page: pageComponent(route.id) };
        return entry;
      });
      publish(resolved);
      return snapshot;
    } catch (error) {
      // 投影失败时不能保留上一次的成功快照：那会让已禁用页面继续可达。
      publish([]);
      throw error;
    }
  })().finally(() => {
    loadPromise = null;
  });
  return loadPromise;
}

export function subscribePluginPageRoutes(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** 允许布局在首次渲染前同步读取的是空快照；真实投影由 effect 刷新。 */
export function pluginPageNavLabel(entry: PluginPageRouteEntry): string {
  return entry.navLabel;
}

/** 插件页 → 宿主 `NavItem` 的纯映射；`labelKey` 在插件项里承载纯文本标题。 */
export function pluginPageNavItem(entry: PluginPageRouteEntry): {
  key: string;
  labelKey: string;
  path: string;
  isPlugin: true;
  pluginName: string;
} {
  return {
    key: entry.navKey,
    labelKey: pluginPageNavLabel(entry),
    path: entry.path,
    isPlugin: true,
    pluginName: entry.pluginName,
  };
}

export function getPluginPageRoutes(): readonly PluginPageRouteEntry[] {
  return snapshot;
}

export function usePluginPageRoutes(): readonly PluginPageRouteEntry[] {
  return useSyncExternalStore(subscribePluginPageRoutes, getPluginPageRoutes, getPluginPageRoutes);
}

/** 从插件 store 的启停/卸载动作触发刷新；调用方无需知道后端投影命令。 */
export function bindPluginRouteStore(): () => void {
  let previous = usePluginStore.getState().plugins;
  return usePluginStore.subscribe((state) => {
    if (state.plugins === previous) { return; }
    previous = state.plugins;
    void refreshPluginPageRoutes();
  });
}
