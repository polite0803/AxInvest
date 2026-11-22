// SPDX-License-Identifier: AGPL-3.0-only

import type { CapabilityDomain } from "@/types/capability";
import { type ComponentType, type LazyExoticComponent, type ReactNode, useSyncExternalStore } from "react";

/**
 * 域页面路由 + 导航注册表 —— 宿主内容区/侧栏不再知道任何域页面。
 *
 * 为什么要有这张表（#36 迁出期）：宿主布局此前从 `@/pages/opc/routes` 直接点名字 import
 * `LazyOpcPage` / `OPC_DOMAIN_ROUTES`，等于「主壳知道 OPC 有哪些页面、各是什么组件」——
 * 域内每加一页都要改宿主文件，且宿主永远摘不掉对域目录的依赖。
 * 改为「域模块只导出一张路由表，本表装载后登记，宿主只渲染注册表投影」后，
 * 域侧增删页面宿主零改动，域整棵迁出仓库时本表的 glob 命中变空即自然退役（不留兼容分支）。
 *
 * 2026-11-10（#36 导航声明权半）：**path / pageKey / 侧栏入口** 也一并归域声明。
 * 此前域模块仍从宿主 `BUILTIN_PAGE_PATH` 取 path，等于「宿主给域页面命名」；
 * 现在 path 是域模块里的字面量，宿主 `pageRegistry` 只留宿主自己的页面。
 * 宿主侧三个消费面（`pageRegistry` 反查、`navRegistry`、`navGroups` 分组）全部改成
 * 「读本表投影」，域目录整棵删除后宿主无需再改一行（这是本批的验收判据）。
 *
 * 为什么用 glob 而不是宿主侧列一份域清单：列清单等于把「有哪些域」这件事放回宿主掌握，
 * 与上面那条迁出判据正相反；glob 是本机制唯一不需要宿主点名域名的装载口。
 *
 * ⚠ 装载形态是硬约束，不许改回「域模块 import 本表并调用 contributeDomainPages」：
 * `import.meta.glob({ eager: true })` 编译成**静态 import**并被提升到本文件求值之前，
 * 于是「注册表 ← 域模块 ← 注册表」成环，域模块执行时本文件的 `const byId` 还在 TDZ，
 * 应用在启动阶段直接 `ReferenceError: Cannot access 'byId' before initialization`
 * （`src/lib/__tests__/domainPageRegistry.test.ts` 的 glob 钉点即为此环而设）。
 * 域模块因此只 **import type**（编译期擦除，不成环），路由表由本文件装载后逐模块登记。
 *
 * 为什么不复用插件 `pages[]` + iframe 通道：那条通道给不了宿主上下文
 * （无 router / store / i18n，沙箱桥只放行两条命令），而 OPC 页面四者都要用。
 */

/** 一条由域贡献的宿主内路由。 */
export interface ContributedPageRoute {
  /** 全局唯一 id：既是 React key，也是覆盖判据（同 id 重复注册 = 替换，不追加） */
  readonly id: string;
  /** URL 路径，可含 `:param` 段。**由域自己声明**（宿主不再有这张表） */
  readonly path: string;
  /** `PageContextProvider` 的 page key —— agent 面板按它取上下文 */
  readonly pageKey: string;
  readonly Page: LazyExoticComponent<ComponentType>;
  /**
   * 侧栏 / 域聚合页入口。挂在哪条路由上，入口就指向哪条 path。
   * 同一页面的多段路由（如 `/opc` 与 `/opc/:tab`）只在**主路由**上挂一次。
   */
  readonly nav?: ContributedNavItem;
  /**
   * 页面所依赖的插件 `name` 或 `id`。未安装/未启用时路由与侧栏入口一起消失。
   *
   * 这是「域整棵迁进插件」的过渡接缝：页面组件仍在宿主编译，但**可见性**已经
   * 由插件状态决定，避免插件未加载时股票页仍然出现（#36 / P6.1 的对外契约）。
   */
  readonly requiresPlugin?: string;
}

/** 一条域贡献的导航项（侧栏 + 域聚合页共用）。 */
export interface ContributedNavItem {
  /** 导航项 key（稳定标识；同 key 覆盖） */
  readonly key: string;
  readonly labelKey: string;
  readonly icon: ReactNode;
  /** 能力域轴的归域（`domainMeta` 的 8 个业务域之一） */
  readonly domain: CapabilityDomain;
  /** 产品导航组 key（`navGroups.NAV_GROUPS`）；组不存在则该项不出现在侧栏 */
  readonly group: string;
  /** 组内排序提示（缺省按声明顺序）；域聚合页始终按声明顺序 */
  readonly groupOrder?: number;
}

/** 已登记导航项（`path` 由所在路由派生，域侧只写一次路径）。 */
export interface ContributedNavEntry extends ContributedNavItem {
  readonly path: string;
  /** 与来源路由同源的插件门槛，消费方只读路由/nav 投影，不直接判它。 */
  readonly requiresPlugin?: string;
}

const byId = new Map<string, ContributedPageRoute>();
const navByKey = new Map<string, ContributedNavEntry>();
let snapshot: readonly ContributedPageRoute[] = [];
let navSnapshot: readonly ContributedNavEntry[] = [];
const listeners = new Set<() => void>();

// null = 插件列表还没投影完（或投影失败）：**不启用门槛**，避免首帧把域页整片抹掉。
// 非 null = 启用中的插件 name/id 集合，只有声明了 requiresPlugin 的域页受它约束。
let enabledPlugins: ReadonlySet<string> | null = null;

function pluginAvailable(required: string | undefined): boolean {
  return !required || enabledPlugins === null || enabledPlugins.has(required);
}

/** 由插件 store 在列表投影后调用；传 null 表示「未知」，不是「没有任何插件」。 */
export function setEnabledPlugins(names: readonly string[] | null): void {
  const next = names === null ? null : new Set(names);
  if (next?.size === enabledPlugins?.size && (!next || [...next].every((v) => enabledPlugins?.has(v)))) {
    return;
  }
  enabledPlugins = next;
  publish();
}

function publish(): void {
  // 必须是新数组引用：useSyncExternalStore 按引用比较快照，原地改会让订阅者看不到更新
  snapshot = [...byId.values()].filter((route) => pluginAvailable(route.requiresPlugin));
  navSnapshot = [...navByKey.values()].filter((entry) => pluginAvailable(entry.requiresPlugin));
  for (const listener of listeners) {
    listener();
  }
}

/** 域模块在自身加载时调用（幂等：同 id / 同 nav.key 覆盖）。 */
export function contributeDomainPages(routes: readonly ContributedPageRoute[]): void {
  for (const route of routes) {
    byId.set(route.id, route);
    if (route.nav) {
      navByKey.set(route.nav.key, {
        ...route.nav,
        path: route.path,
        requiresPlugin: route.requiresPlugin,
      });
    }
  }
  publish();
}

/** 非响应式读口（测试与诊断用；渲染侧请走 `useDomainPageRoutes`）。 */
export function getDomainPageRoutes(): readonly ContributedPageRoute[] {
  return snapshot;
}

export function subscribeDomainPageRoutes(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function useDomainPageRoutes(): readonly ContributedPageRoute[] {
  return useSyncExternalStore(subscribeDomainPageRoutes, getDomainPageRoutes, getDomainPageRoutes);
}

/** 非响应式读口：登记只发生在模块求值期，首帧前已就绪（侧栏/标题/反查都走这里）。 */
export function getContributedNavItems(): readonly ContributedNavEntry[] {
  return navSnapshot;
}

/**
 * pathname → pageKey 的域侧反查（宿主 `pathToPageKey` 在查完自己的表后调用）。
 *
 * 判据与宿主一致：先精确、再参数段路由（`/opc/:tab` 这类单段通配）、最后按 L1 段
 * 兜底（`/opc/domains/finance` 归到 `/opc` 的 pageKey）—— 少了最后一条，域聚合页会
 * 因为「宿主表里已经没有 /opc」而回落到 chat，侧栏高亮与页面上下文一起错位。
 */
export function resolveContributedPageKey(pathname: string): string | undefined {
  let paramMatch: string | undefined;
  for (const route of snapshot) {
    if (route.path === pathname) {
      return route.pageKey;
    }
    if (paramMatch === undefined && route.path.includes(":")) {
      const pattern = route.path
        .split("/")
        .map((seg) => (seg.startsWith(":") ? "[^/]+" : seg.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")))
        .join("/");
      if (new RegExp(`^${pattern}/?$`).test(pathname)) {
        paramMatch = route.pageKey;
      }
    }
  }
  if (paramMatch !== undefined) {
    return paramMatch;
  }
  const l1 = `/${pathname.slice(1).split("/")[0]}`;
  for (const route of snapshot) {
    if (route.path === l1) {
      return route.pageKey;
    }
  }
  return undefined;
}

/** 域路由模块的装载契约：`src/pages/<域>/routes.tsx` 必须导出常量 `DOMAIN_PAGES`。 */
export interface DomainRoutesModule {
  readonly DOMAIN_PAGES: readonly ContributedPageRoute[];
}

/**
 * 装载约定：`src/pages/<域>/routes.tsx` 导出 `DOMAIN_PAGES`，本文件装载后逐模块登记。
 *
 * `eager` 是必须的 —— 惰性 import 会让注册晚于首帧渲染，路由表在挂载后才补齐。
 * 被装载的模块只构造 `lazy()` 包装（页面本体仍是按需 chunk），不会把域代码打进主包。
 */
const loadedDomainRouteModules = import.meta.glob<DomainRoutesModule>("../pages/*/routes.tsx", { eager: true });

/** 已装载的域路由模块路径（诊断与测试钉点用：glob 命中为 0 通常意味着域目录改名，而不是「没有域页面」）。 */
export function loadedDomainRouteSources(): string[] {
  return Object.keys(loadedDomainRouteModules);
}

// 登记必须发生在 import 提升之后、也就是本文件模块体的最后：见文件头 TDZ 那段硬约束。
for (const routes of Object.values(loadedDomainRouteModules)) {
  contributeDomainPages(routes.DOMAIN_PAGES);
}
