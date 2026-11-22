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

/** 旧 `/opc/domain/:pageId` → 插件页 `{pluginSlug, pageId}` 的兼容映射。 */
export interface PluginDomainRouteMapping {
  readonly legacyPath: string;
  readonly pluginSlug: string;
  readonly pageId: string;
  /** 页面上下文键，与侧栏/标题/路由三方同源。 */
  readonly navKey: string;
}

let snapshot: readonly PluginPageRouteEntry[] = [];
let loadPromise: Promise<readonly PluginPageRouteEntry[]> | null = null;
// 并发启停时，旧 invoke 的回包可能晚于新状态落地。generation 标记最新请求，
// completedGeneration 标记已发布快照的轮次；过期轮次一律丢弃，不 publish。
let generation = 0;
let completedGeneration = 0;
const listeners = new Set<() => void>();

// 首帧 gate 判据：空快照既可能是「插件还没投影完」，也可能是「确实没有插件页」。
// ready 把两者分开；lastError 让失败态显示原因而不是无限转圈。
let ready = false;
let lastError: unknown = null;
const readyListeners = new Set<() => void>();

function subscribeReady(listener: () => void): () => void {
  readyListeners.add(listener);
  return () => {
    readyListeners.delete(listener);
  };
}
function markReady(error: unknown = null): void {
  ready = true;
  lastError = error;
  for (const listener of readyListeners) { listener(); }
}
export function getPluginPageRoutesReady(): boolean {
  return ready;
}
export function getPluginPageRoutesError(): unknown {
  return lastError;
}
export function usePluginPageRoutesReady(): boolean {
  return useSyncExternalStore(subscribeReady, getPluginPageRoutesReady, getPluginPageRoutesReady);
}

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

async function fetchPluginPageRoutes(): Promise<readonly PluginPageRouteEntry[]> {
  const [pages, packs] = await Promise.all([
    invoke<PluginPagesInfoDto[]>("plugin_list_pages"),
    // 域包只供旧 `/opc/domain/*` 映射使用，不是插件页的前置（见 pluginDomainPageRoutes）。
    // 它失败时仍要把 pages 投影出来，否则任何 OPC 域包查询异常都会让全部插件页入口消失。
    invoke<PluginDomainPacksInfoDto[]>("plugin_list_domain_packs").catch(() => [] as PluginDomainPacksInfoDto[]),
  ]);
  return resolvePluginDomainPageRoutes(
    Array.isArray(pages) ? pages : [],
    Array.isArray(packs) ? packs : [],
  ).map((route) => ({ ...route, Page: pageComponent(route.id) }));
}

/**
 * 重新读取后端两个实时投影。
 *
 * 并发调用共享同一个 worker；worker 内循环消费 latest generation：
 * 启停引发的连续刷新只让最后一轮 publish，早到/慢到的回包都不会覆盖新状态。
 */
export async function refreshPluginPageRoutes(): Promise<readonly PluginPageRouteEntry[]> {
  ++generation;
  if (loadPromise) { return loadPromise; }
  loadPromise = (async () => {
    while (completedGeneration < generation) {
      const round = generation;
      try {
        const resolved = await fetchPluginPageRoutes();
        completedGeneration = round;
        // 只有最新一轮允许发布；过期回包连快照都不能碰，否则会短暂显示已禁用页面。
        if (round === generation) {
          publish(resolved);
          markReady();
        }
      } catch (error) {
        completedGeneration = round;
        // 过期轮次的失败与成功一样丢弃；只有最新一轮的错误才交给调用方。
        if (round === generation) {
          // 投影失败时不能保留上一次的成功快照：那会让已禁用页面继续可达。
          publish([]);
          markReady(error);
          throw error;
        }
      }
    }
    return snapshot;
  })().finally(() => {
    loadPromise = null;
    // publish() 的订阅者可能在 worker 结束前再次请求刷新，补起漏掉的轮次。
    if (completedGeneration < generation) { void refreshPluginPageRoutes(); }
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

/** 运行期插件投影中的旧 OPC 域路径兼容映射；插件不可用或未声明时为空。 */
export function getPluginDomainRouteMappings(): readonly PluginDomainRouteMapping[] {
  return snapshot
    .filter((entry): entry is PluginPageRouteEntry & { legacyPath: string } => typeof entry.legacyPath === "string")
    .map((entry) => ({
      legacyPath: entry.legacyPath,
      pluginSlug: entry.pluginSlug,
      pageId: entry.pageId,
      navKey: entry.navKey,
    }));
}

export function usePluginDomainRouteMappings(): readonly PluginDomainRouteMapping[] {
  return useSyncExternalStore(
    subscribePluginPageRoutes,
    getPluginDomainRouteMappings,
    getPluginDomainRouteMappings,
  );
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
