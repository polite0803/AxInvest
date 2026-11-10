// SPDX-License-Identifier: AGPL-3.0-only

import i18n from "@/i18n";
import { invokePluginCommand } from "@/lib/pluginCommand";
import type { PipelineRun } from "@/types/pipeline";
import { createElement } from "react";
import { PipelineRunDetailDrawer } from "../jobs/PipelineRunDetailDrawer";
import { runIdOf, type RunState, type UnifiedRun } from "../model";
import type { TaskSourceAdapter } from "./types";

/**
 * stock-pipeline 源：股票分析管道按日运行（stock_get_pipeline_history/detail）。
 * 无取消/重试命令（盘点 1.2）；步骤仅实时事件（pipeline-step），历史 run 无 steps 可取，
 * 故能力位只开 viewIO（详情抽屉经 pipelineStore 拉 stock_get_pipeline_run_detail）。
 */
export const pipelineAdapter: TaskSourceAdapter = {
  id: "stock-pipeline",
  nameKey: "tasksCenter.source.stockPipeline",
  color: "blue",

  async list(): Promise<UnifiedRun[]> {
    // #248 T2 试点批：插件命名命令（原 get_pipeline_history，双声明期宿主命令保留）
    const rows = await invokePluginCommand<PipelineRun[]>("stock_get_pipeline_history", {
      limit: 50,
    });
    if (!Array.isArray(rows)) {
      return [];
    }
    return rows.map(mapPipelineRun);
  },

  // 详情形态归本源自述（任务 #151）：整条 run 的逐股明细，run 的 `raw` 就是 PipelineRun。
  // 用 createElement 而不是 JSX：适配器是 .ts，不为一个表达式把五个适配器全改成 .tsx。
  renderDetail: ({ run, close }) =>
    createElement(PipelineRunDetailDrawer, {
      run: (run.raw as PipelineRun | undefined) ?? null,
      onClose: close,
    }),
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
