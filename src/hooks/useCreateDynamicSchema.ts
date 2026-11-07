// SPDX-License-Identifier: AGPL-3.0-only

import { useDynamicUIStore } from "@/stores/feature/dynamicUIStore";

/**
 * NL2UI 保存动态 UI Schema 的 action。
 *
 * 抽成 hook 是为了让 agent（AgentWfTab）组件不直接依赖 dynamicUIStore
 * （组件中立性，见 scripts/check-component-neutrality.mjs），统一从这里取。
 */
export function useCreateDynamicSchema() {
  return useDynamicUIStore((s) => s.createSchema);
}
