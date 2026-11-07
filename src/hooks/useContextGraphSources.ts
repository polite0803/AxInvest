// SPDX-License-Identifier: AGPL-3.0-only

import { useKnowledgeStore } from "@/stores/feature/knowledgeStore";
import { useMcpStore } from "@/stores/feature/mcpStore";
import { useMemoryStore } from "@/stores/feature/memoryStore";
import { useSkillExtensionStore } from "@/stores/feature/skillExtensionStore";

/**
 * 上下文图谱的四类源清单接入（自 ContextGraphPanel 抽出，组件中立性消键）：
 * 知识库 / 记忆命名空间 / MCP 服务器 / 已安装技能的只读列表。
 *
 * 保留原组件的 `?? []` 空值兜底形态，渲染行为与原 selector 逐字一致。
 */
export function useContextGraphSources() {
  const knowledgeBases = useKnowledgeStore((s) => s.bases ?? []);
  const memoryNamespaces = useMemoryStore((s) => s.namespaces ?? []);
  const mcpServers = useMcpStore((s) => s.servers ?? []);
  const installedSkills = useSkillExtensionStore((s) => s.skills ?? []);
  return { knowledgeBases, memoryNamespaces, mcpServers, installedSkills };
}
