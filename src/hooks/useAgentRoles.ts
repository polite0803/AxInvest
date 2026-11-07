// SPDX-License-Identifier: AGPL-3.0-only

import { useAgentRoleStore } from "@/stores/feature/agentRoleStore";

/**
 * 智能体角色名册读面（roles + fetchRoles）。
 *
 * 抽成 hook 是为了让 office（AddMemberModal）组件不直接依赖 agentRoleStore
 * （组件中立性，见 scripts/check-component-neutrality.mjs），统一从这里取。
 */
export function useAgentRoles() {
  const roles = useAgentRoleStore((s) => s.roles);
  const fetchRoles = useAgentRoleStore((s) => s.fetchRoles);
  return { roles, fetchRoles };
}
