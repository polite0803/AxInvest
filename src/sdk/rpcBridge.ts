// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Skill RPC Bridge — 宿主侧
 *
 * 负责管理宿主与 Skill iframe 沙箱之间的 postMessage 通信。
 * 提供请求-响应模式（Promise-based）的 RPC 调用。
 *
 * ⚠ 帧词汇（类型串、方法名、参数字段名、超时、错误承载形状）**不在本文件**，
 * 一律取自 `./sandboxFrameProtocol`（任务 #46 的单一来源收敛）。本文件只负责
 * 「宿主收到帧后做什么」：来源判定、权限判定、handler 分派、响应回发。
 * 两个方向各用哪个值判「这帧可以收 / 该投给谁」也只在那个字面定义处写：
 * 出向 targetOrigin 恒取 `SANDBOX_MESSAGE_TARGET_ORIGIN`，入向身份只认
 * `isFrameFromSandboxSource(event, 本桥的 contentWindow)`。
 * 插件内容（panel / page 两种粒度）在这条桥上能调哪两条命令，白名单住在
 * `@/components/plugin/pluginSandboxBridge.ts` + `@/sdk/pluginBridgeProtocol.ts`。
 *
 * @module sdk/rpcBridge
 */

import { logIpcError } from "@/lib/invoke";
import { isStoreReadCovered, isStoreWriteCovered, isWildcardMatch } from "@/lib/skillPermissions";
import {
  buildHostEventFrame,
  buildHostLifecycleFrame,
  buildRpcErrorFrame,
  buildRpcRequestFrame,
  buildRpcResultFrame,
  formatCallId,
  isFailureFrame,
  isFrameFromSandboxSource,
  isRpcRequestFrame,
  isRpcResponseFrame,
  isSandboxContentErrorFrame,
  isSandboxReadyFrame,
  readRpcResponseCallId,
  rpcErrorText,
  SANDBOX_CALL_ID_PREFIX,
  SANDBOX_FRAME_PROTOCOL_TABLE,
  SANDBOX_MESSAGE_TARGET_ORIGIN,
  SANDBOX_RPC_METHODS,
  SANDBOX_RPC_TIMEOUT_MS,
  type SandboxInboundMessageEvent,
} from "./sandboxFrameProtocol";
import type { HostToSkillMessage, SkillHostApi, SkillHostStore, SkillHostUi, SkillToHostMessage } from "./types";

/**
 * RPC 调用超时时间（毫秒）—— 取自帧协议的单一来源。
 *
 * 宿主与沙箱两侧此前各写一份 `15000`：数值巧合不等于语义同源，收成一条之后
 * 「宿主等 15s、沙箱等 20s」这类口径差无处再长出来。
 */
const RPC_TIMEOUT_MS = SANDBOX_RPC_TIMEOUT_MS;

/**
 * 方法参数帧的键名表（宿主 handler 逐个按这些键读参数，沙箱侧按同一张表构造）。
 *
 * 帧本体的字段名由 `Sandbox*Frame` 接口钉住，**参数帧没有接口保护**，是漂移概率最高的一层，
 * 故一律走常量取键。
 */
const ARG = SANDBOX_FRAME_PROTOCOL_TABLE.a;

// ── 宿主侧 RPC 注册表 ──

/** 宿主侧 RPC 管理器 */
export interface HostRpcBridge {
  /** 发送消息到 Skill 沙箱 */
  sendMessage(message: HostToSkillMessage): void;
  /** 调用 Skill 内注册的 RPC 方法 */
  callSkillMethod(
    method: string,
    args?: Record<string, unknown>,
  ): Promise<unknown>;
  /** 发送事件到 Skill 沙箱 */
  emitEvent(event: string, payload?: unknown): void;
  /** 通知 Skill 沙箱生命周期 */
  sendLifecycle(
    phase: "mount" | "unmount",
    props?: Record<string, unknown>,
  ): void;
  /** 销毁桥接 */
  destroy(): void;
}

/**
 * 创建宿主侧 RPC 桥接
 *
 * @param contentWindow Skill iframe 的 contentWindow —— 它同时是**入向帧的身份判据**
 *   （只有 `event.source` 就是它的帧才被认，见 `isFrameFromSandboxSource`）
 * @returns RPC 桥接实例
 */
export function createHostRpcBridge(
  contentWindow: Window | null,
): HostRpcBridge {
  const pendingCalls = new Map<
    string,
    {
      resolve: (value: unknown) => void;
      reject: (error: Error) => void;
      timer: ReturnType<typeof setTimeout>;
    }
  >();
  let callIdCounter = 0;
  let destroyed = false;

  function handleMessage(event: MessageEvent<SkillToHostMessage>) {
    if (destroyed) {
      return;
    }
    // 身份先于内容：别的 iframe（HtmlPreview / ChartPreview / 已导航到远端的沙箱）同样能把帧
    // 投到宿主 window，来源不符直接不看它写了什么。
    if (!isFrameFromSandboxSource(event, contentWindow)) {
      return;
    }

    const msg = event.data;
    if (isSandboxReadyFrame(msg)) {
      // no-op in production; DEV-only logging removed
    } else if (isSandboxContentErrorFrame(msg)) {
      logIpcError("HostRpcBridge skill error")(msg.error);
    }
  }

  // 仅在非测试环境添加监听器
  if (typeof window !== "undefined") {
    window.addEventListener("message", handleMessage);
  }

  return {
    sendMessage(message: HostToSkillMessage): void {
      if (destroyed || !contentWindow) {
        return;
      }
      try {
        contentWindow.postMessage(message, SANDBOX_MESSAGE_TARGET_ORIGIN);
      } catch (e) {
        logIpcError("HostRpcBridge send message failed")(e);
      }
    },

    async callSkillMethod(
      method: string,
      args?: Record<string, unknown>,
    ): Promise<unknown> {
      if (destroyed) {
        throw new Error("Bridge is destroyed");
      }
      if (!contentWindow) {
        throw new Error("No content window");
      }

      const callId = formatCallId(SANDBOX_CALL_ID_PREFIX.host, ++callIdCounter);

      return new Promise<unknown>((resolve, reject) => {
        const responseHandler = (event: MessageEvent<HostToSkillMessage>) => {
          // 响应帧的 callId 是关联键，但「谁发的」仍要判：陌生 source 带着猜得到的 callId
          // 就能把在途 Promise 提前 settle 掉（宿主→沙箱方向的注入面就这一条）。
          if (!isFrameFromSandboxSource(event, contentWindow)) {
            return;
          }
          const msg = event.data;
          if (isRpcResponseFrame(msg) && readRpcResponseCallId(msg) === callId) {
            window.removeEventListener("message", responseHandler);
            const pending = pendingCalls.get(callId);
            if (!pending) {
              return;
            }
            clearTimeout(pending.timer);
            pendingCalls.delete(callId);
            // 成功/失败只问协议层的唯一判据（空串错误同样是失败）
            if (isFailureFrame(msg)) {
              pending.reject(new Error(msg.error));
            } else {
              pending.resolve(msg.result);
            }
          }
        };

        const timer = setTimeout(() => {
          window.removeEventListener("message", responseHandler);
          pendingCalls.delete(callId);
          reject(
            new Error(
              `RPC call "${method}" timed out after ${RPC_TIMEOUT_MS}ms`,
            ),
          );
        }, RPC_TIMEOUT_MS);

        pendingCalls.set(callId, { resolve, reject, timer });

        // 向沙箱发起一次 RPC（与沙箱→宿主方向共用同一个请求帧构造器）
        const requestMsg = buildRpcRequestFrame(callId, method, args);
        try {
          contentWindow.postMessage(requestMsg, SANDBOX_MESSAGE_TARGET_ORIGIN);
        } catch (e) {
          window.removeEventListener("message", responseHandler);
          clearTimeout(timer);
          pendingCalls.delete(callId);
          reject(e);
        }
      });
    },

    emitEvent(event: string, payload?: unknown): void {
      this.sendMessage(buildHostEventFrame(event, payload));
    },

    sendLifecycle(
      phase: "mount" | "unmount",
      props?: Record<string, unknown>,
    ): void {
      this.sendMessage(buildHostLifecycleFrame(phase, props));
    },

    destroy(): void {
      destroyed = true;
      // 清理待处理请求
      for (const [, pending] of pendingCalls) {
        clearTimeout(pending.timer);
        pending.reject(new Error("Bridge destroyed"));
      }
      pendingCalls.clear();
      if (typeof window !== "undefined") {
        window.removeEventListener("message", handleMessage);
      }
    },
  };
}

// ── 宿主 API 桥接（Skill 调用宿主方法时使用） ──

export interface HostApiBridge {
  /**
   * 处理来自沙箱的一次 `message` 事件。
   *
   * ⚠ 入参是**整个事件**而不是裸帧：宿主必须能判「这帧来自哪块文档」，
   * 判据与校验点见 `isFrameFromSandboxSource`。来源不符 ⇒ 丢弃且**不回包**
   * （回包本身就是「外来帧拿到了宿主的应答」这一条信息）。
   */
  handleRpcRequest(event: SandboxInboundMessageEvent): void;
  /** 获取上下文对象（传给沙箱模板使用） */
  getContext(): {
    api: SkillHostApi;
    ui: SkillHostUi;
    store: SkillHostStore;
  };
  /** 销毁 */
  destroy(): void;
}

export interface HostApiBridgeOptions {
  /** 宿主提供的 API 实现 */
  api: SkillHostApi;
  /** 宿主提供的 UI 实现 */
  ui: SkillHostUi;
  /** 宿主提供的 Store 实现 */
  store: SkillHostStore;
  /** Skill 的权限声明 */
  permissions: {
    commands?: string[];
    events?: string[];
    storeRead?: string[];
    storeWrite?: string[];
    navigate?: string[];
  };
  /**
   * Skill 的 contentWindow：既用于发送响应，也是**入向帧的身份判据**
   * （`event.source` 必须就是它）。传 `null` 即「还没接上沙箱」，此时一帧都不认。
   */
  contentWindow: Window | null;
}

/**
 * 创建宿主 API 桥接（Skill 侧调用宿主方法）
 */
export function createHostApiBridge(
  options: HostApiBridgeOptions,
): HostApiBridge {
  const { api, ui, store, permissions, contentWindow } = options;
  let destroyed = false;

  function sendResponse(
    callId: string,
    result?: unknown,
    error?: string,
  ): void {
    if (destroyed || !contentWindow) {
      return;
    }
    // 成功与失败分别走协议里唯一的构造函数：「error 缺省即成功」这条边界只有一处定义
    // （旧写法把两个键同时塞进一帧、靠真值判断，`error: ""` 与 `error: undefined` 语义不同，
    //  但没人写得出来 —— 因为构造入口只有这两个）。
    const frame = error === undefined
      ? buildRpcResultFrame(callId, result)
      : buildRpcErrorFrame(callId, error);
    // 出向 targetOrigin 恒取协议常量；投递对象已经由 contentWindow + 来源判定钉死，
    // 判据本体见 sandboxFrameProtocol 的 SANDBOX_MESSAGE_TARGET_ORIGIN。
    contentWindow.postMessage(frame, SANDBOX_MESSAGE_TARGET_ORIGIN);
  }

  function checkCommand(command: string): boolean {
    return isWildcardMatch(command, permissions.commands ?? []);
  }

  function checkEvent(event: string): boolean {
    return isWildcardMatch(event, permissions.events ?? []);
  }

  function checkStoreRead(storeName: string, selector?: string): boolean {
    return isStoreReadCovered(storeName, selector, permissions.storeRead ?? []);
  }

  function checkStoreWrite(storeName: string, selector?: string): boolean {
    return isStoreWriteCovered(
      storeName,
      selector,
      permissions.storeWrite ?? [],
    );
  }

  function checkNavigate(path: string): boolean {
    return isWildcardMatch(path, permissions.navigate ?? []);
  }

  /**
   * 宿主接受的 RPC 方法表。
   *
   * ⚠ 键一律从 `SANDBOX_RPC_METHODS` 取，**不写方法名字面量**：字面量住在这里就是第二份协议，
   * 沙箱侧改名的时候这里不会跟着红。测试拿 `Object.keys` 与实际收到的帧做对称差集
   * （`src/sdk/__tests__/sandboxFrameProtocol.test.ts`），本表即「宿主侧能力面」的唯一答案。
   */
  const rpcHandlers: Record<
    string,
    (args?: Record<string, unknown>) => Promise<unknown>
  > = {
    // ctx.api
    [SANDBOX_RPC_METHODS.apiInvoke]: async (args) => {
      const command = args?.[ARG.command] as string;
      if (!command) {
        throw new Error("Command name is required");
      }
      if (!checkCommand(command)) {
        throw new Error(
          `Permission denied: command "${command}" is not allowed`,
        );
      }
      return api.invoke(
        command,
        args?.[ARG.args] as Record<string, unknown> | undefined,
      );
    },
    [SANDBOX_RPC_METHODS.apiEmit]: async (args) => {
      const event = args?.[ARG.event] as string;
      if (!event) {
        throw new Error("Event name is required");
      }
      if (!checkEvent(event)) {
        throw new Error(`Permission denied: event "${event}" is not allowed`);
      }
      api.emit(event, args?.[ARG.payload]);
      return undefined;
    },

    // ctx.ui
    [SANDBOX_RPC_METHODS.uiNavigate]: async (args) => {
      const path = args?.[ARG.path] as string;
      if (!path) {
        throw new Error("Path is required");
      }
      if (!checkNavigate(path)) {
        throw new Error(`Permission denied: navigate "${path}" is not allowed`);
      }
      ui.navigate(path);
      return undefined;
    },
    [SANDBOX_RPC_METHODS.uiNotify]: async (args) => {
      ui.notify(
        (args?.[ARG.message] as string) ?? "",
        (args?.[ARG.type] as "info" | "success" | "warning" | "error") ?? "info",
      );
      return undefined;
    },
    [SANDBOX_RPC_METHODS.uiGetTheme]: async () => {
      return ui.getTheme();
    },
    [SANDBOX_RPC_METHODS.uiGetLocale]: async () => {
      return ui.getLocale();
    },

    // ctx.store
    [SANDBOX_RPC_METHODS.storeRead]: async (args) => {
      const storeName = args?.[ARG.storeName] as string;
      if (!storeName) {
        throw new Error("Store name is required");
      }
      const selector = args?.[ARG.selector] as string | undefined;
      if (!checkStoreRead(storeName, selector)) {
        const hint = selector
          ? `cannot read store "${storeName}" field "${selector}"`
          : `cannot read store "${storeName}" (field-level permission required)`;
        throw new Error(`Permission denied: ${hint}`);
      }
      return store.read(storeName, selector);
    },
    [SANDBOX_RPC_METHODS.storeWrite]: async (args) => {
      const storeName = args?.[ARG.storeName] as string;
      if (!storeName) {
        throw new Error("Store name is required");
      }
      const selector = args?.[ARG.selector] as string | undefined;
      if (!checkStoreWrite(storeName, selector)) {
        const hint = selector
          ? `cannot write store "${storeName}" field "${selector}"`
          : `cannot write store "${storeName}" (field-level permission required)`;
        throw new Error(`Permission denied: ${hint}`);
      }
      await store.write(storeName, args?.[ARG.value], selector);
      return undefined;
    },
  };

  return {
    handleRpcRequest(event: SandboxInboundMessageEvent): void {
      if (destroyed) {
        return;
      }
      // 身份先于内容：来源不是本桥对接的那块沙箱 ⇒ 丢弃，且不回包（判据见 isFrameFromSandboxSource）。
      // 这一判据长在宿主自己身上，不依赖调用方先筛过 —— 应用内任何可脚本化的 iframe
      // 都能对宿主 window 发一帧看起来完全合法的 rpc:request。
      if (!isFrameFromSandboxSource(event, contentWindow)) {
        return;
      }
      const msg = event.data;
      // 类型收窄由协议 guard 负责（不在这里比对帧类型字面量）
      if (!isRpcRequestFrame(msg)) {
        return;
      }
      const handler = rpcHandlers[msg.method];
      if (!handler) {
        sendResponse(msg.callId, undefined, `Unknown method: ${msg.method}`);
        return;
      }
      handler(msg.args)
        .then((result) => sendResponse(msg.callId, result))
        .catch((e) => sendResponse(msg.callId, undefined, rpcErrorText(e)));
    },

    getContext() {
      return { api, ui, store };
    },

    destroy() {
      destroyed = true;
    },
  };
}
