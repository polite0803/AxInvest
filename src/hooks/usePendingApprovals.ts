// SPDX-License-Identifier: AGPL-3.0-only

import { useApprovalStore } from "@/stores/feature/approvalStore";

/**
 * 待审批请求列表、加载态及拉取动作。
 *
 * 活源是 approvalStore；抽成 hook 是为了让 chat 等跨域组件不直接依赖
 * approvalStore（组件中立性），统一从这里取。
 */
export function usePendingApprovals() {
  const pendingApprovals = useApprovalStore((s) => s.pendingApprovals);
  const loading = useApprovalStore((s) => s.loading);
  const fetchPendingApprovals = useApprovalStore((s) => s.fetchPendingApprovals);

  return { pendingApprovals, loading, fetchPendingApprovals };
}
