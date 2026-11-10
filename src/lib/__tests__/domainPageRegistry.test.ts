// SPDX-License-Identifier: AGPL-3.0-only

import {
  contributeDomainPages,
  getDomainPageRoutes,
  loadedDomainRouteSources,
  subscribeDomainPageRoutes,
} from "@/lib/domainPageRegistry";
import { BUILTIN_PAGE_PATH } from "@/lib/pageRegistry";
import { type ComponentType, lazy, type LazyExoticComponent } from "react";
import { describe, expect, it, vi } from "vitest";

/**
 * 域页面路由注册表（#36 迁出期：宿主内容区改为渲染注册表投影）。
 *
 * 钉四件事：① glob 装载口真的把 OPC 域模块求值了（装载不到 = 路由静默消失，
 * 页面 404 而不是报错，是本机制最坏的失效形态）；② 注册表内容逐条与宿主路径表一致
 * （path 由 `BUILTIN_PAGE_PATH` 给，域侧不许自写路径）；③ 同 id 覆盖而非追加，
 * 且订阅者能收到新快照引用；④ 本文件能 import 得动注册表本身 —— eager glob 把域模块
 * 提升到注册表求值之前，域模块一旦反向 import 注册表的运行时符号就踩 const TDZ，
 * 套套逻辑地让下面所有断言在启动阶段一起崩（首轮实测即为此，故域侧只许 `import type`）。
 */

/** 注册表条目的 `Page` 只被宿主放进 `<SafeLazyPage>`，测试不渲染 ⇒ 用一个不解析的 lazy 占位。 */
function probePage(name: string): LazyExoticComponent<ComponentType> {
  return lazy(() => new Promise<{ default: ComponentType }>(() => void name));
}

describe("domainPageRegistry", () => {
  it("glob 装载口求值了域路由模块（命中为空 ⇒ 域目录改名，不是「没有域页面」）", () => {
    const sources = loadedDomainRouteSources();
    expect(sources.filter((s) => s.includes("pages/opc/routes"))).toHaveLength(1);
  });

  it("OPC 贡献 17 条路由：主页两段 + 需求发现 + 14 个能力包页", () => {
    expect(getDomainPageRoutes()).toHaveLength(17);
    expect(getDomainPageRoutes().filter((r) => r.id.startsWith("domain:"))).toHaveLength(14);
  });

  it("每条贡献路由的 path 都取自宿主 BUILTIN_PAGE_PATH（域侧不得自写路径）", () => {
    const byId = new Map(getDomainPageRoutes().map((r) => [r.id, r]));
    expect(byId.get("opc:index")?.path).toBe(BUILTIN_PAGE_PATH["automation-operations"]);
    expect(byId.get("opc:tab")?.path).toBe(`${BUILTIN_PAGE_PATH["automation-operations"]}/:tab`);
    expect(byId.get("demand-discovery")?.path).toBe(BUILTIN_PAGE_PATH["demand-discovery"]);
    // 能力包页逐条回查：id 必须是 `domain:<pageKey>`，path 必须等于宿主表里该 key 的值。
    for (const route of byId.values()) {
      if (!route.id.startsWith("domain:")) { continue; }
      expect(route.id).toBe(`domain:${route.pageKey}`);
      expect(route.path).toBe(BUILTIN_PAGE_PATH[route.pageKey]);
    }
    expect(byId.get("domain:finance-analysis")?.pageKey).toBe("finance-analysis");
    expect(byId.get("domain:ai-media-game")?.pageKey).toBe("ai-media-game");
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
