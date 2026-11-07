// SPDX-License-Identifier: AGPL-3.0-only

import { useCapabilityStore } from "@/stores/feature/capabilityStore";

/**
 * capabilityStore 的能力发现面板读面（CapabilityDiscoveryPanel 专用，
 * 组件中立性消键，2026-11 #222 补洞批次）。
 *
 * 活源是 capabilityStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * capabilityStore（组件中立性），统一从这里取。内部为逐条 selector 订阅
 * （与原组件的订阅粒度一字不差），返回值形态与原 selector 严格一致。
 */
export function useCapabilityDiscovery() {
  const passports = useCapabilityStore((s) => s.passports);
  const stats = useCapabilityStore((s) => s.stats);
  const discoveryResult = useCapabilityStore((s) => s.discoveryResult);
  const isLoading = useCapabilityStore((s) => s.isLoading);
  const isDiscovering = useCapabilityStore((s) => s.isDiscovering);
  const error = useCapabilityStore((s) => s.error);
  const listPassports = useCapabilityStore((s) => s.listPassports);
  const getStats = useCapabilityStore((s) => s.getStats);
  const discover = useCapabilityStore((s) => s.discover);
  const evolveCapability = useCapabilityStore((s) => s.evolveCapability);
  return {
    passports,
    stats,
    discoveryResult,
    isLoading,
    isDiscovering,
    error,
    listPassports,
    getStats,
    discover,
    evolveCapability,
  };
}
