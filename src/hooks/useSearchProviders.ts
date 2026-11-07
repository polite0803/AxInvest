// SPDX-License-Identifier: AGPL-3.0-only

import { useSearchStore } from "@/stores/feature/searchStore";
import { useEffect } from "react";

/**
 * 搜索服务商读面接入（自 InputArea 抽出，组件中立性消键）：
 * 服务商清单订阅 + 挂载期空清单预热。
 *
 * 活源是 searchStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * searchStore（组件中立性），统一从这里取。
 */

/** 搜索服务商清单（订阅）。 */
export function useSearchProviders() {
  return useSearchStore((s) => s.providers);
}

/** 挂载期预热：清单为空时拉取一次（空依赖 effect，自 InputArea 逐字搬入）。 */
export function useSearchProvidersBootstrap(): void {
  useEffect(() => {
    if ((useSearchStore.getState().providers ?? []).length === 0) {
      useSearchStore.getState().loadProviders();
    }
  }, []);
}
