// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 插件**页面容器**本体 —— page 粒度（走 asset 协议装载插件自带 bundle）的宿主侧那一份
 * （任务 #64 / `PLAN-shell-plus-plugins.md` §20.4 E2）。
 *
 * 一句话职责：拿到 `(pluginId, page)` 后按 `page.loading` 分给两条**既有**通道 ——
 * `schema` 交动态 UI，`bundle` 交后端解析出的绝对路径 → asset URL → 沙箱 iframe；
 * 桥与命令白名单沿用 panel 那两个构造函数，**不存在第二套 IPC 桥**。
 *
 * ── 三条硬约束（判据本体不在此复述，只标点名落点） ─────────────────────────────
 *
 * 1. **分支判据只取 `page.loading`**（§20.4 E2）。
 *    - `schema` → 动态 UI 那条既有通道：`DynamicUIStandalone` 内部就是
 *      `useDynamicUIStore.getSchema` + `DynamicUIRenderer`，与 `DynamicPageViewer` 同源；
 *      动作回流也走 `usePluginStore.runUiAction`（同 DynamicPageViewer:88-94 那段）。
 *      ⚠ 这一支**绝不调 `plugin_resolve_page_asset`** —— schema 页的 `entry` 是本插件 `ui.json`
 *      的 key（不是文件路径，也不是行 id：行 id 由 [`pluginSchemaRowId`] 在宿主侧拼全），
 *      后端 `manager.rs` 那条解析路对它必报错。拿解析命令当「通用取内容」是这里最容易
 *      犯的错，而且症状是「schema 页永远报错」而不是「分支写错了」。
 *    - `bundle` → `plugin_resolve_page_asset(pluginId, pageId)` → `absolutePath` → `assetUrl()` → iframe `src`。
 *    - `bundle` 分支的 `page.entry` **只用于展示与排错，禁止参与路径拼接**：安装目录包含性 +
 *      `assetProtocol.scope` 两道防线只在后端那一步解析时成立，前端拼串等于把它们绕过去
 *      （`PluginPageDto` 的寻址不变量，与 panel 粒度是同一条）。
 *
 * 2. **`sandbox="allow-scripts"`，绝不加 `allow-same-origin`**。加上就等于把插件代码放进主 origin ——
 *    它能读 localStorage 里的 provider key 与会话 token。作为交换，文档是 opaque origin：宿主只能
 *    经 postMessage 与之通信，「出向 targetOrigin 恒 `"*"`、入向只认 `event.source`」由此成立
 *    （两处唯一定义：`SANDBOX_MESSAGE_TARGET_ORIGIN`、`isFrameFromSandboxSource`）。
 *
 * 3. **ready 是唯一开闸信号，且 `announceReady()` / `dispose()` 是硬要求而不是建议**（§20.4 A/B）。
 *    这两步发生在**插件 bundle 那一侧**（本容器无法代劳），判据写给下一个人看，因为两者的症状都会
 *    把人引到错误的方向上：
 *    - bundle 入口没调 `announceReady()` ⇒ 本容器收不到就绪帧，停在 loading，
 *      `SANDBOX_LOAD_TIMEOUT_MS` 后落 `skill.timeoutError`（「页面白一块并报错」）。
 *      ⚠ **不是「桥不通」**：桥根本没建，IPC 一次都没发 —— 先查产物入口，别去查后端。
 *      生产用法是 `createAxAgentPluginClient()` **不传参** + 顶层求值即创建 + 随即
 *      `announceReady()`；`postTarget/messageSource/timeoutMs/targetOrigin` 是测试注入点，
 *      容器与产物都别传（传了就是把测试替身投进真实投递面）。
 *    - bundle 卸载时没调 `dispose()` ⇒ 该文档内的 `message` 监听器与在途 `setTimeout` 不回收，
 *      泄漏按重试次数累计。本容器卸载路径这一侧对应做的是拆桥 + 摘监听 + 清超时（见下方 cleanup），
 *      **同一处**：宿主拆桥 ⇄ 沙箱释放客户端，两边都必须发生，否则一边留监听、一边留 Promise。
 *
 * ── 为什么不复用 `SkillSandboxContainer`（srcdoc 那条通道的容器） ────────────────
 *
 * - 装载形态在「内容怎么进 webview」这一步就没有共同实现可抽：那边由宿主生成 srcdoc 文档并注入
 *   props/permissions/运行时，这边的文档是插件自己的产物，宿主只提供一个 URL。
 * - 组件中立性门禁（AGENTS「组件中立性规范」）禁止 `src/components/plugin/**` import
 *   `@/components/skill/**`（棘轮基线只减不增）⇒ 跨目录复用这条路本仓明确堵着；要通就得把容器
 *   下沉到 `components/common/`，而那会让 PLAN 里按路径钉着的文档锚点成批腐烂（docs 是禁改面）
 *   ⇒ 那是 leader 的一次独立决策，不是本任务的顺手改动。
 * - 真正需要同源的部分**已经同源**，没有第二份：来源判据 `isFrameFromSandboxSource`、桥
 *   `createHostApiBridge`、命令面 `createPluginBridgeApi` + `createPluginBridgePermissions`、
 *   装载超时 `SANDBOX_LOAD_TIMEOUT_MS`（本批新收的单点，panel 与 page 共用）。
 *   剩下的只有 React 侧的监听/计时接线 —— **改了 srcdoc 容器那两处，请回来复核本文件同名段落**。
 *
 * ⚠ 列表侧（「某包有哪些 pages[]」）由 `plugin_list_pages` 提供（任务 #75）：该投影实时扫描
 * `PluginManifest.pages`，本容器只接「调用方已经拿着 `PluginPageDto`」的形态，**不**自建
 * registry / 平行 manifest / 路由表（禁区 4、§0.7.6 的教训）。
 */

import { DynamicUIStandalone } from "@/components/dynamicUI/DynamicUIStandalone";
import { SchemaIdContext } from "@/components/dynamicUI/SchemaIdContext";
import { createPluginBridgeApi, createPluginBridgePermissions } from "@/components/plugin/pluginSandboxBridge";
import i18n from "@/i18n";
import { translateBackendError } from "@/lib/errorI18n";
import { assetUrl, invoke, logIpcError } from "@/lib/invoke";
import { pluginSchemaRowId } from "@/lib/pluginUiSchema";
import { createHostApiBridge, type HostApiBridge } from "@/sdk/rpcBridge";
import {
  isFrameFromSandboxSource,
  isRpcRequestFrame,
  isSandboxContentErrorFrame,
  isSandboxReadyFrame,
  SANDBOX_LOAD_TIMEOUT_MS,
} from "@/sdk/sandboxFrameProtocol";
import type { SkillHostStore, SkillHostUi, SkillPermissions } from "@/sdk/types";
import { usePluginStore } from "@/stores";
import type { DynamicAction, PluginPageAssetLocationDto, PluginPageDto } from "@/types";
import { Alert, Button, Spin, theme as antdTheme } from "antd";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

/**
 * 页面粒度的权限面只开那两条命令（`createPluginBridgePermissions`），`store.*` / `ui.navigate`
 * 在桥的 handler 表里先过权限判定 ⇒ 下面这些实现**不可达**。
 * 之所以写成抛错而不是空函数：空函数会把「权限面哪天被扩了」变成一次静默的无响应；
 * 抛错则是当场可定位的信号 —— 那时该讨论的是要不要给页面开 store 面，不是这里悄悄返回 undefined。
 */
const PAGE_ONLY_COMMANDS_MESSAGE =
  "PluginPageHost：页面粒度的权限面只开两条命令，此分支不可达；走到这里说明权限面被扩过，请回到 pluginSandboxBridge.ts 显式决定";

export interface PluginPageHostProps {
  /** 宿主绑定的归属：`pluginId` 由调用方给出，iframe 传来的同名字段一律被桥丢弃。 */
  pluginId: string;
  /**
   * 页面声明本身（`manifest.pages[]` 的一项）。分支只读 `loading`：
   * `schema` ⇒ `entry` 是本插件 `ui.json` 的 key（宿主拼全行 id）；
   * `bundle` ⇒ `entry` 只用于展示，禁止参与路径拼接。
   */
  page: PluginPageDto;
  /** 仅用于展示与排错（iframe title、运行时错误日志的归属名）；缺省回落到 `pluginId`。 */
  pluginName?: string;
  style?: React.CSSProperties;
}

/**
 * 插件贡献页面的容器入口。
 *
 * 这里刻意**不放任何 hook**：分支判据只有一个（`page.loading`），早退就不会出现
 * 「同一组件两次渲染走了不同分支导致 hook 顺序变化」那种形态。
 */
export function PluginPageHost({ pluginId, page, pluginName, style }: PluginPageHostProps) {
  if (page.loading === "schema") {
    return <PluginSchemaPage pluginId={pluginId} page={page} style={style} />;
  }
  return <PluginBundlePage pluginId={pluginId} page={page} pluginName={pluginName} style={style} />;
}

/** `loading: "schema"` —— 动态 UI 那条既有通道，容器不碰解析命令也不碰文件系统。 */
function PluginSchemaPage(
  { pluginId, page, style }: { pluginId: string; page: PluginPageDto; style?: React.CSSProperties },
) {
  const runUiAction = usePluginStore((s) => s.runUiAction);

  // 动作回流到本插件自己的 worker —— 与 DynamicPageViewer 的插件来源分支同一件事，
  // 只是这里 `pluginId` 由调用方直接给出（那条是从 schema 记录的 ownerId 推出来的）。
  const handleAction = useCallback(
    (action: DynamicAction): void => {
      void runUiAction(pluginId, action);
    },
    [pluginId, runUiAction],
  );

  // 清单只声明 key（安装期已做 membership 校验）；库里的行 id 由宿主拼全，单点见 `pluginSchemaRowId`。
  const schemaId = pluginSchemaRowId(pluginId, page.entry);

  return (
    <div style={{ flex: 1, overflow: "auto", ...style }}>
      <SchemaIdContext.Provider value={{ schemaId }}>
        <DynamicUIStandalone schemaId={schemaId} onAction={handleAction} />
      </SchemaIdContext.Provider>
    </div>
  );
}

/** `loading: "bundle"` —— asset 协议装插件自带产物，桥与白名单沿用 panel 那一条。 */
function PluginBundlePage({
  pluginId,
  page,
  pluginName,
  style,
}: {
  pluginId: string;
  page: PluginPageDto;
  pluginName?: string;
  style?: React.CSSProperties;
}) {
  const { t } = useTranslation();
  const iframeRef = useRef<HTMLIFrameElement>(null);
  const apiBridgeRef = useRef<HostApiBridge | null>(null);
  const [loadNonce, setLoadNonce] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const translateDeny = useCallback(
    (command: string): string => t("dashboard.pluginPanels.commandDenied", { command }),
    [t],
  );
  const hostApi = useMemo(() => createPluginBridgeApi(pluginId, translateDeny), [pluginId, translateDeny]);
  // 权限面与 panel 共用同一个构造函数：在这里另写一份 commands 即违规（不变量①）
  const permissions = useMemo<SkillPermissions>(() => createPluginBridgePermissions(), []);

  const { token: themeToken } = antdTheme.useToken();
  const hostUi = useMemo<SkillHostUi>(() => ({
    navigate: (): void => {
      throw new Error(PAGE_ONLY_COMMANDS_MESSAGE);
    },
    notify: (): void => {
      throw new Error(PAGE_ONLY_COMMANDS_MESSAGE);
    },
    // 这两条在桥里没有权限闸（`uiGetTheme` / `uiGetLocale` 直取宿主值），给出去的也只有
    // 「当前主题名 / 当前语言」，判据与 panel 粒度逐字同形（SkillSandboxContainer 的 hostUi）。
    getTheme: () => (themeToken.colorBgBase === "#ffffff" ? "light" : "dark"),
    getLocale: () => i18n.language || "zh-CN",
  }), [themeToken.colorBgBase]);
  const hostStore = useMemo<SkillHostStore>(() => ({
    read: async (): Promise<never> => {
      throw new Error(PAGE_ONLY_COMMANDS_MESSAGE);
    },
    write: async (): Promise<never> => {
      throw new Error(PAGE_ONLY_COMMANDS_MESSAGE);
    },
  }), []);

  useEffect(() => {
    // iframe 从第一次提交起就一直在 DOM 里（见本组件末尾 render 的注释）：ready 帧只能由那块文档发，
    // 「加载完才渲染 iframe」会让开闸信号永远不可能到达。
    const iframe = iframeRef.current;
    if (!iframe) {
      // 结构性矛盾的信号（当前渲染形态下不可能发生，真发生说明渲染被改回条件挂载了）。
      // 明确报错比「静默等满超时」可定位得多，故不用 ?? 兜一个空实现。
      setError(i18n.t("skill.iframeNotReady"));
      setLoading(false);
      return;
    }

    let cancelled = false;
    setError(null);
    setLoading(true);

    // 超时与 panel/skill 同一条值、同一种症状：产物没 announceReady() 就走到这里（见文件头约束 3）。
    const loadTimer = setTimeout(() => {
      if (cancelled) {
        return;
      }
      setError(i18n.t("skill.timeoutError"));
      setLoading(false);
    }, SANDBOX_LOAD_TIMEOUT_MS);

    const handleMessage = (event: MessageEvent) => {
      // 身份先于内容：与宿主 srcdoc 侧、桥侧**同一个谓词**（#67 应修 4 / #72）。
      // 应用内任一可脚本化的 iframe 都能对宿主 window 发一帧内容完全合法的 rpc:request。
      if (!isFrameFromSandboxSource(event, iframe.contentWindow)) {
        return;
      }
      const msg = event.data;
      if (isSandboxReadyFrame(msg)) {
        if (cancelled) {
          return;
        }
        // 就绪帧是幂等的（SDK 明确写了「重复调用无害」）⇒ 只建一次桥。
        // 每次都新建会把上一份挤掉，而挤掉的那份 `destroy()` 永不发生 —— 正是
        // 「泄漏按重试次数累计」在宿主这一侧的镜像形态。
        if (!apiBridgeRef.current) {
          apiBridgeRef.current = createHostApiBridge({
            api: hostApi,
            ui: hostUi,
            store: hostStore,
            permissions,
            // 入向身份判据 + 回包投递目标，同一个 contentWindow：来源不符 ⇒ 丢弃且不回包。
            // ⚠ 这里**不传任何 origin**（§20.4 C 的落点，见下方 cleanup 上方的说明）。
            contentWindow: iframe.contentWindow,
          });
        }
        clearTimeout(loadTimer);
        setError(null);
        setLoading(false);
        return;
      }
      if (isSandboxContentErrorFrame(msg)) {
        logIpcError(i18n.t("skill.runtimeError", { name: pluginName ?? pluginId }))(msg.error);
        return;
      }
      if (isRpcRequestFrame(msg)) {
        // 未就绪 ⇒ 不接：桥还没建，请求也没有归属（ready 是唯一开闸信号，约束 3）。
        // 桥自己会再判一次来源 —— 那道判据长在接口上，不靠这里先筛过。
        apiBridgeRef.current?.handleRpcRequest(event);
      }
    };
    // 监听先于 src：文档可能在 addEventListener 之前就已经发出就绪帧（srcdoc 容器同此顺序）
    window.addEventListener("message", handleMessage);

    invoke<PluginPageAssetLocationDto>("plugin_resolve_page_asset", {
      pluginId,
      pageId: page.id,
    })
      .then((location) => {
        if (cancelled) {
          return;
        }
        // 只消费 absolutePath。location.entry 是清单原样回带的值，只为排错存在 ——
        // 拿它拼路径就是绕过安装目录包含性与 assetProtocol.scope（约束 1）。
        iframe.src = assetUrl(location.absolutePath);
      })
      .catch((e) => {
        if (cancelled) {
          return;
        }
        clearTimeout(loadTimer);
        // 四类失败（缺席 / 被拒 / 没锚点 / 被篡改）各有自己的码，11 语言已有译文：
        // 走 translateBackendError 按码取译文，不塌成一条，也不新增任何 key。
        logIpcError(`plugin_resolve_page_asset failed: ${pluginId}/${page.id} (entry=${page.entry})`)(e);
        setError(translateBackendError(e));
        setLoading(false);
      });

    return () => {
      cancelled = true;
      window.removeEventListener("message", handleMessage);
      // 宿主这一侧的「卸载路径」：拆桥 + 摘监听 + 清超时，和沙箱那一侧的 `client.dispose()`
      // 是同一处的两半（缺任一半都会留下不回收的监听或在途 Promise，见约束 3）。
      // 本容器不建 HostRpcBridge：宿主→沙箱方向的 `host:lifecycle` / `host:event` 帧对 bundle 无效
      // （产物没有 window.ctx，且 permissions.events 为空 —— §20.4 C 记的那条），
      // 为它留一座没人接的桥只会让「谁在发帧」变成第二个答案。
      apiBridgeRef.current?.destroy();
      apiBridgeRef.current = null;
      clearTimeout(loadTimer);
    };
  }, [hostApi, hostStore, hostUi, page.entry, page.id, permissions, pluginId, pluginName, loadNonce]);

  /**
   * §20.4 C 的落点（盘上口径已收口，别再照旧文补参数）：
   * 入向（宿主 → 沙箱）的 `allowedOrigin` 形参**在 #67 应修 1 里被整段删除**，
   * `createHostApiBridge` 的三个投递点恒取 `SANDBOX_MESSAGE_TARGET_ORIGIN`（全仓唯一定义处）。
   * ⇒ 容器**不传、也不许再传** origin：
   * - 传宿主 origin 就是应修 1 的原始缺陷（opaque 文档收不到回包，`tauri:` 下必现）；
   * - 传 `"*"` 是在第二处复述判据 —— 判据改了它不会跟着改。
   * 入向身份改由 `contentWindow` + `isFrameFromSandboxSource` 承担，即上面那段监听器与桥内部各一次。
   */

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

  return (
    <div style={{ position: "relative", width: "100%", minHeight: 400, ...style }}>
      <iframe
        ref={iframeRef}
        // key = 重试计数：换 nonce 就重挂一块新 iframe。同一块 iframe 重复赋同一个 src **不会**
        // 重新导航（引擎只在 URL 变化时加载），症状是「点了重试毫无反应，又等满 30 s」。
        key={loadNonce}
        title={page.title || pluginName || pluginId}
        // ⚠ 只允许 allow-scripts。**绝不加 allow-same-origin**：加上即把插件代码放进主 origin，
        // 它能读 localStorage 里的 provider key 与会话 token（约束 2）。
        sandbox="allow-scripts"
        style={{
          width: "100%",
          height: "100%",
          minHeight: 400,
          border: "none",
          backgroundColor: "transparent",
          // 文档没就绪前不露出半截内容；opacity 而非 display —— 后者会让某些引擎推迟加载
          opacity: loading ? 0 : 1,
        }}
      />
      {error && (
        <div style={overlayStyle}>
          <Alert
            type="error"
            showIcon
            // antd 6 在本仓把 Alert 的标题位改名为 title（传 message 会打运行时弃用警告）；
            // 与既有先例 DynamicUIStandalone 的用法同形。
            title={t("error.loadFailed")}
            description={error}
            action={
              <Button size="small" onClick={() => setLoadNonce((n) => n + 1)}>
                {t("common.retry")}
              </Button>
            }
          />
        </div>
      )}
      {loading && !error && (
        <div style={overlayStyle}>
          <Spin size="large" />
        </div>
      )}
    </div>
  );
}
