// i18n-exempt: 股票分析类型定义（含后端数据值枚举，如风险等级/催化剂方向中文标识），类型定义非 UI 文案。
// ── 运行时工具函数（实现已在 @/lib/stock-analysis-utils.ts） ──
// 向后兼容的 re-export，新代码应直接从 @/lib/stock-analysis-utils 导入
export {
  classifySentiment,
  computeEvidenceDrivenConsensus,
  computeEvidenceWeights,
  computeStockConsensus,
  type Consensus,
  getActionColor,
  getActionTagStyle,
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

// 证据质量驱动权重的类型定义
export type {
  AnalystInput,
  AnalystWeight,
  EvidenceConsensus,
  EvidenceWeightReport,
  EvidenceWeightRequest,
  HoldGateResult,
  MarketRegimeInfo,
} from "@/lib/stock-analysis-utils";

// StockActionType / StockRiskLevelType 由 @/lib/stock-analysis-utils 导出
import type { PositionStateType, StockActionType, StockRiskLevelType } from "@/lib/stock-analysis-utils";
export type { PositionStateType, StockActionType, StockRiskLevelType };

// ── 纯类型定义 ──

/**
 * 反思反馈提交结果（对应后端 submit_reflection_feedback 命令返回值）。
 *
 * 接入 FeedbackOrchestrator + ExperiencePipeline 双轨：
 * - Pipeline：反馈 → Experience → 经验池
 * - Orchestrator：计数 + 阈值触发 RLTraining / SkillEvolution
 */
export interface ReflectionFeedbackResult {
  analysisId: string;
  rating: number;
  /** Orchestrator 触发的动作类型 */
  action: "none" | "trigger_rl_training" | "trigger_skill_evolution" | "trigger_pool_size_check";
  orchestratorStats: {
    totalFeedback: number;
    negativeCount: number;
    positiveCount: number;
  };
}

/**
 * 机构一致预期 EPS（对应后端 `consensus_eps` 工具返回数据，
 * 权威定义 `astock-data/src/types.rs::ConsensusEPS`，serde `rename_all = camelCase`）。
 *
 * 消费端注意：`isEstimated = true` 时该 EPS 是**估算值**（vendor 全失败时按挂牌板块取常数兜底），
 * 不得据此做「超预期 / 不及预期」类判定。`estimateSource` 仅 `isEstimated = true` 时有值。
 */
export interface ConsensusEPS {
  stockCode: string;
  consensusEps: number | null;
  consensusTargetPrice: number | null;
  ratingAvg: string | null;
  ratingCount: number | null;
  year: string;
  /** 是否为估算值（非真实一致预期 / 财报数据） */
  isEstimated: boolean;
  /** 估算来源，仅 isEstimated = true 时有值（如 "board_constant:star"） */
  estimateSource?: string | null;
}

export interface StockQuote {
  code: string;
  name: string;
  price: number;
  /** 昨收价,涨跌额 = price - preClose(中国股市惯例) */
  preClose: number;
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
  /** 流通市值（元）；后端 circulating_mv，serde rename_all camelCase */
  circulatingMv: number | null;
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
  /** 累计复权因子 (R3-A)；None 表示未应用复权。后端 adj_factor */
  adjFactor: number | null;
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

/** 单一周期的一组交易价位（阶段1四周期价位映射，与后端 DTO 逐字段对齐） */
export interface HorizonPriceGroup {
  /** 止损档位 %（0.0 = 该周期无交易计划，见 `stopLossPct > 0` 判据） */
  stopLossPct: number;
  /** 止盈档位 % */
  takeProfitPct: number;
  /** 该周期期望持有天数（交易日） */
  expectedHoldingDays: number;
  /** 目标价（绝对价，`currentPrice × (1 + takeProfitPct/100)`） */
  targetPrice: number | null;
  /** 止损价（绝对价，`currentPrice × (1 − stopLossPct/100)`） */
  stopLoss: number | null;
}

/** 四周期价位映射，键为 `ultra_short` / `short` / `mid` / `long` */
export interface HorizonPriceMap {
  ultraShort?: HorizonPriceGroup | null;
  short?: HorizonPriceGroup | null;
  mid?: HorizonPriceGroup | null;
  long?: HorizonPriceGroup | null;
}

/** 阶段 2：单个周期的独立决策（`portfolio-mgr.rhai` `horizon_decision()` 产出） */
export interface HorizonDecision {
  action: string;
  verdict: string;
  positionPct: number;
  /**
   * 该档**上涨胜率**（×100）—— **纯证据口径**：逐档先验 → 逐档证据加权 → 按 √h 折算，
   * **不含** `riskBias`。与荐股链的 `confidence`（`blend_win_rate` /
   * `candidate_score_to_win_rate`）**同一量纲，可直接比较**。
   *
   * 2026-10-02 改：此前本字段是「叠加风险偏置后的**判定值**」⇒ 同一份 JSON 里两个
   * 「置信度」不同口径，并排展示时会把「高风险」误读成「胜率低」，也无法与荐股比较
   * （实测 002812：荐股 78 vs 本字段 39.7，看着像两系统严重对立，实为量纲错配）。
   * 判定值现单列 `confidenceRiskAdjusted`。
   */
  confidence: number;
  /**
   * 该档后验（×100，四舍五入到 0.1）—— Phase C 逐档先验 + 逐档证据加权的结果
   * （**不含**风险偏置，SNR 折算前）。
   */
  posterior: number;
  /**
   * **风险调整后**的置信度（= 本档 `action` 阶梯实际所用值，含风险偏置）。
   * 与 `confidence` 的差额 = 风险门槛造成的下调，供归因。
   */
  confidenceRiskAdjusted?: number;
  stopLossPct: number;
  takeProfitPct: number;
  expectedHoldingDays: number;
  targetPrice: number | null;
  stopLoss: number | null;
  /** 仅超短线（方案 B 降级路径标注） */
  confLowerBound?: number;
  /** 叠加 risk_bias 后的生效后验（SNR 折算前） */
  posteriorEffective?: number;
  /** SNR 折算的锚定持有天数（中线 28 交易日 ⇒ 该档不改） */
  snrAnchorDays?: number;
  /**
   * 该档技术腿吃的是哪一份评分（Phase F 结构性缺席声明）：
   * `tier_native` = 本档专属粒度评分节点出数；
   * `daily_fallback` = 该粒度评分没出数（超短无 60 分钟 / 长线无季度），f1 腿退回主链日线。
   * ⚠ 缺席必须成句，不得压成「评分低」或干脆不显示 —— 见 AGENTS.md 禁区 12 与本轮 §七-F。
   */
  scoreSource?: string;
  /** 证据权重来源：table = 逐档乘数表；fallback_unity = 表缺失，全腿按 1.0 退化 */
  weightsSource?: string;
  /** 该档先验来源：tier = 本档回测收缩；pooled = 全档合并；shared_regime_prior = 共用先验 */
  priorSource?: string;
  /** 该档先验样本数（配合 κ 判断收缩强度是否够可信） */
  priorSamples?: number;
  /** 止损口径：vol = k·σ·√h 导出；fallback_pct = σ 不可得，退回固定百分比档 */
  stopSource?: string;
  /** 仓位口径：risk_budget = min(凯利, 100·R/止损%)；kelly_only / fallback_kelly_x_mult = 降级 */
  positionSource?: string;
  /**
   * 该档证据完整度（×100）：分子=本档实际权重和，分母=逐腿基线表按本档乘数缩放。
   * 与主链 `evidence_pct` 不同口径是有意的（第 5(a) 条）——乘数表缺失时二者相等。
   */
  evidenceScale?: number;
  /** 该档下注的实际赔率 = 止盈% ÷ 止损%（价带之比，第 5(b) 条） */
  odds?: number;
  /** 赔率来源：ladder_ratio = 由本档价带导出；no_stop = 无止损价带（空仓档）⇒ 赔率 0 */
  oddsSource?: string;
  /**
   * 与哪些档的 `posterior` **恒等**（Phase F 同源标注）。
   * 非空不代表算错 —— 它说的是「这两个数字无法互相佐证」，展示层必须注脚化。
   */
  sharesPosteriorWith?: string[];
}

/** 阶段 2：四周期独立决策映射（键 camelCase，对齐 `decisions_by_horizon`） */
export interface DecisionsByHorizon {
  ultraShort?: HorizonDecision | null;
  short?: HorizonDecision | null;
  mid?: HorizonDecision | null;
  long?: HorizonDecision | null;
}

export interface StockDecision {
  action: StockActionType;
  positionPct: number;
  /**
   * P1-2(2026-09-14): 持仓状态（EMPTY / OPENING / HOLDING / TRIMMING）—— 与 `action` **正交**的第二轴。
   *
   * 背景：`action` 表达**方向强度**，`positionPct` 表达仓位大小；「持有 vs 观望」此前是同一
   * 中性档因仓位有无被后端**互改**的两个名字（仓位>0 观望→持有；仓位≤0 持有→观望）。
   *
   * ⚠️ 2026-09-22：本字段 = **本次建议后的**持仓状态（描述「建议建多少仓」），
   * **不参与**展示档判定 —— 展示档已改为「方向档恒等」（`resolveDisplayAction(action)`）。
   * 旧实现用它 + `positionPct` 反推展示档是循环判据：建议仓位来自本次决策，
   * 再拿它决定本次决策的展示名，会把 LLM 措辞的抖动注入结论名
   * （见 `AUDIT-300642-run-variance-2026-09-22.md`）。本字段仅供 UI 单独展示。
   *
   * ⚠️ `null` / `undefined` 的语义是「该记录产生于本字段引入之前，采集时点无此信息」，
   * **不得**读成 `EMPTY`（那会把「不知道」当成「空仓」）。
   */
  positionState?: PositionStateType | null;
  targetPrice: number | null;
  stopLoss: number | null;
  /**
   * 阶段1（PROPOSAL-stock-decision-four-horizon.md）四周期价位映射。
   *
   * 后端 `portfolio-mgr.rhai` 产出的 `horizonPriceMap`（嵌套在 `decision_json`），
   * 经 `stock_analyses.horizon_price_map` 落库、`normalizeDecision` 收敛为 camelCase。
   * 同一决策保留单一 `action`/仓位语义，但目标价/止损按四周期各给一组绝对价，
   * 前端按周期 Tab/分组查看。
   *
   * ⚠️ `null`/`undefined` 语义 = 该记录产生于 stage1 字段引入之前，**无此信息**；
   * 消费端**不得**读成空映射，应按顶层 `targetPrice`/`stopLoss`（主档位）回退。
   */
  horizonPriceMap?: HorizonPriceMap | null;
  /**
   * 阶段 2（PROPOSAL-stock-decision-four-horizon.md）四周期独立决策。
   *
   * 后端 `portfolio-mgr.rhai` `decisions_by_horizon` → 落库 `stock_analyses.horizon_decisions`
   * → `normalizeDecision` 收敛为 camelCase。每周期独立产出 action/仓位/目标价/止损，
   * 前端按周期 Tab 分组展示，**方向矛盾时高亮呈现**（验收项）。
   *
   * ⚠️ `null`/`undefined` 语义 = 该记录产生于 stage2 字段引入之前，**无此信息**；
   * 消费端按主 `action`（`decision_action` 单值）回退。
   */
  decisionsByHorizon?: DecisionsByHorizon | null;
  reasoning: string;
  riskLevel: StockRiskLevelType;
  confidence: number;
  /** 决策方向置信度 (0-100) — 无论买卖方向都体现"多确信"。解决看空决策 confidence 偏低被误读为"不确信" */
  decisionConfidence?: number | null;
  /** 信号强度 (0-100) — 偏离中性的程度，0=完全中性，100=极端强信号 */
  signalStrength?: number | null;
  /** V66: 因子权重坍缩状态 — true 时 positionPct 被强制为 0、decisionConfidence 被减半 */
  weightsCollapsed?: boolean;
  /** V66: 坍缩原因代码: "none" | "dqi_collapsed" | "multi_untrusted" | "low_weight_ratio" */
  collapseReason?: string;
  /** V66: 因子权重占比 (total_weight/max_weight × 100)，用于 Tooltip 展示 */
  weightRatio?: number;
  /** V66: 不可信上游节点数量 */
  untrustedCount?: number;
  /**
   * portfolio-mgr 消费的上游节点中**缺失数据**的清单（如「资金流向(t-hotmoney-data)」）。
   *
   * ⚠️ 本字段的语义**只有一种**：「本该拿到的数据没拿到」（上游节点缺席 / 字段不可得）。
   * **设计性降权不属于这里** —— 它是「本档按周期主动下调某腿权重」，零数据缺失，
   * 走 `weightAdjustments`。2026-10-01 之前两者同挤本字段，后果是**常驻误报**：
   * 估值腿的 0.3/0.5 是常量 ⇒ 凡带估值数据的分析恒推 2 条 ⇒ 每条决策卡都亮
   * 「决策可信度受限 / 数据缺口 2 项」，真缺口被淹没。
   *
   * 命名说明：后端 portfolio-mgr 决策 JSON 里该字段是顶层 snake_case 的 `data_gaps`
   * （唯一一个非 camelCase 的顶层键，且 `stock_workflow/decision.rs` V65 的
   * 一致性算法也按此名读取，故不能改名）；`normalizeDecision` 已统一收敛为
   * 前端 camelCase 的 `dataGaps`，消费处只读 `decision.dataGaps`。
   */
  dataGaps?: string[];
  /**
   * **口径调整**（不是数据缺口）：本档按周期主动降权的证据腿。
   *
   * 与 `dataGaps` 的分界：`dataGaps` = 「本该拿到的数据没拿到」；本字段 = 「本档按设计下调了
   * 某腿权重」，一个字节的数据都没缺。后端 `portfolio-mgr.rhai` 输出顶层 camelCase
   * `weightAdjustments`（结构化，2026-10-01 起），只登记**真被下调**（乘数 < 1）的档。
   *
   * 展示层：按档挂在四档决策面板的注脚上；**不得**计入「数据缺口 N 项」，也**不得**据此
   * 点亮「决策可信度受限」警示条（那正是本字段从 `dataGaps` 拆出来的原因）。
   */
  weightAdjustments?: WeightAdjustment[];
  /** 时间维度: "ultra_short" | "short" | "mid" | "long" */
  timeHorizon?: string | null;
  /** 期望持有天数（交易日） */
  expectedHoldingDays?: number | null;
  /** 目标价预期实现时间框架: "1d" | "1w" | "1m" | "3m" | "6m" */
  targetTimeframe?: string | null;
  /** 后端检测到的 trader 输出自相矛盾（action 与 targetPrice 方向冲突）*/
  isContradictory?: boolean;
  /** V40 修复: quality-gate 判定 D/F 时，该决策来自 quality-fallback 降级路径 */
  isFallback?: boolean;
  /** V50: 双视角一致性调制后的置信度（受 agreement_factor 影响） */
  adjustedConfidence?: number;
  /** V50: 双视角一致性分维度诊断 */
  agreementBreakdown?: AgreementBreakdown;
  /** 跨系统互证：近 14 天趋势智选推荐 vs 本次工作流决策（后端在决策持久化时注入） */
  crossCheck?: RecoCrossCheck;
}

/**
 * 一条「口径调整」：某档对某条决策腿按周期主动降权。
 *
 * 权威来源是 Rust `analysis-engine::evidence_weight::horizon_leg_multipliers()`
 * （经桥表 `DECISION_LEG_ANALYST` 注入为脚本变量 `horizon_leg_weights_json`）——
 * 脚本侧**只读**该表，不手抄任何倍数。故本结构里的 `multiplier` 不是「面板读数」，
 * 而是**公式实际使用的那个数**，可逐位对账。
 */
export interface WeightAdjustment {
  /** 权威档名 snake_case（= `Period::as_str()` = `HORIZON_CAMEL_TO_SNAKE` 的值域） */
  tier: string;
  /** 决策腿（`f5` = 估值；腿→分析师桥表见 `DECISION_LEG_ANALYST`） */
  leg: string;
  /** 该档对该腿的乘数（< 1 = 降权；后端只输出被下调的档） */
  multiplier: number;
}

/** 跨系统互证字段（后端 stock_workflow::hooks::inject_reco_crosscheck 注入，camelCase 对齐） */
export interface RecoCrossCheck {
  /** 智选推荐置信度 (0-100) */
  recoConfidence: number;
  /** 智选风格: "serenity" | "bottleneck" 等 */
  recoStyle: string;
  /** 策略类型: bottleneck / policy / earnings / capital / event / technical */
  recoStrategyType: string;
  /** 推荐周期: "mid" | "long" 等 */
  recoPeriod: string;
  /** 智选建议仓位 (%) */
  recoPositionPct: number;
  /** 智选建议持有天数 */
  recoHoldingDays: number;
  /** 智选落库时价格（行情获取失败时为 0） */
  recoPrice: number;
  /** 推荐生成时间（ISO 8601） */
  recoGeneratedAt: string;
  /** 本次决策生成时间（ISO 8601）——两侧时钟基线显式化 */
  decisionGeneratedAt: string;
  /** 关注热度（冷门/热门等，来自智选 attention_metrics） */
  attentionHeat: string;
  /** 催化剂摘要（最多 3 条） */
  catalysts: { description: string; timeframe: string; confidence: number }[];
  /** 本次工作流决策动作 */
  decisionAction: string;
  /** 本次工作流决策仓位 (%) */
  decisionPositionPct: number;
  /** 本次工作流决策的持仓状态轴（v228，crossCheck 生成时点的快照） */
  decisionPositionState?: string | null;
  /** 是否构成跨系统分歧（智选推荐 vs 工作流否决/观望） */
  divergent: boolean;
  /**
   * 分歧归因（后端 `divergence_attribution` 从决策 JSON **已有**字段派生，不新算一套）。
   *
   * 存在的理由：`divergent` 只说「两个系统不一致」，用户读到的仍是两个并排数字。
   * 本字段回答「谁把工作流结论压下去的」——`drivers` 是判据码（叙事交 i18n），
   * `legs` 是贡献最负的至多三条证据腿，三个后验数字用于「风险门槛跨线」那句话。
   *
   * ⚠ 全部**可缺**：字段缺失即不产该条（宁缺毋滥），渲染端不得补默认值。
   */
  divergence?: {
    /** 判据码：negative_legs / risk_gate_downgrade / below_hold_threshold / dcf_leg_excluded / data_gap */
    drivers?: string[];
    /** 贡献最负的至多三条证据腿（sigma×weight 由最负起排） */
    legs?: { name: string; sigma: number; weight: number }[];
    /** 原始后验（×100，不含风险偏置）——缺阈值时不存在 */
    posteriorRaw?: number;
    /** 生效后验（×100，含风险偏置）——action 阶梯实际所用 */
    posteriorEffective?: number;
    /** 本次决策实际生效的「持有」线（×100），不是常量表抄来的 */
    holdThreshold?: number;
  };
}

// ── 决策仪表盘报告（借鉴 daily_stock_analysis 推送格式）──

/** 风险警报条目 */
export interface RiskAlert {
  description: string;
  severity: "低" | "中" | "高" | string;
  source?: string | null;
}

/** 催化因素条目 */
export interface Catalyst {
  description: string;
  direction: "利好" | "利空" | string;
  timeline?: string | null;
  confidenceScore?: number | null;
}

/** 操作检查清单条目 */
export interface ChecklistItem {
  description: string;
  checked: boolean;
  category: "入场" | "加仓" | "减仓" | "止损" | "止盈" | string;
}

/** 决策仪表盘报告（单只股票，7 段式结构） */
export interface DashboardReport {
  stockCode: string;
  stockName: string;
  analysisDate: string;
  generatedAt: string;
  coreConclusion: string;
  action: string;
  score: number;
  trend: string;
  confidence: number;
  buyPointLow?: number | null;
  buyPointHigh?: number | null;
  /**
   * **交易目标价**（LLM trader 给出的方向性目标）。
   *
   * ⚠️ 与下方的 `intrinsicValue*` 是**两个不同概念**，UI 必须分开标注、不可混称「目标价」：
   *   · 本字段回答「打算在哪卖出」—— 持有/观望档通常为空，也可能等于现价（即无信息量）；
   *   · `intrinsicValue*` 回答「这家公司值多少钱」—— 由 `t-valuation` 客观计算产出。
   * 2026-09-13 实证（603466 风语筑）：两者被同名展示后，用户读到「同一工作流结论矛盾」。
   */
  targetPrice?: number | null;
  stopLoss?: number | null;
  positionPct: number;
  /** 内在价值区间下沿（DCF 悲观档，估值语义） */
  intrinsicValueLow?: number | null;
  /** 内在价值区间上沿（DCF 乐观档，估值语义） */
  intrinsicValueHigh?: number | null;
  /** 内在价值中位（DCF 中性档，估值语义） */
  intrinsicValueMid?: number | null;
  /** 现值（用于在仪表盘内直观对比「内在价值 vs 现价」） */
  currentPrice?: number | null;
  riskAlerts: RiskAlert[];
  catalysts: Catalyst[];
  checklist: ChecklistItem[];
  latestNews?: string | null;
  earningsExpectation?: string | null;
  llmModel?: string | null;
  integrityPassed: boolean;
}

/** 指数行情 */
export interface IndexQuote {
  name: string;
  price: number;
  changePct: number;
}

/** 大盘复盘报告 */
export interface MarketReviewReport {
  reviewDate: string;
  generatedAt: string;
  indices: IndexQuote[];
  advancers?: number | null;
  decliners?: number | null;
  limitUp?: number | null;
  limitDown?: number | null;
  sectorLeaders: string[];
  sectorLaggards: string[];
  llmModel?: string | null;
}

/** 股票摘要（仪表盘汇总） */
export interface StockSummary {
  stockCode: string;
  stockName: string;
  action: string;
  score: number;
  trend: string;
  confidence: number;
}

/** 聚合仪表盘（多只股票汇总） */
export interface DashboardDigest {
  digestDate: string;
  generatedAt: string;
  totalCount: number;
  buyCount: number;
  watchCount: number;
  sellCount: number;
  summaries: StockSummary[];
  marketReview?: MarketReviewReport | null;
}

/** V65: 双视角一致性 6 维度诊断结果 */
export interface AgreementBreakdown {
  total: number;
  actionOk: boolean;
  actionNote: string;
  formulaAction: string;
  llmAction: string;
  /** V65: action 维度原始分 (满分 30) */
  actionScore?: number;
  /** V65: positionPct 维度原始分 (满分 20) */
  positionScore?: number;
  positionGap: number | null;
  /** V65: confidence 维度原始分 (满分 15) */
  confidenceScore?: number;
  confidenceGap: number | null;
  /** V65: riskLevel 维度原始分 (满分 15) */
  riskLevelScore?: number;
  /** V65: 公式 riskLevel 原始值 */
  formulaRiskLevel?: string;
  /** V65: LLM riskLevel 原始值 */
  llmRiskLevel?: string;
  /**
   * V65: evidence_cited 维度原始分 (满分 10)
   *
   * （2026-09-21 移除）原 `dataGapsScore` / `dataGapsSimilarity` 随 data_gaps
   * 一致性维度一并删除：公式侧（rhai 机械枚举字段缺失）与 LLM 侧（trader 自由
   * 文本自述）命名体系不可比 ⇒ Jaccard 恒 0、且惩罚 LLM 的坦诚度。
   * 历史记录的 `decision_json` 里仍带这两个字段，前端**不再读取与展示**。
   */
  evidenceScore?: number;
  /** V65: LLM 引用上游论据数量 */
  evidenceCount?: number;
  conflictType: string;
  /** 向后兼容: f7 自指污染标记 */
  f7WeightPct?: number | null;
  f7FreePosterior?: number | null;
  f7FreeAction?: string | null;
  f7FreeActionScore?: number | null;
}

/** 单个分析师的数据质量诊断条目（对应 data-quality.rhai 的 diagnostics[field]） */
export interface DataQualityDiagItem {
  /** 中文角色名，如"技术面分析师" */
  name: string;
  /** 该分析师预期消费的数据来源（静态描述） */
  expected_data: string;
  /** 实际 confidence 值；-1 表示字段缺失/节点失败 */
  confidence: number;
  /**
   * status 判定为「A ∪ B」：
   * - A = 报告文本失败标记（客观，placeholder_hits > 0）
   * - B = LLM 自评 confidence（主观）
   * untrusted 为 strict_mode 降级兜底
   */
  status: "missing" | "low" | "normal" | "untrusted";
  /** 缺失或低置信的具体原因（正常时为空字符串） */
  gap_reason: string;
  /**
   * 2026-09-12 新增：报告文本命中的失败标记**词种数**（如"无法获取"/"为 null"/"返回空"）。
   * > 0 时即便 confidence >= 50 也判 low（置信度虚高）。
   * ⚠ 这是**词种数**不是出现次数：同一标记出现多次只计 1。
   * 实际出现次数见 `placeholder_occurrences`。
   */
  placeholder_hits?: number;
  /**
   * 2026-09-21 新增：同一判定集下的**实际出现次数**（同一标记多次出现重复计）。
   * 与 `placeholder_hits` 共用剥离 VERDICT 与软标记抑制规则，保证两个口径可对照。
   * 面板并列展示「N 种标记 / 共 M 次出现」，避免用户按原文出现次数核对时误判为计数错误。
   */
  placeholder_occurrences?: number;
  /**
   * 2026-09-21 新增：本节点**自己的**报告质量分（0-100），来源 `data-quality.rhai` 的
   * `report_quality(text, conf)` —— 与全局 `report_quality_score`（10 个节点的**均值**）
   * 同源同口径，但不再被别人平均掉。
   *
   * 用途：面板顶部的 score / grade / good / degraded / gap **全是全局聚合值**
   * （10 张分析师卡片复用同一 modal ⇒ 打开任意节点都显示同一组数字，用户会误读为
   * 「该分析师的分数」甚至「造假」）。本字段让弹窗能显示「这个节点自己写得怎么样」。
   *
   * ⚠️ 这是**事实量**，不是等级：不要据此派生 per-node 字母等级 —— 那会再造出
   * 「两套同名等级」，正是 2026-09-14 才修掉的坑。
   * 旧快照无此字段 ⇒ 可选；面板须能降级展示（不显示该行）。
   */
  report_quality?: number;
}

/** data-quality 节点输出的结构化诊断报告（data-quality.rhai 输出 JSON） */
export interface DataQualityReport {
  grade: "A" | "B" | "C" | "D" | "F";
  score: number;
  /** P1-B3: 报告质量分（0-100），基于字数+关键词覆盖+占位符检测 */
  report_quality_score?: number;
  /** P1-B3: 工具可信度分（0-100），基于 avg_conf + gap/good_count */
  tool_credibility_score?: number;
  /** V58: 因子完整度百分比（0-100），10 个因子数据存在性评估 */
  factor_completeness_pct?: number;
  /** V58: 缺失因子中文名列表（如 ["技术面评分", "共识评分", ...]） */
  missing_factors?: string[];
  /**
   * 2026-09-21 新增：**上游取数缺口**（我方没取到数据，而非标的自身属性）。
   *
   * 与 `missing_factors` 是**两张表**，不可合并：
   *   · `missing_factors` = 本节点直接消费的因子是否存在（其长度与
   *     `pm_compute_factor_completeness` 的分母口径绑死）；
   *   · 本列表 = **上游节点自己有没有取到数**（关于数据的元数据，不是因子本身）。
   *
   * 首个来源：`dcf.assumptions.fcf_data_missing == true` ⇒ 现金流量表数据缺失，
   * DCF 锚定退化为「近 5 年净利均值 × 0.90」代理。
   * ⚠️ 只把「缺数」计入；「当期 FCF ≤ 0」（标的现金流**真为负**）**不算缺口** ——
   * 那是我方取数失败与标的属性的区别，不可合并（同 `fcf_data_missing` 的两态纪律）。
   * ⚠️ 只告警不扣分：本字段出现**不改变** `score` / `grade`。
   */
  upstream_data_gaps?: string[];
  /**
   * 2026-10-02 新增：**VERDICT 专属字段未产出**（第三类缺席）。
   *
   * 三种缺席各占一栏，不可合并（详见 `data-quality.rhai` 定义处注释）：
   *   · `missing_factors`    = 本节点消费的**因子**没值；
   *   · `upstream_data_gaps` = **上游工具**没取到数（我方采集缺陷）；
   *   · 本列表               = 分析师已出结论标签，但角色规范声明的**专属字段**没写
   *     —— 既不是「该维度没有数据」，也不是「没取到数据」，是产出形态缺陷。
   *
   * 实证（688498 运行 `129745a7`）：a-catalyst 五个工具调用全成功、正文判了
   * 「L2业绩拐点级」，verdict 却只剩通用 6 键 ⇒ 修复前 UI 只显示「缺失因子：催化剂等级」，
   * 与真取数故障无法区分。
   * ⚠️ 只告警不扣分：本字段出现**不改变** `score` / `grade` / `factor_completeness_pct`。
   * 旧快照无此字段 ⇒ 可选；面板须能降级展示（不显示该行）。
   */
  verdict_field_gaps?: string[];
  /**
   * 2026-10-02 新增：**估值方法对本标的不适用**（第四类缺席）。
   *
   * 四张表各说一件事，不可合并（详见 `data-quality.rhai` 定义处注释）：
   *   · `missing_factors`           = 本节点消费的**因子**没值；
   *   · `upstream_data_gaps`        = **上游工具**没取到数（我方采集缺陷）；
   *   · `verdict_field_gaps`        = 分析师已出结论标签但**漏写**角色专属字段；
   *   · 本列表                      = 该标的**本就不适用**这一估值方法（**标的属性**）。
   *
   * 实证（000710 运行 `92849db2`）：PE −18、近5年报无正净利 ⇒ DCF 结构性不适用、
   * `upsidePct` 恒 null，而面板只显示「缺失因子：估值上行空间」⇒ 用户按取数故障去查链路，
   * 而链路是好的。判据是机读码 `dcf.unavailableReason == "persistent_loss"`，不看文案。
   * ⚠️ 只告警不扣分：本字段出现**不改变** `score` / `grade` / `factor_completeness_pct`
   *   （由 `item3_dcf_not_applicable_is_a_fourth_kind_of_absence` 锁住）。
   * 旧快照无此字段 ⇒ 可选；面板须能降级展示（不显示该行）。
   */
  method_not_applicable?: string[];
  gap_count: number;
  good_count: number;
  /**
   * 2026-09-12 新增：报告文本含失败标记但自评 confidence >= 50 的分析师数。
   * 这些节点原被计入 good_count（虚高工具可信度），现按 A ∪ B 降级，计入 low 语义。
   */
  degraded_count?: number;
  /**
   * 2026-10-02 新增：分析师**状态三分类**计数 —— 真划分，三者加总 = `status_total_count`。
   *
   * 修掉的歧义：弹窗原先把 `good_count / degraded_count / gap_count` 并排渲染成
   * 「对 N 个分析师的三分类」，但那三者**不是同一根轴** —— `degraded_count` 是
   * 「自评 ≥50 且报告含失败标记」的**虚高子集**（属低置信内部），`gap_count` 是
   * 「untrusted 或无 VERDICT」另一维度，还有「自评 <50 且无标记」一类三个都不落。
   * 实证（000710 运行 `92849db2`）：逐节点表显示 2 行「⚠️ 低置信」，芯片显示 8/1/0，
   * 加总 9 ≠ 10 ⇒ 一个分析师在视觉上凭空消失。
   *
   * 本组字段由 `data-quality.rhai` **逐项数 `diagnostics.status`** 得出，
   * 与下表「状态」列同源同判据 ⇒ 结构上不可能再对不上。
   * `status_missing_count` = 表格里的 `missing` + `untrusted` 两态（都属「本轮无可信结论」，
   *   具体原因在各行「差距原因」列分别说明）。
   * ⚠️ 旧快照无这些字段 ⇒ 弹窗整排不渲染，**不回退**到旧三量（那会把同一歧义再显示一遍）。
   */
  status_normal_count?: number;
  /** 见 `status_normal_count` 的说明。 */
  status_low_count?: number;
  /** 见 `status_normal_count` 的说明。 */
  status_missing_count?: number;
  /** 见 `status_normal_count` 的说明；等于逐节点表行数。 */
  status_total_count?: number;
  /** 2026-09-12 新增：报告文本含失败标记的分析师中文名清单（如 ["资金面","解禁观察"]） */
  placeholder_nodes?: string[];
  /** 2026-09-12 新增：报告文本含失败标记的节点缩写清单（如 ["hm","lk"]），用于定位 agent 节点 */
  placeholder_node_ids?: string[];
  /** 2026-09-12 新增：全部节点命中的失败标记**词种数**合计 */
  placeholder_total_hits?: number;
  /** 2026-09-21 新增：全部节点失败标记的**实际出现次数**合计（与 placeholder_total_hits 同判定集） */
  placeholder_total_occurrences?: number;
  avg_confidence: number;
  total_analysts: number;
  /** 各分析师详细诊断，键为缩写（mk/sent/news/...） */
  diagnostics: Record<string, DataQualityDiagItem>;
  /** 缺失分析师中文名列表 */
  missing_analysts: string[];
  /** 低置信度分析师中文名列表 */
  low_confidence_analysts: string[];
  /** P1-B3: 数据质量问题列表（字数不足/占位符/低置信等） */
  warnings?: string[];
  /** P2-2(2026-08-09): 分析师方向冲突标记（看多 vs 看空各有 ≥2 个有效分析师） */
  direction_conflict?: boolean;
  /** P2-2: 看多方向分析师数 */
  bull_dir_count?: number;
  /** P2-2: 看空方向分析师数 */
  bear_dir_count?: number;
  /** 人类可读的总结文本 */
  summary: string;
}

export interface AnalysisSummary {
  id: string;
  stockCode: string;
  stockName: string;
  analysisDate: string;
  status: string;
  decisionAction: string | null;
  /** 决策仓位百分比（0-100），列表场景由 decisionJson 解析或后端直返 */
  decisionPositionPct: number | null;
  /**
   * 决策持仓状态轴（migration v228）：EMPTY / OPENING / HOLDING / TRIMMING。
   * `null` = 该记录早于 v228（**采集时点没有这个信息**），**不得**读成 EMPTY；
   * 展示层应据 `decisionPositionPct` 派生。见后端 `stock_analyses` entity 注释。
   */
  decisionPositionState: string | null;
  /** 决策 JSON 字符串（含 action/positionPct/confidence 等），列表场景用于渲染决策 Tag */
  decisionJson: string | null;
  createdAt: number;
  updatedAt: number;
  /** "live" | "replay" */
  analysisKind: string;
  /** as_of_date YYYY-MM-DD（仅 replay 模式非空） */
  asOfDate: string | null;
  /** 版本化分析：指向原始分析记录 ID，null 表示首次分析 */
  parentAnalysisId: string | null;
  /**
   * 生成该记录的工作流模板 id：`"stock-analysis"`（完整链）/ `"stock-analysis-fast"`
   * （快速 JEV 链）。`null` = 本列引入前的记录或非模板产出（对话直执行 / 条件单补记），
   * **不得**据此推断链路，按「未知」渲染。
   *
   * 用途：历史列表里两条链的记录形态一致（`analysisKind` 同为 `"live"`），
   * 需要靠本字段区分并打标识，否则用户看到的是「同一只股票两条互相矛盾的记录」
   * 而没有任何线索说明它们来自不同链路。
   */
  templateId: string | null;
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

export type AnalysisStatus = "idle" | "loading" | "running" | "paused" | "completed" | "error" | "cancelled";

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
  maxDrawdownPct: number;
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
  /** 持仓状态轴（v228）；`null` = 记录早于 v228，非 EMPTY */
  decisionPositionState: string | null;
  decisionPositionPct: number | null;
  confidence: number | null;
  status: string;
  outcome: string | null;
  decisionTimeHorizon?: string | null;
  decisionExpectedHoldingDays?: number | null;
}

// ── 荐股结果 (Bug 10 抽离) ──
// 与后端 crates/stock-analysis/src/recommender/types.rs::RecoPick 一一对应,
// 字段顺序、类型、可选性保持一致(camelCase 由 serde rename_all 转换)。

export type StyleKey = "trend" | "value" | "capital" | "reversion" | "watchlist" | "serenity";
export type PeriodKey = "ultra_short" | "short" | "mid" | "long";

/** 荐股单条 pick — 完整字段版,直接对应后端 schema */
export interface RecoPick {
  stockCode: string;
  stockName: string;
  /** 行业/板块(后端 Option + skip_serializing_if=None) */
  sector?: string | null;
  /** 主风格 — 后端 serde(rename_all="lowercase"),必填 */
  style: StyleKey;
  /** 持有周期 — 后端 serde(rename_all="lowercase"),必填 */
  period: PeriodKey;
  /** 当前价 */
  price: number;
  /** 入场下沿 */
  entryLow: number;
  /** 入场上沿 */
  entryHigh: number;
  /** 止损 */
  stopLoss: number;
  /** 目标位 */
  targetPrice: number;
  /** 建议仓位(%) */
  positionPct: number;
  /** 持有天数(后端 u32) */
  holdingDays: number;
  /** 置信度 0-100（后端 u8，**绝对口径**：逐档先验与评分在 logit 空间合成后的概率） */
  confidence: number;
  /** 组内当日分位 0-100（后端 Phase R-C 起与绝对置信分列；同组无差异时缺字段） */
  confidencePercentile?: number;
  /** 该档先验来源：shrunk / own / pooled / neutral_default / absent（absent = 无历史样本，仅评分） */
  priorSource?: string;
  /** 参与该档先验收缩的历史样本数（0 = 无样本） */
  priorSamples?: number;
  /** 止损/止盈口径：vol = k·σ·√h；fallback_pct = σ 不可得退回固定百分比（必须显式呈现） */
  stopSource?: string;
  /** 仓位口径：risk_budget = 风险预算；fallback_kelly_x_mult = 退回经验周期乘数 */
  positionSource?: string;
  /** 命中理由(可能为空数组) */
  reasons: string[];
  /** 风险提示(可能为空数组) */
  riskNotes: string[];
  /** 风格拆分后的副策略 tag,如 ["trend","capital"];空数组时后端跳过序列化 */
  secondaryStyles?: StyleKey[];
  /** true = 系统初筛 / 数据稀疏兜底(无技术信号),false = 主策略真实命中 */
  synthetic?: boolean;
}

/** 候选池来源构成 — 每个来源实际入池的标的数(去重后) */
export interface SeedPoolOrigin {
  /** 来自热股/涨停榜 */
  hot: number;
  /** 来自行业排名的领涨龙头 */
  industry: number;
  /** 来自内置 FALLBACK_STOCKS 样本池 */
  fallback: number;
}

/** 荐股接口响应 — 完整字段版 */
export interface RecoResponse {
  period: PeriodKey;
  /** 按风格分组的 picks,每组 ≤ 10。后端 HashMap<Style, Vec<RecoPick>> */
  picks: Partial<Record<StyleKey, RecoPick[]>>;
  /** 被 vendor 缺失禁用的风格(live 模式下由 vendor 状态决定) */
  disabledStyles: StyleKey[];
  /** as-of 模式下被降级(≠ 缺失)的风格(spec §8)。live 模式恒为空数组。 */
  degradedStyles?: StyleKey[];
  /** degradedStyles 中各风格的具体降级原因,key=styleKey, value=本地化文本 */
  degradedReasons?: Record<string, string>;
  /** 生成时间戳(毫秒) */
  generatedAt: number;
  /** 过滤前的 seed pool 大小(hot + industry 龙头去重后) */
  rawSeedPoolSize: number;
  /**
   * 候选池来源构成。内置样本池是无条件混入的 ⇒ 池非空推不出候选是真实的;
   * hot+industry 均为 0 且 fallback>0 时,本次候选全部来自内置样本池。
   * 三者全 0 = 调用方自备种子(preseed),来源未知,不得据此报警。
   */
  seedPoolOrigin: SeedPoolOrigin;
  /**
   * **本次荐股运行**期间的 as-of 降级切片(按运行边界水位取,不是进程全局累计)。
   * 与降级面板的 `AsOfDegradationEntry` 同形,只少 `as_of`(响应里已有同值 asOfDate)。
   * live 模式为空数组。前端据此把「候选池为何只剩内置样本」讲成真实归因而非套话。
   */
  asofDegradations?: Array<Omit<AsOfDegradationEntry, "as_of">>;
  /** 时间旅行模式截止日 YYYY-MM-DD;live 时 undefined */
  asOfDate?: string;
  /** 模式标签: live / replay / backtest_sweep — 后端 spec §8 注入,必填 */
  mode: string;
  /** 数据获取错误详情(picks 为空时的具体原因)。后端填充,前端据此显示具体错误文本而非泛化的"连接失败" */
  errorDetail?: string;
}

/**
 * 一次拿四档的批量响应（后端 `commands/stock_analysis.rs` 的 `RecoBatchResponse`）。
 *
 * 两个字段必须**一起**渲染：`byHorizon` 缺某档且 `failedHorizons` 有该档 = 该档本轮扫描失败；
 * 只渲染前者会把「失败」显示成「这一档没有推荐」。
 * 键空间是 `Period` 的 snake_case 序列化值（与 `PeriodKey` 一致），不是 camelCase 档名。
 */
export interface RecoBatchResponse {
  byHorizon: Partial<Record<PeriodKey, RecoResponse>>;
  failedHorizons: Partial<Record<PeriodKey, string>>;
}

/**
 * 荐股链逐 (风格, 档位) 的 rank IC 单元（后端 `analysis-engine::recommender::ic::RecoIcCell`）。
 * 门槛与缺席词表与分析链命中率 IC **同一套**（`IC_MIN_SAMPLE=8`；缺席文案复用 reflection 既有键）。
 */
export interface RecoIcCell {
  style: string;
  period: PeriodKey;
  /** Spearman ρ；null = 不可得（原因见 icStatus） */
  rankIc: number | null;
  samples: number;
  icStatus: "ok" | "insufficient_ic_samples" | "degenerate_variance";
  holdingDays?: number | null;
}

/** 单风格汇总：cells 按 `Period::ALL`（短→长）排好，前端不再二次排序。 */
export interface RecoStyleIc {
  style: string;
  cells: RecoIcCell[];
  halfLifeDays: number | null;
  halfLifeStatus: string;
}

/** 矩阵的一格**契约**（来自后端 `style_matrix`，与有没有回测样本无关）。 */
export interface RecoMatrixCell {
  /** 矩阵名目（`serenity`，不是落库写法 `bottleneck`） */
  style: string;
  period: string;
  /** 该格是否出票 */
  active: boolean;
  /** `cell_is_active` | 不成立理由码 | `cell_not_in_matrix` */
  reasonCode: string;
  /** 出票但已知档-因子错配的理由码 */
  misfitCode?: string | null;
  /** 该风格在落库/回测里的实际写法（`serenity` → `["serenity","bottleneck"]`） */
  dbStyles: string[];
}

/** 一格闭环态（后端 `recommender::reco_loop::LoopCellResult`，Phase D）。
 *  「未校准」与「已校准」必须分句 —— 没校准的格不得在 UI 上看起来像校准过。 */
export interface RecoLoopCell {
  /** 矩阵名目（`serenity`，非落库写法） */
  style: string;
  period: string;
  status: "insufficient_samples" | "ic_unmeasurable" | "not_in_matrix" | "ic_non_negative" | "demoted_negative_ic";
  rankIc: number | null;
  samples: number;
  winRate: number | null;
  oldWeight: number;
  newWeight: number;
}

/** `reco_ic_stats` 的响应（Phase R-E：只报数，不回写权重）。 */
export interface RecoIcStats {
  styles: RecoStyleIc[];
  totalCells: number;
  usableCells: number;
  totalSamples: number;
  /** 24 格契约视图：矩阵行/格集合与「为什么不成立」的唯一来源 */
  matrix?: RecoMatrixCell[];
  /** 24 格闭环视图：权重来源、生效闸状态与逐格缺席理由（`recommender::reco_loop`） */
  loop?: RecoLoopView;
}

/** 闭环整体视图：逐格结果 + 生效闸（off/shadow/on）。 */
export interface RecoLoopView {
  gate: string;
  cells: RecoLoopCell[];
  /** 最近一次闭环重算时刻（ms）；0 = 从未重算 */
  lastRecalcAt: number;
}

// ── 窗口涨幅达标漏检核查（`analyze_mover_recall`，PLAN-mover-recall-attribution）──

/** 一档判据：阈值走模板变量（`varName`），窗口天数取唯一档位表（`Period::default_holding_days`）。 */
export interface MoverTierRule {
  period: string;
  /** 模板变量全名（`mover_gain_ultra_short`）——面板据此指认阈值来源，不抄数字 */
  varName: string;
  gainPct: number;
  windowDays: number;
}

/** 三个率。`null` = 分母为 0，**算不出来**（≠ 0%，两者在 UI 上必须不同形）。 */
export interface MoverRecallRates {
  reachability: number | null;
  coverage: number | null;
  unexplainedShare: number | null;
  events: number;
  misses: number;
}

/** 一条窗口涨幅达标事件（口径：绝对涨幅，不含板块涨停语义）。 */
export interface MoverEvent {
  stockCode: string;
  stockName: string;
  period: string;
  /** 窗口右端（= 达标确认日）ISO 日期 */
  anchorDate: string;
  windowDays: number;
  cumGainPct: number;
  maxDailyPct: number;
  thresholdPct: number;
  /** 板块归属（仅用于分组，避免混池假象） */
  marketType: string;
  recommended: boolean;
}

/** 漏检明细行。`scoredOutStyles` 非空 ⇒ 可归因到具体风格（截断留痕命中）。 */
export interface MoverMissRow {
  event: MoverEvent;
  layer: string;
  pickedAnyPeriod: boolean;
  scoredOutStyles: string[];
}

/** 归因分层小计（含板块分组）。 */
export interface MoverLayerRow {
  layer: string;
  count: number;
  byMarketType: [string, number][];
}

/** `analyze_mover_recall` 响应（字段与 Rust DTO 一一对应，禁区 13）。 */
export interface MoverRecallView {
  from: string;
  to: string;
  /** 数据起点声明：早于此没有快照 ⇒ 面板必须原文显示这句，不得留白 */
  dataSince: string;
  /** 已采集交易日数：某档可判定 ⇔ ≥ 该档 windowDays（未达阈 ⇒ 「尚不可判定」而非「无事件」） */
  collectedDays: number;
  universeSize: number;
  universeConfirmed: number;
  rules: MoverTierRule[];
  rates: MoverRecallRates;
  layers: MoverLayerRow[];
  misses: MoverMissRow[];
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
  snippet?: string;
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

// ── 反思命中率（M2/M3：PLAN-stock-decision-hitrate-validation）──
// 后端权威定义 `analysis-engine/src/reflection_stats.rs`（serde camelCase）。

/** 按维度分组的方向命中率与分周期指标；样本 < 5 时命中率为 null（样本不足） */
export interface HitrateGroup {
  key: string;
  /** 已判定（成熟）样本数 */
  samples: number;
  directionHitRate: number | null;
  /** 目标价命中率；无目标价判定样本 → null */
  targetHitRate: number | null;
  /** 该组平均净收益（%）；无样本 → null */
  avgRawReturnPct: number | null;
  /** 该组平均超额收益（%）；无 alpha 样本 → null */
  avgAlphaPct: number | null;
  /**
   * 该组 rank IC（Spearman ρ：决策置信度 vs 实际净收益）—— Phase E 观测面。
   * null 的真实原因由 `icStatus` 点名，不看数字猜。
   */
  rankIc: number | null;
  /** 参与 IC 计算的样本数（同时有置信度与收益者），与 `samples` 不同口径 */
  icSamples: number;
  /**
   * IC 缺席原因：ok / insufficient_ic_samples / no_confidence / degenerate_variance /
   * pre_snr_regime（有置信度但全部缺判定口径水印 ⇒ v104 前的旧记录）
   */
  icStatus: string;
  /** 因「缺判定口径水印」被挡在 IC 分母外的条数（换代进度；归零即全部为当代样本） */
  icRegimeExcluded: number;
  /** 该档期望持有交易日（半衰期拟合自变量）；action 分组与 unknown → null */
  holdingDays: number | null;
}

/** 四周期命中率分组（key 为 ultra_short / short / mid / long / unknown） */
export type HorizonHitrateGroup = HitrateGroup;

/** 命中率聚合结果 —— 消费 strategy_performance + stock_reflections + stock_analyses */
export interface HitrateStats {
  totalSamples: number;
  directionHitRate: number | null;
  targetHitRate: number | null;
  avgRawReturnPct: number | null;
  avgAlphaPct: number | null;
  /** 样本中来自旧单周期字段回退（legacy）的条数 */
  legacySamples: number;
  byAction: HitrateGroup[];
  byHorizon: HitrateGroup[];
  /**
   * 预测半衰期（交易日）：由逐档 rank IC 的指数衰减拟合导出（IC(h)≈IC₀·e^(−λh)）。
   * null = 无法拟合（有效档 < 3 / IC 非正 / 拟合优度不达标），不是「无限长」。
   * Phase E 仅观测，不回写任何权重。
   */
  signalHalfLifeDays: number | null;
  /** 参与半衰期拟合的档数（有持有期且 IC 有值） */
  usableIcTiers: number;
}

// ── 四周期反思结果（批次 3/4：horizon_results_json 结构化）──
// 后端权威定义 `stock_workflow/reflection.rs::build_horizon_results_json` 落库 JSON。

/** 四周期状态枚举（与后端 HorizonStatus serde snake_case 对齐） */
export type HorizonStatus = "mature" | "immature" | "unavailable" | "legacy";

/** 单周期决策 */
export interface HorizonResultDecision {
  action: string;
  positionPct?: number | null;
  targetPrice?: number | null;
  stopLoss?: number | null;
  confidence?: number | null;
}

/** 单周期市场事实 */
export interface HorizonResultMarket {
  entryPrice?: number | null;
  exitPrice?: number | null;
  returnPct?: number | null;
  alphaPct?: number | null;
  maxDrawdownPct?: number | null;
  targetReached?: boolean | null;
  stopLossTriggered?: boolean | null;
}

/** 单周期客观评价 */
export interface HorizonResultEvaluation {
  /** 1 = 正确, 0 = 错误, null = 不可判定（immature/unavailable/neutral） */
  wasCorrect?: 0 | 1 | null;
  directionMatch?: boolean | null;
  targetHit?: boolean | null;
  mismatch?: string | null;
}

/** 单周期反思条目（对应 horizon_results_json.{horizon}） */
export interface HorizonResultEntry {
  status: HorizonStatus;
  expectedHoldingDays: number;
  actualHoldingDays?: number | null;
  decision: HorizonResultDecision;
  market?: HorizonResultMarket | null;
  evaluation?: HorizonResultEvaluation | null;
  reflection?: Record<string, unknown> | null;
}

/** 四周期结果 Map（后端键为 snake_case，前端消费同键） */
export type HorizonResultsMap = Partial<Record<"ultra_short" | "short" | "mid" | "long", HorizonResultEntry | null>>;

/**
 * as-of 降级的严重度分档（T14）—— 与后端 `axagent_harness::as_of::DegradationKind`
 * 的 serde 输出逐值对齐（改一边必须改另一边）。
 *
 * 动因：降级条目此前只有一种视觉权重 ⇒「个股没有场内期权」和「接口 301 挂了」
 * 长得一样，面板读不出「哪条值得去修」。
 */
export type AsOfDegradationKind = "failure" | "noData" | "structuralGap";

/** `get_asof_degradation_log` 返回的一条记录 */
export interface AsOfDegradationEntry {
  vendor: string;
  method: string;
  reason: string;
  /** 该 DTO 是 wire snake_case 的既成例外（后端字段就叫 as_of），非本仓 camelCase 标准 */
  as_of: string;
  kind: AsOfDegradationKind;
}
