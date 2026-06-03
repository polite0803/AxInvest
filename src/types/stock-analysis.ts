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

/** 股票操作动作枚举 — 内部统一用英文标识，展示时通过 i18n 翻译 */
export const StockAction = {
  BUY: "BUY",
  INCREASE: "INCREASE",
  HOLD: "HOLD",
  REDUCE: "REDUCE",
  SELL: "SELL",
} as const;

export type StockActionType = (typeof StockAction)[keyof typeof StockAction];

/** 中文标签映射（给 LLM 输出兼容 + 解析用） */
export const STOCK_ACTION_LABELS: Record<string, StockActionType> = {
  "买入": StockAction.BUY,
  "增持": StockAction.INCREASE,
  "持有": StockAction.HOLD,
  "减持": StockAction.REDUCE,
  "卖出": StockAction.SELL,
  "BUY": StockAction.BUY,
  "INCREASE": StockAction.INCREASE,
  "HOLD": StockAction.HOLD,
  "REDUCE": StockAction.REDUCE,
  "SELL": StockAction.SELL,
};

/** Action → i18n key */
export function getActionTKey(action: string): string {
  const map: Record<string, string> = {
    BUY: "stockAnalysis.actionBuy",
    INCREASE: "stockAnalysis.actionIncrease",
    HOLD: "stockAnalysis.actionHold",
    REDUCE: "stockAnalysis.actionReduce",
    SELL: "stockAnalysis.actionSell",
  };
  return map[action] ?? "stockAnalysis.actionHold";
}

/** Action → Ant Design Tag 颜色 */
export function getActionColor(action: string): "red" | "green" | "orange" | "blue" | "default" {
  switch (action) {
    case StockAction.BUY:
    case StockAction.INCREASE:
      return "red";
    case StockAction.SELL:
    case StockAction.REDUCE:
      return "green";
    case StockAction.HOLD:
      return "blue";
    default:
      return "default";
  }
}

/** 解析可能的中文/英文 action 为 StockActionType */
export function parseAction(raw: unknown): StockActionType {
  const s = String(raw ?? "").trim();
  return STOCK_ACTION_LABELS[s] ?? StockAction.HOLD;
}

export interface StockDecision {
  action: StockActionType;
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
