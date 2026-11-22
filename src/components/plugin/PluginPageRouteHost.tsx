// SPDX-License-Identifier: AGPL-3.0-only

import { PluginPageHost } from "@/components/plugin/PluginPageHost";
import { usePluginPageRoutes } from "@/lib/pluginRouteRegistry";
import { Result } from "antd";
import { useTranslation } from "react-i18next";
import { useParams } from "react-router-dom";

/**
 * 动态插件域页路由的宿主组件：从 URL 参数取 page id，再从已加载投影取页面声明。
 * 兼容旧 `/opc/domain/:pageId` 时，路由可通过显式参数指向插件页。
 */
export function PluginPageRouteHost(props: {
  pluginSlug?: string;
  pageId?: string;
}) {
  const { t } = useTranslation();
  const params = useParams<{ pluginSlug: string; pageId: string }>();
  const pluginSlug = props.pluginSlug ?? params.pluginSlug;
  const pageId = props.pageId ?? params.pageId;
  const routes = usePluginPageRoutes();
  const route = routes.find(
    (item) => item.pluginSlug === pluginSlug && item.pageId === pageId,
  );
  if (!route) {
    return (
      <div style={{ padding: 48, textAlign: "center" }}>
        <Result status="404" title="404" subTitle={t("error.pageNotFound")} />
      </div>
    );
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
