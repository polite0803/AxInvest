// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 三条不变量在 **page（asset 协议装第三方 bundle）粒度**上也成立（任务 #46 验收条 2）。
 *
 * 与 `pluginSandboxBridge.test.ts` 的分工：那份测的是**宿主侧 API 的调用面**
 * （直接调 `createPluginBridgeApi`），本测的是**一整条链**：
 *
 * ```
 * 第三方 bundle 里的客户端（createAxAgentPluginClient）
 *   → postMessage 帧（sandboxFrameProtocol 构造）
 *   → 宿主接收器（createHostApiBridge，权限面 = createPluginBridgePermissions()）
 *   → 宿主绑定 API（createPluginBridgeApi，pluginId 在此绑定）
 *   → Tauri IPC（@/lib/invoke，本测试里是 mock）
 * ```
 *
 * 只有走完整条链，才能说出「桥对第三方 bundle 可用」这句话是有证据的：
 * 帧名对不上、字段对不上、权限面少一条，都会在这里红，而不是等页面上线后静默失效。
 *
 * 三条不变量：
 * ① 白名单外命令被拒且**不发 IPC**（两侧都拦：客户端不发帧；帧真发出来了宿主也拒）；
 * ② `pluginId` 由宿主绑定 —— bundle 传、伪造帧传、传空串，三种都落到宿主那一份；
 * ③ 命令集与面板共用**同一对象**（`toBe`），不是两份内容相等的清单。
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

import { invoke } from "@/lib/invoke";

import { createPluginBridgeApi, createPluginBridgePermissions, PLUGIN_BRIDGE_COMMANDS } from "../pluginSandboxBridge";
// 兼容别名走的是 panel 侧调用面；同时 import 两侧才能测出「是不是同一份」。
import { createPanelHostApi, PANEL_ALLOWED_COMMANDS } from "../PluginPanelHost";

import { HOST_BOUND_PLUGIN_ID_FIELD, PLUGIN_BRIDGE_COMMAND_NAMES } from "@/sdk/pluginBridgeProtocol";
import {
  AX_AGENT_PLUGIN_BRIDGE_COMMANDS,
  createAxAgentPluginClient,
  type PluginClientMessageEvent,
  type PluginClientMessageSource,
  type PluginClientPostTarget,
} from "@/sdk/pluginClientBridge";
import { createHostApiBridge, type HostApiBridge } from "@/sdk/rpcBridge";
import {
  buildRpcRequestFrame,
  isRpcResponseFrame,
  readRpcResponseError,
  SANDBOX_RPC_METHODS,
  type SandboxRpcResponseFrame,
} from "@/sdk/sandboxFrameProtocol";

vi.mock(import("@/lib/invoke"), async (importOriginal) => {
  const actual = await importOriginal();
  return { ...actual, invoke: vi.fn() };
});

const HOST_BOUND_ID = "own-pack";
const pageDeny = (command: string): string => `page-denied:${command}`;

/**
 * 排空宿主侧的异步 handler 链。
 *
 * `handleRpcRequest` 里是 `handler(args).then(...).catch(...)`，回包落在若干微任务之后；
 * 用 `setTimeout(0)` 而不是 `await Promise.resolve()` —— 后者要数准微任务层数，
 * 层数会随实现改动变（那是实现细节，不该成为测试的前提）。
 */
function flush(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

/** 一条真实的「宿主 ↔ bundle」链路：帧从客户端出、经宿主接收器、落到被 mock 的 IPC。 */
function createWire() {
  const frameLog: unknown[] = [];
  const responseLog: unknown[] = [];
  const listeners: Array<(event: PluginClientMessageEvent) => void> = [];

  let bridge: HostApiBridge | null = null;
  /**
   * 沙箱眼里的「宿主文档」：`createAxAgentPluginClient` 的期望来源（生产即 `window.parent`），
   * 也是宿主回帧时 `event.source` 的取值。与下面的 `contentWindow` / `foreignWindow` 都不是同一对象。
   */
  const hostWindow = { postMessage: (): void => {} } as unknown as Window;
  const contentWindow = {
    postMessage: (message: unknown): void => {
      responseLog.push(message);
      // 宿主回包投给沙箱（沙箱侧的 message 监听器据此 resolve/reject）
      for (const listener of listeners.slice()) {
        listener({ data: message, source: hostWindow });
      }
    },
  } as unknown as Window;
  /**
   * 「应用里另一块 iframe 的 contentWindow」—— 与 `contentWindow` 不是同一对象。
   * 用它当 `event.source` 就是审查应修 4 的攻击形态：帧内容完全合法，但来源不对。
   */
  const foreignWindow = { postMessage: (): void => {} } as unknown as Window;

  const postTarget: PluginClientPostTarget = {
    postMessage: (message: unknown): void => {
      frameLog.push(message);
      if (!bridge) {
        throw new Error("宿主尚未开始接帧");
      }
      // 宿主接收的是**事件**（来源判定要用 source），沙箱侧投来的就是本链路的 contentWindow
      bridge.handleRpcRequest({ data: message, source: contentWindow });
    },
  };
  const messageSource: PluginClientMessageSource = {
    addEventListener: (_type: "message", listener: (event: PluginClientMessageEvent) => void): void => {
      listeners.push(listener);
    },
    removeEventListener: (_type: "message", listener: (event: PluginClientMessageEvent) => void): void => {
      const index = listeners.indexOf(listener);
      if (index >= 0) {
        listeners.splice(index, 1);
      }
    },
  };

  // 宿主侧的三件套：绑定 API + 权限面 + 通用接收器。与将来页面容器要接的是同一组构造函数。
  // store/ui 的桩一律用未标注的 vi.fn()：宿主接口带泛型（`read<T>() => Promise<T>`），
  // 给实现反而不匹配，且本测试根本不会走到 store 面（权限面只开两条命令）。
  bridge = createHostApiBridge({
    api: createPluginBridgeApi(HOST_BOUND_ID, pageDeny),
    ui: { navigate: vi.fn(), notify: vi.fn(), getTheme: vi.fn(), getLocale: vi.fn() },
    store: { read: vi.fn(), write: vi.fn() },
    permissions: createPluginBridgePermissions(),
    contentWindow,
  });

  const client = createAxAgentPluginClient({ postTarget, messageSource, hostWindow });

  return {
    client,
    frameLog,
    responseLog,
    /**
     * 绕过 bundle 客户端，直接以**事件**打宿主（模拟自己拼帧的插件产物）。
     * `source` 默认是本链路的沙箱窗口；传 `foreignWindow` 即「另一块 iframe 发来的帧」。
     */
    sendRawFrame: (frame: unknown, source: unknown = contentWindow): void => {
      frameLog.push(frame);
      bridge?.handleRpcRequest({ data: frame, source });
    },
    /** 供伪造来源用例使用的、与本链路无关的窗口。 */
    foreignWindow,
    bridgeApi: () => bridge as HostApiBridge,
  };
}

// 文件级复位：本测里多处断言「IPC 恰好被调了几次」，跨用例累计会让结论失真
beforeEach(() => {
  vi.mocked(invoke).mockReset();
  vi.mocked(invoke).mockResolvedValue("ipc-ok");
});

describe("不变量①：白名单外的命令被拒且不发 IPC（page 粒度）", () => {
  it("bundle 客户端连白名单外的命令名都发不出去：无帧、无 IPC", async () => {
    const wire = createWire();
    await expect(wire.client.invoke("plugin_read_panel_asset" as never, { panelId: "x" })).rejects.toThrow(/白名单外/);
    expect(wire.frameLog).toHaveLength(0);
    expect(invoke).not.toHaveBeenCalled();
    wire.client.dispose();
  });

  it("插件产物自己拼帧也过不去：宿主侧拒绝，IPC 一次没发，回包是拒绝文案", async () => {
    const wire = createWire();
    wire.sendRawFrame(
      buildRpcRequestFrame("plugin_1_1", SANDBOX_RPC_METHODS.apiInvoke, {
        command: PLUGIN_BRIDGE_COMMAND_NAMES.executeCommand + "_x",
        args: { [HOST_BOUND_PLUGIN_ID_FIELD]: "victim-pack" },
      }),
    );
    await flush();

    expect(invoke).not.toHaveBeenCalled();
    const reply = wire.responseLog[0];
    expect(isRpcResponseFrame(reply)).toBe(true);
    expect(readRpcResponseError(reply as SandboxRpcResponseFrame)).toContain("is not allowed");
    wire.client.dispose();
  });

  it("权限面（宿主交给沙箱容器的 commands）就是那两条，读资产/安装类命令不在内", () => {
    const permissions = createPluginBridgePermissions();
    expect([...(permissions.commands ?? [])].sort()).toEqual([...PLUGIN_BRIDGE_COMMANDS].sort());
    for (
      const command of ["plugin_read_panel_asset", "plugin_resolve_page_asset", "plugin_install", "get_dashboard_stats"]
    ) {
      expect(permissions.commands).not.toContain(command);
    }
  });

  it("未知 RPC 方法名同样不发 IPC", async () => {
    const wire = createWire();
    wire.sendRawFrame(buildRpcRequestFrame("plugin_2_2", "api.unknownMethod" as never, {}));
    await flush();
    expect(invoke).not.toHaveBeenCalled();
    expect(readRpcResponseError(wire.responseLog[0] as SandboxRpcResponseFrame)).toContain("Unknown method");
    wire.client.dispose();
  });

  /**
   * 审查应修 4 的证据：**内容完全合法、但来源不对**的一帧。
   *
   * 攻击形态是应用内任一可脚本化的 iframe 对宿主 window 发帧 —— 帧过得了 guard、命令在白名单内、
   * pluginId 由宿主绑定，因此**除来源判定外没有任何一层拦得住它**（它会借这个包的身份打到 worker）。
   * 断言打在宿主的行为上：不发 IPC、**也不回包**（回包本身就是「外来帧拿到了宿主应答」这条信息）。
   */
  it("来源不是本沙箱的帧：宿主不调 IPC、不回包", async () => {
    const wire = createWire();
    wire.sendRawFrame(
      buildRpcRequestFrame("plugin_fk_1", SANDBOX_RPC_METHODS.apiInvoke, {
        command: PLUGIN_BRIDGE_COMMAND_NAMES.uiAction,
        args: { action: { type: "borrowed-identity" } },
      }),
      wire.foreignWindow,
    );
    await flush();

    expect(invoke, "宿主借别人的帧执行了白名单命令").not.toHaveBeenCalled();
    expect(wire.responseLog, "宿主给来历不明的帧回了包").toHaveLength(0);
    wire.client.dispose();
  });

  it("同一帧换个来源：本沙箱的照常执行（来源判定不是「一律拒绝」）", async () => {
    vi.mocked(invoke).mockResolvedValue("ipc-ok");
    const wire = createWire();
    const frame = buildRpcRequestFrame("plugin_ok_1", SANDBOX_RPC_METHODS.apiInvoke, {
      command: PLUGIN_BRIDGE_COMMAND_NAMES.uiAction,
      args: { action: { type: "mine" } },
    });
    wire.sendRawFrame(frame, wire.foreignWindow);
    await flush();
    expect(invoke).not.toHaveBeenCalled();

    wire.sendRawFrame(frame);
    await flush();
    expect(invoke).toHaveBeenCalledTimes(1);
    wire.client.dispose();
  });
});

describe("不变量②：pluginId 由宿主绑定（page 粒度）", () => {
  it("bundle 传别人的 pluginId：客户端剥掉 → 宿主再绑自己的 → IPC 只见宿主那一份", async () => {
    const wire = createWire();
    await wire.client.invoke(PLUGIN_BRIDGE_COMMAND_NAMES.uiAction, {
      [HOST_BOUND_PLUGIN_ID_FIELD]: "victim-pack",
      action: { type: "ping" },
    } as never);

    // 帧里根本没有 pluginId
    const frame = wire.frameLog[0] as { args: { args: Record<string, unknown> } };
    expect(frame.args.args).not.toHaveProperty(HOST_BOUND_PLUGIN_ID_FIELD);

    // IPC 收到的是宿主绑定的归属
    expect(invoke).toHaveBeenCalledTimes(1);
    const [command, args] = vi.mocked(invoke).mock.calls[0];
    expect(command).toBe(PLUGIN_BRIDGE_COMMAND_NAMES.uiAction);
    expect(args).toEqual({ action: { type: "ping" }, [HOST_BOUND_PLUGIN_ID_FIELD]: HOST_BOUND_ID });
    wire.client.dispose();
  });

  it("绕过客户端、帧里硬塞 pluginId：宿主仍然丢弃并覆盖", async () => {
    const wire = createWire();
    wire.sendRawFrame(
      buildRpcRequestFrame("plugin_3_3", SANDBOX_RPC_METHODS.apiInvoke, {
        command: PLUGIN_BRIDGE_COMMAND_NAMES.uiAction,
        args: { [HOST_BOUND_PLUGIN_ID_FIELD]: "victim-pack", action: { type: "poke" } },
      }),
    );
    // handleRpcRequest 内部是 async handler：排空微任务队列把 IPC 调用逼出来
    await flush();

    expect(invoke).toHaveBeenCalledTimes(1);
    expect(vi.mocked(invoke).mock.calls[0][1]).toEqual({
      action: { type: "poke" },
      [HOST_BOUND_PLUGIN_ID_FIELD]: HOST_BOUND_ID,
    });
    wire.client.dispose();
  });

  it("空串 pluginId 不能覆盖宿主绑定的归属", async () => {
    const wire = createWire();
    await wire.client.executeCommand({ [HOST_BOUND_PLUGIN_ID_FIELD]: "" } as never);
    expect(vi.mocked(invoke).mock.calls[0][1]).toEqual({ [HOST_BOUND_PLUGIN_ID_FIELD]: HOST_BOUND_ID });
    wire.client.dispose();
  });

  it("宿主绑定的 id 就是容器传入的那一个（换插件即换归属，与页面无关）", async () => {
    vi.mocked(invoke).mockResolvedValue("ipc-ok");
    const other = createPluginBridgeApi("another-pack", pageDeny);
    await other.invoke(PLUGIN_BRIDGE_COMMAND_NAMES.executeCommand, { name: "x" });
    expect(vi.mocked(invoke).mock.calls[0][1]).toEqual({ name: "x", [HOST_BOUND_PLUGIN_ID_FIELD]: "another-pack" });
  });
});

describe("不变量③：命令集与面板共用同一对象（page 粒度）", () => {
  it("panel 别名、page 调用面、协议清单、bundle 客户端导出 = 同一个数组对象", () => {
    expect(PANEL_ALLOWED_COMMANDS).toBe(PLUGIN_BRIDGE_COMMANDS);
    expect(AX_AGENT_PLUGIN_BRIDGE_COMMANDS).toBe(PLUGIN_BRIDGE_COMMANDS);
    expect(createPanelHostApi).toBe(createPluginBridgeApi);
    // 清单内容仍是那两条（扩条必须同时改测试，见 pluginSandboxBridge.test.ts 不变量①）
    expect([...PLUGIN_BRIDGE_COMMANDS].sort()).toEqual([
      PLUGIN_BRIDGE_COMMAND_NAMES.uiAction,
      PLUGIN_BRIDGE_COMMAND_NAMES.executeCommand,
    ].sort());
  });

  it("走完整条链：白名单内两条命令都能拿到宿主结果，回包形状一致", async () => {
    vi.mocked(invoke).mockResolvedValue("worker-reply");
    const wire = createWire();

    await expect(wire.client.uiAction({ type: "refresh" })).resolves.toBe("worker-reply");
    await expect(wire.client.executeCommand({ name: "greet" } as never)).resolves.toBe("worker-reply");

    expect(vi.mocked(invoke).mock.calls.map((call) => call[0]).sort()).toEqual([
      PLUGIN_BRIDGE_COMMAND_NAMES.executeCommand,
      PLUGIN_BRIDGE_COMMAND_NAMES.uiAction,
    ].sort());
    // 两次回包都是「成功响应帧」，结果载体同一
    for (const reply of wire.responseLog) {
      expect(isRpcResponseFrame(reply)).toBe(true);
      expect(readRpcResponseError(reply as never)).toBeUndefined();
      expect((reply as { result: unknown }).result).toBe("worker-reply");
    }
    wire.client.dispose();
  });

  it("宿主 IPC 报错时，插件拿到的是原样的错误文本（deny 文案由容器注入，业务错误不被吞）", async () => {
    vi.mocked(invoke).mockRejectedValue(new Error("PLUGIN_WORKER_TIMEOUT"));
    const wire = createWire();
    await expect(wire.client.uiAction({ type: "refresh" })).rejects.toThrow("PLUGIN_WORKER_TIMEOUT");
    wire.client.dispose();
  });
});
