// SPDX-License-Identifier: AGPL-3.0-only

/**
 * P3 编排前端纯校验与状态投影（无 IO、可直接单测）。
 *
 * 判据与后端 `bot_workflow_steps.rs` 的 `parse_steps` / `validate_dag` 对齐：
 * 前端只做「提前拦住明显错误」，错误码与后端 IPC-P3 §4 逐字一致，
 * 最终裁决仍在命令层（前端校验不能替代后端校验）。
 */

import type {
  BotAssignment,
  BotWorkflowRun,
  BotWorkflowRunStatus,
  BotWorkflowStep,
  BotWorkflowValidationCode,
  BotWorkflowValidationIssue,
  BotWorkflowValidationResult,
} from "@/types";

/** 定义上限（MODEL-P3 §2.1.1：步骤 ≤12、依赖 ≤11、并发 ≤4）。 */
export const WORKFLOW_STEP_LIMIT = 12;
export const WORKFLOW_DEPENDENCY_LIMIT = 11;
export const WORKFLOW_MAX_CONCURRENCY = 4;
export const WORKFLOW_TOTAL_TIMEOUT_SECS = 900;

const STEP_KEY_RE = /^[A-Za-z0-9_-]{1,32}$/;
const STEP_REF_RE = /\{\{steps\.([A-Za-z0-9_-]+)\.output\}\}/g;

/** P3 错误码 → locale `error.<CODE>` 键，供 UI 直接交给 `t()`。 */
export function workflowErrorI18nKey(code: string | null | undefined): string | null {
  if (!code || !/^BOT_(WORKFLOW|DELEGATE)_[A-Z_]+$/.test(code)) {
    return null;
  }
  return `error.${code}`;
}

/** 已知可被前端预校验发出的错误码（其余错误码一律交给后端返回）。 */
export const FRONTEND_VALIDATION_CODES: readonly BotWorkflowValidationCode[] = [
  "BOT_WORKFLOW_STEP_INVALID",
  "BOT_WORKFLOW_CYCLE",
  "BOT_WORKFLOW_DEPARTMENT_REQUIRED",
];

function issue(code: BotWorkflowValidationCode, stepKey?: string): BotWorkflowValidationIssue {
  return stepKey ? { code, stepKey } : { code };
}

/**
 * 定义期 DAG 与字段纯校验：一次性收集全部问题，避免用户改一处报一处。
 * `dependsOn` 自引用归入 CYCLE（与后端 validate_dag 同码）。
 */
export function validateWorkflowSteps(steps: BotWorkflowStep[]): BotWorkflowValidationResult {
  const issues: BotWorkflowValidationIssue[] = [];
  if (!Array.isArray(steps) || steps.length === 0 || steps.length > WORKFLOW_STEP_LIMIT) {
    return { ok: false, issues: [issue("BOT_WORKFLOW_STEP_INVALID")] };
  }

  const keys = new Set<string>();
  for (const step of steps) {
    if (!STEP_KEY_RE.test(step.stepKey ?? "") || keys.has(step.stepKey)) {
      issues.push(issue("BOT_WORKFLOW_STEP_INVALID", step.stepKey));
    }
    keys.add(step.stepKey);
    if (
      !step.botId || typeof step.promptTemplate !== "string"
      || step.promptTemplate.length < 1 || step.promptTemplate.length > 20000
    ) {
      issues.push(issue("BOT_WORKFLOW_STEP_INVALID", step.stepKey));
    }
    if (!Number.isInteger(step.timeoutSecs) || step.timeoutSecs < 1 || step.timeoutSecs > 300) {
      issues.push(issue("BOT_WORKFLOW_STEP_INVALID", step.stepKey));
    }
    if (step.onFailure !== "abort" && step.onFailure !== "continue") {
      issues.push(issue("BOT_WORKFLOW_STEP_INVALID", step.stepKey));
    }
    if (
      !Array.isArray(step.dependsOn) || step.dependsOn.length > WORKFLOW_DEPENDENCY_LIMIT
      || step.dependsOn.some((key) => typeof key !== "string" || !STEP_KEY_RE.test(key))
    ) {
      issues.push(issue("BOT_WORKFLOW_STEP_INVALID", step.stepKey));
    }
  }

  for (const step of steps) {
    for (const dep of step.dependsOn ?? []) {
      if (!keys.has(dep)) {
        issues.push(issue("BOT_WORKFLOW_STEP_INVALID", step.stepKey));
      }
      if (dep === step.stepKey) {
        issues.push(issue("BOT_WORKFLOW_CYCLE", step.stepKey));
      }
    }
    for (const match of step.promptTemplate.matchAll(STEP_REF_RE)) {
      if (!keys.has(match[1])) {
        issues.push(issue("BOT_WORKFLOW_STEP_INVALID", step.stepKey));
      }
    }
  }

  // 拓扑排序（Kahn）：入度归零不了的节点留在环里。
  const indegree = new Map<string, number>(steps.map((s) => [s.stepKey, 0]));
  const downstream = new Map<string, string[]>(steps.map((s) => [s.stepKey, []]));
  for (const step of steps) {
    for (const dep of new Set(step.dependsOn ?? [])) {
      if (!indegree.has(dep)) {
        continue;
      }
      indegree.set(step.stepKey, (indegree.get(step.stepKey) ?? 0) + 1);
      downstream.get(dep)?.push(step.stepKey);
    }
  }
  const queue = [...indegree.entries()].filter(([, degree]) => degree === 0).map(([key]) => key);
  let visited = 0;
  while (queue.length > 0) {
    const key = queue.shift() as string;
    visited += 1;
    for (const next of downstream.get(key) ?? []) {
      const degree = (indegree.get(next) ?? 0) - 1;
      indegree.set(next, degree);
      if (degree === 0) {
        queue.push(next);
      }
    }
  }
  if (visited !== steps.length && !issues.some((i) => i.code === "BOT_WORKFLOW_CYCLE")) {
    issues.push(issue("BOT_WORKFLOW_CYCLE"));
  }

  return { ok: issues.length === 0, issues };
}

/** 步骤终态判据（与 P2-b `isTerminalStatus` 同语义，避免 UI 层交叉 import）。 */
export function isAssignmentTerminal(status: BotAssignment["status"]): boolean {
  return status === "completed" || status === "failed" || status === "cancelled";
}

/**
 * 运行态投影：根步骤 completed 但子链仍在途时 run 仍显示 running（ADR-P3-2 ⑤ / MODEL-P3 §2.3.1）。
 * failed/cancelled 是显式终态，不被子树状态覆盖。
 */
export function effectiveRunStatus(run: BotWorkflowRun, assignments: BotAssignment[]): BotWorkflowRunStatus {
  const inFlight = assignments.some((a) => a.workflowRunId === run.id && !isAssignmentTerminal(a.status));
  if (inFlight && run.status !== "failed" && run.status !== "cancelled") {
    return "running";
  }
  return run.status;
}

/** run 终态判据：终态按钮禁用依此计算（取消仅对 queued/running/waiting 有效）。 */
export function isRunTerminal(run: BotWorkflowRun, assignments: BotAssignment[]): boolean {
  const status = effectiveRunStatus(run, assignments);
  return status === "completed" || status === "failed" || status === "cancelled";
}

/** 运行态 → i18n 键（与 I18N-KEYS-P3 §2.2 六态逐字对应）。 */
export function runStatusI18nKey(status: BotWorkflowRunStatus): string {
  return `bot.workflowRun.status${status.charAt(0).toUpperCase()}${status.slice(1)}`;
}

/**
 * 按 rootId 取链并按 depth→createdAt 排序（链视图顺序：L1 → L2 → L3）。
 * 根行 rootId 恒为自身 id（MODEL-P3 §2.3），无 rootId 的老数据退化为按自身分组。
 */
export function selectDelegationChain(assignments: BotAssignment[], rootId: string): BotAssignment[] {
  return assignments
    .filter((a) => (a.rootId || a.id) === rootId)
    .slice()
    .sort((a, b) => a.depth - b.depth || a.createdAt - b.createdAt || a.id.localeCompare(b.id));
}

/** 所有链根的 id（去重、按创建时间排序），供链视图下拉选择。 */
export function delegationRootIds(assignments: BotAssignment[]): string[] {
  const roots = new Map<string, number>();
  for (const a of assignments) {
    const root = a.rootId || a.id;
    roots.set(root, Math.min(roots.get(root) ?? a.createdAt, a.createdAt));
  }
  return [...roots.entries()].sort((a, b) => a[1] - b[1] || a[0].localeCompare(b[0])).map(([id]) => id);
}

/** 部门筛选：实例级编排对所有部门可见（PLAN-P3 §6「部门筛选含 instance 编排」）。 */
export function filterWorkflowsByDepartment<
  T extends { ownerScope: "instance" | "department"; departmentId: string | null },
>(
  workflows: T[],
  departmentId: string | null,
): T[] {
  if (departmentId === null) {
    return workflows;
  }
  return workflows.filter((w) => w.ownerScope === "instance" || w.departmentId === departmentId);
}
