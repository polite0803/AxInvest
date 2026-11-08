// SPDX-License-Identifier: AGPL-3.0-only

import { useSkillStore } from "@/stores/feature/skillStore";
import type { Skill } from "@/types";
import { useCallback, useEffect, useState } from "react";

/**
 * 技能目录：skills 列表读 + toggleSkill 写。
 *
 * 活源是 skillStore；抽成 hook 是为了让 agent 等跨域组件不直接依赖
 * skillStore（组件中立性），统一从这里取。
 * 首次为空时自动 loadSkills，并订阅后续变更；toggle 静默失败（对齐原组件行为）。
 */
export function useSkillDirectory() {
  const [skills, setSkills] = useState<Skill[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    let unsub: (() => void) | undefined;
    (async () => {
      try {
        const store = useSkillStore.getState();
        if (store.skills.length === 0) {
          await store.loadSkills();
        }
        if (!cancelled) {
          setSkills(useSkillStore.getState().skills);
          setLoading(false);
          unsub = useSkillStore.subscribe((s) => {
            if (!cancelled) {
              setSkills(s.skills);
            }
          });
        }
      } catch {
        if (!cancelled) { setLoading(false); }
      }
    })();
    return () => {
      cancelled = true;
      unsub?.();
    };
  }, []);

  const toggleSkill = useCallback(async (name: string, enabled: boolean) => {
    try {
      await useSkillStore.getState().toggleSkill(name, enabled);
    } catch {
      // 静默失败
    }
  }, []);

  return { skills, loading, toggleSkill };
}
