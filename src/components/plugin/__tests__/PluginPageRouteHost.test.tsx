// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";

const h = vi.hoisted(() => ({
  routes: [] as Array<{
    pluginSlug: string;
    pageId: string;
    pluginId: string;
    pluginName: string;
    page: {
      id: string;
      title: string;
      entry: string;
      loading: "bundle";
    };
  }>,
}));

vi.mock("@/lib/pluginRouteRegistry", () => ({
  usePluginPageRoutes: () => h.routes,
}));

vi.mock("@/components/plugin/PluginPageHost", () => ({
  PluginPageHost: ({ pluginId }: { pluginId: string }) => <div data-testid="page-host" data-plugin-id={pluginId} />,
}));

vi.mock("react-router-dom", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react-router-dom")>();
  return { ...actual, useParams: () => ({ pluginSlug: "security", pageId: "overview" }) };
});

import { PluginPageRouteHost } from "@/components/plugin/PluginPageRouteHost";

describe("PluginPageRouteHost — 路由命中与终态", () => {
  beforeEach(() => {
    h.routes = [];
  });

  it("路由已从插件投影移除时显示明确 404，而不是永久 loading", () => {
    render(<PluginPageRouteHost />);

    expect(screen.getByText("404")).toBeTruthy();
    expect(screen.getByText(i18n.t("error.pageNotFound"))).toBeTruthy();
    expect(screen.queryByText(i18n.t("common.loading"))).toBeNull();
  });

  it("命中动态插件页时把宿主绑定的 pluginId 交给页面容器", () => {
    h.routes = [{
      pluginSlug: "security",
      pageId: "overview",
      pluginId: "plugin-opc-security",
      pluginName: "Security",
      page: {
        id: "overview",
        title: "Overview",
        entry: "pages/index.html",
        loading: "bundle",
      },
    }];
    render(<PluginPageRouteHost />);

    expect(screen.getByTestId("page-host").getAttribute("data-plugin-id")).toBe("plugin-opc-security");
  });
});
