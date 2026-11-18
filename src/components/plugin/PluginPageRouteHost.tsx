// SPDX-License-Identifier: AGPL-3.0-only

import { PluginPageHost } from "@/components/plugin/PluginPageHost";
import { usePluginPageRoutes } from "@/lib/pluginRouteRegistry";
import { useTranslation } from "react-i18next";
import { useParams } from "react-router-dom";

/** 动态插件域页路由的宿主组件：从 URL 参数取 page id，再从已加载投影取页面声明。 */
export function PluginPageRouteHost() {
  const { t } = useTranslation();
  const { pluginSlug, pageId } = useParams<{ pluginSlug: string; pageId: string }>();
  const routes = usePluginPageRoutes();
  const route = routes.find(
    (item) => item.pluginSlug === pluginSlug && item.pageId === pageId,
  );
  if (!route) {
    return <div style={{ padding: 24 }}>{t("common.loading")}</div>;
  }
  return (
    <PluginPageHost
      pluginId={route.pluginId}
      pluginName={route.pluginName}
      page={route.page}
      style={{ flex: 1, minHeight: 0 }}
    />
  );
}
