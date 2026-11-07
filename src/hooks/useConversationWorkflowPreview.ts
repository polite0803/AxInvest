// SPDX-License-Identifier: AGPL-3.0-only

import { useWorkflowEditorStore } from "@/stores/feature/workflowEditorStore";

/**
 * 会话工作流预览的加载 action。
 *
 * 抽成 hook 是为了让 chat（ChatSidebar）组件不直接依赖 workflowEditorStore
 * （组件中立性，见 scripts/check-component-neutrality.mjs），统一从这里取。
 */
export function useConversationWorkflowPreview() {
  return useWorkflowEditorStore((s) => s.loadConversationWorkflowPreview);
}
