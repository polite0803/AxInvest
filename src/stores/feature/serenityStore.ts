import { create } from "zustand";

// ── 类型 ──

export type StepStage =
  | "loading"
  | "scanning"
  | "decomposing"
  | "identifying"
  | "mapping"
  | "saving"
  | "done"
  | "error";

export interface Catalyst {
  type: string;
  description: string;
  expected_timeframe: string;
  confidence: number;
  trigger_condition?: string;
}

export interface ExitSignals {
  technology_disruption_risk?: string;
  capacity_oversupply_risk?: string;
  new_entrant_risk?: string;
  demand_slowdown_risk?: string;
  overall_exit_urgency?: string;
}

export interface AttentionMetrics {
  coverage_change_3m?: string;
  search_heat?: string;
  relative_volume?: string;
  consensus_gap?: string;
  attention_score?: number;
}

export interface SerenityCandidate {
  stockCode?: string;
  stock_name?: string;
  stockName?: string;
  stock_code?: string;
  relevance?: string;
  serenityScore?: number;
  serenity_score?: number;
  confidence?: number;
  strategy_type?: string;
  strategyType?: string;
  bottleneckProduct?: string;
  bottleneck_product?: string;
  primaryRisk?: string;
  primary_risk?: string;
  catalysts?: Catalyst[];
  exit_signals?: ExitSignals;
  exitSignals?: ExitSignals;
  attention_metrics?: AttentionMetrics;
  attentionMetrics?: AttentionMetrics;
  /** 推荐生成时间（历史记录回填时携带；实时候选缺省，前端按当前时间兜底） */
  generated_at?: string;
  generatedAt?: string;
  /** 建议持有天数（缺省 20，serenity 固定 mid 周期） */
  holding_days?: number;
  holdingDays?: number;
}

export interface TrendInfo {
  trendName?: string;
  trend_name?: string;
  bottleneck_candidate?: string;
  confidence?: number;
}

/// 单个节点的执行日志
export interface StepLog {
  nodeId: string;
  status: string;
  output?: unknown;
  error?: string;
  /**
   * 节点失败的**结构化错误码**（取值域 = `commands/error_code.rs` 的 `stock_workflow` 域，
   * 如 `STOCK_WORKFLOW_STEP_CANCELLED` / `STOCK_WORKFLOW_TIMEOUT` / `STOCK_WORKFLOW_STEP_FAILED`）。
   * `null` = 后端明示「本事件无失败」；`undefined` = 旧载荷 ⇒ 判定须用 `typeof errorCode === "string"`。
   *
   * 与 `error` 的分工：**`errorCode` 负责判定与主文案，`error` 负责技术详情**。
   * 渲染层走 `lib/errorI18n.ts::translateFailureText`（有码取 11 语言译文，无码/未收录回退原文），
   * `error` 原文仅作展开态的详情行保留。**禁止**用 `error.startsWith(...)` 反推语义
   * —— 后端 `NodeError::Io` 变体是 `transparent` 的，原文不带码前缀，串嗅探在该变体上必然失配。
   */
  errorCode?: string | null;
  elapsedMs?: number;
  totalNodes?: number;
  completedNodes?: number;
  timestamp: number;
}

// ── Store ──

interface SerenityState {
  running: boolean;
  stage: StepStage;
  candidates: SerenityCandidate[];
  trends: TrendInfo[];
  error: string | null;
  /**
   * `error` 的**技术详情**行（可选）。
   *
   * 与 `error` 的分工：`error` 是**主文案**（有结构化码时已由渲染层本地化为 11 语言译文），
   * 本字段是给排查用的**未本地化原文**（DB 报错原文、`NodeError` 自由文本等）——
   * 两者可同时展示，故「本地化」不必以「丢原因」为代价。
   *
   * `null` = 无额外详情（主文案已自足，例如旧载荷下 `error` 本身就是原文）⇒
   * 渲染层**不得**渲染空行，也不得渲染与 `error` 相同的内容。
   */
  errorDetail: string | null;
  completedNodes: number;
  totalNodes: number;
  steps: StepLog[];
  currentNodeId: string | null;
  /**
   * 工作流结束时，a-candidate-mapper 节点返回的"为什么没有候选"原因。
   * 当上游三个瓶颈节点均返回 data_gaps=true 时，模型会在 arguments.summary
   * 给出反幻觉说明；此字段在 candidates 为空时供前端展示，避免用户看到
   * 一个无解释的 Empty 占位。
   */
  emptyReason: string | null;

  setRunning: (v: boolean) => void;
  setStage: (s: StepStage) => void;
  setCandidates: (c: SerenityCandidate[]) => void;
  setTrends: (t: TrendInfo[]) => void;
  setError: (e: string | null, detail?: string | null) => void;
  setCompletedNodes: (n: number) => void;
  setTotalNodes: (n: number) => void;
  addStep: (log: StepLog) => void;
  setCurrentNode: (id: string | null) => void;
  setEmptyReason: (r: string | null) => void;
  clearSteps: () => void;
  reset: () => void;
}

const initialState = {
  running: false,
  stage: "done" as StepStage,
  candidates: [] as SerenityCandidate[],
  trends: [] as TrendInfo[],
  error: null as string | null,
  errorDetail: null as string | null,
  completedNodes: 0,
  totalNodes: 0,
  steps: [] as StepLog[],
  currentNodeId: null as string | null,
  emptyReason: null as string | null,
};

export const useSerenityStore = create<SerenityState>((set) => ({
  ...initialState,
  setRunning: (v) => set({ running: v }),
  setStage: (s) => set({ stage: s }),
  setCandidates: (c) => set({ candidates: c }),
  setTrends: (t) => set({ trends: t }),
  // `detail` 缺省即清空，而非「保持上一次」：`setError(text)` 的调用点（工作流级失败）
  // 不传详情，若默认沿用旧值，上一次的 DB 原文会串台挂到新的失败文案下面。
  // 清空条件挂在 `e` 上（`setError(null)` 清错误时必须连详情一起清）。
  setError: (e, detail = null) => set({ error: e, errorDetail: e ? detail : null }),
  setCompletedNodes: (n) => set({ completedNodes: n }),
  setTotalNodes: (n) => set({ totalNodes: n }),
  addStep: (log) =>
    set((s) => {
      const idx = s.steps.findIndex((item) => item.nodeId === log.nodeId);
      if (idx >= 0) {
        // upsert：同节点更新状态 + output/error/elapsedMs，不追加重复
        const updated = [...s.steps];
        updated[idx] = { ...updated[idx], ...log };
        return { steps: updated };
      }
      return { steps: [...s.steps, log] };
    }),
  setCurrentNode: (id) => set({ currentNodeId: id }),
  setEmptyReason: (r) => set({ emptyReason: r }),
  clearSteps: () => set({ steps: [], currentNodeId: null }),
  reset: () => set(initialState),
}));
