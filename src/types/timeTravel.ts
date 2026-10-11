// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 时间旅行批次复盘 —— 与后端 `commands/time_travel_batch.rs` DTO 对齐。
 * 字段一律 camelCase（serde rename_all = "camelCase"）。
 */

/** 周期档名（与 harness Period::as_str 对齐；ultrashort 为历史兼容写法） */
export type TimeTravelPeriod = "ultra_short" | "short" | "mid" | "long";

/** 发起时间旅行批次的请求 */
export interface TimeTravelBatchRequest {
  /** 固定锚点日 YYYY-MM-DD；与 tradingDaysAgo 二选一（date 优先） */
  date?: string | null;
  /** 相对北京今日回看的交易日数 */
  tradingDaysAgo?: number | null;
  /** 参与的周期档；空数组 = 全四档 */
  periods: TimeTravelPeriod[];
  minConfidence: number;
  topN: number;
  /** MVP 恒 true */
  runReco: boolean;
  /** MVP 不支持，必须 false */
  runSerenity: boolean;
  /** 是否在 guard 外立即反思已到期档 */
  reflect: boolean;
}

/** 批次中单只标的的结构化执行结果 */
export interface TimeTravelBatchAnalysis {
  stockCode: string;
  stockName: string;
  period: string;
  confidence: number;
  analysisId: string | null;
  error: string | null;
}

/** 单个复盘档的批次级汇总 */
export interface TimeTravelHorizonSummary {
  period: string;
  holdingDays: number;
  hindsightDate: string;
  /** hindsight 是否已到期（≤ 北京今日） */
  due: boolean;
  total: number;
  completed: number;
  pending: number;
  failed: number;
}

/** 批次运行状态（轮询 get_time_travel_batch 返回） */
export interface TimeTravelBatchState {
  batchId: string;
  asOf: string;
  tradingDaysAgo: number | null;
  /** running / completed / partial / failed */
  status: "running" | "completed" | "partial" | "failed";
  /** analysis / reflection */
  phase: "analysis" | "reflection";
  message: string | null;
  total: number;
  ok: number;
  fail: number;
  analyses: TimeTravelBatchAnalysis[];
  horizons: TimeTravelHorizonSummary[];
  reflectionSummary: string | null;
  errorCode: string | null;
  errorDetail: string | null;
  createdAtMs: number;
  updatedAtMs: number;
  finishedAtMs: number | null;
}
