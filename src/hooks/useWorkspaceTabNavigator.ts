// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 切换工作台 Tab 的**唯一入口**：同时写 store 与 URL(?ws=)，保证二者永不漂移。
 *
 * 为什么必须集中：WorkspaceHub 有「URL 的 ?ws= ⇒ store」的读侧同步。若某个调用方
 * 只调 `setActiveTab` 不改 URL，读侧会把 Tab 拉回 URL 里的旧值（表现为切了又弹回来）。
 *
 * 历史栈策略：
 *   - 已在工作台（/chat）内切 Tab ⇒ replace（视图切换不该淹没后退键）
 *   - 从其它页面（/invest、/opc/...）进入工作台 ⇒ push（保留来路，后退能回业务页）
 */

import { BUILTIN_PAGE_PATH } from "@/lib/pageRegistry";
import { buildWorkspaceTabSearch, type WorkspaceTab } from "@/lib/workspaceTabs";
import { useWorkspaceTabStore } from "@/stores";
import { useCallback } from "react";
import { useLocation, useNavigate } from "react-router-dom";

export function useWorkspaceTabNavigator(): (tab: WorkspaceTab) => void {
  const navigate = useNavigate();
  const location = useLocation();

  return useCallback(
    (tab: WorkspaceTab) => {
      useWorkspaceTabStore.getState().setActiveTab(tab);
      const isOnHub = location.pathname === BUILTIN_PAGE_PATH.chat
        || location.pathname === "/"
        || location.pathname === "";
      // 已在工作台内切 Tab：保留当前无关参数（如 stockCode）；从业务页进入工作台：
      // **不继承**业务页查询串（/invest?tab=workspace&view=trade 这种参数带进 /chat
      // 会被下游误读成工作台语义），只写 ws。
      const baseSearch = isOnHub ? location.search : "";
      navigate(
        {
          pathname: BUILTIN_PAGE_PATH.chat,
          search: buildWorkspaceTabSearch(baseSearch, tab),
        },
        { replace: isOnHub },
      );
    },
    [navigate, location.pathname, location.search],
  );
}
