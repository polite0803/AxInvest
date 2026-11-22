// SPDX-License-Identifier: AGPL-3.0-only

import type { ContributedPageRoute } from "@/lib/domainPageRegistry";
import { DollarCircleFilled, HistoryOutlined } from "@ant-design/icons";
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
 * 仍保留给 `/invest/replay` 旧模板串消费；侧栏入口本批已随插件门槛迁进本文件的 `nav`。）
 */

const LazyInvestPage: LazyExoticComponent<ComponentType> = lazy(() =>
  import("@/pages/InvestPage").then((m) => ({ default: m.InvestPage }))
);
const LazyReplayWorkbenchPage: LazyExoticComponent<ComponentType> = lazy(() =>
  import("@/pages/ReplayWorkbenchPage").then((m) => ({ default: m.ReplayWorkbenchPage }))
);

/**
 * 投资域主页路由：字段结构与 OPC 的路由声明同构。`requiresPlugin` 把页面的**可见性**
 * 绑到 stock 插件状态；侧栏入口由本文件同一条声明的 `nav` 派生，避免出现「路由消失但
 * 入口还在」的 404 按钮（`getNavItems()` 是「宿主内置 + 域贡献」直接拼接，宿主侧
 * `finance-investment` 静态项已同步删除，不能在这里重复贡献同 key）。
 */
export const DOMAIN_PAGES: readonly ContributedPageRoute[] = [
  {
    id: "invest:index",
    path: "/invest",
    pageKey: "finance-investment",
    Page: LazyInvestPage,
    // 股票页的**可见性**归 stock 插件：未装/未启用时路由与侧栏入口都不存在。
    // 页面组件本身仍在宿主编译（完整 bundle 外迁尚待 P6.1 前端移植），但对外契约
    // 已经与插件生命周期一致 —— 不能让用户看到插件未加载就存在的股票页。
    requiresPlugin: "stock",
    // 侧栏入口也走本表投影：宿主 navRegistry 的同 key 静态项已删，禁用后入口一并消失。
    nav: {
      key: "finance-investment",
      labelKey: "nav.financeInvestment",
      icon: <DollarCircleFilled style={{ fontSize: 17 }} />,
      domain: "finance",
      group: "invest",
    },
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
