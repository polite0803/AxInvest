// SPDX-License-Identifier: AGPL-3.0-only

/**
 * PluginPageExplorer（插件页 Tab 容器）的最小行为用例：
 *   - 清单为空 ⇒ 显式空态，不白屏；
 *   - 清单有页 + 真机 ⇒ 渲染 PluginPageHost（pluginId 取顶层 id）；
 *   - 浏览器模式 ⇒ 显式说明态，不把页交给宿主容器（asset/schema 两路在浏览器都不成立）。
 *
 * 与 PluginPageHost.test.tsx 分工：那个测 page 粒度容器内部（iframe/桥/超时），
 * 本文件只测「发现与列举层」的分支，故把 PluginPageHost mock 成一个探针。
 */
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";

// invoke 与 isTauri 都从 @/lib/invoke 来，一起 mock
const h = vi.hoisted(() => ({
  invoke: vi.fn(),
  isTauri: true,
  routeListeners: new Set<() => void>(),
}));
vi.mock("@/lib/invoke", () => ({
  invoke: h.invoke,
  isTauri: () => h.isTauri,
  logIpcError: () => () => {},
}));

vi.mock("@/lib/pluginRouteRegistry", () => ({
  subscribePluginPageRoutes: (listener: () => void) => {
    h.routeListeners.add(listener);
    return () => h.routeListeners.delete(listener);
  },
}));

// PluginPageHost 换成探针：只断言它被选中页的参数正确调用，不跑它的 iframe/桥逻辑
vi.mock("@/components/plugin/PluginPageHost", () => ({
  PluginPageHost: ({ pluginId, pluginName, page }: {
    pluginId: string;
    pluginName?: string;
    page: { id: string; title: string };
  }) => (
    <div
      data-testid="page-host"
      data-plugin-id={pluginId}
      data-plugin-name={pluginName}
      data-page-id={page.id}
      data-page-title={page.title}
    />
  ),
}));

import { PluginPageExplorer } from "@/components/plugin/PluginPageExplorer";

const enabledPlugin = {
  id: "own-pack",
  name: "Own Pack",
  version: "1.0.0",
  description: "desc",
  author: null,
  enabled: true,
  pages: [{ id: "home", title: "行情首页", entry: "dist/pages/main.js", loading: "bundle" as const }],
};
const disabledPlugin = { ...enabledPlugin, id: "off-pack", name: "Off Pack", enabled: false };

describe("PluginPageExplorer — 清单与渲染", () => {
  beforeEach(() => {
    h.invoke.mockReset();
    h.isTauri = true;
    h.routeListeners.clear();
  });

  it("清单为空 ⇒ 显示显式空态，不白屏", async () => {
    h.invoke.mockResolvedValue([]);
    render(<PluginPageExplorer />);

    expect(await screen.findByText(i18n.t("plugin.pages.empty"))).toBeTruthy();
    expect(screen.queryByTestId("page-host")).toBeNull();
  });

  it("禁用插件的页不列入（enabled 过滤）", async () => {
    h.invoke.mockResolvedValue([disabledPlugin]);
    render(<PluginPageExplorer />);

    expect(await screen.findByText(i18n.t("plugin.pages.empty"))).toBeTruthy();
    expect(screen.queryByTestId("page-host")).toBeNull();
  });

  it("清单有页 + 真机 ⇒ 默认渲染第一页给 PluginPageHost，pluginId 取顶层 id", async () => {
    h.invoke.mockResolvedValue([enabledPlugin]);
    render(<PluginPageExplorer />);

    const host = await screen.findByTestId("page-host");
    expect(host.getAttribute("data-plugin-id")).toBe("own-pack");
    expect(host.getAttribute("data-plugin-name")).toBe("Own Pack");
    expect(host.getAttribute("data-page-id")).toBe("home");
    expect(h.invoke).toHaveBeenCalledWith("plugin_list_pages");
  });

  it("浏览器模式 ⇒ 显示说明态，不把页交给宿主容器（不白屏也不报错）", async () => {
    h.isTauri = false;
    h.invoke.mockResolvedValue([enabledPlugin]);
    render(<PluginPageExplorer />);

    expect(await screen.findByText(i18n.t("plugin.pages.browserNotice"))).toBeTruthy();
    await waitFor(() => {
      expect(screen.queryByTestId("page-host")).toBeNull();
    });
  });

  it("运行期禁用插件后，刷新通知让已消失的页从列表移除", async () => {
    h.invoke.mockResolvedValueOnce([enabledPlugin]).mockResolvedValueOnce([]);
    render(<PluginPageExplorer />);

    expect(await screen.findByTestId("page-host")).toBeTruthy();
    for (const listener of h.routeListeners) { listener(); }

    await waitFor(() => {
      expect(screen.queryByTestId("page-host")).toBeNull();
    });
    expect(await screen.findByText(i18n.t("plugin.pages.empty"))).toBeTruthy();
    expect(h.invoke).toHaveBeenCalledTimes(2);
  });

  it("首次加载失败后，启停刷新会重试并保留可观察错误态", async () => {
    h.invoke
      .mockRejectedValueOnce(new Error("backend down"))
      .mockResolvedValueOnce([enabledPlugin]);
    render(<PluginPageExplorer />);

    expect(await screen.findByText("backend down")).toBeTruthy();
    for (const listener of h.routeListeners) { listener(); }

    expect(await screen.findByTestId("page-host")).toBeTruthy();
    expect(h.invoke).toHaveBeenCalledTimes(2);
  });

  it("清单加载失败按后端错误码显示译文，不把 JSON 串暴露给用户", async () => {
    h.invoke.mockRejectedValue(
      new Error(JSON.stringify({
        code: "PLUGIN_UI_ACTION_FAILED",
        category: "unrecoverable",
        detail: "raw backend detail",
      })),
    );
    render(<PluginPageExplorer />);

    expect(await screen.findByText(i18n.t("error.PLUGIN_UI_ACTION_FAILED"))).toBeTruthy();
    expect(screen.queryByText(/PLUGIN_UI_ACTION_FAILED/)).toBeNull();
    expect(screen.queryByText(/"detail"/)).toBeNull();
  });
});
