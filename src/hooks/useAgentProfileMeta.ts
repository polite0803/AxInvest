// SPDX-License-Identifier: AGPL-3.0-only

import { useAgentStore } from "@/stores/feature/agentStore";
import { useExpertStore } from "@/stores/feature/expertStore";
import { useCallback } from "react";

/**
 * Agent 名册 fallback 链：expertStore.getRoleById → agentStore.getProfileById。
 *
 * 返回解析函数（getState 快照读，非订阅），供调用方在 useMemo 等非渲染路径
 * 按 profileId 解析；命中后取 agentRole / icon / name 字段。
 * 抽成 hook 是为了让 workflow 等跨域组件不直接依赖这两个 store（组件中立性）。
 */
export function useAgentProfileMeta() {
  return useCallback((profileId: string) => {
    return useExpertStore.getState().getRoleById(profileId)
      ?? useAgentStore.getState().getProfileById(profileId);
  }, []);
}
