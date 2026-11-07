// SPDX-License-Identifier: AGPL-3.0-only

import { usePromptTemplateStore } from "@/stores/feature/promptTemplateStore";

/**
 * 提示模板清单读面（templates + loading + loadTemplates）。
 *
 * 抽成 hook 是为了让 chat（PromptTemplateSelector）组件不直接依赖
 * promptTemplateStore（组件中立性，见 scripts/check-component-neutrality.mjs），
 * 统一从这里取。与 usePromptTemplateUsage（使用计数 action）是两个不同读面。
 */
export function usePromptTemplates() {
  const templates = usePromptTemplateStore((s) => s.templates);
  const loading = usePromptTemplateStore((s) => s.loading);
  const loadTemplates = usePromptTemplateStore((s) => s.loadTemplates);
  return { templates, loading, loadTemplates };
}
