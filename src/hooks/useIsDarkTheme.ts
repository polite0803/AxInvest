// SPDX-License-Identifier: AGPL-3.0-only

import { useResolvedDarkMode } from "@/hooks/useResolvedDarkMode";
import { useSettingsStore } from "@/stores";
import type { ThemePreset } from "@/theme/shadcnTheme";

/** 当前生效的深色状态：预设优先，其次用户设置，最后系统偏好。 */
export function useIsDarkTheme(): boolean {
  const themeMode = useSettingsStore((s) => s.settings.themeMode);
  const themePreset = useSettingsStore((s) => s.settings.themePreset) as
    | ThemePreset
    | undefined;
  return useResolvedDarkMode(themeMode, themePreset);
}
