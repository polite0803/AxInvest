// SPDX-License-Identifier: AGPL-3.0-only

import { useMultiAgentStore } from "@/stores/feature/multiAgentStore";

/**
 * 多智能体委派的订阅切片：固定角色列表 + 委派状态与动作。
 *
 * 内部为逐条 selector 订阅（浅比较切片，不整 store 返回），
 * 组装为对象返回。
 *
 * 活源是 multiAgentStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * multiAgentStore（组件中立性），统一从这里取。
 */
export function useMultiAgentDelegate() {
  const roles = useMultiAgentStore((s) => s.roles);
  const delegating = useMultiAgentStore((s) => s.delegating);
  const fetchRoles = useMultiAgentStore((s) => s.fetchRoles);
  const delegateTask = useMultiAgentStore((s) => s.delegateTask);

  return { roles, delegating, fetchRoles, delegateTask };
}
