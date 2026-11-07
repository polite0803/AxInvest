// SPDX-License-Identifier: AGPL-3.0-only

import { useSkillStore } from "@/stores/feature/skillStore";

/**
 * 技能增改写操作（createSkill / patchSkill / editSkill）。
 *
 * 抽成 hook 是为了让 chat（SkillCreateEditModal）组件不直接依赖 skillStore
 * （组件中立性，见 scripts/check-component-neutrality.mjs），统一从这里取。
 */
export function useSkillMutations() {
  const createSkill = useSkillStore((s) => s.createSkill);
  const patchSkill = useSkillStore((s) => s.patchSkill);
  const editSkill = useSkillStore((s) => s.editSkill);
  return { createSkill, patchSkill, editSkill };
}
