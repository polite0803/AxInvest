// SPDX-License-Identifier: AGPL-3.0-only

import { fireEvent, render, screen } from "@testing-library/react";
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

vi.mock("@/hooks/usePluginGallery", () => ({
  usePluginGallery: () => ({
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
    registry: [],
    registryLoading: false,
    loadRegistry: vi.fn(),
  }),
}));

import { PluginGallery } from "../PluginGallery";

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
    source: "builtin",
    permissions: [],
    pages: [],
    integrityStatus: "unknown",
    pinnedHash: null,
    commands: [],
    eventStats: null,
    ...overrides,
  };
}

describe("PluginGallery 的 eventStats 渲染", () => {
  it("eventStats 为 null 时显示「未订阅」，不显示成 0 丢弃", () => {
    pluginsRef.current = [plugin({ eventStats: null })];
    const { container } = render(<PluginGallery />);
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
    const { container } = render(<PluginGallery />);
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
    const { container } = render(<PluginGallery />);
    // 若前端把 unavailable 再加一遍，这里会变成 4。
    expect(container.textContent).toContain("chat.plugins.marketplace.eventStats|0,2");
    expect(container.textContent).not.toContain("eventStats|0,4");
  });

  it("列表展示来源、权限、页面与完整性状态，并支持按这些字段筛选", () => {
    pluginsRef.current = [
      plugin({
        id: "p1",
        name: "alpha",
        source: "./alpha-plugin",
        permissions: ["read:files"],
        pages: ["overview"],
        integrityStatus: "verified",
      }),
      plugin({
        id: "p2",
        name: "beta",
        source: "./beta-plugin",
        permissions: ["write:files"],
        pages: ["settings"],
        integrityStatus: "mismatch",
      }),
    ];
    const { container } = render(<PluginGallery />);
    expect(container.textContent).toContain("chat.plugins.marketplace.integrityVerified");
    expect(container.textContent).toContain("chat.plugins.marketplace.integrityMismatch");

    fireEvent.change(
      screen.getByPlaceholderText("chat.plugins.marketplace.listSearchPlaceholder"),
      { target: { value: "read:files" } },
    );
    expect(container.textContent).toContain("alpha");
    expect(container.textContent).not.toContain("./beta-plugin");

    fireEvent.change(
      screen.getByPlaceholderText("chat.plugins.marketplace.listSearchPlaceholder"),
      { target: { value: "settings" } },
    );
    expect(container.textContent).toContain("./beta-plugin");
    expect(container.textContent).not.toContain("./alpha-plugin");
  });

  it("筛选无结果时显示空态", () => {
    pluginsRef.current = [plugin({})];
    const { container } = render(<PluginGallery />);
    fireEvent.change(
      screen.getByPlaceholderText("chat.plugins.marketplace.listSearchPlaceholder"),
      { target: { value: "不存在" } },
    );
    expect(container.textContent).toContain("chat.plugins.marketplace.listEmpty");
  });

  it("按来源类型、启用状态与完整性状态分面筛选", () => {
    pluginsRef.current = [
      plugin({
        id: "release-on",
        name: "release-on",
        source: "axagent-release+https://example.com/release.json",
        enabled: true,
        integrityStatus: "verified",
      }),
      plugin({
        id: "legacy-off",
        name: "legacy-off",
        source: "./local-plugin",
        enabled: false,
        integrityStatus: "unpinned",
      }),
    ];
    const { container } = render(<PluginGallery />);

    fireEvent.mouseDown(screen.getByLabelText("chat.plugins.marketplace.sourceTypeFilter"));
    fireEvent.click(screen.getByText("chat.plugins.marketplace.sourceTypeRelease"));
    expect(container.textContent).toContain("release-on");
    expect(container.textContent).not.toContain("legacy-off");

    // 来源筛选保持“发布来源”时，禁用分面筛不出已启用的插件。
    fireEvent.mouseDown(screen.getByLabelText("chat.plugins.marketplace.enabledFilter"));
    fireEvent.click(screen.getByText("chat.plugins.marketplace.disabledOnly"));
    expect(container.textContent).toContain("chat.plugins.marketplace.listEmpty");

    // 切回“全部来源”后，禁用 + 未固定哈希只剩旧版安装记录。
    fireEvent.mouseDown(screen.getByLabelText("chat.plugins.marketplace.sourceTypeFilter"));
    fireEvent.click(screen.getByText("chat.plugins.marketplace.sourceTypeAll"));
    expect(container.textContent).toContain("legacy-off");
    expect(container.textContent).not.toContain("release-on");

    fireEvent.mouseDown(screen.getByLabelText("chat.plugins.marketplace.integrityFilter"));
    fireEvent.click(screen.getByText("chat.plugins.marketplace.integrityVerified"));
    expect(container.textContent).toContain("chat.plugins.marketplace.listEmpty");
  });
});
