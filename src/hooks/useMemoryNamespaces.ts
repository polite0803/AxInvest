// SPDX-License-Identifier: AGPL-3.0-only

import { useMemoryStore } from "@/stores/feature/memoryStore";

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
