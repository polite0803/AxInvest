// SPDX-License-Identifier: AGPL-3.0-only

import type { ContributedPageRoute } from "@/lib/domainPageRegistry";
import { BUILTIN_PAGE_PATH } from "@/lib/pageRegistry";
import { type ComponentType, lazy, type LazyExoticComponent } from "react";

/**
 * OPC 域页面的懒加载入口 + 路由表声明。
 *
 * 住域目录而不是宿主主壳：域内增删页面只改本文件。宿主内容区不再 import 本模块的任何符号
 * （#36 迁出期），装载口是 `src/lib/domainPageRegistry.ts` 里按「pages 下一级目录 +
 * routes.tsx」写的 glob，本文件导出的 `DOMAIN_PAGES` 由注册表求值后登记。
 *
 * ⚠ 本文件对注册表只能 `import type`：注册表 eager 装载本模块，若反向 import 它的运行时符号，
 * 就形成「注册表 ← 本模块 ← 注册表」的环并踩中 const 的 TDZ，应用在启动阶段直接报错。
 *
 * ⚠ 只声明「哪个组件填哪条路由」：path 与 pageKey 仍取自宿主路径表 `BUILTIN_PAGE_PATH`
 * （侧栏高亮与 path→key 反查都从它派生）。把 path 声明权也交出去是 OPC 整棵迁出仓库那一刀的
 * 事，与导航项贡献面同批做，否则侧栏会拿到 key 却查不到 path。
 */

const OPC_HOME = BUILTIN_PAGE_PATH["automation-operations"];

const LazyOpcPage: LazyExoticComponent<ComponentType> = lazy(() =>
  import("./OpcPage").then((m) => ({ default: m.OpcPage }))
);
const LazyDemandDiscoveryPage: LazyExoticComponent<ComponentType> = lazy(() =>
  import("./DemandDiscoveryPage").then((m) => ({ default: m.DemandDiscoveryPage }))
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

/** 能力包页：[BuiltinPageKey（即能力域导航 key）, 页面组件] —— id 用 key 本身，注册即覆盖。 */
const DOMAIN_PACK_PAGES: ReadonlyArray<[string, LazyExoticComponent<ComponentType>]> = [
  ["finance-analysis", LazyFinanceInvestDomainPage],
  ["finance-accounting", LazyAccountingDomainPage],
  ["automation-sales", LazySalesGrowthDomainPage],
  ["automation-projects", LazyProjectManagementDomainPage],
  ["automation-consulting", LazyConsultingDomainPage],
  ["automation-ecommerce", LazyEcommerceDomainPage],
  ["devops-software", LazySoftwareDevDomainPage],
  ["devops-security", LazySecurityDomainPage],
  ["data-geospatial", LazyGeospatialDomainPage],
  ["data-ai-research", LazyAiResearchDomainPage],
  ["content-media", LazyContentMediaDomainPage],
  ["content-design", LazyDesignDomainPage],
  ["content-education", LazyEducationDomainPage],
  ["ai-media-game", LazyGameDevDomainPage],
];

/** 交给宿主注册表登记的路由表：OPC 主页两段 + 需求发现 + 14 个能力包页。 */
export const DOMAIN_PAGES: readonly ContributedPageRoute[] = [
  // OPC 主页两段路由：`/opc` 与 `/opc/<tab>`（tab 缺省时由 OpcPage 回落首个 Tab）
  { id: "opc:index", path: OPC_HOME, pageKey: "automation-operations", Page: LazyOpcPage },
  { id: "opc:tab", path: `${OPC_HOME}/:tab`, pageKey: "automation-operations", Page: LazyOpcPage },
  {
    id: "demand-discovery",
    path: BUILTIN_PAGE_PATH["demand-discovery"],
    pageKey: "demand-discovery",
    Page: LazyDemandDiscoveryPage,
  },
  ...DOMAIN_PACK_PAGES.map(([pageKey, Page]) => ({
    id: `domain:${pageKey}`,
    path: BUILTIN_PAGE_PATH[pageKey],
    pageKey,
    Page,
  })),
];
