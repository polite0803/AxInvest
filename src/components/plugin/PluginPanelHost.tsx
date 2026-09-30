// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 插件仪表盘面板宿主 —— 变体 C（沙箱 iframe）
 *
 * 面板内容来自插件安装目录内的文本资产（`manifest.dashboardPanels[].frontendEntry`），
 * 由 `plugin_read_panel_asset` 取出后交给 `sandbox="allow-scripts"` 的 iframe。
 * 插件代码因此**不进入主 origin**：它读不到 localStorage 里的 provider key 与会话 token。
 * 作为交换，theme token 与业务数据只能经宿主显式下发（`panel.props` + 两条通道）。
 *
 * 见 `PLAN-dashboard-consolidation.md` §6.4 / §6.6。
 */

import { translateBackendError } from "@/lib/errorI18n";
import { invoke } from "@/lib/invoke";
import { Card, Space, Tag, Typography } from "antd";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { SkillSandboxContainer } from "@/components/skill/SkillSandboxContainer";
import type { SkillHostApi, SkillPermissions } from "@/sdk/types";
import type { DashboardPluginInfoDto, PluginDashboardPanelDto } from "@/types";

const { Text } = Typography;

/**
 * 面板沙箱仅有的两条通道。**不含通用 invoke 白名单** —— 面板能要求的后端能力到此为止，
 * 想多一条都得回来改这里。
 */
export const PANEL_ALLOWED_COMMANDS = ["plugin_ui_action", "plugin_execute_command"] as const;

type DeniedTranslator = (command: string) => string;

/**
 * 面板侧的宿主 API。单独导出是为了让「绑定 pluginId」这条能被测试钉住：
 * iframe 传来的 `pluginId` **一律丢弃**，否则一个面板就能驱动另一个插件的 worker。
 */
export function createPanelHostApi(
  pluginId: string,
  deny: DeniedTranslator,
): SkillHostApi {
  return {
    invoke: async <T = unknown>(command: string, args?: Record<string, unknown>): Promise<T> => {
      if (!(PANEL_ALLOWED_COMMANDS as readonly string[]).includes(command)) {
        throw new Error(deny(command));
      }
      const { pluginId: _ignored, ...rest } = args ?? {};
      return invoke<T>(command, { ...rest, pluginId });
    },
    // 面板不接宿主事件总线：`emit` 存在只是为了满足 SkillHostApi 形状
    emit: (): void => {},
  };
}

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
    () => createPanelHostApi(plugin.id, translateDeny),
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

  const permissions = useMemo<SkillPermissions>(
    () => ({ commands: [...PANEL_ALLOWED_COMMANDS] }),
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
