// SPDX-License-Identifier: AGPL-3.0-only

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { mocks } = vi.hoisted(() => ({
  mocks: {
    invoke: vi.fn(),
    validateSource: vi.fn(),
    installPlugin: vi.fn(),
    listen: vi.fn(() => Promise.resolve(() => {})),
    error: vi.fn(),
    success: vi.fn(),
    translate: (key: string) => key,
  },
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: mocks.translate }),
}));

vi.mock("@/hooks/usePluginGallery", () => ({
  usePluginGallery: () => ({
    validateSource: mocks.validateSource,
    installPlugin: mocks.installPlugin,
    error: null,
  }),
}));

vi.mock("@/lib/invoke", () => ({
  invoke: mocks.invoke,
  listen: mocks.listen,
}));

vi.mock("@/lib/toast", () => ({
  message: {
    error: mocks.error,
    success: mocks.success,
  },
}));

import { DashboardPluginsSettings } from "../DashboardPluginsSettings";

const SOURCE = "https://example.com/release.json";
const NORMALIZED = `axagent-release+${SOURCE}`;
const TOKEN = "signed-preview-token";
const installOutcome = {
  pluginId: "remote-plugin",
  version: "1.0.0",
  installPath: "/mock/installed/remote-plugin",
};

describe("DashboardPluginsSettings 远程安装与刷新", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.validateSource.mockReset();
    mocks.installPlugin.mockReset();
    mocks.listen.mockClear();
    mocks.error.mockReset();
    mocks.success.mockReset();
    mocks.invoke.mockResolvedValue([]);
    mocks.validateSource.mockResolvedValue({ previewToken: TOKEN });
    mocks.installPlugin.mockResolvedValue(installOutcome);
  });

  it("只接受发布地址，预览后走宿主安装链并提示手动启用", async () => {
    render(<DashboardPluginsSettings />);
    await waitFor(() => expect(mocks.listen).toHaveBeenCalledWith("plugin-state-changed", expect.any(Function)));
    await screen.findByPlaceholderText("settings.dashboardPlugins.releasePlaceholder");

    fireEvent.change(
      screen.getByPlaceholderText("settings.dashboardPlugins.releasePlaceholder"),
      { target: { value: SOURCE } },
    );
    fireEvent.click(screen.getByRole("button", { name: "settings.dashboardPlugins.install" }));

    await waitFor(() => expect(mocks.validateSource).toHaveBeenCalledWith(NORMALIZED));
    await waitFor(() => expect(mocks.installPlugin).toHaveBeenCalledWith(NORMALIZED, TOKEN));
    expect(mocks.invoke).not.toHaveBeenCalledWith("plugin_install", expect.anything());
    expect(mocks.success).toHaveBeenCalledWith("settings.dashboardPlugins.installedNeedEnable");
  });

  it("非发布地址不安装", async () => {
    render(<DashboardPluginsSettings />);
    await screen.findByPlaceholderText("settings.dashboardPlugins.releasePlaceholder");
    fireEvent.change(
      screen.getByPlaceholderText("settings.dashboardPlugins.releasePlaceholder"),
      { target: { value: "/tmp/local-plugin" } },
    );
    const installButton = screen.getByRole("button", { name: "settings.dashboardPlugins.install" });
    expect(installButton.hasAttribute("disabled")).toBe(true);
    expect(mocks.error).not.toHaveBeenCalled();
    expect(mocks.validateSource).not.toHaveBeenCalled();
    expect(mocks.installPlugin).not.toHaveBeenCalled();
  });

  it("安装失败时不误报启用成功", async () => {
    mocks.installPlugin.mockResolvedValue(null);
    render(<DashboardPluginsSettings />);
    await screen.findByPlaceholderText("settings.dashboardPlugins.releasePlaceholder");
    fireEvent.change(
      screen.getByPlaceholderText("settings.dashboardPlugins.releasePlaceholder"),
      { target: { value: SOURCE } },
    );
    fireEvent.click(screen.getByRole("button", { name: "settings.dashboardPlugins.install" }));
    await waitFor(() => expect(mocks.installPlugin).toHaveBeenCalledWith(NORMALIZED, TOKEN));
    expect(mocks.success).not.toHaveBeenCalledWith("settings.dashboardPlugins.installedNeedEnable");
  });

  it("刷新失败时不显示刷新成功", async () => {
    mocks.invoke
      .mockResolvedValueOnce([])
      .mockRejectedValueOnce(new Error("list failed"));
    render(<DashboardPluginsSettings />);
    fireEvent.click(await screen.findByRole("button", { name: "settings.dashboardPlugins.refresh" }));
    await waitFor(() => expect(mocks.error).toHaveBeenCalledWith("settings.plugins.loadFailed"));
    expect(mocks.success).not.toHaveBeenCalledWith("settings.dashboardPlugins.refreshSuccess");
  });
});
