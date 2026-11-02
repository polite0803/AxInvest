// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 插件侧桥**客户端运行时**的行为测试（任务 #46 第 1 项）。
 *
 * 覆盖两件事：
 * 1. bundle 客户端发出的帧与宿主收到/回发的帧**逐字段对得上**（协议由 `sandboxFrameProtocol` 保证，
 *    这里验行为：请求→响应、成功→resolve、错误→reject、超时→reject、陌生 callId→丢弃）。
 * 2. 这份产物**能被第三方 bundle 携带**：三个 vendored 文件的 import 只允许指向集合内部，
 *    出现 `@/` 别名或 `@tauri-apps/*` 即红 —— 那会把宿主的 IPC 层拖进插件产物，
 *    第三方在自己的仓库里根本装不出来（「SDK 发布是终局的硬前置」的可执行版本）。
 */

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it, vi } from "vitest";

import { HOST_BOUND_PLUGIN_ID_FIELD, PLUGIN_BRIDGE_COMMAND_NAMES } from "@/sdk/pluginBridgeProtocol";
import {
  AX_AGENT_PLUGIN_BRIDGE_COMMANDS,
  type AxAgentPluginClientOptions,
  createAxAgentPluginClient,
  type PluginClientMessageEvent,
  type PluginClientMessageSource,
  type PluginClientPostTarget,
} from "@/sdk/pluginClientBridge";
import {
  buildRpcErrorFrame,
  buildRpcResultFrame,
  SANDBOX_FRAME_TYPES,
  SANDBOX_RPC_ARG_KEYS,
  SANDBOX_RPC_METHODS,
} from "@/sdk/sandboxFrameProtocol";

/** 可在测试里驱动的假宿主投递面。 */
function createFakeHost() {
  const posted: unknown[] = [];
  const listeners: Array<(event: PluginClientMessageEvent) => void> = [];
  /** 沙箱眼里的「宿主文档」：既是客户端期望的来源，也是宿主回帧的 `event.source`。 */
  const hostWindow = { postMessage: (): void => {} };
  /** 应用里另一块可脚本化文档 —— 与 `hostWindow` 不是同一对象，即 #78 要拒的伪造来源。 */
  const foreignWindow = { postMessage: (): void => {} };
  const postTarget: PluginClientPostTarget = {
    postMessage: (message: unknown): void => {
      posted.push(message);
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
  return {
    postTarget,
    messageSource,
    posted,
    hostWindow,
    foreignWindow,
    /** 宿主回发一帧给沙箱；`source` 缺省是宿主自己，传 `foreignWindow` 即伪造来源。 */
    deliver: (frame: unknown, source: unknown = hostWindow): void => {
      for (const listener of listeners.slice()) {
        listener({ data: frame, source });
      }
    },
    /** dispose 之后仍能拿到监听器引用（模拟宿主/浏览器把已排队消息递进来的通路）。 */
    captureListener: (): ((event: PluginClientMessageEvent) => void) | undefined => listeners[0],
    listenerCount: (): number => listeners.length,
  };
}

/** 用假宿主创建客户端：三处注入面（投递/接收/期望来源）一起给，生产用法不传参。 */
function createClient(
  host: ReturnType<typeof createFakeHost>,
  options: AxAgentPluginClientOptions = {},
) {
  return createAxAgentPluginClient({
    postTarget: host.postTarget,
    messageSource: host.messageSource,
    hostWindow: host.hostWindow,
    ...options,
  });
}

/** 最近一次发出的帧（断言用）。 */
function lastFrame(host: ReturnType<typeof createFakeHost>): Record<string, unknown> {
  const frame = host.posted[host.posted.length - 1];
  expect(frame, "客户端没有发出任何帧").toBeTruthy();
  return frame as Record<string, unknown>;
}

describe("客户端发出的帧形状", () => {
  it("uiAction 走 api.invoke 请求帧，业务参数原样带上且不包含 pluginId", async () => {
    const host = createFakeHost();
    const client = createClient(host);

    const promise = client.uiAction({ type: "refresh" });
    host.deliver(buildRpcResultFrame(lastFrame(host).callId as string, { handled: true }));
    await expect(promise).resolves.toEqual({ handled: true });

    const frame = lastFrame(host);
    expect(Object.keys(frame).sort()).toEqual(["args", "callId", "method", "type"].sort());
    expect(frame.type).toBe("rpc:request");
    expect(frame.method).toBe(SANDBOX_RPC_METHODS.apiInvoke);
    expect(String(frame.callId)).toMatch(/^plugin_\d+_\d+$/);
    const envelope = frame.args as Record<string, unknown>;
    expect(Object.keys(envelope).sort()).toEqual([SANDBOX_RPC_ARG_KEYS.args, SANDBOX_RPC_ARG_KEYS.command].sort());
    expect(envelope.command).toBe(PLUGIN_BRIDGE_COMMAND_NAMES.uiAction);
    expect(envelope.args).toEqual({ action: { type: "refresh" } });
    client.dispose();
  });

  it("调用方硬塞 pluginId 时，客户端就把它剥掉（不给插件「能指定归属」的假接口）", async () => {
    const host = createFakeHost();
    const client = createClient(host);

    // 类型上 pluginId 是 never（编译期挡 TS 调用方），这里用 cast 模拟纯 JS 的第三方产物
    const promise = client.invoke(
      PLUGIN_BRIDGE_COMMAND_NAMES.executeCommand,
      { [HOST_BOUND_PLUGIN_ID_FIELD]: "victim-pack", name: "greet" } as never,
    );
    const frame = lastFrame(host);
    host.deliver(buildRpcResultFrame(frame.callId as string, null));
    await promise;

    const envelope = frame.args as { args: Record<string, unknown> };
    expect(envelope.args).not.toHaveProperty(HOST_BOUND_PLUGIN_ID_FIELD);
    expect(envelope.args.name).toBe("greet");
    client.dispose();
  });

  it("executeCommand 除 pluginId 外一律透传（业务键名不在此钉死，归后端 schema）", async () => {
    const host = createFakeHost();
    const client = createClient(host);

    const promise = client.executeCommand({ name: "greet", input: { a: 1, b: [2, 3] } } as never);
    const frame = lastFrame(host);
    host.deliver(buildRpcResultFrame(frame.callId as string, null));
    await promise;

    const envelope = frame.args as { command: string; args: Record<string, unknown> };
    expect(envelope.command).toBe(PLUGIN_BRIDGE_COMMAND_NAMES.executeCommand);
    expect(envelope.args).toEqual({ name: "greet", input: { a: 1, b: [2, 3] } });
    client.dispose();
  });

  it("调用方不传业务参数时，信封里只有 command（不带空 args）", async () => {
    const host = createFakeHost();
    const client = createClient(host);
    const promise = client.executeCommand();
    const frame = lastFrame(host);
    host.deliver(buildRpcResultFrame(frame.callId as string, null));
    await promise;
    // 帧本体永远带 args（那是信封），但信封内部不传业务参数时不该多一个空对象
    const envelope = frame.args as Record<string, unknown>;
    expect(Object.keys(envelope)).toEqual([SANDBOX_RPC_ARG_KEYS.command]);
    expect(envelope.command).toBe(PLUGIN_BRIDGE_COMMAND_NAMES.executeCommand);
    client.dispose();
  });
});

describe("白名单：客户端这一层就不放行", () => {
  it("白名单外的命令名被拒绝，且**一帧都不发**", async () => {
    const host = createFakeHost();
    const client = createClient(host);

    // 类型面写不出这个命令名，cast 模拟运行期被擦除后的纯 JS 调用
    await expect(client.invoke("plugin_read_panel_asset" as never, { panelId: "x" }))
      .rejects
      .toThrow(/白名单外/);
    expect(host.posted).toHaveLength(0);
    client.dispose();
  });

  it("空串与 undefined 命令名同样被拒且不发帧", async () => {
    const host = createFakeHost();
    const client = createClient(host);
    await expect(client.invoke("" as never)).rejects.toThrow(/白名单外/);
    await expect(client.invoke(undefined as never)).rejects.toThrow(/白名单外/);
    expect(host.posted).toHaveLength(0);
    client.dispose();
  });

  it("导出的命令清单与宿主侧是同一个数组对象", () => {
    expect([...AX_AGENT_PLUGIN_BRIDGE_COMMANDS].sort()).toEqual([
      PLUGIN_BRIDGE_COMMAND_NAMES.uiAction,
      PLUGIN_BRIDGE_COMMAND_NAMES.executeCommand,
    ].sort());
  });
});

describe("响应处理", () => {
  it("宿主回错误帧 → 以 Error 拒绝，消息就是宿主给的文本", async () => {
    const host = createFakeHost();
    const client = createClient(host);

    const promise = client.uiAction({ type: "boom" });
    const callId = lastFrame(host).callId as string;
    host.deliver(buildRpcErrorFrame(callId, "PLUGIN_PANEL_ASSET_BLOCKED"));
    await expect(promise).rejects.toThrow("PLUGIN_PANEL_ASSET_BLOCKED");
    client.dispose();
  });

  it("宿主回**空串**错误帧 → 仍然 reject（审查应修 2：判据不是真值，三通道同口径）", async () => {
    // 触发条件是真实存在的：handler 抛空串、后端 Err(String) 空消息都会给出 ""
    const host = createFakeHost();
    const client = createClient(host);

    const promise = client.uiAction({ type: "empty-error" });
    const callId = lastFrame(host).callId as string;
    host.deliver(buildRpcErrorFrame(callId, ""));

    const settled = await promise.then((): null => null).catch((e: unknown) => e as Error);
    expect(settled, "空串错误被当成成功 resolve").toBeInstanceOf(Error);
    expect((settled as Error).message).toBe("");
    client.dispose();
  });

  it("成功帧带 result: undefined 仍判成功（error 键不在 ⇒ 不是失败）", async () => {
    const host = createFakeHost();
    const client = createClient(host);

    const promise = client.executeCommand({ name: "noop" } as never);
    const callId = lastFrame(host).callId as string;
    host.deliver(buildRpcResultFrame(callId, undefined));
    await expect(promise).resolves.toBeUndefined();
    client.dispose();
  });

  it("陌生 callId 的响应帧被忽略（不影响在途调用）", async () => {
    const host = createFakeHost();
    const client = createClient(host);

    const promise = client.executeCommand({ name: "slow" } as never);
    host.deliver(buildRpcResultFrame("plugin_9999_0000", "不该被认领"));
    const callId = lastFrame(host).callId as string;
    host.deliver(buildRpcResultFrame(callId, "真正的结果"));
    await expect(promise).resolves.toBe("真正的结果");
    client.dispose();
  });

  /**
   * #78：来源身份先于内容。
   *
   * 攻击形态：沙箱文档里再嵌一块可脚本化文档，它对沙箱 window 投一帧带**猜得到的 callId**
   * 的合法响应 —— 内容层（帧类型 + callId 全对）过得去，只有来源不对。
   * 断言打在行为上：这一帧不得 settle 在途 Promise；同一帧换回宿主来源才结算。
   */
  it("外来来源用猜到的 callId 伪造响应 ⇒ 不 settle 在途调用；换回宿主来源才结算", async () => {
    const host = createFakeHost();
    const client = createClient(host);

    const promise = client.executeCommand({ name: "victim" } as never);
    const callId = lastFrame(host).callId as string;
    let settledBy = "";
    const tracked = promise.then(
      (value) => {
        settledBy = `resolve:${String(value)}`;
      },
      (error: unknown) => {
        settledBy = `reject:${String((error as Error).message)}`;
      },
    );

    host.deliver(buildRpcResultFrame(callId, "stolen"), host.foreignWindow);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(settledBy, "陌生来源用猜得到的 callId 结算了在途调用").toBe("");

    host.deliver(buildRpcResultFrame(callId, "from-host"));
    await expect(promise).resolves.toBe("from-host");
    await tracked;
    client.dispose();
  });

  it("非帧消息（宿主外的其它 postMessage）被忽略", async () => {
    const host = createFakeHost();
    const client = createClient(host);
    expect(() => host.deliver({ hello: "world" })).not.toThrow();
    expect(() => host.deliver("string-frame")).not.toThrow();
    expect(() => host.deliver(null)).not.toThrow();
    client.dispose();
  });

  it("超时后拒绝并清理在途表（宿主不响应不会永久悬挂）", async () => {
    // 用真定时器 + 极短超时：vi.useFakeTimers() 会连带影响 vitest 自己的用例超时器，
    // 这里只关心「到点必拒」，不值得为省 20ms 引入一条环境耦合。
    const host = createFakeHost();
    const client = createClient(host, { timeoutMs: 20 });
    const promise = client.uiAction({ type: "never-answered" });
    let settledBeforeTimer = false;
    promise.catch(() => {
      settledBeforeTimer = true;
    });
    await new Promise((resolve) => setTimeout(resolve, 5));
    expect(settledBeforeTimer).toBe(false);
    await expect(promise).rejects.toThrow(/20ms 未收到宿主响应/);
    // 超时后在途表已清：宿主迟到回包不该再打到任何调用上
    const lateCallId = (frame => (frame as { callId: string }).callId)(lastFrame(host));
    expect(() => host.deliver(buildRpcResultFrame(lateCallId, "迟到的结果"))).not.toThrow();
    client.dispose();
  });
});

describe("就绪上报（page 粒度复用容器生命周期的前提）", () => {
  it("announceReady 发一帧只带类型判别字段的内容", () => {
    const host = createFakeHost();
    const client = createClient(host);

    client.announceReady();
    expect(host.posted).toHaveLength(1);
    expect(host.posted[0]).toEqual({ type: SANDBOX_FRAME_TYPES.ready });
    client.dispose();
  });

  it("dispose 之后不再上报，也不抛错", () => {
    const host = createFakeHost();
    const client = createClient(host);
    client.dispose();
    expect(() => client.announceReady()).not.toThrow();
    expect(host.posted).toHaveLength(0);
  });

  it("投递面已脱离文档时静默（后续请求各自报错，不在这里炸）", () => {
    const host = createFakeHost();
    const client = createClient(host, {
      postTarget: {
        postMessage: (): void => {
          throw new Error("detached");
        },
      },
    });
    expect(() => client.announceReady()).not.toThrow();
    client.dispose();
  });
});

describe("生命周期", () => {
  it("dispose 解绑监听并拒绝所有在途调用", async () => {
    const host = createFakeHost();
    const client = createClient(host);
    expect(host.listenerCount()).toBe(1);

    const pending = client.executeCommand({ name: "in-flight" } as never);
    const rejected = expect(pending).rejects.toThrow(/已释放/);
    client.dispose();
    await rejected;

    expect(host.listenerCount()).toBe(0);
    // 释放后再调用不再发帧
    const before = host.posted.length;
    await expect(client.uiAction({ type: "after-dispose" })).rejects.toThrow(/已释放/);
    expect(host.posted.length).toBe(before);
  });

  it("postMessage 抛错（iframe 已脱离文档）时该次调用被拒绝且不留在途表", async () => {
    const throwing: PluginClientPostTarget = {
      postMessage: (): void => {
        throw new Error("detached");
      },
    };
    const host = createFakeHost();
    const client = createClient(host, { postTarget: throwing });
    await expect(client.uiAction({ type: "x" })).rejects.toThrow("detached");
    client.dispose();
  });

  it("离开文档环境就明确报错，而不是造一个空转客户端", () => {
    // 缺省投递面取自 window；把 window 抹掉即「不在沙箱文档里」的形态
    vi.stubGlobal("window", undefined);
    try {
      expect(() => createAxAgentPluginClient()).toThrow(/需要浏览器窗口环境/);
    } finally {
      vi.unstubAllGlobals();
    }
    // 显式注入两侧投递面时不依赖 window（测试与 SSR 侧的用法）
    const host = createFakeHost();
    const client = createClient(host);
    expect(typeof client.invoke).toBe("function");
    client.dispose();
  });
});

/**
 * #78 的口径：**dispose 之后不可复用**（不提供 reset / 复活）。
 * 三条各一个用例：新调用立即拒绝、在途调用不悬挂、迟到消息不再 settle。
 */
describe("dispose 后不可复用（任务 #78）", () => {
  it("(a) dispose 之后的新调用立即拒绝，且一帧都不发", async () => {
    const host = createFakeHost();
    const client = createClient(host);
    client.dispose();
    const before = host.posted.length;

    await expect(client.invoke(PLUGIN_BRIDGE_COMMAND_NAMES.uiAction, { action: { type: "x" } } as never))
      .rejects.toThrow(/已释放（dispose 后不可再调用）/);
    await expect(client.uiAction({ type: "x" })).rejects.toThrow(/已释放/);
    await expect(client.executeCommand({ name: "x" } as never)).rejects.toThrow(/已释放/);
    expect(host.posted.length, "dispose 之后仍发出了帧").toBe(before);
  });

  it("(b) dispose 时在途调用立即以「已释放」拒绝，不永久挂起", async () => {
    const host = createFakeHost();
    const client = createClient(host);
    const pending = client.executeCommand({ name: "in-flight" } as never);
    expect(lastFrame(host).callId).toBeTruthy();

    const outcome = pending.then(
      (): null => null,
      (error: unknown) => error,
    );
    client.dispose();

    // 与「悬挂」竞速：50ms 内必须已拒绝，否则视为永久挂起
    const settled = await Promise.race([
      outcome,
      new Promise<string>((resolve) => setTimeout(() => resolve("still-pending"), 50)),
    ]);
    expect(settled, "dispose 之后在途调用仍在悬挂").not.toBe("still-pending");
    expect(settled).toBeInstanceOf(Error);
    expect((settled as Error).message).toMatch(/已释放/);
  });

  it("(c) dispose 之后到达的响应帧不得再 settle（含被捕获的监听器引用）", async () => {
    const host = createFakeHost();
    const client = createClient(host);
    const listener = host.captureListener();
    expect(listener, "客户端没有注册 message 监听器").toBeTruthy();

    const pending = client.executeCommand({ name: "late" } as never);
    const callId = lastFrame(host).callId as string;
    let settlements = 0;
    const tracked = pending.then(
      () => {
        settlements += 1;
      },
      () => {
        settlements += 1;
      },
    );

    client.dispose();
    await expect(pending).rejects.toThrow(/已释放/);
    await tracked;
    expect(settlements).toBe(1);

    // 摘监听不是唯一通路：宿主/浏览器仍可能把已排队的消息递给被捕获的 listener 引用。
    // 同一 callId、来源也正确 —— 唯一挡住它的是 dispose 这一层。
    expect(() => listener!({ data: buildRpcResultFrame(callId, "late-result"), source: host.hostWindow })).not
      .toThrow();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(settlements, "dispose 之后迟到帧又 settle 了一次").toBe(1);
  });
});

describe("沙箱客户端可携带性（第三方 bundle 能装得出来）", () => {
  /** vendored 文件集合：第三方原样复制这三个文件即可。 */
  const VENDORED = [
    "src/sdk/sandboxFrameProtocol.ts",
    "src/sdk/pluginBridgeProtocol.ts",
    "src/sdk/pluginClientBridge.ts",
  ] as const;

  /** 去掉块注释与行注释 —— 文档里提到 `@/lib/invoke` 是**说明**，不是依赖。 */
  function stripComments(source: string): string {
    const noBlock = source.replace(/\/\*[\s\S]*?\*\//g, " ");
    return noBlock
      .split("\n")
      .map((line) => {
        const m = /(^|[^:])\/\//.exec(line);
        return m ? line.slice(0, (m.index ?? 0) + (m[1] ? m[1].length : 0)) : line;
      })
      .join("\n");
  }

  /**
   * 取代码里**所有**指向模块的说明符 —— 四种形态一个都不许漏（审查应修 3）。
   *
   * 旧版只匹配行首 `import ... from`，于是
   * `export { invoke } from "@/lib/invoke";`、`await import("@tauri-apps/api/core")`、
   * `require("@/lib/invoke")` 三种形态**全部漏检**（命中 0），而 README 承诺「一添加内部依赖就报红」。
   * 现在覆盖：① 静态 import（含 `import type`）② 副作用 import ③ re-export
   * （`export {a} from` / `export * from` / `export * as ns from`）④ 动态 `import()` 与 `require()`。
   */
  function moduleReferrers(text: string): string[] {
    const code = stripComments(text);
    const patterns = [
      // ① 静态 import（含 import type / import type { X }）
      /(?:^|\n)\s*import(?:\s+type)?\s*(?:[^;'"]*?)\s*from\s*["']([^"']+)["']/g,
      // ② 副作用 import
      /(?:^|\n)\s*import\s*["']([^"']+)["']/g,
      // ③ re-export：具列表 / 星号 / 星号改名，一律带 from
      /(?:^|\n)\s*export(?:\s+type)?\s*(?:\{[^}]*\}|\*(?:\s+as\s+[A-Za-z_$][\w$]*)?)\s*from\s*["']([^"']+)["']/g,
      // ④ 动态 import()（含 `await import(...)`、`return import(...)`）
      /\bimport\s*\(\s*["']([^"']+)["']\s*\)/g,
      // ④ 同一位置的 require()
      /\brequire\s*\(\s*["']([^"']+)["']\s*\)/g,
    ];
    const out: string[] = [];
    for (const source of patterns) {
      const re = new RegExp(source.source, "g");
      let match: RegExpExecArray | null;
      while ((match = re.exec(code)) !== null) {
        out.push(match[1] as string);
      }
    }
    return out;
  }

  /**
   * 更粗的一层兜底：**被禁说明符的字符串字面量**本身不许出现在三件套代码里（已剥注释）。
   *
   * 为什么要有这条：提取器认的是「形态」，把说明符拆成变量、拼串、`const p = "@tauri-apps/…"`
   * 再喂给运行时构造器，就能绕过形态匹配。这条不认形态，只认「这些串有没有出现在代码里」，
   * 代价是它**故意宽** —— 连注释外的任何出现都算，包括字符串常量。注释里的引用仍然豁免，
   * 豁免机制逐字可见：`stripComments` 先把块注释与行注释剥掉，剩下的才算代码。
   */
  const BANNED_SPECIFIER_LITERALS = ["@/lib/invoke", "@tauri-apps", "@/", "tauri-apps/api"] as const;

  function bannedLiterals(text: string): string[] {
    const code = stripComments(text);
    return BANNED_SPECIFIER_LITERALS.filter((banned) => code.includes(banned));
  }

  /** 三件套源码文本（按 VENDORED 顺序）。 */
  function vendoredSources(): Array<{ path: string; code: string }> {
    return VENDORED.map((path) => ({ path, code: readFileSync(resolve(process.cwd(), path), "utf8") }));
  }

  it("三个文件的依赖全部是集合内的相对路径（无宿主 IPC、无别名、无第三方包）", () => {
    const all = vendoredSources().map(({ code }) => moduleReferrers(code));
    for (const [index, specifiers] of all.entries()) {
      for (const spec of specifiers) {
        expect(
          spec,
          `${VENDORED[index]} 引入了集合外的依赖「${spec}」⇒ 第三方 bundle 装不出来`,
        ).toMatch(/^\.\/[A-Za-z0-9_.-]+$/);
      }
    }
    // 分母非零：协议叶子模块合法地没有 import，但客户端与桥协议模块必须有
    expect(all[1].length, "pluginBridgeProtocol.ts 没抓到 import ⇒ 提取器写偏").toBeGreaterThan(0);
    expect(all[2].length, "pluginClientBridge.ts 没抓到 import ⇒ 提取器写偏").toBeGreaterThan(0);
  });

  it("代码里不出现 Tauri 与宿主 IPC 层的模块说明符", () => {
    for (const { code } of vendoredSources()) {
      for (const spec of moduleReferrers(code)) {
        expect(spec).not.toContain("@tauri-apps");
        expect(spec).not.toContain("@/lib/invoke");
        expect(spec).not.toMatch(/^@\//);
      }
    }
  });

  it("兜底面：三件套的代码里连被禁说明符的字面量都不许出现", () => {
    for (const { path, code } of vendoredSources()) {
      expect(
        bannedLiterals(code),
        `${path} 的代码里出现了被禁说明符字面量 ⇒ 无论它被藏成什么形态，第三方都装不出来`,
      ).toEqual([]);
    }
  });

  /**
   * 提取器与兜底面各自**植入必命中**（README「一添加内部依赖就报红」靠这条兑现）。
   *
   * 只扫真实文件会有静默绿：正则写偏 + 消费方恰好没写 ⇒ 门永不红，承诺仍是空话。
   */
  it("可携带性检测面自证：四种形态植入即命中，合法形态不误伤", () => {
    expect(moduleReferrers(`export { invoke } from "@/lib/invoke";`)).toEqual(["@/lib/invoke"]);
    expect(moduleReferrers(`const m = await import("@tauri-apps/api/core");`)).toEqual(["@tauri-apps/api/core"]);
    expect(moduleReferrers(`const m = require("@/lib/invoke");`)).toEqual(["@/lib/invoke"]);
    expect(moduleReferrers(`import { X } from "@/sdk/types";`)).toEqual(["@/sdk/types"]);
    expect(moduleReferrers(`import "./side-effect";`)).toEqual(["./side-effect"]);
    expect(moduleReferrers(`export * as ns from "./rel";`)).toEqual(["./rel"]);
    expect(moduleReferrers(`export type { X } from "./rel";`)).toEqual(["./rel"]);
    // 反例：注释里的引用是说明；集合内的相对 import 是合法形态
    expect(moduleReferrers(`// import { invoke } from "@/lib/invoke";\nconst a = 1;`)).toEqual([]);
    expect(moduleReferrers(`/** 不能依赖 @tauri-apps/api/core */\nimport { X } from "./rel";`)).toEqual(["./rel"]);
    expect(bannedLiterals(`const p = "@tauri-apps/api/core";\nconst d = p;`)).toContain("@tauri-apps");
    expect(bannedLiterals(`/** 说明：不依赖 @/lib/invoke */\nconst a = 1;`)).toEqual([]);
  });

  it("宿主侧确实复用了同一份命令清单（不是插件侧自己维护一套）", () => {
    const host = readFileSync(resolve(process.cwd(), "src/components/plugin/pluginSandboxBridge.ts"), "utf8");
    expect(host).toContain("@/sdk/pluginBridgeProtocol");
    const client = readFileSync(resolve(process.cwd(), "src/sdk/pluginClientBridge.ts"), "utf8");
    expect(client).toContain("./pluginBridgeProtocol");
  });
});
