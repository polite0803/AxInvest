// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 插件页面容器（任务 #64）的容器级行为用例。
 *
 * 补的是 #67 交付时明确记为「未覆盖」的那一格（PLAN §25.2 第 4 条：容器级行为用例没补，
 * 要渲染整组件 + jsdom 事件派发）。安全边界本身长在桥自己身上，但**容器接错照样形同虚设**：
 * 下面每条都从「渲染出来的 DOM + 真事件派发」起步，断言打在外层可观察的行为上
 * （iframe 的 src / sandbox、IPC 有没有发生、用户看到哪句译文），不去回头调用判据函数自证。
 *
 * 与既有测试的分工：
 * - `pluginPageBridge.test.ts`：一整条桥链（bundle 客户端 → 宿主接收器 → IPC）。
 * - `pluginSandboxBridge.test.ts`：宿主侧 API 的调用面与三条不变量。
 * - `sandboxFrameProtocol.test.ts` 门 E/F：srcdoc 产物的真实行为 + 出向/入向 origin 判据。
 * - 本文件：**page 容器**的分支判据、装载时序、来源闸、超时/重试/卸载、按错误码取译文。
 */

import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { App as AntdApp } from "antd";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";
import { invoke } from "@/lib/invoke";
import { PLUGIN_BRIDGE_COMMAND_NAMES } from "@/sdk/pluginBridgeProtocol";
import {
  buildRpcRequestFrame,
  SANDBOX_FRAME_TYPES,
  SANDBOX_LOAD_TIMEOUT_MS,
  SANDBOX_RPC_ARG_KEYS,
  SANDBOX_RPC_METHODS,
} from "@/sdk/sandboxFrameProtocol";
import type { PluginPageDto } from "@/types";

vi.mock(import("@/lib/invoke"), async (importOriginal) => {
  const actual = await importOriginal();
  return { ...actual, invoke: vi.fn(), logIpcError: () => () => {} };
});

// schema 分支只验「走没走那条通道、传进去的是不是 schema id」，不验动态 UI 渲染本身
vi.mock("@/components/dynamicUI/DynamicUIStandalone", () => ({
  DynamicUIStandalone: ({ schemaId }: { schemaId: string }) => (
    <div data-testid="schema-channel" data-schema-id={schemaId} />
  ),
}));

const runUiAction = vi.fn(async (): Promise<null> => null);
vi.mock("@/stores", () => ({
  usePluginStore: (selector: (state: { runUiAction: typeof runUiAction }) => unknown) => selector({ runUiAction }),
}));

import { PluginPageHost } from "@/components/plugin/PluginPageHost";

const PLUGIN_ID = "own-pack";
/** 清单原值：故意起一个「像路径」的名字，好让「参与拼接」的缺陷在断言里当场显形。 */
const ENTRY = "dist/pages/main.js";
const ABSOLUTE_PATH = "/home/dev/.axagent/plugins/installed/own-pack/dist/pages/main.js";

const bundlePage: PluginPageDto = { id: "home", title: "行情首页", entry: ENTRY, loading: "bundle" };
const schemaPage: PluginPageDto = { id: "form", title: "录入表", entry: "schema-42", loading: "schema" };

/** 只渲染 bundle 粒度那一支，返回容器与其中那块 iframe。 */
function renderBundleHost(page: PluginPageDto = bundlePage) {
  const view = render(
    <AntdApp>
      <PluginPageHost pluginId={PLUGIN_ID} page={page} pluginName="Own Pack" />
    </AntdApp>,
  );
  const iframe = view.container.querySelector("iframe") as HTMLIFrameElement;
  return { ...view, iframe };
}

/**
 * 沙箱文档 → 宿主投一帧。
 *
 * `source` 给该 iframe 的 `contentWindow` 是真实形态；给 `window`（本测试里另一块文档）
 * 就是 #72/#67 应修 4 要拒的那种外来帧 —— 帧内容可以完全合法。
 * 外层包 `act`：合法帧会触发容器 setState（建桥 + 撤 loading），不包就是 React 的 act 警告。
 */
function sendFromSandbox(frame: unknown, source: unknown): void {
  act(() => {
    window.dispatchEvent(new MessageEvent("message", { data: frame, source: source as Window, origin: "null" }));
  });
}

/** 排空 promise 链（`invoke` 的 then 里才赋 src，桥的回包也落在微任务之后）。 */
async function flush(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

/** 一条与真后端同形的解析结果。 */
const resolvedLocation = {
  pluginId: PLUGIN_ID,
  pageId: bundlePage.id,
  entry: ENTRY,
  loading: "bundle",
  absolutePath: ABSOLUTE_PATH,
};

function mockResolveOk(location: unknown = resolvedLocation): void {
  vi.mocked(invoke).mockImplementation(
    (async (cmd: string) => {
      if (cmd === "plugin_resolve_page_asset") {
        return location;
      }
      return "ipc-ok";
    }) as never,
  );
}

/** 已发生的某条 IPC 调用次数（用来区分「桥没建」与「桥建了但被拒」）。 */
function ipcCalls(command: string): unknown[][] {
  return vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === command);
}

beforeEach(() => {
  vi.mocked(invoke).mockReset();
  // 浏览器模式没有 asset 协议：把 Tauri 的两个开关按真实形态补上，让**真的** assetUrl() 走到底
  vi.stubGlobal("isTauri", true);
  vi.stubGlobal("__TAURI_INTERNALS__", { convertFileSrc: (p: string): string => `asset://localhost/${p}` });
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe("分支判据只取 loading（§20.4 E2）", () => {
  it("schema 页：一次都不调解析命令，entry 经命名空间化交给动态 UI 通道", async () => {
    const view = render(
      <AntdApp>
        <PluginPageHost pluginId={PLUGIN_ID} page={schemaPage} />
      </AntdApp>,
    );
    await flush();

    expect(invoke, "schema 页调了 plugin_resolve_page_asset —— 后端那条解析路对 schema 必报错").not.toHaveBeenCalled();
    const channel = view.getByTestId("schema-channel");
    // f0a8a7afa 起 schema 注册通道要求全局唯一 row id（plugin.<pluginId>.<entry>），
    // 裸 entry 会在多插件间撞名 ⇒ 断言命名空间化形态而非裸 entry。
    expect(channel.getAttribute("data-schema-id")).toBe("plugin.own-pack.schema-42");
  });

  it("bundle 页：只调解析命令，参数只有 (pluginId, pageId)", async () => {
    mockResolveOk();
    renderBundleHost();
    await flush();

    expect(invoke).toHaveBeenCalledTimes(1);
    expect(vi.mocked(invoke).mock.calls[0]).toEqual(["plugin_resolve_page_asset", {
      pluginId: PLUGIN_ID,
      pageId: "home",
    }]);
  });
});

describe("装载形态：后端绝对路径 → assetUrl() → iframe src（§20.4 E2 / E3）", () => {
  it("src 由 assetUrl() 生成，完全等于后端 absolutePath 的换 URL 结果", async () => {
    mockResolveOk();
    const { iframe } = renderBundleHost();
    await waitFor(() => {
      expect(iframe.getAttribute("src")).toBeTruthy();
    });
    expect(iframe.getAttribute("src")).toBe(`asset://localhost/${ABSOLUTE_PATH}`);
  });

  it("换一个 entry、同一 absolutePath ⇒ src 逐字不变（entry 没参与路径构造）", async () => {
    mockResolveOk({ ...resolvedLocation, entry: "totally-different-name.js" });
    const { iframe } = renderBundleHost();
    await waitFor(() => {
      expect(iframe.getAttribute("src")).toBeTruthy();
    });
    expect(iframe.getAttribute("src")).toBe(`asset://localhost/${ABSOLUTE_PATH}`);
  });

  it("sandbox 只有 allow-scripts，绝不出现 allow-same-origin", () => {
    mockResolveOk();
    const { iframe } = renderBundleHost();
    expect(iframe.getAttribute("sandbox")).toBe("allow-scripts");
    expect(iframe.getAttribute("sandbox")).not.toContain("allow-same-origin");
  });

  it("iframe 在 loading 期就在 DOM 里（ready 只能由它发出，晚挂载即死锁）", async () => {
    vi.mocked(invoke).mockImplementation((() => new Promise(() => {})) as never);
    const { container, iframe } = renderBundleHost();
    await flush();

    expect(iframe).toBeTruthy();
    expect(container.textContent).not.toContain(i18n.t("skill.iframeNotReady"));
  });
});

describe("ready 是唯一开闸信号（§20.4 A/B）", () => {
  beforeEach(() => {
    mockResolveOk();
  });

  it("宿主来源的就绪帧之前，合法请求不执行；之后同一帧照常执行并绑宿主 pluginId", async () => {
    const { iframe } = renderBundleHost();
    await flush();
    const request = buildRpcRequestFrame("plugin_1_1", SANDBOX_RPC_METHODS.apiInvoke, {
      [SANDBOX_RPC_ARG_KEYS.command]: PLUGIN_BRIDGE_COMMAND_NAMES.uiAction,
      [SANDBOX_RPC_ARG_KEYS.args]: { action: { type: "ping" } },
    });

    sendFromSandbox(request, iframe.contentWindow);
    await flush();
    expect(ipcCalls(PLUGIN_BRIDGE_COMMAND_NAMES.uiAction), "还没就绪就建了桥：开闸信号被绕过").toHaveLength(0);

    sendFromSandbox({ type: SANDBOX_FRAME_TYPES.ready }, iframe.contentWindow);
    await flush();
    sendFromSandbox(request, iframe.contentWindow);
    await flush();

    const calls = ipcCalls(PLUGIN_BRIDGE_COMMAND_NAMES.uiAction);
    expect(calls).toHaveLength(1);
    expect(calls[0]![1]).toEqual({ action: { type: "ping" }, pluginId: PLUGIN_ID });
  });

  it("外来来源的「就绪帧」不开闸：不建桥、撤不掉加载超时", async () => {
    vi.useFakeTimers();
    const { iframe } = renderBundleHost();
    await flush();

    sendFromSandbox({ type: SANDBOX_FRAME_TYPES.ready }, window);
    await act(async () => {
      vi.advanceTimersByTime(SANDBOX_LOAD_TIMEOUT_MS + 1);
    });

    expect(screen.getByText(i18n.t("skill.timeoutError")), "外来 ready 帧把加载超时撤了").toBeTruthy();
    expect(iframe.getAttribute("src")).toBe(`asset://localhost/${ABSOLUTE_PATH}`);
  });

  it("外来来源的合法请求帧不发 IPC；同一帧换成宿主来源即照常执行（防「一律拒绝」假绿）", async () => {
    const { iframe } = renderBundleHost();
    await flush();
    sendFromSandbox({ type: SANDBOX_FRAME_TYPES.ready }, iframe.contentWindow);
    await flush();

    const request = buildRpcRequestFrame("plugin_2_1", SANDBOX_RPC_METHODS.apiInvoke, {
      [SANDBOX_RPC_ARG_KEYS.command]: PLUGIN_BRIDGE_COMMAND_NAMES.uiAction,
      [SANDBOX_RPC_ARG_KEYS.args]: { action: { type: "borrowed" } },
    });
    sendFromSandbox(request, window);
    await flush();
    expect(ipcCalls(PLUGIN_BRIDGE_COMMAND_NAMES.uiAction), "借别人的身份打了 worker").toHaveLength(0);

    sendFromSandbox(request, iframe.contentWindow);
    await flush();
    expect(ipcCalls(PLUGIN_BRIDGE_COMMAND_NAMES.uiAction)).toHaveLength(1);
  });

  it("就绪帧重复投递是幂等的：只建一座桥（挤掉旧桥即漏 destroy）", async () => {
    const { iframe } = renderBundleHost();
    await flush();

    sendFromSandbox({ type: SANDBOX_FRAME_TYPES.ready }, iframe.contentWindow);
    await flush();
    sendFromSandbox({ type: SANDBOX_FRAME_TYPES.ready }, iframe.contentWindow);
    await flush();

    sendFromSandbox(
      buildRpcRequestFrame("plugin_3_1", SANDBOX_RPC_METHODS.apiInvoke, {
        [SANDBOX_RPC_ARG_KEYS.command]: PLUGIN_BRIDGE_COMMAND_NAMES.executeCommand,
        [SANDBOX_RPC_ARG_KEYS.args]: { name: "x" },
      }),
      iframe.contentWindow,
    );
    await flush();

    // 两座桥并存时这一帧会被处理两次 ⇒ 两次 IPC。一次即证明只有一座桥。
    expect(ipcCalls(PLUGIN_BRIDGE_COMMAND_NAMES.executeCommand)).toHaveLength(1);
  });

  it("产物忘了 announceReady ⇒ 30 s 落 skill.timeoutError（症状是白一块并报错，不是桥不通）", async () => {
    vi.useFakeTimers();
    renderBundleHost();
    await flush();
    expect(screen.queryByText(i18n.t("skill.timeoutError"))).toBeNull();

    await act(async () => {
      vi.advanceTimersByTime(SANDBOX_LOAD_TIMEOUT_MS + 1);
    });
    expect(screen.getByText(i18n.t("skill.timeoutError"))).toBeTruthy();
  });

  it("点重试会重新解析一次（重挂 iframe，而不是给同一 URL 再赋一次值）", async () => {
    vi.useFakeTimers();
    const { container } = renderBundleHost();
    await flush();
    await act(async () => {
      vi.advanceTimersByTime(SANDBOX_LOAD_TIMEOUT_MS + 1);
    });
    const before = vi.mocked(invoke).mock.calls.length;

    await act(async () => {
      // antd Button 会在两个汉字间插一个空格（渲染成「重 试」），故按 role + 正则取，不按精确文本
      fireEvent.click(screen.getByRole("button", { name: /重\s*试/ }));
    });
    expect(vi.mocked(invoke).mock.calls.length).toBe(before + 1);
    expect(container.querySelector("iframe")).toBeTruthy();
  });

  it("卸载后摘掉监听：再来的合法帧不打到 IPC", async () => {
    const { iframe, unmount } = renderBundleHost();
    await flush();
    sendFromSandbox({ type: SANDBOX_FRAME_TYPES.ready }, iframe.contentWindow);
    await flush();
    const sandboxWindow = iframe.contentWindow;
    unmount();

    sendFromSandbox(
      buildRpcRequestFrame("plugin_4_1", SANDBOX_RPC_METHODS.apiInvoke, {
        [SANDBOX_RPC_ARG_KEYS.command]: PLUGIN_BRIDGE_COMMAND_NAMES.uiAction,
      }),
      sandboxWindow,
    );
    await flush();
    expect(ipcCalls(PLUGIN_BRIDGE_COMMAND_NAMES.uiAction)).toHaveLength(0);
  });

  it("白名单外的命令即便在就绪后也拿不到 IPC（页面粒度共用 panel 那条白名单）", async () => {
    const { iframe } = renderBundleHost();
    await flush();
    sendFromSandbox({ type: SANDBOX_FRAME_TYPES.ready }, iframe.contentWindow);
    await flush();

    sendFromSandbox(
      buildRpcRequestFrame("plugin_5_1", SANDBOX_RPC_METHODS.apiInvoke, {
        [SANDBOX_RPC_ARG_KEYS.command]: "plugin_install",
      }),
      iframe.contentWindow,
    );
    await flush();

    expect(ipcCalls("plugin_install"), "白名单外的命令打到了 IPC").toHaveLength(0);
  });
});

describe("失败按错误码分支，i18n 零新增（§20.4 E6）", () => {
  const codes = [
    "PLUGIN_PANEL_ASSET_UNAVAILABLE",
    "PLUGIN_PANEL_ASSET_BLOCKED",
    "PLUGIN_PANEL_ASSET_UNPINNED",
    "PLUGIN_PANEL_ASSET_TAMPERED",
  ] as const;

  it.each(codes)("%s ⇒ 显示该码在 11 语言里已有的译文，而不是裸码或 JSON 串", async (code) => {
    vi.mocked(invoke).mockRejectedValue(
      new Error(JSON.stringify({ code, category: "validation", detail: "raw backend detail" })),
    );
    const { container } = renderBundleHost();
    await waitFor(() => {
      expect(container.textContent).toContain(i18n.t(`error.${code}`));
    });
    expect(container.textContent).not.toContain(code);
    expect(container.textContent).not.toContain('"detail"');
  });

  it("浏览器模式（mock 侧无 asset 协议）给的也是同一条码 ⇒ 容器仍走译文", async () => {
    vi.stubGlobal("isTauri", false);
    vi.mocked(invoke).mockRejectedValue(
      new Error(JSON.stringify({ code: "PLUGIN_PANEL_ASSET_UNAVAILABLE", category: "validation" })),
    );
    const { container } = renderBundleHost();
    await waitFor(() => {
      expect(container.textContent).toContain(i18n.t("error.PLUGIN_PANEL_ASSET_UNAVAILABLE"));
    });
  });
});
