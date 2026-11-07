// SPDX-License-Identifier: AGPL-3.0-only

import { useTaskShapeApprovalStore } from "@/stores/feature/taskShapeApprovalStore";

/**
 * 任务形态审批读面（待审批单 + 加载态 + 通过/驳回）。
 *
 * 抽成 hook 是为了让 chat（TaskShapeApprovalModal）组件不直接依赖
 * taskShapeApprovalStore（组件中立性，见 scripts/check-component-neutrality.mjs），
 * 统一从这里取。
 */
export function useTaskShapeApproval() {
  const pendingApproval = useTaskShapeApprovalStore((s) => s.pendingApproval);
  const loading = useTaskShapeApprovalStore((s) => s.loading);
  const approve = useTaskShapeApprovalStore((s) => s.approve);
  const reject = useTaskShapeApprovalStore((s) => s.reject);
  return { pendingApproval, loading, approve, reject };
}
