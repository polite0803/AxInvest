// SPDX-License-Identifier: AGPL-3.0-only

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { mocks } = vi.hoisted(() => ({
  mocks: {
    invoke: vi.fn(),
    installPlugin: vi.fn(),
    installError: vi.fn(),
    open: vi.fn(),
    refreshPluginPageRoutes: vi.fn(),
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
    installPlugin: mocks.installPlugin,
    error: mocks.installError(),
  }),
}));

vi.mock("@/lib/invoke", () => ({
  invoke: mocks.invoke,
}));

vi.mock("@/lib/pluginRouteRegistry", () => ({
  refreshPluginPageRoutes: mocks.refreshPluginPageRoutes,
}));

vi.mock("@/lib/toast", () => ({
  message: {
    error: mocks.error,
    success: mocks.success,
  },
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: mocks.open,
}));

import { DashboardPluginsSettings } from "../DashboardPluginsSettings";

const LOCAL_SOURCE = "/tmp/local-plugin";
const installOutcome = {
  pluginId: "local-plugin",
  version: "1.0.0",
  installPath: "/tmp/installed/local-plugin",
};

describe("DashboardPluginsSettings 安装与刷新", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.installPlugin.mockReset();
    mocks.installError.mockReset();
    mocks.open.mockReset();
    mocks.refreshPluginPageRoutes.mockReset();
    mocks.error.mockReset();
    mocks.success.mockReset();
    mocks.invoke.mockResolvedValue([]);
    mocks.installPlugin.mockResolvedValue(installOutcome);
    mocks.installError.mockReturnValue(null);
    mocks.open.mockResolvedValue(LOCAL_SOURCE);
    mocks.refreshPluginPageRoutes.mockResolvedValue([]);
  });

  it("本地目录安装统一走 pluginStore，不绕过宿主安装命令与状态链", async () => {
    render(<DashboardPluginsSettings />);

    fireEvent.click(await screen.findByRole("button", { name: "settings.dashboardPlugins.install" }));

    await waitFor(() => expect(mocks.installPlugin).toHaveBeenCalledWith(LOCAL_SOURCE));
    expect(mocks.invoke).not.toHaveBeenCalledWith("plugin_install", expect.anything());
    expect(mocks.refreshPluginPageRoutes).toHaveBeenCalledTimes(1);
    expect(mocks.success).toHaveBeenCalledWith("settings.dashboardPlugins.enabled");
  });

  it("安装失败时不刷新插件页面路由、不误报启用成功", async () => {
    mocks.installError.mockReturnValue("integrity failed");
    mocks.installPlugin.mockResolvedValue(null);
    render(<DashboardPluginsSettings />);

    fireEvent.click(await screen.findByRole("button", { name: "settings.dashboardPlugins.install" }));

    await waitFor(() => expect(mocks.installPlugin).toHaveBeenCalledWith(LOCAL_SOURCE));
    expect(mocks.refreshPluginPageRoutes).not.toHaveBeenCalled();
    expect(mocks.success).not.toHaveBeenCalledWith("settings.dashboardPlugins.enabled");
    expect(await screen.findByRole("alert")).toHaveTextContent("settings.plugins.installFailed");
  });

  it("取消目录选择时既不安装也不刷新", async () => {
    mocks.open.mockResolvedValue(null);
    render(<DashboardPluginsSettings />);

    fireEvent.click(await screen.findByRole("button", { name: "settings.dashboardPlugins.install" }));

    await waitFor(() => expect(mocks.open).toHaveBeenCalledTimes(1));
    expect(mocks.installPlugin).not.toHaveBeenCalled();
    expect(mocks.refreshPluginPageRoutes).not.toHaveBeenCalled();
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
