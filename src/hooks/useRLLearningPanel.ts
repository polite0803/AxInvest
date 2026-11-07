// SPDX-License-Identifier: AGPL-3.0-only

import { useDomainLearningStore } from "@/stores/feature/domainLearningStore";

/**
 * RL 学习面板读面（经验池统计 + 策略优化 + 自动学习历史 + 配置读写）。
 *
 * 抽成 hook 是为了让 opc（RLLearningPanel）组件不直接依赖 domainLearningStore
 * （组件中立性，见 scripts/check-component-neutrality.mjs），统一从这里取。
 * 返回形态与原组件解构完全一致，不改任何逻辑。
 */
export function useRLLearningPanel() {
  const loadRLStats = useDomainLearningStore((s) => s.loadRLStats);
  const loadConfig = useDomainLearningStore((s) => s.loadConfig);
  const triggerOptimization = useDomainLearningStore((s) => s.triggerOptimization);
  const rlStats = useDomainLearningStore((s) => s.rlStats);
  const rlGlobalStats = useDomainLearningStore((s) => s.rlGlobalStats);
  const rlPolicyUpdates = useDomainLearningStore((s) => s.rlPolicyUpdates);
  const autoLearningHistory = useDomainLearningStore((s) => s.autoLearningHistory);
  const rlLoading = useDomainLearningStore((s) => s.rlLoading);
  const getConfig = useDomainLearningStore((s) => s.getConfig);
  return {
    loadRLStats,
    loadConfig,
    triggerOptimization,
    rlStats,
    rlGlobalStats,
    rlPolicyUpdates,
    autoLearningHistory,
    rlLoading,
    getConfig,
  };
}
