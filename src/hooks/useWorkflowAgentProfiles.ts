// SPDX-License-Identifier: AGPL-3.0-only

import { useAgentStore } from "@/stores/feature/agentStore";
import type { AgentProfile } from "@/types";
import type { CreateAgentProfileInput } from "@/types";

/**
 * 工作流节点属性面板的专家档案同步态读面（自 AgentPropertyPanel 抽出，
 * 组件中立性消键）：档案是否已加载 + 拉取 action。
 *
 * 查询单个档案用 useAgentProfileLookup（getProfileById）。
 */
export function useWorkflowAgentProfileSync() {
  const agentProfilesLoaded = useAgentStore((s) => s.loaded);
  const loadAgentProfiles = useAgentStore((s) => s.loadProfiles);
  return { agentProfilesLoaded, loadAgentProfiles };
}

/**
 * 非订阅式一次性读取全部专家档案（等价于 store.getState().getAllProfiles()，
 * 供回调内单次查询，不建立订阅）。
 */
export function listAllAgentProfiles(): AgentProfile[] {
  return useAgentStore.getState().getAllProfiles();
}

/**
 * 非订阅式一次性创建自定义专家档案（等价于
 * store.getState().createCustomProfile(input)，供回调内单次调用，不建立订阅）。
 */
export function createCustomAgentProfile(
  input: CreateAgentProfileInput,
): Promise<AgentProfile> {
  return useAgentStore.getState().createCustomProfile(input);
}
