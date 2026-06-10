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
  UNCERTAIN: "UNCERTAIN",
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
  "uncertain": StockAction.UNCERTAIN,
  "UNCERTAIN": StockAction.UNCERTAIN,
};

/** Action → i18n key */
export function getActionTKey(action: string): string {
  const map: Record<string, string> = {
    BUY: "stockAnalysis.actionBuy",
    INCREASE: "stockAnalysis.actionIncrease",
    HOLD: "stockAnalysis.actionHold",
    REDUCE: "stockAnalysis.actionReduce",
    SELL: "stockAnalysis.actionSell",
    UNCERTAIN: "stockAnalysis.actionUncertain",
  };
  return map[action] ?? "stockAnalysis.actionUncertain";
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
    case StockAction.UNCERTAIN:
      return "default";
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
 * - JSON 中有 stance / action / bull_score / bear_score / positionPct 等结构化字段时
 *   直接按规则映射，准确率远高于子串匹配。
 * - 子串匹配作为回退，覆盖非 JSON 格式的纯文本报告。
 *   回退采用"谁多就判谁"的简单多数规则（不再用 65% 严阈值，
 *   避免 LLM 偏正偏差导致"提到风险就看中性"）。
 */
export function classifySentiment(report: string): "bullish" | "bearish" | "neutral" {
  // 1) 尝试从 JSON 结构化字段提取
  const json = tryParseJson(report);
  if (json) {
    // 1a) stance 字段 — 中文方向词，覆盖各 agent 不同的命名空间
    const stanceRaw = json["stance"] ?? json["view"] ?? json["sentiment"] ?? json["verdict"];
    const stance = String(stanceRaw ?? "").trim();
    if (stance) {
      const lower = stance.toLowerCase();
      // 多头方向词
      if (
        stance.includes("买入") || stance.includes("增持") || stance.includes("看多")
        || stance.includes("做多") || stance.includes("看涨") || stance.includes("多头")
        || stance.includes("利好") || stance.includes("上涨") || stance.includes("乐观")
        || stance.includes("上行") || stance.includes("流入") || stance.includes("扫货")
        || stance.includes("强于") || stance.includes("超配") || stance.includes("加仓")
        || lower.includes("bull") || lower.includes("buy") || lower.includes("overweight")
      ) {
        return "bullish";
      }
      // 空头方向词
      if (
        stance.includes("卖出") || stance.includes("减持") || stance.includes("看空")
        || stance.includes("做空") || stance.includes("看跌") || stance.includes("空头")
        || stance.includes("利空") || stance.includes("下跌") || stance.includes("悲观")
        || stance.includes("下行") || stance.includes("流出") || stance.includes("出货")
        || stance.includes("弱于") || stance.includes("低配") || stance.includes("减仓")
        || lower.includes("bear") || lower.includes("sell") || lower.includes("underweight")
      ) {
        return "bearish";
      }
      // 中性方向词
      if (
        stance.includes("观望") || stance.includes("中性") || stance.includes("平衡")
        || stance.includes("震荡") || stance.includes("同步") || stance.includes("保守")
        || stance.includes("放缓") || stance.includes("持有") || stance.includes("hold")
        || lower.includes("neutral") || lower.includes("hold")
      ) {
        return "neutral";
      }
    }
    // 1b) action 字段（BUY/INCREASE/SELL/REDUCE/HOLD 或中文 买入/增持/...）
    const action = String(json["action"] ?? "").trim();
    if (action) {
      const a = action.toUpperCase();
      if (a === "BUY" || a === "INCREASE") { return "bullish"; }
      if (a === "SELL" || a === "REDUCE") { return "bearish"; }
      if (a === "HOLD") { return "neutral"; }
      if (action.includes("买入") || action.includes("增持")) { return "bullish"; }
      if (action.includes("卖出") || action.includes("减持")) { return "bearish"; }
      if (action.includes("持有") || action.includes("观望")) { return "neutral"; }
    }
    // 1c) bull_score / bear_score 数字打分（0-10，分开打分）
    const bullScoreRaw = json["bull_score"] ?? json["bullScore"];
    const bearScoreRaw = json["bear_score"] ?? json["bearScore"];
    if (bullScoreRaw != null || bearScoreRaw != null) {
      const bullScore = Number(bullScoreRaw ?? 0);
      const bearScore = Number(bearScoreRaw ?? 0);
      if (Number.isFinite(bullScore) && Number.isFinite(bearScore)) {
        const diff = bullScore - bearScore;
        if (diff > 0) { return "bullish"; }
        if (diff < 0) { return "bearish"; }
      }
    }
    // 1d) positionPct 仓位 — trader / debator 输出，0-100
    const posPctRaw = json["positionPct"] ?? json["position_pct"];
    if (posPctRaw != null) {
      const posPct = Number(posPctRaw);
      if (Number.isFinite(posPct)) {
        if (posPct >= 6) { return "bullish"; }
        if (posPct < 0) { return "bearish"; }
        return "neutral";
      }
    }
    // 1e) recommendation / rating 字段
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

  // 2) 回退：计分制子串匹配（多数表决 — 谁多判谁，不再用 65% 严阈值）
  //    旧 65/35 阈值会让"看好 + 提示风险"的真实推荐被误判为中性，
  //    现在改成简单多数：bull > 0 且 bear === 0 也算看多；bull > bear 即看多。
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
  // 短词判定：仅当"短且是英文"才走单词边界正则（避免 "bull" 误中 "bullet"）。
  // 中文/混合词用 includes 更可靠 —— \b 在中文里不生效。
  const wordBoundary = (w: string) => /^[a-z]+$/i.test(w) && w.length <= 3;
  let bull = 0;
  let bear = 0;
  for (const w of bullishWords) {
    if (wordBoundary(w)) {
      const re = new RegExp(`\\b${w}\\b`, "gi");
      bull += (lower.match(re) ?? []).length;
    } else {
      bull += lower.split(w).length - 1;
    }
  }
  for (const w of bearishWords) {
    if (wordBoundary(w)) {
      const re = new RegExp(`\\b${w}\\b`, "gi");
      bear += (lower.match(re) ?? []).length;
    } else {
      bear += lower.split(w).length - 1;
    }
  }
  if (bull === 0 && bear === 0) { return "neutral"; }
  // 简单多数：谁多判谁，差距为 0 才算真正分歧
  if (bull > bear) { return "bullish"; }
  if (bear > bull) { return "bearish"; }
  return "neutral";
}

/** 共识枚举：在 bullish / bearish / neutral 之外加一个 divided
 *  表示多空双方都有一定占比但都未到 65% 阈值。 */
export type Sentiment = "bullish" | "bearish" | "neutral";
export type Consensus = Sentiment | "divided";

/** 一只股票的分析师共识（用于荐股列表与该股分析结果做交叉验证） */
export interface StockConsensus {
  bullish: number;
  bearish: number;
  neutral: number;
  total: number;
  /** 共识标签（多空比例决定） */
  consensus: Consensus;
  /** 时间戳（毫秒）— 用于排序/淘汰过老数据 */
  updatedAt: number;
}

/** 工具：对单只股票的多份分析师报告做投票聚合，输出 StockConsensus */
export function computeStockConsensus(
  reports: Record<string, string>,
  updatedAt = Date.now(),
): StockConsensus {
  let bullish = 0;
  let bearish = 0;
  let neutral = 0;
  for (const raw of Object.values(reports)) {
    const s = classifySentiment(raw);
    if (s === "bullish") { bullish++; }
    else if (s === "bearish") { bearish++; }
    else { neutral++; }
  }
  const total = bullish + bearish + neutral;
  let consensus: Consensus = "neutral";
  if (total > 0) {
    const bullRatio = bullish / total;
    const bearRatio = bearish / total;
    if (bullRatio > 0.65) { consensus = "bullish"; }
    else if (bearRatio > 0.65) { consensus = "bearish"; }
    else if (bullRatio > 0 && bearRatio > 0) { consensus = "divided"; }
  }
  return { bullish, bearish, neutral, total, consensus, updatedAt };
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
  "catalyst-analyst": "催化剂与叙事分析师",
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

// ── 荐股策略回测（Strategy Backtest）──

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

// ── Decision Timeline（Phase 8）──

// ── 荐股面板历史分析关联（P0-1）──

/** 个股最近一次分析的摘要，用于荐股 panel 展示 */
export interface LatestAnalysisSummary {
  analysisId: string;
  analysisDate: string;
  decisionAction: string; // BUY / HOLD / SELL / uncertain
  decisionPositionPct: number | null;
  confidence: number | null; // 加权置信度 0-100
  status: string;
  outcome: string | null; // win / loss / pending
}

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
