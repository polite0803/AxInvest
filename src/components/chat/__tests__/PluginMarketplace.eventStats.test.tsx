// SPDX-License-Identifier: AGPL-3.0-only

import { render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

const { pluginsRef } = vi.hoisted(() => ({ pluginsRef: { current: [] as unknown[] } }));

vi.mock("react-i18next", () => ({
  // 只回键名 + 参数值，断言时能同时看出「用了哪个键」和「传了什么数」。
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) => opts ? `${key}|${Object.values(opts).join(",")}` : key,
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

vi.mock("@/lib/invoke", () => ({
  invoke: vi.fn().mockResolvedValue([]),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

vi.mock("@/lib/toast", () => ({
  message: { error: vi.fn(), success: vi.fn(), info: vi.fn() },
}));

vi.mock("@/stores", () => ({
  usePluginStore: (sel: (s: Record<string, unknown>) => unknown) =>
    sel({
      plugins: pluginsRef.current,
      loading: false,
      installing: null,
      validating: false,
      loadPlugins: vi.fn(),
      validateSource: vi.fn(),
      installPlugin: vi.fn(),
      enablePlugin: vi.fn(),
      disablePlugin: vi.fn(),
      uninstallPlugin: vi.fn(),
    }),
  useCapabilityStore: (sel: (s: Record<string, unknown>) => unknown) =>
    sel({ registry: [], isLoading: false, loadRegistry: vi.fn() }),
}));

import { PluginMarketplace } from "../PluginMarketplace";

function plugin(overrides: Record<string, unknown>) {
  return {
    id: "p1",
    name: "demo",
    version: "1.0.0",
    description: "",
    kind: "external",
    enabled: true,
    tools: [],
    mcpServers: [],
    skills: [],
    commands: [],
    eventStats: null,
    ...overrides,
  };
}

describe("PluginMarketplace 的 eventStats 渲染", () => {
  it("eventStats 为 null 时显示「未订阅」，不显示成 0 丢弃", () => {
    pluginsRef.current = [plugin({ eventStats: null })];
    const { container } = render(<PluginMarketplace />);
    expect(container.textContent).toContain("chat.plugins.marketplace.eventStatsNone");
    expect(container.textContent).not.toContain("chat.plugins.marketplace.eventStats|");
  });

  it("有订阅时把 delivered / droppedTotal 原样透出", () => {
    pluginsRef.current = [
      plugin({
        eventStats: {
          delivered: 7,
          droppedTotal: 3,
          droppedSerialize: 1,
          droppedInvoke: 2,
          droppedUnavailable: 1,
          droppedWait: 0,
        },
      }),
    ];
    const { container } = render(<PluginMarketplace />);
    expect(container.textContent).toContain("chat.plugins.marketplace.eventStats|7,3");
  });

  it("droppedUnavailable 是 droppedInvoke 的子集，不算进显示的总数", () => {
    pluginsRef.current = [
      plugin({
        eventStats: {
          delivered: 0,
          droppedTotal: 2,
          droppedSerialize: 0,
          droppedInvoke: 2,
          droppedUnavailable: 2,
          droppedWait: 0,
        },
      }),
    ];
    const { container } = render(<PluginMarketplace />);
    // 若前端把 unavailable 再加一遍，这里会变成 4。
    expect(container.textContent).toContain("chat.plugins.marketplace.eventStats|0,2");
    expect(container.textContent).not.toContain("eventStats|0,4");
  });
});
