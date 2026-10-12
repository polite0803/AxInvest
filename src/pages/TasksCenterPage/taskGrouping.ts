// SPDX-License-Identifier: AGPL-3.0-only

/** 任务中心「计划任务」分组（FR-2.1）。 */
export type ScheduledTaskGroup = "invest" | "opc" | "system" | "custom";

/**
 * 投资自动化内置 taskType 白名单（与后端种子任务 / executor handler 对齐）。
 * trend-screening / serenity 工作流走 workflowId 判据，不在此表。
 */
const INVEST_TASK_TYPES = new Set([
  "stock-recommendation",
  "trend-screening",
  "pool-scan",
  "watchlist-scan",
  "stock-analysis",
  "batch-reflection",
  "validate-decisions",
  "decision-backtest",
]);

/** 系统维护内置 taskType 白名单。 */
const SYSTEM_TASK_TYPES = new Set([
  "backup",
  "cleanup",
  "daily_summary",
  "knowledge_source_fetch_all",
]);

/**
 * 拥有内置执行 handler 的 taskType（与后端 `BUILTIN_TASK_TYPES` 逐字对齐，
 * scheduled_task.rs execute_scheduled_task 分支 2）。其余任务必须绑定 workflowId
 * 才能手动执行（分支 1）；裸 custom/general 无执行器（分支 3 报错）。
 */
const BUILTIN_EXECUTABLE_TYPES = new Set([
  "knowledge_source_fetch_all",
  "stock-recommendation",
  "watchlist-scan",
  "pool-scan",
  "batch-reflection",
  "validate-decisions",
  "opc_demand_scan",
  "decision-backtest",
]);

type TaskLike = {
  taskType?: string | null;
  workflowId?: string | null;
};

/**
 * 判定任务分组：
 * 1. OPC：taskType 以 `opc` 起头（`opc-demand-discovery` / `opc_demand_scan`），
 *    或 workflowId 以 `opc-` / `opc_` 起头；
 * 2. 投资：taskType 在投资白名单，或 workflowId 为 trend-screening / 含 serenity；
 * 3. 系统：taskType 在系统白名单；
 * 4. 其余（custom / general / 普通工作流）兜底自定义。
 */
export function classifyScheduledTask(task: TaskLike): ScheduledTaskGroup {
  const taskType = (task.taskType ?? "").toLowerCase();
  const workflowId = (task.workflowId ?? "").toLowerCase();

  if (taskType.startsWith("opc") || workflowId.startsWith("opc-") || workflowId.startsWith("opc_")) {
    return "opc";
  }
  if (
    INVEST_TASK_TYPES.has(taskType)
    || workflowId === "trend-screening"
    || workflowId.includes("serenity")
  ) {
    return "invest";
  }
  if (SYSTEM_TASK_TYPES.has(taskType)) {
    return "system";
  }
  return "custom";
}

/**
 * 是否允许「立即执行」（对齐后端 execute_scheduled_task 的三条分支）：
 * 绑定工作流（分支 1）或内置 handler 白名单（分支 2）才可执行；
 * 裸 custom/general 置灰（分支 3 必报 MANUAL_RUN_NOT_SUPPORTED）。
 */
export function canExecuteNow(task: TaskLike): boolean {
  if (task.workflowId) {
    return true;
  }
  return BUILTIN_EXECUTABLE_TYPES.has((task.taskType ?? "").toLowerCase());
}

/**
 * 是否为系统种子内置任务（删除后重启不会自动重建，删除确认需额外警示）。
 * 三个内置分组（投资 / OPC / 系统）均来自种子；自定义组为用户自建。
 */
export function isBuiltinTask(task: TaskLike): boolean {
  return classifyScheduledTask(task) !== "custom";
}
