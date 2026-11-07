// SPDX-License-Identifier: AGPL-3.0-only

import { usePromptTemplateStore } from "@/stores/feature/promptTemplateStore";

/**
 * 工作流属性面板的提示词模板读面（自 AgentPropertyPanel / LLMPropertyPanel
 * 抽出，组件中立性消键）：模板列表 + 拉取 action。
 *
 * 保持原组件的无 selector 全量订阅形态（store 任意字段变化都重渲染），
 * 重渲染时机与原实现逐字一致，不收窄订阅面。
 * 使用计数 action 另见 usePromptTemplateUsage。
 */
export function useWorkflowPromptTemplates() {
  const { templates, loadTemplates } = usePromptTemplateStore();
  return { templates, loadTemplates };
}
