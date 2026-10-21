// SPDX-License-Identifier: AGPL-3.0-only

import i18n from "@/i18n";
import { invoke } from "@/lib/invoke";
import type { PipelineRun } from "@/types";
import { runIdOf, type RunState, type UnifiedRun } from "../model";
import type { TaskSourceAdapter } from "./types";

/**
 * stock-pipeline 源：股票分析管道按日运行（get_pipeline_history/detail）。
 * 无取消/重试命令（盘点 1.2）；步骤仅实时事件（pipeline-step），历史 run 无 steps 可取，
 * 故能力位只开 viewIO（详情抽屉拉 get_pipeline_run_detail）。
 */
export const pipelineAdapter: TaskSourceAdapter = {
  id: "stock-pipeline",
  nameKey: "tasksCenter.source.stockPipeline",
  color: "blue",

  async list(): Promise<UnifiedRun[]> {
    const rows = await invoke<PipelineRun[]>("get_pipeline_history", { limit: 50 });
    if (!Array.isArray(rows)) {
      return [];
    }
    return rows.map(mapPipelineRun);
  },
};

export function mapPipelineRun(run: PipelineRun): UnifiedRun {
  const state = mapPipelineState(run.status);
  const normalizedStatus = run.status?.toLowerCase();
  const failedCount = run.summary
    ? run.summary.analysis.newFailed + run.summary.analysis.reassessFailed
    : 0;
  return {
    id: runIdOf("stock-pipeline", run.id),
    source: "stock-pipeline",
    nativeId: run.id,
    name: `${i18n.t("tasksCenter.source.stockPipeline")} · ${run.runDate}`,
    trigger: "manual",
    state,
    attempts: 0,
    badges: normalizedStatus === "interrupted"
      ? ["interrupted"]
      : normalizedStatus === "completed" && failedCount > 0
      ? ["partial"]
      : undefined,
    startedAt: run.startedAt,
    finishedAt: run.completedAt ?? undefined,
    durationMs: run.completedAt !== null ? run.completedAt - run.startedAt : undefined,
    error: run.errorMessage ? { message: run.errorMessage } : undefined,
    capabilities: {
      viewIO: true,
    },
    raw: run,
  };
}

export function mapPipelineState(status: string): RunState {
  switch (status?.toLowerCase()) {
    case "running":
      return "running";
    case "completed":
    case "success":
      return "succeeded";
    case "failed":
    case "error":
      return "failed";
    case "interrupted":
      return "canceled";
    case "canceled":
    case "cancelled":
      return "canceled";
    case "paused":
      return "paused";
    case "queued":
    case "pending":
      return "queued";
    default:
      // 未知状态不得默认成 running：那会让僵尸行继续显示「运行中」并触发轮询
      return "failed";
  }
}
