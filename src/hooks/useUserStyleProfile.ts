// SPDX-License-Identifier: AGPL-3.0-only

import { useShallow } from "zustand/react/shallow";

import { useStyleStore } from "@/stores/feature/styleStore";

/**
 * 用户风格档案（styleStore）的中介读口。
 *
 * 活源是 styleStore；抽成 hook 是为了让 layout / settings / common 等
 * 跨域组件不直接依赖 styleStore（组件中立性），统一从这里取。
 * 切片集合取自既有全部消费方（UserProfileModal / UserProfileSettings /
 * StylePreviewPanel），用 useShallow 保持订阅面稳定。
 */
export function useUserStyleProfile() {
  return useStyleStore(
    useShallow((s) => ({
      currentProfile: s.currentProfile,
      appliedStyle: s.appliedStyle,
      isApplying: s.isApplying,
      loadStyleProfile: s.loadStyleProfile,
      getStats: s.getStats,
      applyStyleToCode: s.applyStyleToCode,
      adjustStyleDimension: s.adjustStyleDimension,
      resetToDefaults: s.resetToDefaults,
    })),
  );
}
