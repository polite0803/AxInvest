// SPDX-License-Identifier: AGPL-3.0-only

/** `/invest` 兼容跳转桥的三态：插件页可达 → 跳转；插件不可用 → 降级；投影挂起 → 超时降级。 */

import i18n from "@/i18n";
import { render, screen, waitFor } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { MemoryRouter, useLocation } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

let routes: Array<Record<string, string>> = [];
let ready = true;

vi.mock("@/lib/pluginRouteRegistry", () => ({
  usePluginPageRoutes: () => routes,
  usePluginPageRoutesReady: () => ready,
}));

import { LegacyInvestRedirect } from "../LegacyInvestRedirect";

function LocationProbe() {
  const location = useLocation();
  return <div data-testid="bridge-location">{location.pathname}</div>;
}

function renderBridge() {
  return render(
    <MemoryRouter initialEntries={["/invest?tab=workspace&stockCode=600519"]}>
      <I18nextProvider i18n={i18n}>
        <LegacyInvestRedirect />
        <LocationProbe />
      </I18nextProvider>
    </MemoryRouter>,
  );
}

beforeEach(() => {
  routes = [];
  ready = true;
  vi.useRealTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("LegacyInvestRedirect", () => {
  it("stock 插件页可用时跳转到插件路由，不渲染任何宿主股票页", () => {
    routes = [{
      pluginId: "stock@bundled",
      pluginName: "stock",
      pluginSlug: "stock-bundled",
      pageId: "stock-home",
      path: "/plugins/stock-bundled/stock-home",
    }];
    renderBridge();
    expect(screen.getByTestId("bridge-location").textContent).toBe("/plugins/stock-bundled/stock-home");
  });

  it("stock 未启用时显示明确的插件不可用降级，不跳到不存在的插件路由", () => {
    renderBridge();
    expect(screen.getByTestId("bridge-location").textContent).toBe("/invest");
    expect(screen.getByText(i18n.t("plugin.pages.empty"))).toBeTruthy();
  });

  it("插件投影超时后退出加载态并显示中文失败提示", async () => {
    ready = false;
    renderBridge();
    expect(screen.getByText(i18n.t("common.loading"))).toBeTruthy();
    // 真实计时器：3 秒兜底；waitFor 默认 1 秒不够，这里显式放宽。
    await waitFor(() => {
      expect(screen.getByText(i18n.t("error.loadFailed"))).toBeTruthy();
    }, { timeout: 5000 });
  });
});
