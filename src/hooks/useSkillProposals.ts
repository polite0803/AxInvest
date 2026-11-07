// SPDX-License-Identifier: AGPL-3.0-only

import { useSkillStore } from "@/stores/feature/skillStore";

/**
 * 技能提案读面（清单 + 加载 + 从提案建技能）。
 *
 * 抽成 hook 是为了让 chat（SkillProposalPanel，含其导出的 useSkillProposalBadge）
 * 组件不直接依赖 skillStore（组件中立性，见 scripts/check-component-neutrality.mjs），
 * 统一从这里取。
 */
export function useSkillProposals() {
  const skillProposals = useSkillStore((s) => s.skillProposals);
  const loadSkillProposals = useSkillStore((s) => s.loadSkillProposals);
  const createSkillFromProposal = useSkillStore((s) => s.createSkillFromProposal);
  return { skillProposals, loadSkillProposals, createSkillFromProposal };
}
