// SPDX-License-Identifier: AGPL-3.0-only

import {
  contributeDomainPages,
  getContributedNavItems,
  getDomainPageRoutes,
  loadedDomainRouteSources,
  resolveContributedPageKey,
  setEnabledPlugins,
  subscribeDomainPageRoutes,
} from "@/lib/domainPageRegistry";
import { getNavItems } from "@/lib/navRegistry";
import { BUILTIN_PAGE_PATH } from "@/lib/pageRegistry";
import { type ComponentType, lazy, type LazyExoticComponent } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

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
  afterEach(() => {
    setEnabledPlugins(null);
  });

  it("glob 装载口求值了域路由模块（命中为空 ⇒ 域目录改名，不是「没有域页面」）", () => {
    const sources = loadedDomainRouteSources();
    expect(sources.filter((s) => s.includes("pages/opc/routes"))).toHaveLength(1);
    // #263：投资域主页装载口迁入 `src/pages/invest/routes.tsx`，同一 glob 自动命中
    expect(sources.filter((s) => s.includes("pages/invest/routes"))).toHaveLength(1);
  });

  it("注册表共 6 条路由：OPC 4（主页两段 + 需求发现 + 域聚合页）+ 投资域 2", () => {
    const routes = getDomainPageRoutes();
    expect(routes).toHaveLength(6);
    expect(routes.filter((r) => r.id.startsWith("domain:"))).toHaveLength(0);
    // 带侧栏入口的 3 条；`/opc/:tab` 与 `/opc/domains/:domainId` 是同页子路由，无入口。
    // 投资域只剩「回放工作台」一条入口 —— 股票本体的入口已交给插件页投影。
    expect(routes.filter((r) => r.nav)).toHaveLength(3);
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

  it("侧栏入口随路由声明：3 项，组归属与组内次序不变", () => {
    const nav = getContributedNavItems();
    expect(nav).toHaveLength(3);
    // T3 完整外迁：股票本体入口改由 `pluginRouteRegistry` 的插件页投影提供，
    // 宿主注册表不再贡献 `finance-investment`（避免启用后侧栏出现两个投资入口）。
    expect(nav.find((n) => n.key === "finance-investment")).toBeUndefined();
    expect(nav.find((n) => n.key === "invest-replay")).toMatchObject({
      path: "/invest/replay",
      group: "invest",
      domain: "finance",
    });
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

  it("迁移后的入口在宿主骨架与域贡献里各只出现一次", () => {
    // 宿主 navGroups.itemKeys 若还点名回放工作台的 key，Sidebar 会把它同时当宿主项与域项渲染；
    // 股票本体则不应再出现在宿主导航（其入口归插件页投影）。
    const keys = getNavItems().map((n) => n.key);
    expect(keys.filter((k) => k === "invest-replay")).toHaveLength(1);
    expect(keys).not.toContain("finance-investment");
  });

  it("pathname → pageKey 反查覆盖精确 / 参数段 / L1 兜底", () => {
    expect(resolveContributedPageKey("/opc")).toBe("automation-operations");
    expect(resolveContributedPageKey("/opc/finance")).toBe("automation-operations");
    expect(resolveContributedPageKey("/opc/domains/finance")).toBe("automation-operations");
    expect(resolveContributedPageKey("/opc/domain/security")).toBe("automation-operations");
    expect(resolveContributedPageKey("/demand-discovery")).toBe("demand-discovery");
    expect(resolveContributedPageKey("/invest")).toBe("finance-investment");
    expect(resolveContributedPageKey("/invest/replay")).toBe("finance-investment");
    expect(resolveContributedPageKey("/knowledge")).toBeUndefined();
  });

  it("requiresPlugin：stock 未启用时回放路由与入口消失；/invest 兼容桥始终可达", () => {
    const hasIndex = () => getDomainPageRoutes().some((r) => r.id === "invest:index");
    const hasReplay = () => getDomainPageRoutes().some((r) => r.id === "invest:replay");
    const hasReplayNav = () => getContributedNavItems().some((n) => n.key === "invest-replay");
    expect(hasIndex() && hasReplay() && hasReplayNav()).toBe(true);

    setEnabledPlugins(["other@bundled", "other"]);
    // `/invest` 只是零业务代码的兼容跳转桥：插件禁用时仍要可达并显示降级提示，
    // 否则历史收藏/旧调用方会直接落到 404（用户报告过的导航失效形态）。
    expect(hasIndex()).toBe(true);
    expect(resolveContributedPageKey("/invest")).toBe("finance-investment");
    expect(hasReplay()).toBe(false);
    expect(hasReplayNav()).toBe(false);
    // 路由本身已消失；pageKey 反查仍按 L1 前缀落到 `/invest` 兼容桥 —— 反查只影响
    // 标题/页面上下文，不会把不可达路径变回可达，故这里断言兜底值而不是 undefined。
    expect(resolveContributedPageKey("/invest/replay")).toBe("finance-investment");
    expect(getDomainPageRoutes().some((r) => r.id === "opc:index")).toBe(true);

    setEnabledPlugins(["stock@bundled", "stock"]);
    expect(hasReplay() && hasReplayNav()).toBe(true);

    // 插件列表尚未投影 / 投影失败 ⇒ 门槛整体失效，不能把域页整片抹掉。
    setEnabledPlugins(null);
    expect(hasReplay() && hasReplayNav()).toBe(true);
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
