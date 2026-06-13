// ── 运行时工具函数（实现已在 @/lib/stock-analysis-utils.ts） ──
// 向后兼容的 re-export，新代码应直接从 @/lib/stock-analysis-utils 导入
export {
  ANALYST_NAMES,
  classifySentiment,
  computeStockConsensus,
  type Consensus,
  getActionColor,
  getActionTKey,
  getRiskColor,
  getRiskTKey,
  getSignalColor,
  parseAction,
  parseRiskLevel,
  type Sentiment,
  STOCK_ACTION_LABELS,
  STOCK_RISK_LABELS,
  StockAction,
  type StockConsensus,
  StockRiskLevel,
} from "@/lib/stock-analysis-utils";

// StockActionType / StockRiskLevelType 由 @/lib/stock-analysis-utils 导出
import type { StockActionType, StockRiskLevelType } from "@/lib/stock-analysis-utils";
export type { StockActionType, StockRiskLevelType };

// ── 纯类型定义 ──

export interface StockQuote {
  code: string;
  name: string;
  price: number;
  open: number;
  high: number;
  low: number;
  volume: number;
  amount: number;
  changePct: number;
  turnoverRate: number;
  pe: number | null;
  pb: number | null;
  totalMv: number | null;
  limitUp: number | null;
  limitDown: number | null;
  isSt: boolean;
  timestamp: string;
}

export interface KLine {
  date: string;
  open: number;
  high: number;
  low: number;
  close: number;
  volume: number;
  amount: number;
  turnoverRate: number | null;
}

/** R3-B 财报披露事件 — 后端 `get_earnings_calendar` 返回结构 */
export type EarningsEventType =
  | "preliminary"
  | "express"
  | "formal"
  | "shareholders_meeting"
  | "other";

export interface EarningsEvent {
  stockCode: string;
  stockName: string;
  eventDate: string;
  eventType: EarningsEventType | string;
  period: string | null;
  detail: string | null;
  source: string | null;
  createdAt: number;
}

export interface StockSearchResult {
  code: string;
  name: string;
  market: string;
}

export interface AnalysisConfig {
  maxDebateRounds: number;
  klinePeriod: string;
  klineLimit: number;
  newsLimit: number;
}

export interface StockDecision {
  action: StockActionType;
  positionPct: number;
  targetPrice: number | null;
  stopLoss: number | null;
  reasoning: string;
  riskLevel: StockRiskLevelType;
  confidence: number;
}

export interface AnalysisSummary {
  id: string;
  stockCode: string;
  stockName: string;
  analysisDate: string;
  status: string;
  decisionAction: string | null;
  createdAt: number;
}

export interface AnalysisEvent {
  type:
    | "started"
    | "dataLoaded"
    | "analystProgress"
    | "analystReport"
    | "debateRound"
    | "riskAssessment"
    | "investmentPlan"
    | "decision"
    | "error";
  payload: Record<string, unknown>;
}

export type AnalysisStatus = "idle" | "loading" | "running" | "completed" | "error";

// ── 回测类型 ──

export interface BacktestResult {
  stockCode: string;
  analysisDate: string;
  decisionAction: string;
  decisionConfidence: number;
  entryPrice: number | null;
  exitPrice: number;
  holdingDays: number;
  returnPct: number;
  wasCorrect: boolean;
  maxDrawdown: number;
}

export interface BacktestStats {
  totalAnalyses: number;
  accuracyPct: number;
  avgReturnPct: number;
  avgMaxDrawdownPct: number;
  avgConfidence: number;
  alphaPct: number | null;
}

// ── 荐股策略回测 ──

export interface StrategyStats {
  strategyId: string;
  style: string;
  period: string;
  totalSignals: number;
  winCount: number;
  lossCount: number;
  winRatePct: number;
  avgReturnPct: number;
  totalReturnPct: number;
  avgMaxDrawdownPct: number;
  maxConsecutiveLosses: number;
  sharpeRatio: number | null;
  profitFactor: number | null;
}

export interface GroupBacktestResult {
  label: string;
  stockCount: number;
  strategies: Record<string, StrategyStats>;
}

export interface BacktestComparisonResponse {
  positive: GroupBacktestResult;
  negative: GroupBacktestResult;
  positiveStocks: string[];
  negativeStocks: string[];
  skipped: string[];
}

// ── 荐股信号 ──

export interface StrategySignalResult {
  strategyId: string;
  stockCode: string;
  stockName: string;
  signalDate: string;
  entryPrice: number;
  exitPrice: number;
  holdingDays: number;
  returnPct: number;
  wasProfitable: boolean;
  maxDrawdownPct: number;
}

// ── 历史分析摘要 ──

/** 个股最近一次分析的摘要，用于荐股 panel 展示 */
export interface LatestAnalysisSummary {
  analysisId: string;
  analysisDate: string;
  decisionAction: string;
  decisionPositionPct: number | null;
  confidence: number | null;
  status: string;
  outcome: string | null;
}

// ── 决策时间线类型 ──

/** 时间线 4 阶段：扫描 → 诊断 → 辩论 → 决策 */
export type TimelinePhase = "scan" | "diagnose" | "debate" | "decide";

/** 节点状态：pending(未开始)/ running(进行中)/ done(完成)/ failed(失败) */
export type TimelineNodeStatus = "pending" | "running" | "done" | "failed";

/** 节点证据引用：点击 EvidenceChip 时跳转到对应侧栏面板 */
export interface EvidenceRef {
  tabKey: "market" | "analyze" | "execute";
  panelKey: string;
  anchor?: string;
  snippet: string;
}

/** 单个时间线节点 */
export interface TimelineNode {
  id: string;
  phase: TimelinePhase;
  agentId: string;
  agentName: string;
  title: string;
  summary: string;
  confidence: number;
  status: TimelineNodeStatus;
  evidenceRefs: EvidenceRef[];
  children?: TimelineNode[];
  startedAt?: number;
  finishedAt?: number;
}
