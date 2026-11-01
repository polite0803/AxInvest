// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 插件沙箱桥的通道约束 —— panel 与 page 两种装载粒度共用同一份白名单。
 *
 * 任务 #39 验收条 3 要求钉住的三条不变量（逐条对应下面的 describe）：
 * ① **白名单命令集不扩大**，且页面与面板用的是同一份清单 —— 不是两份可以各自漂移的清单；
 * ② **`pluginId` 由宿主绑定**，iframe 传来的值一律丢弃；
 * ③ **非白名单命令当场拒绝且不发 IPC**（拒绝发生在 IPC 之前，不是"发出去再被后端拒"）。
 *
 * 「page 粒度」在本批没有容器组件可挂（缺 manifest 的逐页声明与后端的条目路径解析，见交付报告），
 * 但它调用桥的形态是确定的：同一个 `createPluginBridgeApi` + 同一个 `createPluginBridgePermissions`，
 * 只有拒绝文案不同（文案由调用方注入）。因此这里按**调用面**参数化覆盖两种粒度，
 * 等页面容器落地时，它用的白名单与 pluginId 绑定已经被这份测试钉住了，不会再长出一条平行通道。
 *
 * ⚠ 本文件**刻意不断言** `plugin_execute_command` 的业务参数形状 —— 它的 schema/权限语义正由
 * 另一会话改造，这里写死键名就会两边口径漂移。只断言「除 `pluginId` 外的键原样透传」。
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

import { invoke } from "@/lib/invoke";

import {
  type BridgeDeniedTranslator,
  createPluginBridgeApi,
  createPluginBridgePermissions,
  isPluginBridgeCommand,
  PLUGIN_BRIDGE_COMMANDS,
} from "../pluginSandboxBridge";
// 兼容别名走的是 panel 侧调用面；同时 import 两侧才能测出「是不是同一份」。
import { createPanelHostApi, PANEL_ALLOWED_COMMANDS } from "../PluginPanelHost";

vi.mock(import("@/lib/invoke"), async (importOriginal) => {
  const actual = await importOriginal();
  return { ...actual, invoke: vi.fn() };
});

/** panel 侧文案（与 `dashboard.pluginPanels.commandDenied` 同义，测试里不需要真实译文）。 */
const panelDeny: BridgeDeniedTranslator = (command) => `panel-denied:${command}`;
/** page 侧文案：粒度只影响措辞，不影响权限面 —— 这正是本批刻意不拆子集的理由。 */
const pageDeny: BridgeDeniedTranslator = (command) => `page-denied:${command}`;

/** 两种粒度对桥的调用面。除文案外必须完全等价，否则就是在造第二套通道。 */
const SURFACES = [
  { name: "panel", build: (pluginId: string) => createPanelHostApi(pluginId, panelDeny), denyPrefix: "panel-denied" },
  { name: "page", build: (pluginId: string) => createPluginBridgeApi(pluginId, pageDeny), denyPrefix: "page-denied" },
] as const;

describe("不变量①：白名单命令集不扩大，且两种粒度共用同一份", () => {
  it("通道清单就是这两条，多一条都要回来改这里", () => {
    expect([...PLUGIN_BRIDGE_COMMANDS].sort()).toEqual(
      ["plugin_execute_command", "plugin_ui_action"].sort(),
    );
  });

  it("panel 侧的旧名字与桥模块是同一个对象，不是第二份清单", () => {
    expect(PANEL_ALLOWED_COMMANDS).toBe(PLUGIN_BRIDGE_COMMANDS);
    expect(createPanelHostApi).toBe(createPluginBridgeApi);
  });

  it("权限面（传给沙箱容器的 commands）与白名单逐字相同，不多不少", () => {
    const permissions = createPluginBridgePermissions();
    expect([...(permissions.commands ?? [])].sort()).toEqual(
      [...PLUGIN_BRIDGE_COMMANDS].sort(),
    );
    // 除这两条之外一律不在权限面里，尤其是通用命令与宿主自定义协议命令。
    for (const command of ["plugin_read_panel_asset", "plugin_install", "get_dashboard_stats", "invoke"]) {
      expect(isPluginBridgeCommand(command)).toBe(false);
      expect(permissions.commands).not.toContain(command);
    }
  });

  it("isPluginBridgeCommand 与白名单同源判定", () => {
    for (const command of PLUGIN_BRIDGE_COMMANDS) {
      expect(isPluginBridgeCommand(command)).toBe(true);
    }
  });
});

describe.each(SURFACES)("$name 粒度的桥行为", ({ build, denyPrefix }) => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
    vi.mocked(invoke).mockResolvedValue("ok");
  });

  it("③ 白名单外的命令直接拒绝，且不发出 IPC", async () => {
    const api = build("own-pack");
    await expect(api.invoke("get_dashboard_stats", {})).rejects.toThrow(`${denyPrefix}:get_dashboard_stats`);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("③ 读取类后端命令同样被拒：面板/页面不得自行取别的资产", async () => {
    const api = build("own-pack");
    await expect(api.invoke("plugin_read_panel_asset", { pluginId: "own-pack", panelId: "x" })).rejects
      .toThrow(`${denyPrefix}:plugin_read_panel_asset`);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("② 丢弃 iframe 传来的 pluginId，强制绑定本内容所属插件", async () => {
    const api = build("own-pack");
    await api.invoke("plugin_ui_action", { pluginId: "victim-pack", action: { type: "ping" } });

    expect(invoke).toHaveBeenCalledTimes(1);
    const [command, args] = vi.mocked(invoke).mock.calls[0];
    expect(command).toBe("plugin_ui_action");
    expect(args).toEqual({ action: { type: "ping" }, pluginId: "own-pack" });
  });

  it("② 不带 pluginId 时由宿主补上，仍然只有宿主那一份", async () => {
    const api = build("own-pack");
    await api.invoke("plugin_ui_action", { action: { type: "pong" } });

    expect(vi.mocked(invoke).mock.calls[0][1]).toEqual({ action: { type: "pong" }, pluginId: "own-pack" });
  });

  it("② execute_command 同样绑定 pluginId；其余键原样透传（不断言业务键名）", async () => {
    const api = build("own-pack");
    await api.invoke("plugin_execute_command", {
      pluginId: "victim-pack",
      passthroughSample: { a: 1 },
    });

    const [, args] = vi.mocked(invoke).mock.calls[0];
    expect(args).toEqual({ passthroughSample: { a: 1 }, pluginId: "own-pack" });
    // 只钉「归属」这一条业务不变量：外部无法改写它。
    expect((args as { pluginId: string }).pluginId).toBe("own-pack");
  });

  it("② 宿主绑定的 pluginId 不被 iframe 的空串覆盖", async () => {
    const api = build("own-pack");
    await api.invoke("plugin_ui_action", { pluginId: "" });

    expect(vi.mocked(invoke).mock.calls[0][1]).toEqual({ pluginId: "own-pack" });
  });

  it("emit 不发 IPC：桥没给事件面开第二条通道", () => {
    const api = build("own-pack");
    expect(() => api.emit("anything", { a: 1 })).not.toThrow();
    expect(invoke).not.toHaveBeenCalled();
  });
});
