// SPDX-License-Identifier: AGPL-3.0-only

import { useSkillExtensionStore } from "@/stores/feature/skillExtensionStore";

/**
 * 命令面板的技能扩展命令接入（自 CommandPalette 抽出，组件中立性消键）：
 * skillExtensionStore 注册的动态命令列表。
 */
export function useSkillCommands() {
  return useSkillExtensionStore((s) => s.commands);
}
