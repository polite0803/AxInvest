// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 插件仪表盘面板宿主 —— 变体 C（沙箱 iframe）
 *
 * 面板内容来自插件安装目录内的文本资产（`manifest.dashboardPanels[].frontendEntry`），
 * 由 `plugin_read_panel_asset` 取出后交给 `sandbox="allow-scripts"` 的 iframe。
 * 插件代码因此**不进入主 origin**：它读不到 localStorage 里的 provider key 与会话 token。
 * 作为交换，theme token 与业务数据只能经宿主显式下发（`panel.props` + 两条通道）。
 *
 * 装载粒度的分工（任务 #39 / `PLAN-shell-plus-plugins.md` §0.12.5 裁定 A，客户端运行时由 #46 补齐）：
 * - **panel（本文件）**：文本资产 → `srcdoc`，桥 = `pluginSandboxBridge.ts` 的那一条；
 *   客户端运行时由宿主注入（`sdk/sandboxTemplate.ts` 生成的内联脚本）。
 * - **page（asset 协议装插件自带 bundle）**：入口由后端 `plugin_resolve_page_asset` 给出绝对路径，
 *   前端 `convertFileSrc` 换 asset URL 交给沙箱容器；**桥与白名单不变**（同一个
 *   `createPluginBridgeApi` + 同一个 `createPluginBridgePermissions`）。
 *   宿主不再包 `srcdoc` ⇒ 第三方 bundle 里必须自带客户端运行时
 *   （`@/sdk/pluginClientBridge.ts`，零依赖、可原样 vendored），否则那条桥对插件产物不可用。
 *   容器本身还缺一个「列出某包 pages[]」的投影面，需求已随 #46 交付报告移交（不动 Rust 故本批不建）。
 * - 表单 / 看板类继续走 `DynamicPageViewer` 的 UISchema 通道，与本文件无关。
 *
 * 见 `PLAN-dashboard-consolidation.md` §6.4 / §6.6。
 */

import { translateBackendError } from "@/lib/errorI18n";
import { invoke } from "@/lib/invoke";
import { Card, Space, Tag, Typography } from "antd";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { createPluginBridgeApi, createPluginBridgePermissions } from "@/components/plugin/pluginSandboxBridge";
import { SkillSandboxContainer } from "@/components/shared/SkillSandboxContainer";
import type { SkillPermissions } from "@/sdk/types";
import type { DashboardPluginInfoDto, PluginDashboardPanelDto } from "@/types";

const { Text } = Typography;

/**
 * 桥与命令白名单住在 `pluginSandboxBridge.ts` —— 那里是 panel 与 page 两种装载粒度**共用**的
 * 唯一定义点。本文件保留旧名字只是为了不断既有调用面（含 `__tests__/PluginPanelHost.bridge.test.ts`）：
 * 这两个别名与源模块是同一个对象，不是第二份清单，改不动也扩不了。
 */
export {
  createPluginBridgeApi as createPanelHostApi,
  PLUGIN_BRIDGE_COMMANDS as PANEL_ALLOWED_COMMANDS,
} from "@/components/plugin/pluginSandboxBridge";

function PluginPanelCard({
  plugin,
  panel,
}: {
  plugin: DashboardPluginInfoDto;
  panel: PluginDashboardPanelDto;
}) {
  const { t } = useTranslation();

  const translateDeny = useCallback(
    (command: string) => t("dashboard.pluginPanels.commandDenied", { command }),
    [t],
  );
  const hostApi = useMemo(
    () => createPluginBridgeApi(plugin.id, translateDeny),
    [plugin.id, translateDeny],
  );

  // 资产路径不接收前端传参：容器把 entry 传进来也只用于展示，真实路径由后端按
  // (pluginId, panelId) 从 manifest 解析 —— 前端无法用串改的 entry 探测目录。
  const loadAsset = useCallback(async (): Promise<string> => {
    try {
      const asset = await invoke<{ source: string }>("plugin_read_panel_asset", {
        pluginId: plugin.id,
        panelId: panel.id,
      });
      return asset.source;
    } catch (e) {
      // 四种缺席（没声明 / 被拒 / 没锚点 / 被篡改）各有自己的码与译文，不塌成一条
      throw new Error(translateBackendError(e));
    }
  }, [plugin.id, panel.id]);

  // 权限面与 page 粒度共用同一个构造函数：想在这里另写一份 commands 即是违规
  // （不变量①），只能回到 `pluginSandboxBridge.ts` 改。
  const permissions = useMemo<SkillPermissions>(
    () => createPluginBridgePermissions(),
    [],
  );

  const body = panel.frontendEntry
    ? (
      <SkillSandboxContainer
        skillName={plugin.name}
        componentId={panel.id}
        componentConfig={{ entry: panel.frontendEntry, props: panel.props }}
        permissions={permissions}
        loadAsset={loadAsset}
        hostApiOverride={hostApi}
        style={{ width: "100%", minHeight: panel.size === "small" ? 160 : 280 }}
      />
    )
    : (
      // 只声明了标题的面板：显式说明「没有内容来源」，不渲染成空白卡冒充已就绪
      <Text type="secondary">{t("dashboard.pluginPanels.noSource", { component: panel.componentName })}</Text>
    );

  return (
    <Card
      size="small"
      style={{ marginBottom: 12 }}
      title={panel.title}
      extra={<Tag>{t("dashboard.pluginPanels.providedBy", { plugin: plugin.name })}</Tag>}
    >
      {body}
    </Card>
  );
}

/** 用量仪表盘页里的插件面板区：面板声明的唯一展示面（管理面在 设置 › 仪表盘面板）。 */
export function PluginDashboardPanels() {
  const { t } = useTranslation();
  const [plugins, setPlugins] = useState<DashboardPluginInfoDto[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    invoke<DashboardPluginInfoDto[]>("dashboard_list_plugins")
      .then((list) => {
        if (!cancelled) {
          setPlugins(list);
        }
      })
      .catch((e) => {
        if (!cancelled) {
          setError(translateBackendError(e));
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const enabled = (plugins ?? []).filter((p) => p.enabled);

  return (
    <div style={{ marginTop: 16 }}>
      <Text strong>{t("dashboard.pluginPanels.title")}</Text>
      {error && (
        <div style={{ marginTop: 8 }}>
          <Text type="danger">{error}</Text>
        </div>
      )}
      {!error && enabled.length === 0 && (
        <div style={{ marginTop: 8 }}>
          <Text type="secondary">{t("dashboard.pluginPanels.empty")}</Text>
        </div>
      )}
      <Space direction="vertical" size={0} style={{ width: "100%", marginTop: 8 }}>
        {(["main", "sidebar", "header", "footer"] as const).flatMap((position) =>
          enabled.flatMap((plugin) =>
            plugin.panels
              .filter((panel) => panel.position === position)
              .map((panel) => <PluginPanelCard key={`${plugin.id}:${panel.id}`} plugin={plugin} panel={panel} />)
          )
        )}
      </Space>
    </div>
  );
}
