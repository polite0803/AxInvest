// SPDX-License-Identifier: AGPL-3.0-only

import { useUserProfileStore } from "@/stores/feature/userProfileStore";

/**
 * 当前用户档案（订阅；自 ChatViewMessages 抽出，组件中立性消键）。
 *
 * 活源是 userProfileStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * userProfileStore（组件中立性），统一从这里取。
 */
export function useUserProfile() {
  return useUserProfileStore((s) => s.profile);
}
