// SPDX-License-Identifier: AGPL-3.0-only

import type { ContributedPageRoute } from "@/lib/domainPageRegistry";
import { type ComponentType, lazy, type LazyExoticComponent } from "react";

/**
 * 投资域页面的懒加载入口 + 路由声明（#263 第一批：装载权交回域自身）。
 *
 * 住域目录而不是宿主主壳：此前 `ContentArea.tsx` 直接 `lazy(() => import("@/pages/InvestPage"))`，
 * 宿主布局层持有一条命中股票域 feRe 的说明符（`audit-l1-blockers` 前端反向阻塞 foreign 点）。
 * 形态逐字对照 `src/pages/opc/routes.tsx`（#230 / #260 先例）：
 * 宿主内容区与侧栏都不 import 本模块的任何符号，装载口是 `src/lib/domainPageRegistry.ts`
 * 里按「pages 下一级目录 + routes.tsx」写的 eager glob，本文件导出的 `DOMAIN_PAGES`
 * 由注册表求值后登记并投影给 `<Routes>`。
 *
 * ⚠ 本文件对注册表只能 `import type`：注册表 eager 装载本模块，若反向 import 它的运行时符号，
 * 就形成「注册表 ← 本模块 ← 注册表」的环并踩中 const 的 TDZ，应用在启动阶段直接报错。
 *
 * ⚠ path / pageKey 由本文件**字面量**声明（#260 之后 OPC 的同一口径）：不再从宿主
 * `BUILTIN_PAGE_PATH` 取路径 —— 那等于把命名权留在主壳。（宿主表里 `"finance-investment": "/invest"`
 * 本批仍保留：侧栏导航项 `navRegistry` 与 `/invest/replay` 旧模板串还在消费它，
 * 导航声明权的移交属后续批次，本批边界只交回**装载权**。）
 */

const LazyInvestPage: LazyExoticComponent<ComponentType> = lazy(() =>
  import("@/pages/InvestPage").then((m) => ({ default: m.InvestPage }))
);

/**
 * 无侧栏入口的页面路由：字段结构与 OPC 的 `EXTRA_ROUTES` 同构（`ContributedPageRoute`
 * 的 `nav` 为可选字段，缺省即不出现在侧栏投影里）。投资域主页的侧栏入口此前就由宿主
 * `navRegistry.builtinNavItems`（key `finance-investment`）持有，本批不重复贡献一份
 * 同 key 导航项（`getNavItems()` 是「宿主内置 + 域贡献」直接拼接，重复 key 会在侧栏长出双入口）。
 */
export const DOMAIN_PAGES: readonly ContributedPageRoute[] = [
  { id: "invest:index", path: "/invest", pageKey: "finance-investment", Page: LazyInvestPage },
];
