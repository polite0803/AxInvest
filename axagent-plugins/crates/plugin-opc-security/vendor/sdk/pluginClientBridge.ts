// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 插件侧桥**客户端运行时**（page 粒度 = 走 asset 协议装载的第三方 bundle 用的那一份）。
 *
 * 为什么必须有这个文件（`PLAN-shell-plus-plugins.md` §4.4「SDK 发布是终局的硬前置」的具体载体）：
 * 桥的客户端运行时今天只存在于宿主注入的 `srcdoc` 文档模板里（`sandboxTemplate.ts` 生成的那段内联
 * JS）。panel 走 `srcdoc`，宿主可以把运行时一并写进文档；page 走 asset 协议装**插件自己编译的
 * 产物**，宿主不再包 `srcdoc` ⇒ 那份 bundle 里没有任何东西对上宿主 `postMessage` 的协议，
 * 桥对第三方 bundle 不可用。本文件就是补上的那半边。
 *
 * —— 发布形态：**零依赖 TS 源码**（不是编译产物）——
 * 1. 插件产物由插件作者在自己的仓库里编译（vite / rollup / esbuild 均可），源码模块直接进它的
 *    构建图即可，不需要我们这边再挂一条前端打包链，也不需要 npm 发包（仓内无前端 SDK 发布通道）。
 * 2. 源码形态让**类型**一起出境：`invoke()` 的参数类型是 `PluginBridgeCommand` 联合，
 *    第三方在编译期就写不出白名单外的命令名；`PluginSandboxArgs` 让 `pluginId` 在类型层面不可传。
 *    编译产物做不到这条（`.mjs` 无类型 = 白名单只剩运行期一层）。
 * 3.  checked-in 产物会成为**第三份协议副本**，与本批「单一来源」的目标正相反：
 *    协议值只允许住在 `sandboxFrameProtocol.ts` / `pluginBridgeProtocol.ts` 两个文件里。
 *
 * ⇒ **vendored 文件集合就是这三份**（第三方把它们原样复制进自己的仓库，一字不改）：
 *   `src/sdk/sandboxFrameProtocol.ts`、`src/sdk/pluginBridgeProtocol.ts`、`src/sdk/pluginClientBridge.ts`。
 *   「原样复制」可机械校验：下面的 `沙箱客户端可携带性` 用例断言这三个文件的 import **只**指向
 *   集合内部（无 `@/` 别名、无 `@tauri-apps/*`、无 `@/lib/invoke`）—— 一旦有人给本文件加一条
 *   应用内部依赖，该用例立刻红，第三方 bundle 也就不会被一个装不出来的 SDK 骗到。
 *
 * 安全边界（与宿主侧同口径，本文件不扩权）：
 * - 只有 {@link PLUGIN_BRIDGE_COMMANDS} 那两条命令可表达；没有通用的 `call(method, ...)` 出口。
 * - **不发 `pluginId`**：归属由宿主绑定（`pluginSandboxBridge.ts` 的不变量②）。
 *   沙箱侧传了也白传，但本文件仍主动剥掉，理由不是防越权（宿主已经防了），而是不给插件
 *   一个「看起来能指定归属」的假接口 —— 会写错代码的接口比报错的接口危险。
 * - **出向投递**的 targetOrigin 取 `SANDBOX_MESSAGE_TARGET_ORIGIN`：判据本体（为什么恒是 `"*"`、
 *   以及为什么「入向身份」不归 origin 管）只在那一处定义，本文件只引用、不复述。
 *   **入向身份两侧同一条谓词**：宿主接收侧判 `event.source === 沙箱 contentWindow`，
 *   本客户端判 `event.source === 宿主文档 window.parent`，都走
 *   `sandboxFrameProtocol.ts` 的 `isFrameFromSandboxSource`（任务 #78：此前这里只按 `callId`
 *   认领 —— 沙箱文档里若再嵌一块可脚本化文档，猜一个 callId 就能提前 settle 在途 Promise）。
 *   来源判定之后才按 `callId` 认领，且只认自己发出去过的；陌生 callId 一律丢弃。
 * - **`dispose()` 之后不可复用**：新调用立即拒绝、在途调用以「已释放」拒绝、迟到消息不再 settle。
 * - 只暴露 `api.invoke` 一条 RPC 方法：`ui.*` / `store.*` / `api.emit` 对第三方 bundle 不开放
 *   （事件面要独立的白名单，宿主状态是明确的不可越边界）。
 *
 * 与 `srcdoc` 模板的关系：**同一份协议、两条投递路径**，不是第二套 IPC 桥。
 * 两侧都不碰 Tauri IPC —— IPC 只发生在宿主进程侧（`@/lib/invoke`），这正是沙箱的意义。
 *
 * @module sdk/pluginClientBridge
 */

import {
  isPluginBridgeCommand,
  omitHostBoundPluginId,
  PLUGIN_BRIDGE_COMMAND_NAMES,
  PLUGIN_BRIDGE_COMMANDS,
  PLUGIN_UI_ACTION_ARG_KEY,
  type PluginBridgeCommand,
  type PluginSandboxArgs,
} from "./pluginBridgeProtocol";
import {
  buildRpcRequestFrame,
  buildSandboxReadyFrame,
  formatCallId,
  isFailureFrame,
  isFrameFromSandboxSource,
  isRpcResponseFrame,
  readRpcResponseCallId,
  SANDBOX_CALL_ID_PREFIX,
  SANDBOX_FRAME_PROTOCOL_TABLE,
  SANDBOX_MESSAGE_TARGET_ORIGIN,
  SANDBOX_RPC_ARG_KEYS,
  SANDBOX_RPC_METHODS,
  SANDBOX_RPC_TIMEOUT_MS,
  type SandboxRpcRequestFrame,
} from "./sandboxFrameProtocol";

// ── 投递面（宿主文档 = 父窗口） ─────────────────────────────────────

/**
 * 沙箱收到的消息事件（只声明用到的 `data` / `source`）。
 *
 * 刻意不写成 DOM 的 `MessageEvent`：本文件要能被原样 vendored 进任何编译目标，
 * 结构化最小面既贴合真实事件对象，也让测试可以直接喂对象而不必造真事件。
 *
 * `source` 是**发送方文档的 WindowProxy**（`MessageEvent.source`）。它是沙箱侧唯一可用的
 * 身份判据 —— opaque 文档的 `event.origin` 恒为 `"null"`，与任何别的 iframe 分不开。
 * 实测缺口（任务 #78）：只按 `callId` 认领时，沙箱文档里若再嵌一块可脚本化文档，
 * 它凭猜到的 callId 就能把本客户端的在途 Promise 提前 settle。
 */
export interface PluginClientMessageEvent {
  readonly data: unknown;
  readonly source?: unknown;
}

/** 沙箱内容把帧交给宿主的最小面（生产环境即 `window.parent`）。 */
export interface PluginClientPostTarget {
  postMessage(message: unknown, targetOrigin: string): void;
}

/** 沙箱内容接收宿主帧的最小面（生产环境即 `window`）。 */
export interface PluginClientMessageSource {
  addEventListener(type: "message", listener: (event: PluginClientMessageEvent) => void): void;
  removeEventListener(type: "message", listener: (event: PluginClientMessageEvent) => void): void;
}

export interface AxAgentPluginClientOptions {
  /** 帧的投递目标；缺省 `window.parent`。注入点是给测试用的，不是给插件用的。 */
  postTarget?: PluginClientPostTarget;
  /** 宿主帧的接收源；缺省 `window`。同上。 */
  messageSource?: PluginClientMessageSource;
  /**
   * 入向帧的**期望来源**（生产环境即 `window.parent`）。同上：只为测试注入。
   *
   * 与 `postTarget` 不是一回事：前者是「把帧投给谁」，这里是「只认谁发来的」。
   * 缺省取 `window.parent`；取不到（离开文档环境）时一律不认帧 —— 与
   * `isFrameFromSandboxSource` 的「期望来源为空 ⇒ 一帧都不认」同一条边界。
   */
  hostWindow?: unknown;
  /** targetOrigin；缺省见 {@link SANDBOX_MESSAGE_TARGET_ORIGIN}。 */
  targetOrigin?: string;
  /** 单次 RPC 往返超时；缺省见 {@link SANDBOX_RPC_TIMEOUT_MS}。 */
  timeoutMs?: number;
}

/**
 * 交给插件产物的调用面：**只有两条命令**。
 *
 * `invoke` 的命令参数被约束在 `PluginBridgeCommand` 联合上（白名单外的名字编译期就写不出来），
 * 但类型在运行期会被擦除 ⇒ 本文件同时在运行期判一次白名单，**不通过就不发帧**
 * （与宿主不变量③同一口径）。真正的拒绝仍发生在宿主侧：客户端这一层只是让插件作者
 * 当场拿到可定位的错，而不是「发出去再被宿主拒」。
 */
export interface AxAgentPluginClient {
  /** 发起一次白名单命令，返回宿主给的结果；宿主报错（含拒绝）时以 `Error` 拒绝。 */
  invoke<C extends PluginBridgeCommand>(command: C, args?: PluginSandboxArgs): Promise<unknown>;
  /** 把动态 UI 动作回流给**本插件**的 worker（`plugin_ui_action`，后端形参 `action`）。 */
  uiAction(action: unknown): Promise<unknown>;
  /**
   * 按名字调用本插件声明的命令（`plugin_execute_command`）。
   *
   * 参数**原样透传**（只剥 `pluginId`），形状归后端 schema：现状是两个键
   * `name`（命令名）与 `input`（入参），按 `commands/plugin.rs` 的形参 camelCase 校验；
   * 本 SDK **不**把它做成类型约束 —— 该命令的 schema / 权限语义沿「#30 命令通道改真授权 →
   * #49 命令帧」继续演进（`PLAN-shell-plus-plugins.md` §16 还挂着它的后续 #59 / #60），
   * 写死键名就会与后端口径漂移。同一件事在本仓只有一个任务号口径，另见
   * `pluginBridgeProtocol.ts` 里 `PLUGIN_UI_ACTION_ARG_KEY` 的注释。
   */
  executeCommand(args?: PluginSandboxArgs): Promise<unknown>;
  /**
   * 解绑监听并拒绝所有在途调用（组件卸载时调用，避免帧回调打在已卸载的产物上）。
   *
   * **之后不可复用**（任务 #78 的口径）：后续 `invoke` / `uiAction` / `executeCommand`
   * 立即以「已释放」拒绝、在途 Promise 以「已释放」拒绝、迟到消息不再 settle。
   * 需要继续用就新建一个客户端 —— 不提供「reset / 复活」语义。
   */
  dispose(): void;
  /**
   * 向宿主播报「内容已就绪」（`skill:ready` 帧）。
   *
   * 为什么需要显式调用：宿主侧的沙箱容器（`SkillSandboxContainer`）**收到就绪帧才开始接 RPC**，
   * 并配一条 30s 加载超时。panel 走 `srcdoc` 时那段内联运行时自动发这一帧；page 走 asset 协议时
   * 宿主不再包文档，所以这一帧必须由 bundle 自己发 —— 不调用的话容器会一直等到超时并落错误态。
   *
   * 只需在入口模块里调一次（重复调用无害，宿主对 ready 是幂等处理）。
   */
  announceReady(): void;
}

interface PendingCall {
  resolve: (value: unknown) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

/** 白名单命令清单（与宿主侧 `PLUGIN_BRIDGE_COMMANDS` 是同一个数组对象，非副本）。 */
export const AX_AGENT_PLUGIN_BRIDGE_COMMANDS = PLUGIN_BRIDGE_COMMANDS;

/** 把真实父窗口适配到最小投递面（DOM 的 `postMessage` 重载与结构化签名不互通，只在这里接缝一次）。 */
function adaptParentWindow(parent: Window): PluginClientPostTarget {
  return {
    postMessage: (message: unknown, targetOrigin: string): void => {
      parent.postMessage(message, targetOrigin);
    },
  };
}

/** 把真实文档窗口适配到最小接收面（同上：DOM 事件签名与结构化签名的唯一接缝）。 */
function adaptWindowSource(win: Window): PluginClientMessageSource {
  return {
    addEventListener: (type: "message", listener: (event: PluginClientMessageEvent) => void): void => {
      // 结构化最小面与 DOM `EventListener` 的签名不互通（`Event` 上没有 `data`），
      // 这里只做一次「双 cast」接缝：真实事件对象一定带 `data`，接口的窄化是故意的。
      win.addEventListener(type, listener as unknown as EventListener);
    },
    removeEventListener: (type: "message", listener: (event: PluginClientMessageEvent) => void): void => {
      win.removeEventListener(type, listener as unknown as EventListener);
    },
  };
}

/**
 * 创建一个插件侧桥客户端。
 *
 * 用法（插件 bundle 内）：
 * ```ts
 * import { createAxAgentPluginClient } from "./sdk/pluginClientBridge";
 *
 * const plugin = createAxAgentPluginClient();
 * const result = await plugin.uiAction({ type: "refresh" });
 * await plugin.executeCommand({ name: "greet", input: { 名字: "世界" } });
 * ```
 *
 * ⚠ 宿主未开始接帧时，请求会等到超时后拒绝，不会静默悬挂 —— 超时值是协议表里的同一条，两侧一致。
 */
export function createAxAgentPluginClient(options: AxAgentPluginClientOptions = {}): AxAgentPluginClient {
  const globalWindow = typeof window === "undefined" ? undefined : window;
  const rawTarget = options.postTarget ?? (globalWindow ? adaptParentWindow(globalWindow.parent) : undefined);
  const rawSource = options.messageSource ?? (globalWindow ? adaptWindowSource(globalWindow) : undefined);
  if (!rawTarget || !rawSource) {
    // 明确报错而不是造一个「在 node 里也假装能跑」的空转客户端：
    // 这条链只在沙箱文档里成立，离开文档就是接入方式错了，越早说越好。
    throw new Error(
      "createAxAgentPluginClient: 需要浏览器窗口环境（window.parent / window）或显式注入 postTarget 与 messageSource",
    );
  }
  // 守卫之后固定成非空局部量：闭包（request / dispose）里用这两个，避免可空类型穿透
  const target: PluginClientPostTarget = rawTarget;
  const source: PluginClientMessageSource = rawSource;

  const targetOrigin = options.targetOrigin ?? SANDBOX_MESSAGE_TARGET_ORIGIN;
  const timeoutMs = options.timeoutMs ?? SANDBOX_RPC_TIMEOUT_MS;
  // 入向期望来源：生产环境恒为宿主文档 = window.parent（与 srcdoc 运行时的
  // HOST_SOURCE_WINDOW 同一个对象身份）。注入点只为测试与 SSR 侧，容器/插件不要传。
  const hostWindow: unknown = options.hostWindow ?? (globalWindow ? globalWindow.parent : undefined);
  const pending = new Map<string, PendingCall>();
  let sequence = 0;
  let disposed = false;

  function handleMessage(event: PluginClientMessageEvent): void {
    // dispose 之后一律拒绝（任务 #78）。监听器此刻通常已被摘掉，但摘监听不是唯一通路：
    // 宿主/浏览器仍可能把已排队的消息递给被捕获的 listener 引用 —— 这一层兜住它，
    // 在途 Promise 已在 dispose 时以「已释放」拒绝，迟到帧不得再 settle 任何东西。
    if (disposed) {
      return;
    }
    // 身份先于内容：与宿主接收侧、srcdoc 运行时同一条谓词（sandboxFrameProtocol.ts）。
    // 沙箱内再嵌一块可脚本化文档时，它伪造的同 callId 响应帧在这里被丢弃，
    // 不会提前 settle 本客户端的在途 Promise。
    if (!isFrameFromSandboxSource({ data: event.data, source: event.source }, hostWindow)) {
      return;
    }
    const frame = event.data;
    if (!isRpcResponseFrame(frame)) {
      return;
    }
    const callId = readRpcResponseCallId(frame);
    if (!callId) {
      return;
    }
    const entry = pending.get(callId);
    if (!entry) {
      // 陌生 callId：不属于本客户端发起的任一在途调用（宿主重启 / 已超时 / 别的实例），丢弃。
      return;
    }
    clearTimeout(entry.timer);
    pending.delete(callId);
    // 空串也判失败（审查应修 2）：判据是「error 是不是字符串」，三通道共用协议层那一条
    if (isFailureFrame(frame)) {
      entry.reject(new Error(frame.error));
    } else {
      entry.resolve(frame.result);
    }
  }

  source.addEventListener("message", handleMessage);

  function request(command: PluginBridgeCommand, args?: Record<string, unknown>): Promise<unknown> {
    if (disposed) {
      return Promise.reject(new Error("插件桥客户端已释放（dispose 后不可再调用）"));
    }
    const callId = formatCallId(SANDBOX_CALL_ID_PREFIX.plugin, ++sequence);
    // 不变量②的客户端那一半：帧里不出现 pluginId（宿主那一半见 pluginSandboxBridge.ts）。
    const envelope: Record<string, unknown> = { [SANDBOX_RPC_ARG_KEYS.command]: command };
    if (args !== undefined) {
      envelope[SANDBOX_RPC_ARG_KEYS.args] = omitHostBoundPluginId(args);
    }
    const frame: SandboxRpcRequestFrame = buildRpcRequestFrame(
      callId,
      SANDBOX_RPC_METHODS.apiInvoke,
      envelope,
    );

    return new Promise<unknown>((resolve, reject) => {
      const timer = setTimeout(() => {
        pending.delete(callId);
        reject(new Error(`插件桥调用「${command}」超时（${timeoutMs}ms 未收到宿主响应）`));
      }, timeoutMs);
      pending.set(callId, { resolve, reject, timer });

      try {
        target.postMessage(frame, targetOrigin);
      } catch (error) {
        clearTimeout(timer);
        pending.delete(callId);
        reject(error instanceof Error ? error : new Error(String(error)));
      }
    });
  }

  return {
    invoke: (command, args) => {
      // 运行期再判一次白名单：泛型约束 `C extends PluginBridgeCommand` 只挡编译期，
      // 类型在运行期被擦除，纯 JS 的第三方 bundle 传什么都不拦。
      // 不通过就**不发帧**（与宿主不变量③同一口径：拒绝发生在投递之前，
      // 不是「发出去再被宿主拒」—— 那样一个错误命令名会在宿主侧留下无意义的拒绝日志，
      // 插件作者收到的仍是「桥坏了」而不是「你用错了命令」）。
      if (!isPluginBridgeCommand(command)) {
        return Promise.reject(
          new Error(
            `插件桥白名单外的命令「${String(command)}」：可用清单见 AX_AGENT_PLUGIN_BRIDGE_COMMANDS，`
              + `新增通道需改 pluginBridgeProtocol.ts 并经宿主侧评审`,
          ),
        );
      }
      return request(command, args);
    },
    uiAction: (action) =>
      request(PLUGIN_BRIDGE_COMMAND_NAMES.uiAction, {
        [PLUGIN_UI_ACTION_ARG_KEY]: action,
      }),
    executeCommand: (args) => request(PLUGIN_BRIDGE_COMMAND_NAMES.executeCommand, args),
    announceReady(): void {
      if (disposed) {
        return;
      }
      // 就绪帧无业务字段：宿主据此才开始接 RPC（与 srcdoc 模板里的自动上报同一步）
      // ⚠ 帧在 try 之外构造：只有 postMessage 本身允许被吞（iframe 已脱离文档），
      //   编码错误若也被静默掉，症状就是「宿主永远等不到 ready」这种最难查的形态。
      const frame = buildSandboxReadyFrame();
      try {
        target.postMessage(frame, targetOrigin);
      } catch {
        // iframe 已脱离文档：没有可通知的宿主，静默即可（后续请求会各自报错）
      }
    },
    dispose(): void {
      disposed = true;
      source.removeEventListener("message", handleMessage);
      for (const entry of pending.values()) {
        clearTimeout(entry.timer);
        entry.reject(new Error("插件桥客户端已释放（dispose）"));
      }
      pending.clear();
    },
  };
}

/**
 * 沙箱内可用的协议表（与 `srcdoc` 模板注入的是**同一个对象**）。
 *
 * 存在的理由只有一个：插件产物若要手写帧（例如自己实现流式协议），必须与宿主同源。
 * 常规用法请走 {@link createAxAgentPluginClient}，不要拿这张表自己拼帧。
 */
export const AX_AGENT_PLUGIN_PROTOCOL_TABLE = SANDBOX_FRAME_PROTOCOL_TABLE;
