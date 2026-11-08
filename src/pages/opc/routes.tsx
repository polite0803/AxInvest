// SPDX-License-Identifier: AGPL-3.0-only

import { type ComponentType, lazy, type LazyExoticComponent } from "react";

/**
 * OPC 域页面的懒加载入口与路由表。
 *
 * 住域目录而不是宿主主壳：域内增删页面只改本文件，主壳只知道「有一张 OPC 路由表」，
 * 不再逐页持有组件名与模块路径（#36 迁出期把宿主侧耦合面收成一个模块）。
 */

export const LazyOpcPage = lazy(() => import("./OpcPage").then((m) => ({ default: m.OpcPage })));
export const LazyDemandDiscoveryPage = lazy(() =>
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

/** 能力包页路由表：[BuiltinPageKey（即能力域导航 key）, 页面组件] */
export const OPC_DOMAIN_ROUTES: ReadonlyArray<
  [string, LazyExoticComponent<ComponentType>]
> = [
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
