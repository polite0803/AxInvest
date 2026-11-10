// SPDX-License-Identifier: AGPL-3.0-only

import type { ContributedNavItem, ContributedPageRoute } from "@/lib/domainPageRegistry";
import type { CapabilityDomain } from "@/types/capability";
import {
  AimOutlined,
  AppstoreFilled,
  BgColorsOutlined,
  BookFilled,
  CalculatorFilled,
  CarryOutFilled,
  CodeFilled,
  EnvironmentFilled,
  PlaySquareFilled,
  RiseOutlined,
  RobotFilled,
  SafetyCertificateFilled,
  ShoppingFilled,
  TeamOutlined,
  VideoCameraFilled,
} from "@ant-design/icons";
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

const LazyFinanceInvestDomainPage = lazy(() =>
  import("./domains/DomainPages").then((m) => ({ default: m.FinanceInvestPage }))
);
const LazyAccountingDomainPage = lazy(() =>
  import("./domains/DomainPages").then((m) => ({ default: m.AccountingPage }))
);
const LazySalesGrowthDomainPage = lazy(() =>
  import("./domains/DomainPages").then((m) => ({ default: m.SalesGrowthPage }))
);
const LazyProjectManagementDomainPage = lazy(() =>
  import("./domains/DomainPages").then((m) => ({ default: m.ProjectManagementPage }))
);
const LazyConsultingDomainPage = lazy(() =>
  import("./domains/DomainPages").then((m) => ({ default: m.ConsultingPage }))
);
const LazyEcommerceDomainPage = lazy(() => import("./domains/DomainPages").then((m) => ({ default: m.EcommercePage })));
const LazySoftwareDevDomainPage = lazy(() =>
  import("./domains/DomainPages").then((m) => ({ default: m.SoftwareDevPage }))
);
const LazySecurityDomainPage = lazy(() => import("./domains/DomainPages").then((m) => ({ default: m.SecurityPage })));
const LazyGeospatialDomainPage = lazy(() =>
  import("./domains/DomainPages").then((m) => ({ default: m.GeospatialPage }))
);
const LazyAiResearchDomainPage = lazy(() =>
  import("./domains/DomainPages").then((m) => ({ default: m.AiResearchPage }))
);
const LazyContentMediaDomainPage = lazy(() =>
  import("./domains/DomainPages").then((m) => ({ default: m.ContentMediaPage }))
);
const LazyDesignDomainPage = lazy(() => import("./domains/DomainPages").then((m) => ({ default: m.DesignPage })));
const LazyEducationDomainPage = lazy(() => import("./domains/DomainPages").then((m) => ({ default: m.EducationPage })));
const LazyGameDevDomainPage = lazy(() => import("./domains/DomainPages").then((m) => ({ default: m.GameDevPage })));

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
 * 顺序即域聚合页（DomainHubPage）的展示顺序；侧栏按 `group` + `groupOrder` 归组。
 * `groupOrder` 与旧 `navGroups.ts` 的排列一致，避免侧栏组内次序变化。
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
  {
    id: "domain:finance-analysis",
    pageKey: "finance-analysis",
    path: "/opc/domain/finance-invest",
    domain: "finance",
    group: "packs",
    groupOrder: 0,
    labelKey: "nav.financeAnalysis",
    icon: navIcon(TeamOutlined),
    Page: LazyFinanceInvestDomainPage,
  },
  {
    id: "domain:finance-accounting",
    pageKey: "finance-accounting",
    path: "/opc/domain/accounting",
    domain: "finance",
    group: "packs",
    groupOrder: 1,
    labelKey: "nav.financeAccounting",
    icon: navIcon(CalculatorFilled),
    Page: LazyAccountingDomainPage,
  },
  {
    id: "domain:automation-sales",
    pageKey: "automation-sales",
    path: "/opc/domain/sales-growth",
    domain: "automation",
    group: "packs",
    groupOrder: 2,
    labelKey: "nav.automationSales",
    icon: navIcon(RiseOutlined),
    Page: LazySalesGrowthDomainPage,
  },
  {
    id: "domain:automation-projects",
    pageKey: "automation-projects",
    path: "/opc/domain/project-management",
    domain: "automation",
    group: "packs",
    groupOrder: 3,
    labelKey: "nav.automationProjects",
    icon: navIcon(CarryOutFilled),
    Page: LazyProjectManagementDomainPage,
  },
  {
    id: "domain:automation-consulting",
    pageKey: "automation-consulting",
    path: "/opc/domain/consulting",
    domain: "automation",
    group: "packs",
    groupOrder: 4,
    labelKey: "nav.automationConsulting",
    icon: navIcon(TeamOutlined),
    Page: LazyConsultingDomainPage,
  },
  {
    id: "domain:automation-ecommerce",
    pageKey: "automation-ecommerce",
    path: "/opc/domain/ecommerce",
    domain: "automation",
    group: "packs",
    groupOrder: 5,
    labelKey: "nav.automationEcommerce",
    icon: navIcon(ShoppingFilled),
    Page: LazyEcommerceDomainPage,
  },
  {
    id: "domain:devops-software",
    pageKey: "devops-software",
    path: "/opc/domain/software-dev",
    domain: "devops",
    group: "packs",
    groupOrder: 6,
    labelKey: "nav.devopsSoftware",
    icon: navIcon(CodeFilled),
    Page: LazySoftwareDevDomainPage,
  },
  {
    id: "domain:devops-security",
    pageKey: "devops-security",
    path: "/opc/domain/security",
    domain: "devops",
    group: "packs",
    groupOrder: 7,
    labelKey: "nav.devopsSecurity",
    icon: navIcon(SafetyCertificateFilled),
    Page: LazySecurityDomainPage,
  },
  {
    id: "domain:data-geospatial",
    pageKey: "data-geospatial",
    path: "/opc/domain/geospatial",
    domain: "data_analysis",
    group: "packs",
    groupOrder: 8,
    labelKey: "nav.dataGeospatial",
    icon: navIcon(EnvironmentFilled),
    Page: LazyGeospatialDomainPage,
  },
  {
    id: "domain:data-ai-research",
    pageKey: "data-ai-research",
    path: "/opc/domain/ai-research",
    domain: "data_analysis",
    group: "packs",
    groupOrder: 9,
    labelKey: "nav.dataAiResearch",
    icon: navIcon(RobotFilled),
    Page: LazyAiResearchDomainPage,
  },
  {
    id: "domain:content-media",
    pageKey: "content-media",
    path: "/opc/domain/content-media",
    domain: "content_creation",
    group: "packs",
    groupOrder: 10,
    labelKey: "nav.contentMedia",
    icon: navIcon(VideoCameraFilled),
    Page: LazyContentMediaDomainPage,
  },
  {
    id: "domain:content-design",
    pageKey: "content-design",
    path: "/opc/domain/design",
    domain: "content_creation",
    group: "packs",
    groupOrder: 11,
    labelKey: "nav.contentDesign",
    icon: navIcon(BgColorsOutlined),
    Page: LazyDesignDomainPage,
  },
  {
    id: "domain:content-education",
    pageKey: "content-education",
    path: "/opc/domain/education",
    domain: "content_creation",
    group: "packs",
    groupOrder: 12,
    labelKey: "nav.contentEducation",
    icon: navIcon(BookFilled),
    Page: LazyEducationDomainPage,
  },
  {
    id: "domain:ai-media-game",
    pageKey: "ai-media-game",
    path: "/opc/domain/game-dev",
    domain: "ai_media",
    group: "packs",
    groupOrder: 13,
    labelKey: "nav.aiMediaGame",
    icon: navIcon(PlaySquareFilled),
    Page: LazyGameDevDomainPage,
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

/** 交给宿主注册表登记的路由表：16 个带侧栏入口的页面 + 2 条同页子路由。 */
export const DOMAIN_PAGES: readonly ContributedPageRoute[] = [
  ...NAV_PAGE_DECLARATIONS.map(navRouteOf),
  ...EXTRA_ROUTES,
];
