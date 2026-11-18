// SPDX-License-Identifier: AGPL-3.0-only

import { PluginPageHost } from "@/components/plugin/PluginPageHost";
import { getPluginPageRoutes } from "@/lib/pluginRouteRegistry";
import { useParams } from "react-router-dom";

/** 动态插件域页路由的宿主组件：从 URL 参数取 page id，再从已加载投影取页面声明。 */
export function PluginPageRouteHost() {
  const { pluginSlug, pageId } = useParams<{ pluginSlug: string; pageId: string }>();
  const route = getPluginPageRoutes().find(
    (item) => item.pluginSlug === pluginSlug && item.pageId === pageId,
  );
  if (!route) {
    return null;
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
