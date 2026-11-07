// SPDX-License-Identifier: AGPL-3.0-only

import { useEvolutionStore } from "@/stores/feature/evolutionStore";

/**
 * evolutionStore 的进化面板读面（EvolutionConsentModal / EvolutionEvidencePanel
 * 两个 chat 组件共用，组件中立性消键，2026-11 #222 补洞批次）。
 *
 * 活源是 evolutionStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * evolutionStore（组件中立性），统一从这里取。内部为逐条 selector 订阅
 * （与原组件的订阅粒度一字不差），返回值形态与原 selector 严格一致。
 */

/** 能力缺口待审批面：待处理提案映射 + 应答动作引用（EvolutionConsentModal 专用）。 */
export function useEvolutionConsent() {
  const pendingConsent = useEvolutionStore((s) => s.pendingConsent);
  const respondConsent = useEvolutionStore((s) => s.respondConsent);
  return { pendingConsent, respondConsent };
}

/**
 * 进化证据面：决策后验证据 + 加载态 + 拉取动作引用
 * （EvolutionEvidencePanel 专用；evidence / loading 为组件局部名，
 * 对应 store 字段 evolutionEvidence / evidenceLoading，仅按原名转发。
 * store 态本身是全量，不按会话过滤 —— 拉取时由调用方给
 * fetchEvolutionEvidence 传 conversationId，与原组件一致）。
 */
export function useEvolutionEvidence() {
  const evidence = useEvolutionStore((s) => s.evolutionEvidence);
  const loading = useEvolutionStore((s) => s.evidenceLoading);
  const fetchEvolutionEvidence = useEvolutionStore((s) => s.fetchEvolutionEvidence);
  return { evidence, loading, fetchEvolutionEvidence };
}
