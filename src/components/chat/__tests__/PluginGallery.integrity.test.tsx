// SPDX-License-Identifier: AGPL-3.0-only

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { mocks } = vi.hoisted(() => ({
  mocks: {
    validateSource: vi.fn(),
    installPlugin: vi.fn(),
    updatePlugin: vi.fn(),
  },
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) => opts ? `${key}|${JSON.stringify(opts)}` : key,
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

const { galleryState } = vi.hoisted(() => ({
  galleryState: { plugins: [] as Array<Record<string, unknown>> },
}));

vi.mock("@/hooks/usePluginGallery", () => ({
  usePluginGallery: () => ({
    plugins: galleryState.plugins,
    loading: false,
    installing: null,
    validating: false,
    loadPlugins: vi.fn(),
    validateSource: mocks.validateSource,
    installPlugin: mocks.installPlugin,
    updatePlugin: mocks.updatePlugin,
    enablePlugin: vi.fn(),
    disablePlugin: vi.fn(),
    uninstallPlugin: vi.fn(),
    error: null,
    registry: [],
    registryLoading: false,
    loadRegistry: vi.fn(),
  }),
}));

vi.mock("@/lib/toast", () => ({
  message: { error: vi.fn(), success: vi.fn(), info: vi.fn() },
}));

vi.mock("@/lib/invoke", () => ({
  isTauri: () => true,
  invoke: vi.fn(),
}));

import { PluginGallery } from "../PluginGallery";

const SOURCE = "https://example.com/release.json";
const NORMALIZED = `axagent-release+${SOURCE}`;
const TOKEN = "signed-preview-token";

function manifest() {
  return {
    name: "demo",
    version: "1.0.0",
    description: "demo plugin",
    permissions: [],
    pages: [],
    defaultEnabled: true,
    hooks: {},
    tools: [],
    mcpServers: [],
    skills: [],
    capabilities: [],
    previewToken: TOKEN,
  };
}

function findButton(name: string): HTMLElement | undefined {
  return screen.getAllByRole("button").find((button) => button.textContent === name);
}

describe("PluginGallery 远程-only 安装", () => {
  beforeEach(() => {
    galleryState.plugins = [];
    mocks.validateSource.mockReset();
    mocks.installPlugin.mockReset();
    mocks.updatePlugin.mockReset();
    mocks.validateSource.mockResolvedValue(manifest());
    mocks.installPlugin.mockResolvedValue({
      pluginId: "demo@external",
      version: "1.0.0",
      installPath: "/mock/demo",
    });
  });

  it("非发布地址不能发起验证", async () => {
    render(<PluginGallery />);
    fireEvent.change(
      screen.getByPlaceholderText("chat.plugins.marketplace.releasePlaceholder"),
      { target: { value: "https://example.com/plugin.git" } },
    );
    expect(findButton("chat.plugins.marketplace.install")?.hasAttribute("disabled")).toBe(true);
    await waitFor(() => expect(mocks.validateSource).not.toHaveBeenCalled());
  });

  it("粘贴发布地址后预览，确认安装只回传内部 token", async () => {
    render(<PluginGallery />);
    fireEvent.change(
      screen.getByPlaceholderText("chat.plugins.marketplace.releasePlaceholder"),
      { target: { value: SOURCE } },
    );
    fireEvent.click(findButton("chat.plugins.marketplace.install")!);

    await waitFor(() => expect(mocks.validateSource).toHaveBeenCalledWith(NORMALIZED));
    expect(await screen.findByText("chat.plugins.marketplace.installSource")).toBeInTheDocument();
    expect(mocks.installPlugin).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "chat.plugins.marketplace.confirmInstall" }));
    await waitFor(() => expect(mocks.installPlugin).toHaveBeenCalledWith(NORMALIZED, TOKEN));
  });
});

describe("PluginGallery 远程-only 更新", () => {
  const installedPlugin = {
    id: "demo@external",
    name: "demo",
    version: "1.0.0",
    description: "demo plugin",
    kind: "external",
    enabled: false,
    tools: [],
    mcpServers: [],
    skills: [],
    source: NORMALIZED,
    permissions: [],
    pages: [],
    integrityStatus: "unknown",
    pinnedHash: null,
    commands: [],
    eventStats: null,
  };

  beforeEach(() => {
    galleryState.plugins = [installedPlugin];
    mocks.validateSource.mockReset();
    mocks.installPlugin.mockReset();
    mocks.updatePlugin.mockReset();
    mocks.validateSource.mockResolvedValue(manifest());
    mocks.updatePlugin.mockResolvedValue({
      pluginId: installedPlugin.id,
      oldVersion: "1.0.0",
      newVersion: "1.1.0",
      installPath: "/mock/demo",
    });
  });

  it("更新先预览再确认，只回传内部 token", async () => {
    render(<PluginGallery />);
    fireEvent.click(await screen.findByRole("button", { name: "chat.plugins.marketplace.update" }));
    const validateButton = await waitFor(() => {
      const button = findButton("chat.plugins.marketplace.validate");
      expect(button).toBeDefined();
      return button!;
    });
    expect(validateButton.hasAttribute("disabled")).toBe(false);
    fireEvent.click(validateButton);
    await waitFor(() => expect(mocks.validateSource).toHaveBeenCalledWith(NORMALIZED));
    const confirmButton = await waitFor(() => {
      const button = findButton("chat.plugins.marketplace.confirmUpdate");
      expect(button && !button.hasAttribute("disabled")).toBe(true);
      return button!;
    });
    fireEvent.click(confirmButton);
    await waitFor(() => expect(mocks.updatePlugin).toHaveBeenCalledWith(installedPlugin.id, TOKEN));
  });
});
