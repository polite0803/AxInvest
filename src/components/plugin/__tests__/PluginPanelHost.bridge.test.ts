// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 插件面板桥的通道约束
 *
 * 钉两件事：
 * 1. 只有 `plugin_ui_action` / `plugin_execute_command` 可走，其余一律拒绝且**不发 IPC**；
 * 2. `pluginId` 由宿主绑定 —— iframe 里传来的值被丢弃，否则一个面板可以驱动
 *    另一个插件的 worker（跨插件越权）。
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

import { invoke } from "@/lib/invoke";

import { createPanelHostApi, PANEL_ALLOWED_COMMANDS } from "../PluginPanelHost";

vi.mock(import("@/lib/invoke"), async (importOriginal) => {
  const actual = await importOriginal();
  return { ...actual, invoke: vi.fn() };
});

const deny = (command: string) => `denied:${command}`;

describe("createPanelHostApi", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
    vi.mocked(invoke).mockResolvedValue("ok");
  });

  it("通道清单就是这两条，多一条都要回来改这里", () => {
    expect([...PANEL_ALLOWED_COMMANDS].sort()).toEqual(
      ["plugin_execute_command", "plugin_ui_action"].sort(),
    );
  });

  it("白名单外的命令直接拒绝，且不发出 IPC", async () => {
    const api = createPanelHostApi("own-pack", deny);
    await expect(api.invoke("get_dashboard_stats", {})).rejects.toThrow("denied:get_dashboard_stats");
    expect(invoke).not.toHaveBeenCalled();
  });

  it("丢弃 iframe 传来的 pluginId，强制绑定本面板所属插件", async () => {
    const api = createPanelHostApi("own-pack", deny);
    await api.invoke("plugin_ui_action", { pluginId: "victim-pack", action: { type: "ping" } });

    expect(invoke).toHaveBeenCalledTimes(1);
    const [command, args] = vi.mocked(invoke).mock.calls[0];
    expect(command).toBe("plugin_ui_action");
    expect(args).toEqual({ pluginId: "own-pack", action: { type: "ping" } });
  });

  it("execute_command 同样绑定 pluginId，其余参数原样透传", async () => {
    const api = createPanelHostApi("own-pack", deny);
    await api.invoke("plugin_execute_command", {
      pluginId: "victim-pack",
      commandName: "greet",
      input: { a: 1 },
    });

    const [, args] = vi.mocked(invoke).mock.calls[0];
    expect(args).toEqual({ commandName: "greet", input: { a: 1 }, pluginId: "own-pack" });
  });
});
