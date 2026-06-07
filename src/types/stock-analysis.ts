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

/** 风险等级枚举 */
export const StockRiskLevel = {
  HIGH: "HIGH",
  MID: "MID",
  LOW: "LOW",
} as const;

export type StockRiskLevelType = (typeof StockRiskLevel)[keyof typeof StockRiskLevel];

/** 中文风险等级 → 枚举映射 */
export const STOCK_RISK_LABELS: Record<string, StockRiskLevelType> = {
  "高": StockRiskLevel.HIGH,
  "中": StockRiskLevel.MID,
  "低": StockRiskLevel.LOW,
  "high": StockRiskLevel.HIGH,
  "mid": StockRiskLevel.MID,
  "low": StockRiskLevel.LOW,
};

/** 解析风险等级 */
export function parseRiskLevel(raw: unknown): StockRiskLevelType {
  const s = String(raw ?? "").trim().toLowerCase();
  if (s.includes("高") || s === "high") { return StockRiskLevel.HIGH; }
  if (s.includes("低") || s === "low") { return StockRiskLevel.LOW; }
  return StockRiskLevel.MID;
}

/** riskLevel → i18n key */
export function getRiskTKey(level: string): string {
  const map: Record<string, string> = {
    HIGH: "stockAnalysis.riskHigh",
    MID: "stockAnalysis.riskMid",
    LOW: "stockAnalysis.riskLow",
  };
  return map[level] ?? "stockAnalysis.riskMid";
}

/** riskLevel → CSS 颜色变量 */
export function getRiskColor(level: string): string {
  switch (level) {
    case StockRiskLevel.HIGH:
      return "var(--sa-red)";
    case StockRiskLevel.LOW:
      return "var(--sa-green)";
    default:
      return "var(--sa-amber)";
  }
}

/** 尝试解析报告字符串为 JSON 对象（复用 AnalystReportCard.tryParse 的轻量版） */
function tryParseJson(text: string): Record<string, unknown> | null {
  try {
    const trimmed = text.trim();
    if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
      try {
        return JSON.parse(trimmed) as Record<string, unknown>;
      } catch { /* try below */ }
    }
    const m = trimmed.match(/```(?:json)?\s*([\s\S]*?)\s*```/);
    if (m) {
      try {
        return JSON.parse(m[1].trim()) as Record<string, unknown>;
      } catch { /* try below */ }
    }
    const fb = trimmed.indexOf("{");
    const lb = trimmed.lastIndexOf("}");
    if (fb !== -1 && lb !== -1 && lb > fb) {
      try {
        return JSON.parse(trimmed.slice(fb, lb + 1)) as Record<string, unknown>;
      } catch { /* ignore */ }
    }
  } catch { /* not json */ }
  return null;
}

/**
 * 分析师报告情感分类（先解析 JSON 提取结构化字段，再回退子串匹配）
 * - JSON 中有 action/view/sentiment 字段时直接映射，准确率远高于子串匹配
 * - 子串匹配作为回退，覆盖非 JSON 格式的纯文本报告
 */
export function classifySentiment(report: string): "bullish" | "bearish" | "neutral" {
  // 1) 尝试从 JSON 结构化字段提取
  const json = tryParseJson(report);
  if (json) {
    // action 字段（BUY/INCREASE/SELL/REDUCE/HOLD）
    const action = String(json["action"] ?? "").trim();
    if (action) {
      const a = action.toUpperCase();
      if (a === "BUY" || a === "INCREASE") { return "bullish"; }
      if (a === "SELL" || a === "REDUCE") { return "bearish"; }
      if (a === "HOLD") { return "neutral"; }
    }
    // view 字段（bullish/bearish/neutral 或 看多/看空/中性）
    const view = String(json["view"] ?? json["sentiment"] ?? "").trim().toLowerCase();
    if (view) {
      if (view.includes("bull") || view.includes("看多") || view.includes("乐观") || view.includes("利好")) {
        return "bullish";
      }
      if (view.includes("bear") || view.includes("看空") || view.includes("悲观") || view.includes("利空")) {
        return "bearish";
      }
      if (view.includes("neutral") || view.includes("中性") || view.includes("观望")) {
        return "neutral";
      }
    }
    // recommendation 字段
    const rec = String(json["recommendation"] ?? json["rating"] ?? "").trim().toLowerCase();
    if (rec) {
      if (rec.includes("buy") || rec.includes("买入") || rec.includes("增持") || rec.includes("看涨")) {
        return "bullish";
      }
      if (rec.includes("sell") || rec.includes("卖出") || rec.includes("减持") || rec.includes("看跌")) {
        return "bearish";
      }
      if (rec.includes("hold") || rec.includes("持有") || rec.includes("中性") || rec.includes("观望")) {
        return "neutral";
      }
    }
  }

  // 2) 回退：子串匹配（覆盖非 JSON 报告）
  // 注意：必须先检查 bullish 再检查 bearish，因为"看涨"包含"涨"但不包含"跌"
  const lower = report.toLowerCase();
  const bullishWords = [
    "买入",
    "增持",
    "看多",
    "做多",
    "推荐",
    "看好",
    "乐观",
    "上涨",
    "看涨",
    "利好",
    "强势",
    "突破",
    "买入持有",
    "强烈推荐",
    "建议买入",
    "建议增持",
    "bull",
    "bullish",
    "outperform",
    "overweight",
    "strong buy",
  ];
  const bearishWords = [
    "卖出",
    "减持",
    "看空",
    "做空",
    "回避",
    "看跌",
    "悲观",
    "下跌",
    "利空",
    "弱势",
    "破位",
    "卖出回避",
    "强烈回避",
    "建议卖出",
    "建议减持",
    "bear",
    "bearish",
    "underperform",
    "underweight",
    "strong sell",
  ];
  for (const w of bullishWords) { if (lower.includes(w)) { return "bullish"; } }
  for (const w of bearishWords) { if (lower.includes(w)) { return "bearish"; } }
  return "neutral";
}

/** 信号标签 → Tag 颜色（启发式子串匹配，信号为 LLM 自由文本） */
export function getSignalColor(signal: string): "green" | "red" | "blue" {
  const s = signal.toLowerCase();
  if (s.includes("买") || s.includes("多") || s.includes("涨") || s.includes("牛")) { return "green"; }
  if (s.includes("卖") || s.includes("空") || s.includes("跌") || s.includes("熊")) { return "red"; }
  return "blue";
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

// ── 回测类型 ──────────────────────────────────────────────

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

// ── Decision Timeline（Phase 8）──

/** 时间线 4 阶段：扫描 → 诊断 → 辩论 → 决策 */
export type TimelinePhase = "scan" | "diagnose" | "debate" | "decide";

/** 节点状态：pending(未开始)/ running(进行中)/ done(完成)/ failed(失败) */
export type TimelineNodeStatus = "pending" | "running" | "done" | "failed";

/** 节点证据引用：点击 EvidenceChip 时跳转到对应侧栏面板 */
export interface EvidenceRef {
  /** 目标 tab key；时间线 4 阶段之外，落在 market/analyze/execute 上 */
  tabKey: "market" | "analyze" | "execute";
  /** 与 sheet panels key 对齐 */
  panelKey: string;
  /** 可选 anchor 锚点 */
  anchor?: string;
  /** 一句话证据摘要 */
  snippet: string;
}

/** 单个时间线节点：一名 agent 在某 phase 的一次执行 */
export interface TimelineNode {
  id: string;
  phase: TimelinePhase;
  agentId: string;
  agentName: string;
  title: string;
  summary: string;
  /** 0-1；后端暂无原始置信度，估算自 confidencePct / 100 */
  confidence: number;
  status: TimelineNodeStatus;
  evidenceRefs: EvidenceRef[];
  /** 辩论阶段 bull/bear 回合用 children 表达 */
  children?: TimelineNode[];
  startedAt?: number;
  finishedAt?: number;
}
