// SPDX-License-Identifier: AGPL-3.0-only

/**
 * `/invest` 旧入口的兼容跳转桥（T3 完整外迁后的宿主残留边界）。
 *
 * 投资业务（原 `InvestHub` 六个 Tab）已整棵迁入 stock 插件的 bundle 页，宿主不再
 * 编译任何股票页面组件。本组件只做两件事，**不含任何股票业务代码**：
 *   1. stock 插件已启用且贡献了 bundle 页 ⇒ `<Navigate replace>` 到
 *      `/plugins/<slug>/<pageId>`（路径由 `pluginRouteRegistry` 实时投影，不硬编码）；
 *   2. 插件未启用 / 投影未就绪 ⇒ 显示明确降级提示，绝不回退渲染宿主股票页。
 *
 * 为什么保留这条路由而不是连路径一起删：`ReplayWorkbench`（`/invest/replay`）
 * 与若干历史调用方仍构造 `/invest?...` 链接；直接删会让它们落到 404 ——
 * 正是用户报告过的「导航几乎是错误的」同一种失效形态。保留一条零业务代码的
 * 跳转桥既守住旧链可达性，又不把股票组件重新编进宿主 bundle。
 *
 * ⚠ 本文件不得 import `@/components/invest/**`、`@/components/stock-analysis/**`
 * 或任何股票页面壳 —— 那会把整棵已外迁的组件树重新拉回宿主编译图。
 */

import { usePluginPageRoutes, usePluginPageRoutesReady } from "@/lib/pluginRouteRegistry";
import { Button, Result } from "antd";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Navigate, useNavigate } from "react-router-dom";

/** 插件快照迟迟不就绪时的兜底上限（与阶段 2 首帧 gate 的 3 秒超时同口径）。 */
const READY_TIMEOUT_MS = 3000;

/**
 * stock 插件页在运行期投影里的稳定标识。插件 id 在安装形态下可能带 `@bundled` 后缀
 * （slug 随之变成 `stock-bundled`），所以同时认 manifest 的 `name` 与清单里固定的
 * `pageId = stock-home` —— 后者是页面自身的契约，最抗安装形态变化。
 */
function isStockPlugin(route: {
  pluginId: string;
  pluginName: string;
  pluginSlug: string;
  pageId: string;
}): boolean {
  return route.pluginId === "stock"
    || route.pluginName === "stock"
    || route.pluginSlug === "stock"
    || route.pageId === "stock-home";
}

export function LegacyInvestRedirect() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const pluginRoutes = usePluginPageRoutes();
  const ready = usePluginPageRoutesReady();
  const [timedOut, setTimedOut] = useState(false);

  const stockPage = pluginRoutes.find(isStockPlugin);
  const hasStockPage = stockPage !== undefined;
  useEffect(() => {
    if (ready || hasStockPage) { return; }
    const timer = window.setTimeout(() => setTimedOut(true), READY_TIMEOUT_MS);
    return () => window.clearTimeout(timer);
  }, [ready, hasStockPage]);

  if (stockPage) {
    // 旧链上的 `?tab=` / `?stockCode=` 不向插件页透传：bundle 页有自己的内部
    // Tab 与股票输入，宿主无法替它解释这些参数；透传反而会造出「看起来像
    // 生效、实际被忽略」的假深链。
    return <Navigate to={stockPage.path} replace />;
  }

  if (!ready && !timedOut) {
    return (
      <div className="flex items-center justify-center h-full w-full" style={{ minHeight: 200 }}>
        {t("common.loading")}
      </div>
    );
  }

  // 超时后走与普通降级相同的「插件不可用」结果页：插件投影挂了也不能无限转圈。
  return (
    <Result
      status="info"
      title={t("invest.title")}
      subTitle={timedOut && !ready ? t("error.loadFailed") : t("plugin.pages.empty")}
      extra={
        <Button type="primary" onClick={() => navigate("/settings")}>
          {t("nav.settings")}
        </Button>
      }
    />
  );
}
