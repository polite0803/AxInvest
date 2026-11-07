// SPDX-License-Identifier: AGPL-3.0-only

import { useSkillExtensionStore } from "@/stores";
import type { MergedToolbarButton } from "@/stores/feature/skillExtensionStore";

export type { MergedToolbarButton };

/**
 * 技能扩展工具栏按钮数据接入（自 SkillToolbar 抽出，组件中立性消键）：
 * SkillToolbar 下沉 common 后是纯渲染基元，store 消费收敛到本 hook。
 */
export function useSkillToolbarButtons(): MergedToolbarButton[] {
  return useSkillExtensionStore((s) => s.toolbarButtons);
}
