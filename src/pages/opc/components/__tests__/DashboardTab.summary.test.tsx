// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DashboardTab } from "../DashboardTab";

const invokePluginCommandMock = vi.fn();
const invokeMock = vi.fn();

vi.mock("react-i18next", () => ({
  initReactI18next: {
    type: "3rdParty",
    init: vi.fn(),
  },
  useTranslation: () => ({
    t: (key: string) => key,
  }),
}));

vi.mock("@/hooks/useIsDarkTheme", () => ({
  useIsDarkTheme: () => false,
}));

vi.mock("@/lib/pluginCommand", () => ({
  invokePluginCommand: (...args: unknown[]) => invokePluginCommandMock(...args),
}));

vi.mock("@/lib/pluginCommandResolve", () => ({
  resolvePluginForCommand: async () => "opc@bundled",
}));

vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("@/lib/echartsInstance", () => ({
  ensureECharts: () => ({ setOption: vi.fn(), resize: vi.fn(), dispose: vi.fn() }),
}));

const SUMMARY = {
  totalRevenue: 12345.68,
  totalInvoices: 7,
  activeProjects: 3,
  totalCustomers: 5,
  recentKpis: [
    {
      id: "kpi-1",
      domainPackId: "",
      name: "月度收入",
      value: 1,
      unit: "元",
      period: "2026-11",
      recordedAt: 1_700_000_000_000,
    },
  ],
  revenueTrend: [
    {
      id: "rev-1",
      amount: 12345.68,
      currency: "CNY",
      category: "sales",
      description: "",
      recordedAt: 1_700_000_000_000,
    },
  ],
};

describe("DashboardTab 看板总览（#267 迁插件通道）", () => {
  beforeEach(() => {
    invokePluginCommandMock.mockReset();
    invokeMock.mockReset();
    invokePluginCommandMock.mockResolvedValue(SUMMARY);
    // 其余四条只读插件命令（内容资产 / 发布计划 / 博客 / 落地页）走
    // `plugin_execute_command` 并 JSON.parse 回帧文本 ⇒ 空数组是合法回帧。
    invokeMock.mockResolvedValue("[]");
  });

  it("走插件命令 opc_get_dashboard_summary（零入参），按 camelCase 字段渲染", async () => {
    render(<DashboardTab />);

    await waitFor(() => expect(invokePluginCommandMock).toHaveBeenCalled());
    expect(invokePluginCommandMock.mock.calls[0]?.[0]).toBe("opc_get_dashboard_summary");
    expect(invokePluginCommandMock.mock.calls[0]?.[1]).toEqual({});

    expect(await screen.findByText("月度收入")).toBeTruthy();
    // antd Statistic 把整数与小数拆成两个 span（`12,345` + `.68`），断言整数段。
    expect(screen.getByText("12,345")).toBeTruthy();
  });

  it("负控：取数失败时落失败空态，不会渲染任何金额", async () => {
    invokePluginCommandMock.mockRejectedValue(new Error("seam down"));

    render(<DashboardTab />);

    expect(await screen.findByText("opc.dashboard.loadFailed")).toBeTruthy();
    expect(screen.queryByText("12,345")).toBeNull();
  });
});
