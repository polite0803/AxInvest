// SPDX-License-Identifier: AGPL-3.0-only

import { getXtermTheme, useThemeStore } from "@/stores/feature/themeStore";
import { useCallback } from "react";

/**
 * 终端 xterm 配色：currentTheme 订阅 + getXtermTheme 映射（themeStore 活源）。
 *
 * 抽成 hook 是为了让 terminal 等组件不直接依赖 themeStore（组件中立性）。
 * getThemeName 供一次性初始化等非订阅路径读快照，
 * 对齐原 useThemeStore.getState().currentTheme 语义。
 */
export function useXtermTheme() {
  const currentTheme = useThemeStore((s) => s.currentTheme);
  const getThemeName = useCallback(() => useThemeStore.getState().currentTheme, []);
  return { currentTheme, getThemeName, getXtermTheme };
}
