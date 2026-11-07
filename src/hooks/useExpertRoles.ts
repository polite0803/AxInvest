// SPDX-License-Identifier: AGPL-3.0-only

import { useExpertStore } from "@/stores/feature/expertStore";
import type { AgentProfile } from "@/types";

/**
 * 专家角色选择器的订阅切片：机构角色数据 + 各写操作 action 引用。
 *
 * 内部为逐条 selector 订阅（与原组件逐条订阅等价，未扩大订阅面），
 * 组装为对象返回。
 *
 * 活源是 expertStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * expertStore（组件中立性），统一从这里取。
 */
export function useExpertRoles() {
  const getAllRoles = useExpertStore((s) => s.getAllRoles);
  const importAgencyExperts = useExpertStore((s) => s.importAgencyExperts);
  const loadAgencyRoles = useExpertStore((s) => s.loadAgencyRoles);
  const clearAgencyExperts = useExpertStore((s) => s.clearAgencyExperts);
  const deleteAgencyExpert = useExpertStore((s) => s.deleteAgencyExpert);
  const updateAgencyExpert = useExpertStore((s) => s.updateAgencyExpert);
  const exportAgencyExperts = useExpertStore((s) => s.exportAgencyExperts);
  const agencyRoles = useExpertStore((s) => s.agencyRoles);
  const agencyLoaded = useExpertStore((s) => s.agencyLoaded);
  const removeCustomRole = useExpertStore((s) => s.removeCustomRole);
  const updateCustomRole = useExpertStore((s) => s.updateCustomRole);
  const exportCustomRoles = useExpertStore((s) => s.exportCustomRoles);

  return {
    getAllRoles,
    importAgencyExperts,
    loadAgencyRoles,
    clearAgencyExperts,
    deleteAgencyExpert,
    updateAgencyExpert,
    exportAgencyExperts,
    agencyRoles,
    agencyLoaded,
    removeCustomRole,
    updateCustomRole,
    exportCustomRoles,
  };
}

/**
 * 非订阅式一次性新增自定义专家角色（等价于 store.getState() 直写）。
 */
export function addExpertRole(role: AgentProfile): void {
  useExpertStore.getState().addCustomRole(role);
}
