// SPDX-License-Identifier: AGPL-3.0-only

import { useLocalToolStore } from "@/stores/feature/localToolStore";

/**
 * 工作流属性面板的工具组读面（自 AgentPropertyPanel / CodePropertyPanel /
 * ToolPropertyPanel 抽出，组件中立性消键）。
 *
 * 保持原组件的无 selector 全量订阅形态（store 任意字段变化都重渲染），
 * 重渲染时机与原实现逐字一致，不收窄订阅面。
 */
export function useWorkflowToolGroups() {
  const { groups, loadGroups } = useLocalToolStore();
  return { groups, loadGroups };
}
