// SPDX-License-Identifier: AGPL-3.0-only

import i18n from "@/i18n";
import { invoke } from "@/lib/invoke";
import type { MoverRecallBatchState } from "@/types";
import { runIdOf, type RunState, type UnifiedRun } from "../model";
import type { TaskSourceAdapter } from "./types";

/**
 * mover-recall-analysis 源：妖股漏检 → as-of 历史补分析批次
 * （list_mover_recall_analysis_batches）。无取消/暂停命令；失败事件可再次发起批次重试。
 * partial 映射 succeeded+partial badge，interrupted 映射 canceled+interrupted badge。
 */
export const moverRecallAnalysisAdapter: TaskSourceAdapter = {
  id: "mover-recall-analysis",
  nameKey: "tasksCenter.source.moverRecallAnalysis",
  color: "volcano",

  async list(): Promise<UnifiedRun[]> {
    const rows = await invoke<MoverRecallBatchState[]>(
      "list_mover_recall_analysis_batches",
      { limit: 50 },
    );
    if (!Array.isArray(rows)) {
      return [];
    }
    return rows.map(mapBatchSummary);
  },
};

export function mapBatchSummary(batch: MoverRecallBatchState): UnifiedRun {
  const state = mapBatchState(batch.status);
  // 进度按「已处理完（成功+失败）/ 总数」：失败也是处理完成，不能只数成功
  const done = batch.ok + batch.fail;
  const progress = batch.total > 0
    ? Math.min(100, Math.round((done / batch.total) * 100))
    : undefined;
  return {
    id: runIdOf("mover-recall-analysis", batch.batchId),
    source: "mover-recall-analysis",
    nativeId: batch.batchId,
    name: `${i18n.t("tasksCenter.source.moverRecallAnalysis")} · ${batch.ok}/${batch.total}`,
    trigger: "manual",
    state,
    stage: undefined,
    progress,
    attempts: 0,
    startedAt: batch.createdAtMs,
    finishedAt: batch.finishedAtMs ?? undefined,
    durationMs: batch.finishedAtMs !== null
      ? batch.finishedAtMs - batch.createdAtMs
      : undefined,
    error: batch.errorDetail
      ? { message: batch.errorDetail }
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

function mapBatchState(status: MoverRecallBatchState["status"]): RunState {
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
