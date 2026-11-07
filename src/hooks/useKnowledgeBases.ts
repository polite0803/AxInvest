// SPDX-License-Identifier: AGPL-3.0-only

import { useKnowledgeStore } from "@/stores/feature/knowledgeStore";
import { useEffect } from "react";

/**
 * 知识库清单读面（bases + loadBases）。
 *
 * 抽成 hook 是为了让 chat（ChatSidebar / InputArea）/ wiki（WikiDetailPanel）
 * 等跨域组件不直接依赖 knowledgeStore（组件中立性，
 * 见 scripts/check-component-neutrality.mjs），统一从这里取。
 */
export function useKnowledgeBases() {
  const knowledgeBases = useKnowledgeStore((s) => s.bases);
  const loadKnowledgeBases = useKnowledgeStore((s) => s.loadBases);
  return { knowledgeBases, loadKnowledgeBases };
}

/**
 * 挂载期知识库清单预热（InputArea 专用，effect 体逐字搬入）：
 * 清单为空时拉一次。
 */
export function useKnowledgeBasesBootstrap(): void {
  useEffect(() => {
    if ((useKnowledgeStore.getState().bases ?? []).length === 0) {
      useKnowledgeStore.getState().loadBases();
    }
  }, []);
}
