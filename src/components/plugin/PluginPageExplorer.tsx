// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 「插件页」工作台 Tab 的薄容器 —— 插件贡献页面的**发现与列举**层。
 *
 * 职责边界：列表真相源只有 `plugin_list_pages`（后端 PluginManager 的实时投影，禁区 4），
 * 本组件不建平行 manifest / 路由表；拿到清单后按 `enabled` 过滤，选中一页交给
 * 同目录的 `PluginPageHost`（page 粒度容器）装载，不在本层复述它的分支判据。
 *
 * 选中策略：左侧列表 + 右侧内容，默认第一页（本地 state）。为什么不同时渲染全部页：
 * 一页 = 一个 iframe（bundle）或一套动态 UI 实例（schema），多页并挂会重复建桥/监听、
 * 抢焦点，且保活 Tab 下这些实例切走不会卸载 —— 单页选中是最小且无泄漏的形态。
 *
 * 浏览器模式（npm run dev）：`isTauri() === false` 时，plugin_list_pages 的 mock 只给
 * 演示清单，但 schema 页依赖 `get_dynamic_ui_schema`（mock 无预置 schema ⇒ 会显示错误
 * Alert）、bundle 页依赖 asset 协议（浏览器没有）⇒ 两条装载路在浏览器都不成立。
 * 因此这里显式降级为**说明态**而不是把页交给宿主容器，既不全白屏也不报错；真机
 * （tauri）下照常走 PluginPageHost。
 */

import { translateBackendError } from "@/lib/errorI18n";
import { invoke, isTauri } from "@/lib/invoke";
import { subscribePluginPageRoutes } from "@/lib/pluginRouteRegistry";
import type { PluginPageDto, PluginPagesInfoDto } from "@/types";
import { Alert, Empty, Select, Spin, Typography } from "antd";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { PluginPageHost } from "./PluginPageHost";

/** 一个可展示的页面 = 插件信息 + 该插件的一个 page（把顶层 id 作为 pluginId 带下去）。 */
interface SelectablePage {
  pluginId: string;
  pluginName: string;
  page: PluginPageDto;
}

/** 把插件清单摊平为「可展示页面」列表；过滤 disabled 与无 pages 的插件。 */
function flattenPages(infos: readonly PluginPagesInfoDto[]): SelectablePage[] {
  const out: SelectablePage[] = [];
  for (const info of infos) {
    if (!info.enabled) {
      continue;
    }
    for (const page of info.pages) {
      out.push({ pluginId: info.id, pluginName: info.name, page });
    }
  }
  return out;
}

export function PluginPageExplorer() {
  const { t } = useTranslation();
  const [infos, setInfos] = useState<PluginPagesInfoDto[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // 选中项存「pluginId/pageId」复合键而非索引：清单刷新后顺序变化也不会指向错页。
  const [selectedKey, setSelectedKey] = useState<string | null>(null);

  // 插件启停/卸载会刷新插件页路由快照；该通知同时驱动本列表重读 `plugin_list_pages`。
  // 真值源仍是后端实时投影，这里只重新拉一次，不在前端维护第二份 enabled 状态。
  useEffect(() => {
    let cancelled = false;
    const load = (initial: boolean) => {
      setLoading(initial);
      setError(null);
      invoke<PluginPagesInfoDto[]>("plugin_list_pages")
        .then((list) => {
          if (cancelled) {
            return;
          }
          setInfos(Array.isArray(list) ? list : []);
        })
        .catch((e: unknown) => {
          if (cancelled) {
            return;
          }
          setError(translateBackendError(e));
        })
        .finally(() => {
          if (!cancelled) {
            setLoading(false);
          }
        });
    };
    load(true);
    const unsubscribe = subscribePluginPageRoutes(() => load(false));
    return () => {
      cancelled = true;
      unsubscribe();
    };
  }, []);

  const pages = useMemo(() => flattenPages(infos), [infos]);
  const selected = pages.find((p) => `${p.pluginId}/${p.page.id}` === selectedKey) ?? pages[0];

  if (loading) {
    return (
      <div style={{ display: "flex", alignItems: "center", justifyContent: "center", height: "100%", minHeight: 200 }}>
        <Spin size="large" />
      </div>
    );
  }

  if (error) {
    return (
      <div style={{ padding: 24 }}>
        <Alert type="error" showIcon title={t("plugin.pages.loadFailed")} description={error} />
      </div>
    );
  }

  // 浏览器演示态：清单可能是 mock 给的占位页，明确告知「真机才装载」，不进 PluginPageHost。
  if (!isTauri()) {
    return (
      <div style={{ padding: 24, height: "100%", overflow: "auto" }}>
        <Alert
          type="info"
          showIcon
          title={t("plugin.pages.title")}
          description={t("plugin.pages.browserNotice")}
        />
        {pages.length > 0 && (
          <Typography.Paragraph style={{ marginTop: 16 }}>
            {t("plugin.pages.detected", { count: pages.length })}
          </Typography.Paragraph>
        )}
      </div>
    );
  }

  if (pages.length === 0) {
    return (
      <div style={{ padding: 24, height: "100%", overflow: "auto" }}>
        <Empty description={t("plugin.pages.empty")} />
      </div>
    );
  }

  const options = pages.map((p) => ({
    value: `${p.pluginId}/${p.page.id}`,
    label: `${p.pluginName} · ${p.page.title || p.page.id}`,
  }));

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", minHeight: 0 }}>
      <div style={{ flexShrink: 0, padding: "12px 16px", display: "flex", alignItems: "center", gap: 12 }}>
        <Typography.Text strong>{t("plugin.pages.title")}</Typography.Text>
        <Select
          aria-label={t("plugin.pages.selectPage")}
          value={selected ? `${selected.pluginId}/${selected.page.id}` : undefined}
          options={options}
          onChange={(value: string) => setSelectedKey(value)}
          style={{ minWidth: 260 }}
        />
      </div>
      <div style={{ flex: 1, minHeight: 0, display: "flex" }}>
        {selected && (
          <PluginPageHost
            pluginId={selected.pluginId}
            pluginName={selected.pluginName}
            page={selected.page}
            style={{ flex: 1, minHeight: 0 }}
          />
        )}
      </div>
    </div>
  );
}
