// SPDX-License-Identifier: AGPL-3.0-only

import { invoke } from "@/lib/invoke";
import type { BackgroundTaskInfo } from "@/types";
import { runIdOf, type RunState, type UnifiedRun } from "../model";
import type { TaskSourceAdapter } from "./types";

/**
 * background-tasks 源：`background_tasks` 表。
 *
 * 两个写入来源：chat/agent 的 bash 型后台任务，以及同步慢命令的内嵌台账
 * （`run_tracked`：收盘采集 / 收盘历史补齐 / 荐股策略回测——命令仍同步返回
 * 报告给调用面板，这里只投影运行记录，让手动触发的慢命令有终态与耗时可见）。
 * 状态未知一律映射 failed，不得假 running（同 stockAnalysis 源纪律）。
 */
export const backgroundTasksAdapter: TaskSourceAdapter = {
  id: "background-tasks",
  nameKey: "tasksCenter.source.backgroundTasks",
  color: "geekblue",

  async list(): Promise<UnifiedRun[]> {
    const rows = await invoke<BackgroundTaskInfo[]>("list_background_tasks");
    if (!Array.isArray(rows)) {
      return [];
    }
    return rows.map(mapBackgroundTask);
  },
};

export function mapBackgroundTask(task: BackgroundTaskInfo): UnifiedRun {
  const state = mapTaskStatus(task.status);
  return {
    id: runIdOf("background-tasks", task.id),
    source: "background-tasks",
    nativeId: task.id,
    // 标题由后端写入时已按业务语境组织（如「采集收盘快照 2026-10-23」），原样展示
    name: task.title,
    // 行上没有触发来源列（来源在 task_events.actor，list 命令不返回）⇒ 如实 unknown
    trigger: "unknown",
    state,
    stage: undefined,
    progress: undefined,
    attempts: task.attempt,
    startedAt: task.createdAt,
    finishedAt: task.finishedAt ?? undefined,
    durationMs: task.finishedAt != null ? task.finishedAt - task.createdAt : undefined,
    error: state === "failed" && task.output
      ? { message: task.output }
      : undefined,
    badges: undefined,
    capabilities: {
      viewIO: true,
    },
    raw: task,
  };
}

function mapTaskStatus(status: string): RunState {
  switch (status) {
    case "pending":
      return "queued";
    case "running":
      return "running";
    case "completed":
      return "succeeded";
    case "failed":
      return "failed";
    case "stopped":
      return "canceled";
    default:
      return "failed";
  }
}
