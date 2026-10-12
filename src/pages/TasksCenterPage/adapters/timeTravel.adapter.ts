// SPDX-License-Identifier: AGPL-3.0-only

import i18n from "@/i18n";
import { invoke } from "@/lib/invoke";
import type { TimeTravelBatchSummary } from "@/types";
import { runIdOf, type RunState, type UnifiedRun } from "../model";
import type { TaskSourceAdapter } from "./types";

/**
 * time-travel-batch 源：时间旅行批次复盘（list_time_travel_batches）。
 * 无取消/暂停/重试命令（盘点 1.3）；阶段 analysis/reflection，
 * partial 映射 succeeded+partial badge，interrupted 映射 canceled+interrupted badge。
 */
export const timeTravelAdapter: TaskSourceAdapter = {
  id: "time-travel-batch",
  nameKey: "tasksCenter.source.timeTravelBatch",
  color: "purple",

  async list(): Promise<UnifiedRun[]> {
    const rows = await invoke<TimeTravelBatchSummary[]>("list_time_travel_batches", {
      limit: 50,
    });
    if (!Array.isArray(rows)) {
      return [];
    }
    return rows.map(mapBatchSummary);
  },
};

export function mapBatchSummary(batch: TimeTravelBatchSummary): UnifiedRun {
  const state = mapBatchState(batch.status);
  const stage = batch.phase === "reflection"
    ? i18n.t("replayWorkbench.batch.phaseReflection")
    : i18n.t("replayWorkbench.batch.phaseAnalysis");
  const progress = batch.total > 0
    ? Math.min(100, Math.round((batch.ok / batch.total) * 100))
    : undefined;
  return {
    id: runIdOf("time-travel-batch", batch.batchId),
    source: "time-travel-batch",
    nativeId: batch.batchId,
    name: `${i18n.t("tasksCenter.source.timeTravelBatch")} · ${batch.asOf}`,
    trigger: "manual",
    state,
    stage,
    progress,
    attempts: 0,
    startedAt: batch.createdAtMs,
    finishedAt: batch.finishedAtMs ?? undefined,
    durationMs: batch.finishedAtMs !== null
      ? batch.finishedAtMs - batch.createdAtMs
      : undefined,
    error: batch.errorDetail
      ? { code: batch.errorCode ?? undefined, message: batch.errorDetail }
      : batch.message
      ? { message: batch.message }
      : undefined,
    badges: batch.status === "partial"
      ? ["partial"]
      : batch.status === "interrupted"
      ? ["interrupted"]
      : undefined,
    capabilities: {
      viewIO: true,
    },
    raw: batch,
  };
}

function mapBatchState(
  status: TimeTravelBatchSummary["status"],
): RunState {
  switch (status) {
    case "running":
      return "running";
    case "completed":
    case "partial":
      return "succeeded";
    case "failed":
      return "failed";
    case "interrupted":
      return "canceled";
  }
}
