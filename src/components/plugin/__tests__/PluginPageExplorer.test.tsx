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
const h = vi.hoisted(() => ({ invoke: vi.fn(), isTauri: true }));
vi.mock("@/lib/invoke", () => ({
  invoke: h.invoke,
  isTauri: () => h.isTauri,
  logIpcError: () => () => {},
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
});
