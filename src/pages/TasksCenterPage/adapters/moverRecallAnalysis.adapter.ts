// SPDX-License-Identifier: AGPL-3.0-only

import i18n from "@/i18n";
import { invoke } from "@/lib/invoke";
import type { MoverRecallBatchState } from "@/types";
import { mapRunTrigger, runIdOf, type RunState, type UnifiedRun } from "../model";
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
    // 名称只放「批次类型 + 批次日期」：此前是 `<中文名> · ${ok}/${total}`，
    // 而 ok/total 是**进度**不是身份 —— 两个刚起的批次都是 `· 0/60`，
    // 列表里出现两行同名，读起来就是"名称不正确"（2026-10-29 用户实证）。
    // 进度的独立列是 `progress`，不该重复塞进名称。
    // 也不放 batchId 前 8 位：那是 id 串，与"名称"语义相反（同类毛病见 cron 的
    // `vldec-18b830d5`）。补分析批次由 cron 每天 06:00 起一批 ⇒ 日期即天然身份。
    name: `${i18n.t("tasksCenter.source.moverRecallAnalysis")} · ${formatBatchDate(batch.createdAtMs)}`,
    // cron 每天 06:00 也会建批次 ⇒ 不能一律标手动；旧批次没有来源 ⇒ unknown
    trigger: mapRunTrigger(batch.trigger),
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

/** 批次日期（本地历日，仅年-月-日）：作为名称里的身份，避开进度与 id 串。 */
function formatBatchDate(createdAtMs: number): string {
  return new Intl.DateTimeFormat(i18n.language, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).format(new Date(createdAtMs));
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
