// SPDX-License-Identifier: AGPL-3.0-only

import { useGatewayLinkStore } from "@/stores/feature/gatewayLinkStore";
import { useEffect } from "react";

/**
 * 网关链读面接入（自 InputArea 抽出，组件中立性消键）：
 * 链接清单挂载期预热 + 网关会话创建动作。
 *
 * 活源是 gatewayLinkStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * gatewayLinkStore（组件中立性），统一从这里取。
 */

/** 挂载期预热：链接清单为空时拉取一次（空依赖 effect，自 InputArea 逐字搬入）。 */
export function useGatewayLinksBootstrap(): void {
  useEffect(() => {
    if ((useGatewayLinkStore.getState().links ?? []).length === 0) {
      useGatewayLinkStore.getState().fetchLinks();
    }
  }, []);
}

/** 创建网关链会话的 action 引用（store 创建期绑定的稳定引用）。 */
export function useCreateGatewayConversation() {
  return useGatewayLinkStore((s) => s.createGatewayConversation);
}

/**
 * 网关链清单（订阅；GatewayMenu 消键，2026-11 #222 补洞批次）。
 * 原组件为 `useGatewayLinkStore((s) => s.links)`，返回形态一字不差。
 */
export function useGatewayLinks() {
  return useGatewayLinkStore((s) => s.links);
}
