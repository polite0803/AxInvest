// SPDX-License-Identifier: AGPL-3.0-only

import { useKnowledgeStore } from "@/stores/feature/knowledgeStore";

/**
 * 知识文件浏览读面（分类表格 + 搜索/排序 + 定位/删除）。
 *
 * 抽成 hook 是为了让 files（FilesContent）组件不直接依赖 knowledgeStore
 * （组件中立性，见 scripts/check-component-neutrality.mjs），统一从这里取。
 * 返回形态与原组件解构完全一致，不改任何逻辑。
 */
export function useKnowledgeFileBrowser() {
  const rows = useKnowledgeStore((s) => s.rows);
  const search = useKnowledgeStore((s) => s.search);
  const error = useKnowledgeStore((s) => s.error);
  const loadCategory = useKnowledgeStore((s) => s.loadCategory);
  const setSearch = useKnowledgeStore((s) => s.setSearch);
  const setSortKey = useKnowledgeStore((s) => s.setSortKey);
  const clearError = useKnowledgeStore((s) => s.clearError);
  const revealEntry = useKnowledgeStore((s) => s.revealEntry);
  const deleteEntry = useKnowledgeStore((s) => s.deleteEntry);
  return { rows, search, error, loadCategory, setSearch, setSortKey, clearError, revealEntry, deleteEntry };
}
