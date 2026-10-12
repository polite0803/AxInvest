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

/**
 * 批次状态：
 * - running 运行中
 * - completed 全部成功
 * - partial 部分失败 / best-effort 链降级
 * - failed 批次级失败
 * - interrupted 进程异常退出后重启收敛（as-of 封闭世界无法跨进程续跑）
 */
export type TimeTravelBatchStatus =
  | "running"
  | "completed"
  | "partial"
  | "failed"
  | "interrupted";

/** 批次运行阶段 */
export type TimeTravelBatchPhase = "analysis" | "reflection";

/** 批次运行状态（轮询 get_time_travel_batch 返回） */
export interface TimeTravelBatchState {
  batchId: string;
  asOf: string;
  tradingDaysAgo: number | null;
  status: TimeTravelBatchStatus;
  phase: TimeTravelBatchPhase;
  message: string | null;
  total: number;
  ok: number;
  fail: number;
  /** 智能荐股链候选行数（合并去重前，四档展开） */
  recoPicks: number;
  /** 趋势智选链候选行数（合并去重前，mid/long 两档展开） */
  serenityPicks: number;
  /** 趋势智选工作流失败原因（best-effort 降级：荐股链正常时批次仍继续，终态 partial） */
  serenityError: string | null;
  analyses: TimeTravelBatchAnalysis[];
  horizons: TimeTravelHorizonSummary[];
  reflectionSummary: string | null;
  errorCode: string | null;
  errorDetail: string | null;
  createdAtMs: number;
  updatedAtMs: number;
  finishedAtMs: number | null;
}

/**
 * 批次历史摘要（list_time_travel_batches 返回，summary_only=true）：
 * 标量计数完整，不含 analyses / horizons / reflectionSummary 明细；
 * 明细走 get_time_travel_batch 单取。
 */
export type TimeTravelBatchSummary = Omit<
  TimeTravelBatchState,
  "analyses" | "horizons" | "reflectionSummary"
>;
