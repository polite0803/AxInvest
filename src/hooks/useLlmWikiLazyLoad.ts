// SPDX-License-Identifier: AGPL-3.0-only

import { useLlmWikiStore } from "@/stores/feature/llmWikiStore";
import { useEffect } from "react";

/**
 * LLM Wiki 数据的挂载期懒加载（自 InputArea 抽出，组件中立性消键）。
 * wikis 为空时拉取一次；依赖必须为空数组，避免 store 在 IPC 失败时
 * set 新空数组导致引用变化 → useEffect 重新执行 → 无限循环。
 */
export function useLlmWikiLazyLoad(): void {
  useEffect(() => {
    if ((useLlmWikiStore.getState().wikis ?? []).length === 0) {
      useLlmWikiStore.getState().loadWikis();
    }
  }, []);
}
