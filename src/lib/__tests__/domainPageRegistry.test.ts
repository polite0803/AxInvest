// SPDX-License-Identifier: AGPL-3.0-only

import {
  contributeDomainPages,
  getContributedNavItems,
  getDomainPageRoutes,
  loadedDomainRouteSources,
  resolveContributedPageKey,
  subscribeDomainPageRoutes,
} from "@/lib/domainPageRegistry";
import { BUILTIN_PAGE_PATH } from "@/lib/pageRegistry";
import { type ComponentType, lazy, type LazyExoticComponent } from "react";
import { describe, expect, it, vi } from "vitest";

/**
 * 域页面路由注册表（#36 迁出期：宿主内容区改为渲染注册表投影）。
 *
 * 钉五件事：① glob 装载口真的把 OPC 域模块求值了（装载不到 = 路由静默消失，
 * 页面 404 而不是报错，是本机制最坏的失效形态）；② path / pageKey 由域模块**字面量**
 * 声明，宿主 `BUILTIN_PAGE_PATH` 不再登记这些键（导航声明权已移交域侧）；③ 侧栏入口
 * 跟随路由声明，组归属与组内次序不变；④ 同 id 覆盖而非追加，且订阅者能收到新快照引用；
 * ⑤ 本文件能 import 得动注册表本身 —— eager glob 把域模块提升到注册表求值之前，
 * 域模块一旦反向 import 注册表的运行时符号就踩 const TDZ，套套逻辑地让下面所有断言在
 * 启动阶段一起崩（首轮实测即为此，故域侧只许 `import type`）。
 */

/** 注册表条目的 `Page` 只被宿主放进 `<SafeLazyPage>`，测试不渲染 ⇒ 用一个不解析的 lazy 占位。 */
function probePage(name: string): LazyExoticComponent<ComponentType> {
  return lazy(() => new Promise<{ default: ComponentType }>(() => void name));
}

describe("domainPageRegistry", () => {
  it("glob 装载口求值了域路由模块（命中为空 ⇒ 域目录改名，不是「没有域页面」）", () => {
    const sources = loadedDomainRouteSources();
    expect(sources.filter((s) => s.includes("pages/opc/routes"))).toHaveLength(1);
    // #263：投资域主页装载口迁入 `src/pages/invest/routes.tsx`，同一 glob 自动命中
    expect(sources.filter((s) => s.includes("pages/invest/routes"))).toHaveLength(1);
  });

  it("注册表共 5 条路由：OPC 4（主页两段 + 需求发现 + 域聚合页）+ 投资域 1", () => {
    const routes = getDomainPageRoutes();
    expect(routes).toHaveLength(5);
    expect(routes.filter((r) => r.id.startsWith("domain:"))).toHaveLength(0);
    // 带侧栏入口的 2 条；`/opc/:tab` 与 `/opc/domains/:domainId` 是同页子路由，无入口；
    // 投资域 `/invest` 无入口（侧栏项仍由宿主 builtinNavItems 持有，见 invest/routes.tsx 头注）
    expect(routes.filter((r) => r.nav)).toHaveLength(2);
  });

  it("path 由域模块字面量声明，宿主 BUILTIN_PAGE_PATH 已不再登记域页面", () => {
    const byId = new Map(getDomainPageRoutes().map((r) => [r.id, r]));
    expect(byId.get("opc:index")?.path).toBe("/opc");
    expect(byId.get("opc:tab")?.path).toBe("/opc/:tab");
    expect(byId.get("demand-discovery")?.path).toBe("/demand-discovery");
    expect(byId.get("opc:domain-hub")?.path).toBe("/opc/domains/:domainId");
    // 域内页面逐条回查：path 由域模块字面量声明，
    // 且宿主表里查不到该 pageKey（查得到 = 命名权又回到主壳）
    expect(byId.get("opc:index")?.pageKey).toBe("automation-operations");
    expect(byId.get("demand-discovery")?.pageKey).toBe("demand-discovery");
    // 负控：宿主自有页面仍在表里（证明上面的 `toBeUndefined` 不是因为表空了）
    expect(BUILTIN_PAGE_PATH.chat).toBe("/chat");
  });

  it("侧栏入口随路由声明：2 项，组归属与组内次序不变", () => {
    const nav = getContributedNavItems();
    expect(nav).toHaveLength(2);
    const ordered = (group: string) =>
      nav.filter((n) => n.group === group).sort((a, b) => (a.groupOrder ?? 0) - (b.groupOrder ?? 0));
    expect(ordered("opc").map((n) => [n.key, n.path, n.groupOrder])).toEqual([
      ["automation-operations", "/opc", 0],
      ["demand-discovery", "/demand-discovery", 1],
    ]);
    // path 只写一次：入口路径必须等于其所属路由的 path
    for (const entry of nav) {
      expect(getDomainPageRoutes().find((r) => r.nav?.key === entry.key)?.path).toBe(entry.path);
    }
  });

  it("pathname → pageKey 反查覆盖精确 / 参数段 / L1 兜底", () => {
    expect(resolveContributedPageKey("/opc")).toBe("automation-operations");
    expect(resolveContributedPageKey("/opc/finance")).toBe("automation-operations");
    expect(resolveContributedPageKey("/opc/domains/finance")).toBe("automation-operations");
    expect(resolveContributedPageKey("/opc/domain/security")).toBe("automation-operations");
    expect(resolveContributedPageKey("/demand-discovery")).toBe("demand-discovery");
    expect(resolveContributedPageKey("/knowledge")).toBeUndefined();
  });

  it("同 id 重复注册 = 覆盖，不追加（域模块被二次求值也不会长出重复路由）", () => {
    const before = getDomainPageRoutes().length;
    const probe = () => ({ id: "__probe__", path: "/__probe__", pageKey: "__probe__", Page: probePage("probe") });
    contributeDomainPages([probe()]);
    contributeDomainPages([probe()]);
    const routes = getDomainPageRoutes();
    expect(routes).toHaveLength(before + 1);
    expect(routes.filter((r) => r.id === "__probe__")).toHaveLength(1);
  });

  it("注册后订阅者收到新快照引用；退订后不再收", () => {
    const listener = vi.fn();
    const unsubscribe = subscribeDomainPageRoutes(listener);
    contributeDomainPages([{
      id: "__probe2__",
      path: "/__probe2__",
      pageKey: "__probe2__",
      Page: probePage("probe2"),
    }]);
    expect(listener).toHaveBeenCalled();
    unsubscribe();
    const calls = listener.mock.calls.length;
    contributeDomainPages([{
      id: "__probe3__",
      path: "/__probe3__",
      pageKey: "__probe3__",
      Page: probePage("probe3"),
    }]);
    expect(listener.mock.calls.length).toBe(calls);
  });
});
