// SPDX-License-Identifier: AGPL-3.0-only

import { type ComponentType, type LazyExoticComponent, useSyncExternalStore } from "react";

/**
 * 域页面路由注册表 —— 宿主内容区不再静态 import 任何域页面组件。
 *
 * 为什么要有这张表（#36 迁出期）：宿主布局此前从 `@/pages/opc/routes` 直接点名字 import
 * `LazyOpcPage` / `OPC_DOMAIN_ROUTES`，等于「主壳知道 OPC 有哪些页面、各是什么组件」——
 * 域内每加一页都要改宿主文件，且宿主永远摘不掉对域目录的依赖。
 * 改为「域模块只导出一张路由表，本表装载后登记，宿主只渲染注册表投影」后，
 * 域侧增删页面宿主零改动，域整棵迁出仓库时本表的 glob 命中变空即自然退役（不留兼容分支）。
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
  /** URL 路径，可含 `:param` 段 */
  readonly path: string;
  /**
   * `PageContextProvider` 的 page key —— 域页面一律挂在宿主页面上下文里
   * （agent 面板按它取上下文）。
   * 路径本身仍归宿主 `pageRegistry` 掌握 —— 本表只负责「哪个组件填哪条路由」。
   */
  readonly pageKey: string;
  readonly Page: LazyExoticComponent<ComponentType>;
}

const byId = new Map<string, ContributedPageRoute>();
let snapshot: readonly ContributedPageRoute[] = [];
const listeners = new Set<() => void>();

function publish(): void {
  // 必须是新数组引用：useSyncExternalStore 按引用比较快照，原地改会让订阅者看不到更新
  snapshot = [...byId.values()];
  for (const listener of listeners) {
    listener();
  }
}

/** 域模块在自身加载时调用（幂等：同 id 覆盖）。 */
export function contributeDomainPages(routes: readonly ContributedPageRoute[]): void {
  for (const route of routes) {
    byId.set(route.id, route);
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
