// SPDX-License-Identifier: AGPL-3.0-only

import { useDynamicUIStore } from "@/stores/feature/dynamicUIStore";

/**
 * schema 卡片的取 schema action。
 *
 * 抽成 hook 是为了让 chat（PluginSchemaCard）组件不直接依赖 dynamicUIStore
 * （组件中立性，见 scripts/check-component-neutrality.mjs），统一从这里取。
 */
export function usePluginSchemaCard() {
  return useDynamicUIStore((s) => s.getSchema);
}
