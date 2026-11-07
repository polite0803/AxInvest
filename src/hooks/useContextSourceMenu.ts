// SPDX-License-Identifier: AGPL-3.0-only

import { useKnowledgeStore } from "@/stores/feature/knowledgeStore";
import { useLlmWikiStore } from "@/stores/feature/llmWikiStore";
import { useMemoryStore } from "@/stores/feature/memoryStore";

/**
 * 上下文源菜单的三类容器清单接入（自 ContextSourceMenu 抽出，组件中立性消键）：
 * 知识库 / 记忆命名空间 / LLM Wiki 的只读列表（原始引用，空值兜底由消费方处理）。
 *
 * 注意：不复用 useMemoryNamespaces——它会额外订阅 loading/loadNamespaces，
 * 给本组件引入原实现没有的重渲染时机，故此处只订阅原 selector 覆盖的读面。
 */
export function useContextSourceMenu() {
  const knowledgeBases = useKnowledgeStore((s) => s.bases);
  const memoryNamespaces = useMemoryStore((s) => s.namespaces);
  const wikis = useLlmWikiStore((s) => s.wikis);
  return { knowledgeBases, memoryNamespaces, wikis };
}
