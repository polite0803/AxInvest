// SPDX-License-Identifier: AGPL-3.0-only

/**
 * SkillSandboxContainer — 统一的 Skill iframe 沙箱容器
 *
 * 所有 Skill UI 都在 sandbox iframe 中运行，通过 postMessage RPC 与宿主通信。
 *
 * 安全特性：
 * - iframe sandbox="allow-scripts"（无 same-origin、无表单、无弹窗）
 * - 权限白名单在 Skill 加载时强制执行
 * - 删除 fetch/XHR 防止网络滥用
 */

import i18n from "@/i18n";
import { invoke, logIpcError } from "@/lib/invoke";
import { createHostApiBridge, createHostRpcBridge } from "@/sdk/rpcBridge";
import type { HostApiBridge, HostRpcBridge } from "@/sdk/rpcBridge";
import {
  isFrameFromSandboxSource,
  isRpcRequestFrame,
  isSandboxContentErrorFrame,
  isSandboxReadyFrame,
  SANDBOX_LOAD_TIMEOUT_MS,
} from "@/sdk/sandboxFrameProtocol";
import { generateSandboxHtml } from "@/sdk/sandboxTemplate";
import type { SkillHostApi, SkillHostStore, SkillHostUi, SkillPermissions } from "@/sdk/types";
import { App as AntdApp, theme as antdTheme } from "antd";
import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { SkillErrorFallback, SkillLoadingSkeleton } from "./SkillErrorFallback";

export interface SkillSandboxContainerProps {
  skillName: string;
  componentId: string;
  componentConfig: Record<string, unknown>;
  permissions?: SkillPermissions;
  style?: React.CSSProperties;
  /**
   * 可选的资产来源注入点 —— 缺省走 `skill_read_asset`。
   *
   * 存在的理由不是复用便利，而是**不复制安全生命周期**：监听先于 `srcdoc`、30s 超时、
   * unmount 清双桥这几段是安全关键路径，插件面板宿主若另写一份，两处漂移的代价不对等。
   * 传空串等异常一律由调用方 throw，容器统一落到 `SkillErrorFallback`。
   */
  loadAsset?: (entry: string) => Promise<string>;
  /**
   * 可选的宿主 API 注入点 —— 缺省为 skill 那套（通用 invoke + 命令白名单）。
   * 插件面板必须传自己的实现：能力面要收窄到该插件的两条通道，
   * 且**由宿主绑定 pluginId**，不接受 iframe 传来的值。
   */
  hostApiOverride?: SkillHostApi;
}

export function SkillSandboxContainer({
  skillName,
  componentId,
  componentConfig,
  permissions,
  style,
  loadAsset,
  hostApiOverride,
}: SkillSandboxContainerProps) {
  const iframeRef = useRef<HTMLIFrameElement>(null);
  const bridgeRef = useRef<HostRpcBridge | null>(null);
  const apiBridgeRef = useRef<HostApiBridge | null>(null);
  const loadTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const retryCountRef = useRef(0);
  const messageHandlerRef = useRef<((event: MessageEvent) => void) | null>(null);
  const rpcHandlerRef = useRef<((event: MessageEvent) => void) | null>(null);

  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // 重试计数：作为 iframe 的 key。同一块 iframe 重复赋同一个 srcdoc 不会重新导航，
  // 只改 state 等下一次 render 会表现为「点了重试毫无反应，又等满 30 s」。
  const [loadNonce, setLoadNonce] = useState(0);

  const { token: themeToken } = antdTheme.useToken();
  const currentTheme: "light" | "dark" = themeToken.colorBgBase === "#ffffff" ? "light" : "dark";
  const navigate = useNavigate();
  // SK-P0-6: 使用 useApp() 获取 notification 实例,继承主题上下文
  const { notification } = AntdApp.useApp();

  const loadSandbox = useCallback(async () => {
    // 重试时先移除上一次绑定的 message 监听，避免监听器累积泄漏
    if (messageHandlerRef.current) {
      window.removeEventListener("message", messageHandlerRef.current);
      messageHandlerRef.current = null;
    }
    if (rpcHandlerRef.current) {
      window.removeEventListener("message", rpcHandlerRef.current);
      rpcHandlerRef.current = null;
    }
    const entry = (componentConfig.entry as string) || "index.html";
    const props = (componentConfig.props as Record<string, unknown>) || {};

    const effectivePermissions: SkillPermissions = {
      commands: permissions?.commands ?? [],
      events: permissions?.events ?? [],
      storeRead: permissions?.storeRead ?? [],
      storeWrite: permissions?.storeWrite ?? [],
      navigate: permissions?.navigate ?? [],
      network: permissions?.network ?? [],
      filesystem: permissions?.filesystem,
      tools: permissions?.tools ?? [],
    };

    const defaultHostApi: SkillHostApi = {
      invoke: async <T = unknown>(
        command: string,
        args?: Record<string, unknown>,
      ): Promise<T> => {
        return invoke<T>(command, args || {});
      },
      emit: (event: string, payload?: unknown): void => {
        window.dispatchEvent(
          new CustomEvent(`skill:${skillName}:${event}`, { detail: payload }),
        );
      },
    };
    const hostApi = hostApiOverride ?? defaultHostApi;

    const hostUi: SkillHostUi = {
      navigate: (path: string): void => {
        navigate(path);
      },
      notify: (
        message: string,
        type: "info" | "success" | "warning" | "error" = "info",
      ): void => {
        // P3-2.25: 对齐 antd NotificationArgsProps：error 类型将 message
        // 作为 description 传递，避免标题截断；其他类型使用 message 作为标题
        if (type === "error") {
          notification.error({
            message: i18n.t("skill.error") || "Error",
            description: message,
            placement: "bottomRight",
          });
        } else {
          notification[type]({ message, placement: "bottomRight" });
        }
      },
      getTheme: (): "light" | "dark" => {
        try {
          const token = antdTheme.getDesignToken?.();
          return token?.colorBgBase === "#ffffff" ? "light" : "dark";
        } catch {
          return "light";
        }
      },
      getLocale: (): string => {
        return i18n.language || "zh-CN";
      },
    };

    const hostStore: SkillHostStore = {
      read: async <T = unknown>(
        storeName: string,
        selector?: string,
      ): Promise<T> => {
        const { getStoreRegistry, initStoreRegistry } = await import("@/lib/storeRegistry");
        await initStoreRegistry();
        const registry = getStoreRegistry();
        const accessor = registry.get(storeName);
        if (!accessor) {
          throw new Error(i18n.t("skill.storeNotFound", { name: storeName }));
        }
        const state = accessor.get() as Record<string, unknown>;
        if (selector) {
          const parts = selector.split(".");
          let result: unknown = state;
          for (const part of parts) {
            if (
              result
              && typeof result === "object"
              && part in (result as Record<string, unknown>)
            ) {
              result = (result as Record<string, unknown>)[part];
            } else {
              return undefined as T;
            }
          }
          return structuredClone(result) as T;
        }
        return structuredClone(state) as T;
      },
      write: async (
        storeName: string,
        value: unknown,
        selector?: string,
      ): Promise<void> => {
        const { getStoreRegistry, initStoreRegistry } = await import("@/lib/storeRegistry");
        await initStoreRegistry();
        const registry = getStoreRegistry();
        const accessor = registry.get(storeName);
        if (!accessor) {
          throw new Error(i18n.t("skill.storeNotWritable", { name: storeName }));
        }
        if (selector && typeof value === "object" && value !== null) {
          const partial: Record<string, unknown> = {};
          const parts = selector.split(".");
          let current = partial;
          for (let i = 0; i < parts.length - 1; i++) {
            current[parts[i]] = {};
            current = current[parts[i]] as Record<string, unknown>;
          }
          current[parts[parts.length - 1]] = value;
          accessor.set(partial);
        } else {
          accessor.set(value);
        }
      },
    };

    setError(null);
    setLoading(true);

    // 加载超时保护
    if (loadTimerRef.current) {
      clearTimeout(loadTimerRef.current);
    }
    loadTimerRef.current = setTimeout(() => {
      setError(i18n.t("skill.timeoutError"));
      setLoading(false);
    }, SANDBOX_LOAD_TIMEOUT_MS);

    try {
      // SK-P0-3: 后端期望参数名 name/file_name,而非 skillName/path
      const htmlContent = loadAsset
        ? await loadAsset(entry)
        : await invoke<string>("skill_read_asset", {
          name: skillName,
          fileName: entry,
        });

      if (!htmlContent || htmlContent.trim().length === 0) {
        throw new Error(i18n.t("skill.entryFileEmpty", { entry }));
      }

      if (bridgeRef.current) {
        bridgeRef.current.destroy();
      }
      if (apiBridgeRef.current) {
        apiBridgeRef.current.destroy();
      }

      const skillId = `${skillName}:${componentId}`;
      const propsJson = JSON.stringify(props);
      const permsJson = JSON.stringify(effectivePermissions);

      const sandboxHtml = generateSandboxHtml({
        skillName,
        skillId,
        props,
        permissions: effectivePermissions,
        htmlContent,
        devMode: import.meta.env.DEV,
      });

      const finalHtml = sandboxHtml
        .replace('"__SKILL_NAME__"', JSON.stringify(skillName))
        .replace('"__SKILL_ID__"', JSON.stringify(skillId))
        .replace("__INITIAL_PROPS__", propsJson)
        .replace("__PERMISSIONS__", permsJson);

      // 先建立 message 监听器（防止 skill:ready 竞态），再设置 srcdoc
      const iframe = iframeRef.current;
      if (!iframe) {
        throw new Error(i18n.t("skill.iframeNotReady"));
      }

      // 注册 message 监听器
      const messageHandler = (event: MessageEvent) => {
        // 身份先于内容：ready 帧会创建两条桥、撤掉 30s 加载超时并切到接请求的模式，
        // 这些副作用不能由别的 iframe（HtmlPreview / ChartPreview / 已导航到远端的沙箱）触发。
        if (!isFrameFromSandboxSource(event, iframe.contentWindow)) {
          return;
        }
        const msg = event.data;
        if (isSandboxReadyFrame(msg)) {
          // iframe 加载完成，发送 mount 生命周期
          const bridge = createHostRpcBridge(iframe.contentWindow!);
          bridgeRef.current = bridge;

          const apiBridge = createHostApiBridge({
            api: hostApi,
            ui: hostUi,
            store: hostStore,
            permissions: effectivePermissions,
            contentWindow: iframe.contentWindow!,
          });
          apiBridgeRef.current = apiBridge;

          bridge.sendLifecycle("mount", props);
          bridge.emitEvent("theme-change", { theme: hostUi.getTheme() });

          if (loadTimerRef.current) {
            clearTimeout(loadTimerRef.current);
            loadTimerRef.current = null;
          }
          setLoading(false);
          retryCountRef.current = 0;

          // 切换到处理请求帧的模式
          window.removeEventListener("message", messageHandler);
          window.addEventListener("message", handleRpc);
        } else if (isSandboxContentErrorFrame(msg)) {
          logIpcError(i18n.t("skill.runtimeError", { name: skillName }))(msg.error);
        }
      };
      messageHandlerRef.current = messageHandler;

      const handleRpc = (event: MessageEvent) => {
        // 来源不符 ⇒ 丢弃（请求帧那一半 `handleRpcRequest` 自己还会判一次：它是安全边界，
        // 容器这一层是为了让 contentError 的日志也不被外来帧刷）
        if (!isFrameFromSandboxSource(event, iframe.contentWindow)) {
          return;
        }
        const msg = event.data;
        if (isRpcRequestFrame(msg) && apiBridgeRef.current) {
          apiBridgeRef.current.handleRpcRequest(event);
        } else if (isSandboxContentErrorFrame(msg)) {
          logIpcError(i18n.t("skill.runtimeError", { name: skillName }))(msg.error);
        }
      };
      rpcHandlerRef.current = handleRpc;

      window.addEventListener("message", messageHandler);
      iframe.srcdoc = finalHtml;
    } catch (e) {
      logIpcError(i18n.t("skill.loadFailed", { name: skillName }))(e);
      setError(String(e));
      setLoading(false);
      if (loadTimerRef.current) {
        clearTimeout(loadTimerRef.current);
        loadTimerRef.current = null;
      }
    }
  }, [
    skillName,
    componentId,
    componentConfig,
    permissions,
    loadAsset,
    hostApiOverride,
    navigate,
    notification,
  ]);

  useEffect(() => {
    // iframe 从第一次提交起就一直在 DOM 里（见文件末尾 render 的注释）：ready 帧只能由
    // 那块文档发出，「加载完才渲染 iframe」会让开闸信号永远不可能到达（#77）。
    // 捕获元素供 cleanup 回收：retry 换 key 时旧 effect 的 cleanup 在新 DOM 提交之后才跑，
    // 那时 iframeRef.current 已经指向新元素，只有闭包里的引用还能清掉旧文档。
    const iframe = iframeRef.current;
    // eslint-disable-next-line react-hooks/set-state-in-effect
    loadSandbox();
    return () => {
      if (messageHandlerRef.current) {
        window.removeEventListener("message", messageHandlerRef.current);
        messageHandlerRef.current = null;
      }
      if (rpcHandlerRef.current) {
        window.removeEventListener("message", rpcHandlerRef.current);
        rpcHandlerRef.current = null;
      }
      if (bridgeRef.current) {
        bridgeRef.current.destroy();
      }
      if (apiBridgeRef.current) {
        apiBridgeRef.current.destroy();
      }
      if (loadTimerRef.current) {
        clearTimeout(loadTimerRef.current);
        loadTimerRef.current = null;
      }
      // 卸载路径 = 沙箱侧文档的回收：先摘监听/拆桥（上面几步），再清内容。
      // 只把 iframe 从 React 树里摘掉不够 —— 某些引擎在元素脱离文档后仍会继续跑旧文档的脚本与定时器。
      if (iframe) {
        iframe.srcdoc = "";
        iframe.removeAttribute("srcdoc");
      }
    };
  }, [loadSandbox, loadNonce]);

  useEffect(() => {
    if (bridgeRef.current) {
      bridgeRef.current.emitEvent("theme-change", { theme: currentTheme });
    }
  }, [currentTheme]);

  const overlayStyle: React.CSSProperties = {
    position: "absolute",
    inset: 0,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    flexDirection: "column",
    gap: 12,
    backgroundColor: "var(--color-bg-container, transparent)",
  };

  // iframe **无条件**渲染：ready 帧只能由它内部那块文档发出，条件挂载 = 开闸信号永远到不了。
  // loading / error 因此只能是叠在它上面的覆盖层，而不是早退替换整棵树（#77）。
  // iframe 绝对铺满外层：尺寸语义收归外层（`...style` 落在那里，面板的 minHeight:160/280 才生效），
  // 覆盖层与 iframe 共用同一个定位盒，不会出现「骨架按外层尺寸、iframe 按自己的 minHeight」两套。
  return (
    <div style={{ position: "relative", width: "100%", minHeight: 400, ...style }}>
      <iframe
        ref={iframeRef}
        // key = 重试计数：换 nonce 就重挂一块新 iframe，srcdoc 在 effect 里重新注入 ⇒ 真正重新导航。
        key={loadNonce}
        title={`Skill: ${skillName}`}
        sandbox="allow-scripts"
        style={{
          position: "absolute",
          inset: 0,
          width: "100%",
          height: "100%",
          border: "none",
          backgroundColor: "transparent",
        }}
      />
      {error && (
        <div style={overlayStyle}>
          <SkillErrorFallback
            error={error}
            skillName={skillName}
            onRetry={() => {
              retryCountRef.current += 1;
              setLoadNonce((n) => n + 1);
            }}
          />
        </div>
      )}
      {loading && !error && (
        <div style={overlayStyle}>
          <SkillLoadingSkeleton />
        </div>
      )}
    </div>
  );
}
