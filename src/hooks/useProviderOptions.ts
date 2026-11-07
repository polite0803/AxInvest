// SPDX-License-Identifier: AGPL-3.0-only

import { useShallow } from "zustand/react/shallow";

import { useProviderStore } from "@/stores/feature/providerStore";

/**
 * LLM 提供商列表读口（providerStore）的中介投影。
 *
 * 活源是 providerStore；抽成 hook 是为了让 multi-agent 等跨域组件不直接
 * 依赖 providerStore（组件中立性），统一从这里取。
 */
export function useProviderOptions() {
  return useProviderStore(
    useShallow((s) => ({
      providers: s.providers,
      fetchProviders: s.fetchProviders,
    })),
  );
}
