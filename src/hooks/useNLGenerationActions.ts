// SPDX-License-Identifier: AGPL-3.0-only

import { useSkillStore } from "@/stores/feature/skillStore";
import { useWorkflowStore } from "@/stores/feature/workflowStore";

/**
 * NL 生成的跨域 action 接入（自 AgentWfTab 抽出，组件中立性消键）：
 * 解析能力来自 workflowStore，技能创建来自 skillStore —— 聚合于本中立 hook，
 * 使 agent 组件不再直接依赖这两个 feature store。
 */
export function useNLGenerationActions() {
  const parseSkillFromNaturalLanguage = useWorkflowStore((s) => s.parseSkillFromNaturalLanguage);
  const parseUIFromNaturalLanguage = useWorkflowStore((s) => s.parseUIFromNaturalLanguage);
  const parseProgress = useWorkflowStore((s) => s.parseProgress);
  const createSkill = useSkillStore((s) => s.createSkill);
  return { parseSkillFromNaturalLanguage, parseUIFromNaturalLanguage, parseProgress, createSkill };
}
