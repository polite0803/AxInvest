// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it } from "vitest";

import type { DashboardPluginInfoDto } from "@/types";
import { onBrowserEvent } from "../browserEvents";
import { handleCommand } from "../browserMock";

const SOURCE = "axagent-release+https://example.com/release.json";

async function previewAndInstall(): Promise<void> {
  const preview = await handleCommand<{ previewToken: string }>("plugin_validate_source", {
    source: SOURCE,
  });
  await handleCommand("plugin_install", {
    source: SOURCE,
    previewToken: preview.previewToken,
  });
}

describe("browserMock 插件状态广播", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("安装/启停/卸载都广播 plugin-state-changed", async () => {
    const events: Array<{ pluginId: string; state: string }> = [];
    const off = onBrowserEvent("plugin-state-changed", (payload) => {
      events.push(payload as { pluginId: string; state: string });
    });
    try {
      await previewAndInstall();
      await handleCommand("plugin_disable", { pluginId: "plugin-1" });
      await handleCommand("plugin_enable", { pluginId: "plugin-1" });
      await handleCommand("plugin_uninstall", { pluginId: "plugin-1" });
    } finally {
      off();
    }
    expect(events).toEqual([
      { pluginId: "plugin-1", state: "installed" },
      { pluginId: "plugin-1", state: "disabled" },
      { pluginId: "plugin-1", state: "enabled" },
      { pluginId: "plugin-1", state: "uninstalled" },
    ]);
  });

  it("dashboard_list_plugins 与 plugin_list 读同一份 mock store", async () => {
    await previewAndInstall();
    const dashboard = await handleCommand<DashboardPluginInfoDto[]>("dashboard_list_plugins", {});
    expect(dashboard).toMatchObject([{ id: "plugin-1", enabled: false }]);
    await handleCommand("plugin_enable", { pluginId: "plugin-1" });
    const afterEnable = await handleCommand<DashboardPluginInfoDto[]>("dashboard_list_plugins", {});
    expect(afterEnable[0]?.enabled).toBe(true);
    await handleCommand("plugin_disable", { pluginId: "plugin-1" });
    const afterDisable = await handleCommand<DashboardPluginInfoDto[]>("dashboard_list_plugins", {});
    expect(afterDisable[0]?.enabled).toBe(false);
  });
});
