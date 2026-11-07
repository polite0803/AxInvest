// SPDX-License-Identifier: AGPL-3.0-only

import { useSkillExtensionStore } from "@/stores/feature/skillExtensionStore";

/**
 * 全局状态栏的技能条目计数接入（自 GlobalStatusBar 抽出，组件中立性消键）。
 *
 * selector 返回长度数字（与原组件一致），条目数组内部变化但长度不变时不触发重渲染。
 */
export function useSkillStatusBarCount(): number {
  return useSkillExtensionStore((s) => s.statusBarItems.length);
}
