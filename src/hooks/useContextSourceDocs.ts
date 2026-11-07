// SPDX-License-Identifier: AGPL-3.0-only

import { useContextSourceStore } from "@/stores/feature/contextSourceStore";
import { useKnowledgeStore } from "@/stores/feature/knowledgeStore";
import { useMemoryStore } from "@/stores/feature/memoryStore";
import { useWikiStore } from "@/stores/feature/wikiStore";

/**
 * 上下文源文档清单接入（自 ContextSourceDocsPanel 抽出，组件中立性消键）：
 * 三类容器（knowledge / memory / wiki）的文档列表 + 懒加载 action，
 * 以及把勾选结果写回 context_sources 表 doc_ids_json 字段的 setDocIds。
 *
 * 注意：不复用 useContextSources（sources/loadSources 读面）——本场景
 * 只消费 setDocIds，订阅面按原组件 selector 逐字保持。
 */
export function useContextSourceDocs() {
  const setDocIds = useContextSourceStore((s) => s.setDocIds);
  const knowledgeDocuments = useKnowledgeStore((s) => s.documents);
  const loadKnowledgeDocuments = useKnowledgeStore((s) => s.loadDocuments);
  const memoryItems = useMemoryStore((s) => s.items);
  const loadMemoryItems = useMemoryStore((s) => s.loadItems);
  const wikiNotes = useWikiStore((s) => s.notes);
  const loadWikiNotes = useWikiStore((s) => s.loadNotes);
  return {
    setDocIds,
    knowledgeDocuments,
    loadKnowledgeDocuments,
    memoryItems,
    loadMemoryItems,
    wikiNotes,
    loadWikiNotes,
  };
}
