// SPDX-License-Identifier: AGPL-3.0-only

import { invoke } from "@/lib/invoke";
import type { ScheduledTask, TaskExecutionRecord } from "@/types";
import { toEpochMs } from "../formatUtils";
import { runIdOf, type RunState, type UnifiedRun } from "../model";
import { canExecuteNow } from "../taskGrouping";
import type { TaskSourceAdapter } from "./types";

/**
 * scheduled-cron 源：CronJob 「定义」与其「最近一次执行」。
 *
 * 注意 cron 与其它源不同——它是**定义**而非运行实例：
 * - list() 把每个任务的 lastResult 合成一条「最近运行」UnifiedRun（零额外 IPC），
 *   从未跑过的任务不出现在 Runs 表（在「计划任务」Tab 管理定义）；
 * - 历史（最近 50 条/任务）走 getHistory；
 * - retry 语义 = 立即执行一次（execute_scheduled_task），仅可执行任务开放。
 */
export const cronAdapter: TaskSourceAdapter = {
  id: "scheduled-cron",
  nameKey: "tasksCenter.source.scheduledCron",
  color: "geekblue",

  async list(): Promise<UnifiedRun[]> {
    const rows = await invoke<ScheduledTask[]>("list_scheduled_tasks");
    if (!Array.isArray(rows)) {
      return [];
    }
    return rows
      .filter((task) => task.lastResult !== null && task.lastRunAt !== null)
      .map((task) => mapLastRun(task));
  },

  async retry(nativeId: string): Promise<void> {
    await invoke("execute_scheduled_task", { taskId: nativeId });
  },

  async getHistory(nativeId: string): Promise<UnifiedRun[]> {
    const rows = await invoke<TaskExecutionRecord[]>("get_task_execution_history", {
      taskId: nativeId,
    });
    if (!Array.isArray(rows)) {
      return [];
    }
    return rows.map((rec) => mapExecutionRecord(nativeId, rec, undefined));
  },
};

/** 任务定义 + 最近一次结果 → UnifiedRun。 */
export function mapLastRun(task: ScheduledTask): UnifiedRun {
  const result = task.lastResult;
  const finishedAt = toEpochMs(task.lastRunAt);
  const state: RunState = !result
    ? "queued"
    : result.skipped
    ? "succeeded"
    : result.success
    ? "succeeded"
    : "failed";
  return {
    id: runIdOf("scheduled-cron", task.id),
    source: "scheduled-cron",
    nativeId: task.id,
    name: task.name,
    trigger: "schedule",
    state,
    badges: result?.skipped ? ["skipped"] : undefined,
    attempts: 0,
    startedAt: finishedAt !== null && result ? finishedAt - (result.durationMs ?? 0) : undefined,
    finishedAt: finishedAt ?? undefined,
    durationMs: result?.durationMs,
    error: result?.error ? { message: result.error } : undefined,
    deepLink: "/tasks?tab=schedules",
    capabilities: {
      retry: canExecuteNow(task),
      viewHistory: true,
      openSource: true,
    },
    raw: task,
  };
}

/** 单条执行历史 → UnifiedRun（任务定义可选，用于补名称）。 */
export function mapExecutionRecord(
  taskId: string,
  rec: TaskExecutionRecord,
  task: ScheduledTask | undefined,
): UnifiedRun {
  return {
    id: runIdOf("scheduled-cron", rec.id),
    source: "scheduled-cron",
    nativeId: rec.task_id || taskId,
    name: task?.name ?? taskId,
    trigger: "schedule",
    state: rec.success ? "succeeded" : "failed",
    attempts: 0,
    startedAt: rec.started_at,
    finishedAt: rec.completed_at ?? undefined,
    durationMs: rec.duration_ms,
    error: rec.error ? { message: rec.error } : undefined,
    capabilities: {
      openSource: true,
    },
  };
}
