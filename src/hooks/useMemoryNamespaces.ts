// SPDX-License-Identifier: AGPL-3.0-only

import { useMemoryStore } from "@/stores/feature/memoryStore";
import { useEffect } from "react";

/**
 * 记忆命名空间数据接入（自 ExtractMemoriesModal 抽出，组件中立性消键）：
 * 命名空间列表 / 加载态 / 拉取 action。
 */
export function useMemoryNamespaces() {
  const namespaces = useMemoryStore((s) => s.namespaces);
  const loadNamespaces = useMemoryStore((s) => s.loadNamespaces);
  const loading = useMemoryStore((s) => s.loading);
  return { namespaces, loading, loadNamespaces };
}

/** 挂载期预热：命名空间为空时拉取一次（空依赖 effect，自 InputArea 逐字搬入）。 */
export function useMemoryNamespacesBootstrap(): void {
  useEffect(() => {
    if ((useMemoryStore.getState().namespaces ?? []).length === 0) {
      useMemoryStore.getState().loadNamespaces();
    }
  }, []);
}
