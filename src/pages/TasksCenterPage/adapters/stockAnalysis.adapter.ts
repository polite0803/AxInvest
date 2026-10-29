// SPDX-License-Identifier: AGPL-3.0-only

import i18n from "@/i18n";
import { analysisRecordPath } from "@/lib/investRoutes";
import { invoke } from "@/lib/invoke";
import type { AnalysisSummary } from "@/types";
import { runIdOf, type RunState, type UnifiedRun } from "../model";
import type { TaskSourceAdapter } from "./types";

/**
 * stock-analysis 源：工作台发起的单股分析（list_stock_analyses）。
 *
 * 只取**根记录**（`parentAnalysisId === null`）：replay 分析是时间旅行批次的子行
 * （实测 338 行里 91 条子行），已由 time-travel-batch 源代表；混进来会让同一批回放
 * 变成「批次一行 + 每票一行」两套计数。
 *
 * 没有取消/重试：`cancel_stock_workflow` 要 workflowId，而列表 DTO 不返回它；
 * 详情跳工作台（deepLink）。
 */
export const stockAnalysisAdapter: TaskSourceAdapter = {
  id: "stock-analysis",
  nameKey: "tasksCenter.source.stockAnalysis",
  color: "gold",

  async list(): Promise<UnifiedRun[]> {
    const rows = await invoke<AnalysisSummary[]>("list_stock_analyses", { limit: 50, offset: 0 });
    if (!Array.isArray(rows)) {
      return [];
    }
    return rows.filter((r) => !r.parentAnalysisId).map(mapStockAnalysisRun);
  },
};

export function mapStockAnalysisRun(row: AnalysisSummary): UnifiedRun {
  const state = mapAnalysisState(row.status);
  // 只用**完成时刻**算耗时。`updatedAt` 会被交易意图审核 / 结果回写顶掉
  // （现网 65% 的 completed 行漂移 ≥ 24 小时），拿它算会显示「跑了好几天」。
  // completedAt 为 null（历史行 / interrupted）⇒ 结束时刻未知，不显示耗时。
  const finishedAt = (state === "succeeded" || state === "failed")
      && row.completedAt != null
      && row.completedAt >= row.createdAt
    ? row.completedAt
    : undefined;
  return {
    id: runIdOf("stock-analysis", row.id),
    source: "stock-analysis",
    nativeId: row.id,
    name: `${i18n.t("tasksCenter.source.stockAnalysis")} · ${row.stockCode} ${row.stockName}`,
    // `stock_analyses` 没有触发来源列（`config_id` 现网 0 行有值）⇒ 定时发起的单股分析
    // 无法与手动区分。当前没有单股定时任务；将来接入须先补列，否则这一列会标错来源。
    trigger: "manual",
    state,
    attempts: 0,
    startedAt: row.createdAt,
    finishedAt,
    durationMs: finishedAt !== undefined ? finishedAt - row.createdAt : undefined,
    badges: state === "canceled" && row.status === "interrupted" ? ["interrupted"] : undefined,
    // 必须走 `investPath`：2026-10-28 起 `/stock-analysis/:id` 已删除，落到 404 页。
    deepLink: analysisRecordPath({ id: row.id, stockCode: row.stockCode, stockName: row.stockName }),
    capabilities: {
      openSource: true,
    },
    raw: row,
  };
}

export function mapAnalysisState(status: string): RunState {
  switch (status?.toLowerCase()) {
    case "running":
      return "running";
    case "completed":
      return "succeeded";
    case "failed":
    case "error":
      return "failed";
    case "interrupted":
    case "canceled":
    case "cancelled":
      return "canceled";
    case "paused":
      return "paused";
    case "queued":
    case "pending":
      return "queued";
    default:
      // 未知状态不得默认成 running：那会让僵死记录继续显示「运行中」并触发轮询
      return "failed";
  }
}
