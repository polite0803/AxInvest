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
  action: string;
  positionPct: number;
  targetPrice: number | null;
  stopLoss: number | null;
  reasoning: string;
  riskLevel: string;
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

export const ANALYST_NAMES: Record<string, string> = {
  // 完整节点名 (a- 前缀去除后)
  "market-analyst": "市场技术分析师",
  "sentiment-analyst": "情绪面分析师",
  "news-analyst": "消息面分析师",
  "fundamentals-analyst": "基本面分析师",
  "policy-analyst": "政策面分析师",
  "hot-money-tracker": "资金面追踪者",
  "lockup-watcher": "筹码面观察者",
  "bull-researcher": "多方研究员",
  "bear-researcher": "空方研究员",
  "aggressive-debator": "激进风险评估师",
  "conservative-debator": "保守风险评估师",
  "neutral-debator": "中性风险评估师",
  "research-manager": "研究经理",
  "portfolio-manager": "投资组合经理",
  // 工作流模板短名称 (a- 前缀去除后, 对应 stock_analysis_setup.rs 中节点 ID)
  "sector": "行业分析师",
  "sentiment": "情绪面分析师",
  "news": "消息面分析师",
  "fundamentals": "基本面分析师",
  "policy": "政策面分析师",
  "hot-money": "资金面追踪者",
  "lockup": "筹码面观察者",
  "research": "研报分析师",
  "trader": "交易规划师",
  "investment-plan": "投资计划",
};
