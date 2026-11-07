// SPDX-License-Identifier: AGPL-3.0-only

import { usePromptTemplateStore } from "@/stores/feature/promptTemplateStore";

/**
 * 提示模板使用计数 action 的中立接入（自 InputArea 抽出，组件中立性消键）。
 */
export function usePromptTemplateUsage() {
  return usePromptTemplateStore((s) => s.incrementUsage);
}
