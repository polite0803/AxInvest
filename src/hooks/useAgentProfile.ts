// SPDX-License-Identifier: AGPL-3.0-only

import { useAgentStore } from "@/stores/feature/agentStore";
import type { AgentProfile } from "@/types";

/**
 * 订阅式专家档案查询：返回 agentStore 的 getProfileById 方法引用
 * （action 引用稳定，订阅不会引发重渲；查询结果由调用方在渲染期求值）。
 *
 * 活源是 agentStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * agentStore（组件中立性），统一从这里取。
 */
export function useAgentProfileLookup() {
  return useAgentStore((s) => s.getProfileById);
}

/**
 * 非订阅式一次性读取专家档案（等价于 store.getState() 直读，
 * 供回调 / 渲染期单次查询，不建立订阅）。
 */
export function getAgentProfile(id: string): AgentProfile | undefined {
  return useAgentStore.getState().getProfileById(id);
}
