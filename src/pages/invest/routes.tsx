// SPDX-License-Identifier: AGPL-3.0-only

import type { ContributedPageRoute } from "@/lib/domainPageRegistry";
import { HistoryOutlined } from "@ant-design/icons";
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
 * 仍保留给 `/invest` 与 `/invest/replay` 的 `pageKey` 反查；回放工作台的侧栏入口
 * 仍在本文件的 `nav` 里随 stock 门槛出入。）
 *
 * T3 完整外迁（2026-11-23）：`invest:index` 不再编译 `InvestPage` / `InvestHub`，
 * 改由 `LegacyInvestRedirect`（零业务代码）把旧链转到 stock 插件 bundle 页；
 * 侧栏入口也一并撤下，启用后的入口由 `pluginRouteRegistry` 的插件页投影提供。
 */

const LazyInvestLegacyRedirect: LazyExoticComponent<ComponentType> = lazy(() =>
  import("@/pages/invest/LegacyInvestRedirect").then((m) => ({ default: m.LegacyInvestRedirect }))
);
const LazyReplayWorkbenchPage: LazyExoticComponent<ComponentType> = lazy(() =>
  import("@/pages/ReplayWorkbenchPage").then((m) => ({ default: m.ReplayWorkbenchPage }))
);

/**
 * 投资域路由：字段结构与 OPC 的路由声明同构。
 *
 * `invest:index` 是**兼容跳转桥**，不是股票页面：
 *   - 不设 `requiresPlugin` —— 插件禁用时也要能把旧链落到明确降级提示，
 *     而不是 404（禁用 ≠ 页面从宇宙中消失，用户仍可能从历史收藏进来）；
 *   - 不设 `nav` —— 启用后的侧栏入口由 `pluginRouteRegistry` 的插件页投影
 *     提供（`/plugins/stock/stock-home`），宿主不再保留第二个「投资」按钮。
 *
 * `invest:replay` 仍是宿主内真实页面（时间旅行回放工作台），保留 stock 门槛：
 * 它依赖 stock 的 as-of 数据面，插件禁用时不应可达。
 */
export const DOMAIN_PAGES: readonly ContributedPageRoute[] = [
  {
    id: "invest:index",
    path: "/invest",
    pageKey: "finance-investment",
    Page: LazyInvestLegacyRedirect,
  },
  {
    // 回放工作台：与投资主页同一 stock 门槛，禁用插件后旧链也不能进入股票业务页。
    // pageKey 沿用 finance-investment，保持页面上下文与改动前一致。
    id: "invest:replay",
    path: "/invest/replay",
    pageKey: "finance-investment",
    Page: LazyReplayWorkbenchPage,
    requiresPlugin: "stock",
    nav: {
      key: "invest-replay",
      labelKey: "nav.replayWorkbench",
      icon: <HistoryOutlined style={{ fontSize: 17 }} />,
      domain: "finance",
      group: "invest",
    },
  },
];
