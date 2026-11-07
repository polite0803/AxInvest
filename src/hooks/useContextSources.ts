// SPDX-License-Identifier: AGPL-3.0-only

import { useContextSourceStore } from "@/stores/feature/contextSourceStore";

/**
 * 上下文源读面（sources + loadSources）。
 *
 * 抽成 hook 是为了让 chat（SourcePickerPanel）组件不直接依赖 contextSourceStore
 * （组件中立性，见 scripts/check-component-neutrality.mjs），统一从这里取。
 */
export function useContextSources() {
  const contextSources = useContextSourceStore((s) => s.sources);
  const loadSources = useContextSourceStore((s) => s.loadSources);
  return { contextSources, loadSources };
}
