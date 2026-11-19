// SPDX-License-Identifier: AGPL-3.0-only

import type { ContributedNavItem, ContributedPageRoute } from "@/lib/domainPageRegistry";
import type { CapabilityDomain } from "@/types/capability";
import { AimOutlined, AppstoreFilled } from "@ant-design/icons";
import { type ComponentType, lazy, type LazyExoticComponent, type ReactNode } from "react";

/**
 * OPC 域页面的懒加载入口 + 路由 / 导航声明。
 *
 * 住域目录而不是宿主主壳：域内增删页面、改路径、改侧栏入口只改本文件。宿主内容区与侧栏
 * 都不 import 本模块的任何符号（#36 迁出期），装载口是 `src/lib/domainPageRegistry.ts`
 * 里按「pages 下一级目录 + routes.tsx」写的 glob，本文件导出的 `DOMAIN_PAGES` 由注册表
 * 求值后登记并投影给 `<Routes>` / 侧栏 / 页面标题 / path→pageKey 反查。
 *
 * ⚠ 本文件对注册表只能 `import type`：注册表 eager 装载本模块，若反向 import 它的运行时符号，
 * 就形成「注册表 ← 本模块 ← 注册表」的环并踩中 const 的 TDZ，应用在启动阶段直接报错。
 *
 * ⚠ path / pageKey 由本文件**字面量**声明（2026-11-10 #36 导航声明权半）：宿主
 * `BUILTIN_PAGE_PATH` 已不再登记这些页面，别再从宿主表取路径 —— 那等于把命名权留在主壳。
 */

const navIcon = (Icon: React.ComponentType<{ style?: React.CSSProperties }>): ReactNode => (
  <Icon style={{ fontSize: 17 }} />
);

const LazyOpcPage: LazyExoticComponent<ComponentType> = lazy(() =>
  import("./OpcPage").then((m) => ({ default: m.OpcPage }))
);
const LazyDemandDiscoveryPage: LazyExoticComponent<ComponentType> = lazy(() =>
  import("./DemandDiscoveryPage").then((m) => ({ default: m.DemandDiscoveryPage }))
);
const LazyDomainHubPage: LazyExoticComponent<ComponentType> = lazy(() =>
  import("@/pages/DomainHubPage").then((m) => ({ default: m.DomainHubPage }))
);

/** 带侧栏入口的页面：一次声明同时产出路由与导航项（path 只写一次）。 */
interface NavPageDeclaration {
  readonly id: string;
  readonly pageKey: string;
  readonly path: string;
  readonly domain: CapabilityDomain;
  readonly group: string;
  readonly groupOrder: number;
  readonly labelKey: string;
  readonly icon: ReactNode;
  readonly Page: LazyExoticComponent<ComponentType>;
}

/**
 * 14 个旧 `/opc/domain/*` 业务页已由运行期插件投影接管：宿主只保留 OPC 壳层、
 * 需求发现与域聚合页，避免编译期页与插件页并行造成同一页面两套来源。
 */
const NAV_PAGE_DECLARATIONS: readonly NavPageDeclaration[] = [
  {
    id: "demand-discovery",
    pageKey: "demand-discovery",
    path: "/demand-discovery",
    domain: "automation",
    group: "opc",
    groupOrder: 1,
    labelKey: "opc.demand.pageTitle",
    icon: navIcon(AimOutlined),
    Page: LazyDemandDiscoveryPage,
  },
  {
    id: "opc:index",
    pageKey: "automation-operations",
    path: "/opc",
    domain: "automation",
    group: "opc",
    groupOrder: 0,
    labelKey: "nav.automationOperations",
    icon: navIcon(AppstoreFilled),
    Page: LazyOpcPage,
  },
];

const navItemOf = (decl: NavPageDeclaration): ContributedNavItem => ({
  key: decl.pageKey,
  labelKey: decl.labelKey,
  icon: decl.icon,
  domain: decl.domain,
  group: decl.group,
  groupOrder: decl.groupOrder,
});

const navRouteOf = (decl: NavPageDeclaration): ContributedPageRoute => ({
  id: decl.id,
  path: decl.path,
  pageKey: decl.pageKey,
  Page: decl.Page,
  nav: navItemOf(decl),
});

/** 无侧栏入口的同页子路由（浅链 / 页内跳转；`/opc/:tab` 的 tab 由 OpcPage 回落）。 */
const EXTRA_ROUTES: readonly ContributedPageRoute[] = [
  { id: "opc:tab", path: "/opc/:tab", pageKey: "automation-operations", Page: LazyOpcPage },
  { id: "opc:domain-hub", path: "/opc/domains/:domainId", pageKey: "automation-operations", Page: LazyDomainHubPage },
];

/** 交给宿主注册表登记的路由表：OPC 壳层与需求发现页 + 2 条同页子路由。 */
export const DOMAIN_PAGES: readonly ContributedPageRoute[] = [
  ...NAV_PAGE_DECLARATIONS.map(navRouteOf),
  ...EXTRA_ROUTES,
];
