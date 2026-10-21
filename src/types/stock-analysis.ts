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

/** 四周期价位映射，键一律 camelCase（snake 形态的存量行在 `normalizeHorizonPriceMap` 里归一） */
export interface HorizonPriceMap {
  ultraShort?: HorizonPriceGroup | null;
  short?: HorizonPriceGroup | null;
  mid?: HorizonPriceGroup | null;
  long?: HorizonPriceGroup | null;
}

/**
 * 阶段 2：单个周期的独立决策。
 *
 * **生产者已换**（R-11，2026-10-04）：本对象 = 四路分支节点各自的输出
 * （`portfolio-mgr-h-{ultra-short,short,mid,long}.rhai`），主链 `portfolio-mgr.rhai` 只做**装配**
 * （追加 `targetPrice`/`stopLoss` 两个绝对价）。四档的腿集、门形态、置信与出场口径**互不相同**，
 * 故以下字段里有若干「仅某档产出」的项 —— 不是可选装饰，是各档算法差异的读数。
 *
 * ⚠ 某一路节点失败/未接线 ⇒ 该档**整个键缺席**（不补占位行），并在顶层 `dataGaps` 点名。
 */
export interface HorizonDecision {
  action: string;
  /**
   * 档内风控否决**前**的原始方向（2026-10-10 B 裁定）：四档 `action` 现在是
   * 档内风控否决后的终值，而 LLM 侧 action 天然未叠加风控政策 ⇒ 两侧方向一致性维度
   * （Rust `decision.rs`）与前端「被降为」成句（`stock-analysis-utils.ts`）
   * 要比较/引用否决前方向时必须读本键；缺席（旧批次记录 / 未否决）退回 `action`。
   * 产端 = 四份 `portfolio-mgr-h-*.rhai`。
   */
  originalAction?: string;
  /** 本档动作是否被档内风控否决（true 时 `action` 已被改写，`originalAction` 保留原值） */
  tierVetoed?: boolean;
  verdict: string;
  /** 该档档位标签（分支脚本自证；装配段用它核对「读到的不是别档输出」） */
  horizon?: string;
  positionPct: number;
  /**
   * 该档**上涨胜率**（×100）—— **纯证据口径**：逐档先验 → 本档分支表加权融合，
   * **不含** `riskBias`。与荐股链的 `confidence`（`blend_win_rate` /
   * `candidate_score_to_win_rate`）**同一量纲，可直接比较**。
   *
   * 2026-10-02 改：此前本字段是「叠加风险偏置后的**判定值**」⇒ 同一份 JSON 里两个
   * 「置信度」不同口径，并排展示时会把「高风险」误读成「胜率低」，也无法与荐股比较
   * （实测 002812：荐股 78 vs 本字段 39.7，看着像两系统严重对立，实为量纲错配）。
   *
   * 2026-10-04 改（R-11）：逐档分支**本就没有**风险偏置这一层（各档的门是自己的形态，
   * 见 `entryGate`/`gateBasis`），故原并列字段 `confidenceRiskAdjusted` 从本类型删除 ——
   * 它是主决策（`StockDecision`）的字段，留在逐档类型里就是一个永不到货的幽灵声明。
   */
  confidence: number;
  /**
   * 该档后验（×100，四舍五入到 0.1）—— 逐档先验 + 本档证据加权的结果（不含风险偏置）。
   */
  posterior: number;
  stopLossPct: number;
  takeProfitPct: number;
  expectedHoldingDays: number;
  targetPrice: number | null;
  stopLoss: number | null;
  /**
   * 该档技术腿吃的是哪一份评分（结构性缺席声明）：
   * `tier_native` = 本档专属粒度评分节点出数；
   * `daily_fallback` = 该粒度评分没出数（超短档无 60 分钟线 / 长线档无月线），f1 腿退回主链日线。
   * ⚠ 缺席必须成句，不得压成「评分低」或干脆不显示 —— 见 AGENTS.md 禁区 12 与本轮 §七-F。
   * 退化时主链装配还会往顶层 `dataGaps` 追加一条（横幅的「数据缺口 N 项」读那里）。
   */
  scoreSource?: string;
  /** 该档先验来源：逐档先验表里的 `source` 列（tier / pooled）；表缺本档那一行 ⇒ `unavailable` */
  priorSource?: string;
  /** 该档先验样本数（配合 κ 判断收缩强度是否够可信） */
  priorSamples?: number;
  /**
   * P1-3（`PLAN-stock-decision-reliability.md` R3/R7）：该档置信度是否已过**校准可得性**门槛
   * （本档判定样本 ≥ `reflection_stats::CALIBRATION_MIN_SAMPLES`）。产端 = 四份
   * `portfolio-mgr-h-*.rhai`，值来自 Rust 注入的逐档先验表 ⇒ 本类型只镜像生产形态。
   * `false`/缺席 = 未校准，主链会把对外置信封顶压到最保守一级（见 `R-CALIB-CAP` 追溯链）。
   * ⚠ 键缺席按未校准处理（fail-closed）；不得读成「0 样本」以外的含义。
   */
  calibrationReady?: boolean;
  /**
   * P2-c（`PLAN-stock-decision-reliability.md`）：该档**原始方向命中率**
   * （0~1，未收缩口径 = `HitrateStats.direction_hit_rate`）；缺席 = 该档样本
   * 未达阈值（此时先验借自全档合并基准，别拿它当本档命中率）。
   */
  priorHitRate?: number;
  /** 止损口径：`vol_band` = k·σ_daily·√本档持有天数 导出；`fallback_pct` = σ 不可得，退回固定百分比档 */
  stopSource?: string;
  /**
   * 本档**按档风险档**（v140 起由分支行带回；产端 = 子模板内的 `cls-risk-level-<档>` 节点）。
   *
   * 为什么它是分支的一个输出而不是主链的旁路读取：风险节点从父图搬进各档子模板后，
   * 子执行只把扇出节点的双键写回父池 ⇒ 主链再按 `cls-risk-level-<档>` 取就是取一个不存在的
   * 路径（`present()` 恒假 ⇒ 按档风险收紧整条静默退役）。带回这一行才是可检的接线。
   */
  riskCategory?: string;
  /** 仓位口径：逐档分支恒为 `kelly_x_position_multiplier`（凯利 × 该档仓位乘数） */
  positionSource?: string;
  /**
   * 该档证据完整度（×100）= 本档**在场方向腿**权重和 ÷ 本档分支表**全部方向腿**权重和
   * （`pm_evidence_scale(tw, dir_total)`）。分母来自注入的逐档分支表，不是主链的合计表 ⇒
   * 与主链 `evidence_pct` 不同口径是有意的（第 5(a) 条）：它回答的是「这一档自己的腿齐不齐」。
   */
  evidenceScale?: number;
  /** 该档下注的实际赔率 = 止盈% ÷ 止损%（各档自己的出场口径，第 5(b) 条） */
  odds?: number;
  /**
   * 该档的**自证三件套**（分支表注入值原样回写；缺表时为 `missing_branch_table`）。
   * 装配段靠 `confidenceMethod` 判定「读到的确实是分支输出」，缺即按缺席处理。
   */
  confidenceMethod?: string;
  /** 该档出场口径：`time_stop` / `target_and_falsified` 等（权威 = `horizon_branch_specs`） */
  exitRule?: string;
  /** 该档入场门形态：`sealed_limit_up` / 带内居中 / DCF 目标未达 等 */
  entryGate?: string;
  /** 入场门判定结果（各档门不同，故不能由展示层反推） */
  entryGatePassed?: boolean;
  /** 门的判定依据：`judged` / `unjudged`（算不出 ≠ 通过，也不得算成未通过）；
   *  超短档 P0-5 起为 `breadth_missing` / `breadth_weak` / `stock_unjudged` /
   *  `limit_up_recent` / `strong_momentum` / `stock_not_strong` 六值之一 */
  gateBasis?: string;
  /**
   * 超短档 P0-5：breadth 门的**个股**前提原始量（近 5 日涨停次数 / 累计涨幅% / 评估天数）。
   * 封板率是市场级量，门必须再验个股自身；K 线不足 ⇒ 整个字段为 null（未判定 ≠ 通过）。
   */
  stockStrength?: {
    limitUpCount?: number;
    cumGainPct?: number;
    days?: number;
  } | null;
  /** 超短档 P0-5：microstructure 个股腿未接线期间动作封顶是否触发（买入/增持 → 持有） */
  p05CapApplied?: boolean;
  /**
   * 超短档 P2a（2026-10-18）：封顶**前**的动作（结构化自证，与 `p05CapApplied` 配对）。
   * 产端 `portfolio-mgr-h-ultra-short.rhai` 恒写本键（未封顶时等于 `action`），
   * 主链摘要「动作=封顶结果（买入被压回…）」读这里，**不从 dataGaps 文案反解**。
   * 消费判定以 `p05CapApplied === true` 为准，本字段只回答「封顶前是什么」。
   */
  actionBeforeCap?: string;
  /** 短线档：时机过滤是否放行（追高/带外判定） */
  timingFiltered?: boolean;
  /** 中线档：价带 z 值（×100，四舍五入到 0.1）；K 线不足 ⇒ null */
  bandZ?: number | null;
  /** 长线档：证伪判据命中的条目（`exitRule=target_and_falsified` 的 falsified 侧） */
  falsifiedBy?: string[];
  /** 长线档：作为过滤条件参与、不进方向融合的腿 */
  filterLegs?: string[];
  /** 长线档：只作背景（role=background / weight 恒 0）的腿 */
  backgroundLegs?: string[];
  /** 中线档：role=riskNote 腿的读数（风险提示，不参与方向） */
  riskNotes?: string[];
  /** 短线档：在场方向腿数与本档要求的下限（门 ②「证据太薄不出方向」的读数） */
  liveDirectionLegs?: number;
  /** 短线档：在场方向腿数下限 */
  requiredDirectionLegs?: number;
  /** 长线档：止盈价来源（`dcf_target` / `no_target`）—— 各档出场口径不同，故逐档标注 */
  takeProfitSource?: string;
  /** 该档**在场**腿明细（含 role/weight/signal/constraint，面板按此列证据） */
  legs?: Array<{
    factor: string;
    role?: string;
    weight?: number;
    signal?: number;
    constraint?: string | null;
  }>;
  /** 该档**缺席**腿（分支表里声明了、但本轮拿不到读数）⇒ 必须成句显示，不得当成 0 分 */
  absentLegs?: string[];
  /** 该档分支自己的缺口说明（顶层 `dataGaps` 另有一份跨档汇总） */
  dataGaps?: string[];
  /**
   * 该档技术腿**实际用的指标窗口**（评分节点回显的 `indicators.windows`，不是「想要的配置」）。
   *
   * 存在理由（PLAN §一○六，裁定 3「让用户看出各档实际几根」）：v136 之后四档各自按尺度取数、
   * 按该档窗口计划出指标，但界面上只有分数没有「几根 bar」⇒ 读者无法区分「这一档算得粗」与
   * 「这一档观点不同」。短/中/长三档的**根数按公式就是同一组**（差别在 bar 的日历跨度：
   * 2 周 / 2 月 / 2 季），这一事实只有把根数与尺度并列出来才可读，否则会被读成三档同一份输入。
   *
   * ⚠ 缺席语义：本字段缺失 = **该行产自 v138 之前的模板代**（旧行不回写），
   * 不是「该档没有窗口」。因此展示侧只在有值时成句，缺席不渲染任何一行 —— 不得压成 0 或空数组。
   */
  scoringWindows?: HorizonScoringWindows | null;
  /** 本档评分所在尺度（`hourly` / `daily` / `weekly` / `monthly`），与 `scoringWindows` 同批出现 */
  scoringScale?: string;
  /**
   * 本档**两带趋势事实**（产端 = `indicators.scaleTrend`，v139）。
   * `diffPct` / `fastSlope` 的缺席形态是 **`null`**（产端 `Option<f64>` 不跳过序列化）⇒
   * 面板必须印「—」而不是 0：慢带非正/上一根不可算是「算不出」，不是「差 0%」。
   */
  scoringTrend?: HorizonScaleTrend | null;
  /** 本档动量的「周期 + 值」对（产端 = `indicators.scaleMomentum`，与 `scoringScale` 同一份口径自证） */
  scoringMomentum?: HorizonScaleMomentum | null;
}

/** 两带趋势回显（权威产端 = `astock-data::indicators::ScaleTrend`，serde camelCase） */
export interface HorizonScaleTrend {
  fastBars: number;
  slowBars: number;
  fast: number;
  slow: number;
  diffPct: number | null;
  fastSlope: number | null;
}

/** 动量「带周期的值」对（权威产端 = `astock-data::indicators::ScaleValue`） */
export interface HorizonScaleMomentum {
  period: number;
  value: number;
}

/**
 * 指标窗口回显（权威产端 = `astock-data::indicators::IndicatorWindows`，serde camelCase）。
 * 与 `HorizonDecision.scoringWindows` 同一条来源链，字段名不得手抄成第二套。
 */
export interface HorizonScoringWindows {
  maPeriods: number[];
  macdFast: number;
  macdSlow: number;
  macdSignal: number;
  rsiPeriods: number[];
  bollPeriod: number;
  volumeLookback: number;
}

/** 阶段 2：四周期独立决策映射（键 camelCase，对齐 `decisions_by_horizon`） */
export interface DecisionsByHorizon {
  ultraShort?: HorizonDecision | null;
  short?: HorizonDecision | null;
  mid?: HorizonDecision | null;
  long?: HorizonDecision | null;
}

/**
 * 一份辩论产物（多空两侧正文 + 档位 + 轮次）。
 *
 * `tier === null` 表示分档之前的历史记录（≤v160：四档共用一份，节点 id 无档位后缀）。
 * v161 起辩论按四档展开（`bull-r1--mid` / `bear-r1--mid`），**同档同轮才是一对**；
 * 呈现层不得把不同 tier 的行按 `round` 合并 —— 那会让后完成的档覆盖先完成的档。
 */
export interface DebateRound {
  /** 档位 snake_case（`ultra_short` / `short` / `mid` / `long`）；null = 历史单份记录 */
  tier: string | null;
  round: number;
  bull: string;
  bear: string;
}

/**
 * LLM（trader 节点）自报的**逐档决策**一行。
 *
 * 字段集 = `src-tauri/config/llm-output-schemas.json` 里 `trader_output.decisions.*` 的 7 个
 * required，也是 `decision.rs::normalize_llm_horizon_decisions` 归一后的落库形态；
 * 两侧由 `scripts/check-horizon-decision-keys.mjs` 对账（改一侧不同批改另一侧即红）。
 *
 * ⚠ 与 `HorizonDecision`（公式侧逐档，35 键）**不是同一种行**：那一侧带腿 / 门 / 先验 / 后验等
 * 自证，这一侧只有 trader 的结论本身。两侧**同键名**（camel）以便逐档并列，但不得互相顶替字段，
 * 也不得把两侧的仓位读成同一个口径（LLM 侧不含档内风控否决与仓位上限）。
 */
export interface LlmHorizonDecision {
  action: string;
  verdict: string;
  positionPct: number;
  confidence: number;
  stopLossPct: number;
  takeProfitPct: number;
  expectedHoldingDays: number;
}

/** 与 `DecisionsByHorizon` 同一族 camel 键；**键缺席 = 该档 LLM 没给**（由 `data_gaps` 逐档点名） */
export interface LlmDecisionsByHorizon {
  ultraShort?: LlmHorizonDecision | null;
  short?: LlmHorizonDecision | null;
  mid?: LlmHorizonDecision | null;
  long?: LlmHorizonDecision | null;
}

/**
 * 一档的「公式 vs LLM」逐档对照结果（产端 `stock_workflow/core.rs` 嵌在
 * `decision.agreementByHorizon`，判据正主 = `decision.rs::compute_horizon_agreements`）。
 *
 * ⚠ **与主档 `agreementBreakdown` 不是同一个计数域**：主档五维 / 内部满分 90，
 * 逐档三维（方向 / 仓位 / 置信）/ 满分 65。两者归一后都叫 0-100，但**不可并排比较**，
 * 所以 `dimsUsed` / `dimsMissing` 必须由产端带出来、由面板原样说给用户。
 * 逐档不含 `riskLevel`（该维只有一侧有）与 `evidenceCited`（论据是一次调用的全局产出）。
 *
 * `total === null && comparable === false` = **无从比对**（成因见 `absence`），
 * 不是 0 分 —— 0 分是「一致得零分」，两者在面板上必须各占一句。
 */
export interface HorizonAgreement {
  /** camel 档位键，与 `decisionsByHorizon` / LLM `decisions` 同族拼写 */
  tier: string;
  /** 档位中文名（产端 `Period::label_zh`，前端不再手抄第四份档名） */
  tierLabel: string;
  comparable: boolean;
  total?: number | null;
  absence?: string | null;
  formulaAction?: string | null;
  llmAction?: string | null;
  actionScore?: number | null;
  actionNote?: string | null;
  actionOk?: boolean | null;
  positionScore?: number | null;
  positionGap?: number | null;
  confidenceScore?: number | null;
  confidenceGap?: number | null;
  conflictType?: string | null;
  /** 拍板 4：该档公式腿里数值来自 LLM 分析师的权重占比（%）。null = 无从判断，**不是 0%** */
  sharedEvidenceWeightPct?: number | null;
  dimsUsed: string[];
  dimsMissing: string[];
}

/**
 * 一档的分歧归因（产端 `decision.divergenceByHorizon`，判据 = `decision.rs::attribute_horizon_divergence`）。
 *
 * 读的是**档行自己的键域**（`legs[].bias` / `absentLegs` / `entryGatePassed` / `tierVetoed`），
 * 与主档 `crossCheck.divergence` 的顶层指针（`/evidence/factors`、`/effective_params/*`）互不重叠，
 * 所以两侧驱动项码表也不同 —— 不要在前端把它们合并成一个列表。
 */
export interface HorizonDivergence {
  tier: string;
  tierLabel: string;
  /** 结构化码表：tier_risk_veto / entry_gate_failed / llm_tier_absent / negative_legs / absent_legs / data_gap / formula_tier_absent */
  drivers: string[];
  entryGate?: string | null;
  entryGatePassed?: boolean | null;
  gateBasis?: string | null;
  /** 档内风控否决的 [否决前方向, 否决后动作] */
  veto?: [string, string] | null;
  /** 贡献最负的至多三条腿：[factor, bias, weight] */
  negativeLegs?: Array<[string, number, number]> | null;
  absentLegs?: string[] | null;
  dataGaps?: string[] | null;
  llmAbsence?: string | null;
}

/**
 * 逐档 LLM 决策的读侧状态（四态互斥）。判据的正主是 Rust 侧
 * `decision.rs::LLM_HORIZON_DECISION_CONTRACT`（代码盖章的代际标记），此处只镜像其读法：
 * · `absent` —— `llm_decision_json` 整列为空（trader 没跑 / 没落库）
 * · `parseFailed` —— 整列不是合法 JSON（含半截围栏），**不进任何分母**
 * · `preContract` —— 可解析但**无代际标记** ⇒ 该记录产自逐档契约上线前
 * · `ok` —— 有标记；此时某档缺键 = 「LLM 未给出该档」（与 `preContract` 不是一回事）
 */
export type LlmHorizonReadState = "absent" | "parseFailed" | "preContract" | "ok";

/**
 * v149(L2/L3)：主档 action 与算法估值结论的**对账结果**（产出方 `portfolio-mgr.rhai`
 * 的 `valuation_reconciliation`）。存在的理由：一次分析里主档说「买入/持有」、长线价值
 * 视角却说「高估」，两者同屏而不说明谁管谁 —— 兑现产端注释「这类矛盾必须一眼可见」。
 */
export interface ValuationReconciliation {
  /**
   * 冲突种类；空串 = 无冲突（正常态）。
   * · `rich_vs_nonbearish` = 算法估值偏空（高估/偏高）而决策方向非空头（买入/增持/持有）
   * · `cheap_vs_bearish`   = 算法估值偏多（低估/合理偏低）而决策方向为空头（减持/卖出）
   */
  conflictKind?: string;
  /** 估值侧的结论原文（`高估` / `偏高` / `低估` / `合理偏低` …） */
  valuationAction?: string;
  /** 冲突发生时、**封顶之前**主档的方向（即与估值相反的那一侧） */
  decisionAction?: string;
  /** 封顶**之后**的最终动作（未封顶时与 `decisionAction` 同值） */
  finalAction?: string;
  /** 是否因「算法估值=高估 ⇒ 禁止加仓档」把 买入/增持 封顶为 持有 */
  capped?: boolean;
  /** 冲突发生时所选档（snake_case）；主链定档那代可能为空 */
  horizon?: string;
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
   * 阶段1 引入、v125 起为 `decisionsByHorizon` 的**投影**（只搬运不计算）。
   *
   * 后端 `portfolio-mgr.rhai` 产出的 `horizonPriceMap`（嵌套在 `decision_json`），经
   * `stock_analyses.horizon_price_map` 落库。**键一律 camelCase**：`normalizeHorizonPriceMap`
   * 会把存量行的 snake 键（`ultra_short`）归一到 camel —— 这一步不是可选的：旧实现只做类型
   * 断言不转键，四个键里只有 `ultraShort` 两族拼写不同，于是「超短档价位行从来不显示、
   * 另外三档正常」，而库里四键齐备（000710 现网实证）。
   *
   * ⚠️ `null`/`undefined` 语义 = 该记录产生于 stage1 字段引入之前，**无此信息**；
   *   消费端**不得**读成空映射，应按顶层 `targetPrice`/`stopLoss`（主档位）回退。
   * ⚠️ **单个档缺席**（那一路分支没产出，口径 A）= 该键不在映射里，而不是值为 null；
   *   两种缺席都要按「该档无计划」处理，不得读成 0%。
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
  /**
   * 主档 action 的**来历**（§五十三 ①，v127）。产出方 `portfolio-mgr.rhai` 的 `action_source`，
   * 值域见 `stock-analysis-utils.ts` 的 `ACTION_SOURCES`（第三处载体是那条并集门）。
   *
   * 存在的理由：主档 action 可能是「所选档分支结论」被后置规则**单向降级**的结果
   * （000710 实测：超短线分支=买入 → 高风险风控否决 → 主档=持有，同屏三处互斥却只能靠
   * 解析中文 reasoning 才能解释）。缺席该字段的记录 = 产生于 v127 之前，**无此信息**，
   * 展示层不得回退成「branch_pick」那种乐观默认。
   */
  actionSource?: string;
  /**
   * P2-c（`PLAN-stock-decision-reliability.md`）：主档 `timeHorizon` 的**定档来源**。
   * 产出方 `portfolio-mgr.rhai` 的锁档段（与 `decision_horizon_source` 列同源），
   * 值域 `HORIZON_SOURCES`（`branch_pick` / `formula_no_branch` / 历史值）。
   * 标签映射见 `stock-analysis-utils.ts` 的 `horizonSourceLabelKey`；缺席 = 记录产
   * 生于该字段引入之前（无此信息），展示层不得回退成乐观默认。
   */
  horizonSource?: string;
  /**
   * 主档 confidence 的口径（v127 起与所选档同源）。值域 `CONFIDENCE_SOURCES`。
   * · `branch_row` = 取自被选中的那一档分支输出（再乘档位无关的视角分歧/数据质量约束）
   * · `main_chain_posterior` = 四路全未产出 ⇒ 退回主链后验口径
   */
  confidenceSource?: string;
  /**
   * v149(L1)：主档 action 的**判定口径**置信度（0-100），与对外展示的 `confidence` 可能不同源。
   *
   * 存在的理由：动作由该口径判出（如所选档未调制后验 ≥0.72 才买入），而展示用的 `confidence`
   * 已被视角分歧系数 × 数据质量上限压低 ⇒ 只看 54.5% 会误读成「这么低为何买入」。
   */
  actionConfidenceBasis?: number | null;
  /**
   * v149(L1)：`actionConfidenceBasis` 的口径来源。
   * 值域 `{branch_row, main_chain_effective_posterior}`（展示用 `CONFIDENCE_SOURCES` 是它的子集）。
   */
  actionConfidenceBasisSource?: string | null;
  /**
   * v149(L2/L3)：主档 action 与算法估值结论的对账结果。`conflictKind` 非空即方向相反，
   * 展示层须显式提示；`capped=true` 表示已把 买入/增持 封顶为 持有（actionSource=valuation_cap）。
   */
  valuationReconciliation?: ValuationReconciliation | null;
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
   * #8 P5 跨轮数据质量熔断态（portfolio-mgr 输出，三态）。
   * · `fused`      = 连续多轮**降档**（C 级及以下 / 缺失）并已触发熔断线 ⇒ 置信度上限已被压到最保守一级
   * · `ok`         = 有观测且未跨阈
   * · `unobserved` = **观测表为空** ⇒ 熔断判据未参与本轮（既不是熔断也不是"证据面健康"）
   * 判据（几轮算熔断、什么等级算异常）唯一在 `dao::repo::data_quality_fuse`，本字段是**已判定结果**。
   * ⚠ 2026-10-10 起 `A` / `B` 都算正常档（原判据 `grade != "A"` 会把「良好」的 B 也当异常）。
   */
  dqiFuseState?: string;
  /** #8 P5 最近连续**降档**（C 级及以下 / 缺失）的轮数（含本轮）；`dqiFuseState` 的佐证数字 */
  dqiStreak?: number;
  /** #8 P5 参与该计数的观测条数；0 = 无观测（不得当成"质量一直好"） */
  dqiObservations?: number;
  /** #8 P5 本轮**实际生效**的置信度上限（grade 阶梯 + 熔断封顶取小值），用于反解置信度是被谁压的 */
  confidenceQualityCap?: number;
  /**
   * portfolio-mgr 消费的上游节点中**缺失数据**的清单（如「资金流向(t-hotmoney-data)」）。
   *
   * ⚠️ 本字段的语义**只有一种**：「本该拿到的数据没拿到」（上游节点缺席 / 字段不可得）。
   * **设计性口径差异不属于这里** —— 逐档分支「这一档用了哪些腿、缺哪些腿」写在
   * 该档自己的 `decisionsByHorizon[].legs / absentLegs / dataGaps` 里。
   * 历史教训（2026-10-01）：设计性降权曾与真缺口同挤本字段 ⇒ **常驻误报**
   * （估值腿的 0.3/0.5 是常量 ⇒ 凡带估值数据的分析恒推 2 条 ⇒ 每张卡都亮
   * 「数据缺口 2 项」，真缺口被淹没）。乘数语义随 R-11 退役后，那条通道整体删除。
   *
   * 命名说明：后端 portfolio-mgr 决策 JSON 里该字段是顶层 snake_case 的 `data_gaps`
   * （唯一一个非 camelCase 的顶层键，且 `stock_workflow/decision.rs` V65 的
   * 一致性算法也按此名读取，故不能改名）；`normalizeDecision` 已统一收敛为
   * 前端 camelCase 的 `dataGaps`，消费处只读 `decision.dataGaps`。
   */
  dataGaps?: string[];
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
  /**
   * B1 逐档对照四行（产端 `core.rs` 嵌在 `decision.agreementByHorizon`）。
   * 键缺席 = 该记录产自逐档对照上线前 ⇒ 面板说「未产出」，**不得**渲染成四个 0 分。
   * 与上面 `agreementBreakdown` 不同计数域（三维 65 制 vs 五维 90 制），不可并排比较。
   */
  agreementByHorizon?: HorizonAgreement[] | null;
  /** B2：逐档分歧归因四行（驱动项码表见 `HorizonDivergence`） */
  divergenceByHorizon?: HorizonDivergence[] | null;
  /** 跨系统互证：近 14 天趋势智选推荐 vs 本次工作流决策（后端在决策持久化时注入） */
  crossCheck?: RecoCrossCheck;
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

// ── 实时监控告警（`IpcEventName::PriceAlertTriggered` 的 payload）──

/**
 * 后端 `axagent_analysis_engine::monitor::MonitorAlert` 的 camelCase 投影。
 *
 * 两侧键名由 `scripts/check-monitor-event-authority.mjs` 逐项对账 ——
 * 这个 payload 过去是在发射点手写 `json!` 摊平出来的，加字段时副本会静默少一键。
 */
export type MonitorAlertType =
  | "stop_loss"
  | "take_profit"
  | "resistance"
  | "support"
  | "change"
  | "volume"
  | string;

export interface MonitorAlertPayload {
  /** `price_alerts.id` —— 回写触发状态按这一条精确命中（同股同类型可能有重复行） */
  alertId: string;
  stockCode: string;
  stockName: string;
  /** 六类告警之一；历史值/未知值消费侧必须按「未知」兜底，不得当成某一类 */
  alertType: MonitorAlertType;
  alertMessage: string;
  currentPrice: number;
  changePct: number;
  /** 后端 `timestamp: String`（求值时刻的 ISO 串，不是行情快照时刻） */
  timestamp: string;
  suggestedAction: string | null;
}

/**
 * 条件单触发产物 —— 后端 `analysis_engine::conditional_order::ConditionTrigger` 的 serde 投影。
 *
 * 与 `MonitorAlertPayload` 是两个实体，不要混用：这一条来自 `conditional_orders` 表，
 * 触发时**不写** `price_alerts`，也不共用告警类型枚举（条件类别与阈值都由后端 `describe()`
 * 给成人话，前端不再自己拼类别串）。
 */
export interface ConditionTriggerPayload {
  orderId: string;
  stockCode: string;
  stockName: string;
  /** 条件描述（含单位）：如「跌破止损价 12.50」 */
  condition: string;
  /** 动作描述：如「卖出全部持仓」「仅通知」 */
  action: string;
  price: number;
  changePct: number;
  /** 触发时刻（UTC ms） */
  triggeredAtMs: number;
  /** 判定依据，缺席原因也写在这里（后端把「无法判定」与「条件未满足」分开给） */
  reason: string;
}

/**
 * 条件单列表项 —— 后端 `commands/conditional_order.rs::ConditionalOrderItem` 的投影。
 *
 * 类别与动作都成对给「机器可读串」+「后端 describe() 的人话」：前者供分支，后者供显示，
 * 前端不再自己拼阈值与单位（`condition_value` 的单位随 `condition_type` 而变，拼错就是假数）。
 */
export interface ConditionalOrderItem {
  orderId: string;
  stockCode: string;
  stockName: string;
  /** `stop_loss` / `take_profit` / `resistance_break` / `support_break` / `change_pct` / `turnover_rate` */
  conditionType: string;
  /** 价格类为元，比率类为百分比数值 */
  conditionValue: number;
  conditionText: string;
  /** `buy` / `sell` / `reduce` / `notify` */
  actionType: string;
  /** `buy` 股数 / `reduce` 比例；`sell` 的 null 是「卖出全部持仓」，`notify` 无参数 */
  actionValue: number | null;
  actionText: string;
  enabled: boolean;
  coolDownMinutes: number;
  maxTriggersPerDay: number;
  triggersToday: number;
  /** 计数归属的北京日 `YYYY-MM-DD`；null = 从未触发 */
  triggerDate: string | null;
  lastTriggeredAt: number | null;
  activeStart: string | null;
  activeEnd: string | null;
}

/** 表里存在但读不回成规则的行 —— 必须与「表里没有」分列显示 */
export interface UnreadableConditionalOrder {
  orderId: string;
  reason: string;
}

export interface ConditionalOrderListView {
  items: ConditionalOrderItem[];
  unreadable: UnreadableConditionalOrder[];
  /** 正在被行情评估的条数（启用且可读回）—— 与 `items.length` 是两个计数域 */
  evaluating: number;
}

// ── 监控引擎运行读数（裁定 12：「监控实际未在取数」必须是可消费的聚合面，不是日志）──

/**
 * 后端 `commands/stock_analysis.rs::MonitorStatusDto` 的投影。
 *
 * 每个键只说一件事，消费侧不得把它们收敛成一句：`assembled=false` 时其余计数**全是 0**，
 * 那些 0 的含义是「没人在取数」，不是「没有失败」；`tradingNow=false` 时轮次计数是
 * 上一盘的旧值。见 `PriceAlertPanel.tsx` 的 `monitorAbsenceSentences`。
 */
export interface MonitorStatus {
  assembled: boolean;
  /** 时段档位码（后端 `TradingSession::code()`）：前端按它查 i18n，不在 IPC 里收中文 */
  session: string;
  tradingNow: boolean;
  pollIntervalSecs: number;
  /** 引擎里**启用中**的告警条数（不是 `price_alerts` 表行数） */
  configsActive: number;
  /** 本轮应取数的股票数（同股多条告警共用一只） */
  roundCodesTotal: number;
  roundCodesFailed: number;
  /** 连续「整轮全部取数失败」的轮数，任一成功即归零；本轮无启用告警也归零 */
  consecutiveAllFailedRounds: number;
  /** 最后一次落数的 UTC ms；0 = 引擎装好但还没跑完第一轮 */
  roundFinishedAtMs: number;
}

/**
 * 组合级信号 —— 后端 `analysis_engine::cross_stock_aggregator::PortfolioSignal` 的 serde 投影。
 *
 * 与 `MonitorAlertPayload`（单只股票的告警）是两个层级：这条由跨股聚合器在**多只同方向**
 * 触发时产出，不写 `price_alerts`。此前它发的是裸字面量 `"portfolio-signal"`，
 * 事件名不在权威表、前端零监听 ⇒ 信号真产出了但没人收（阶段 G 的 G-6）。
 * `suggested_action` 是后端生成的人话（阈值与单位随主导信号而变，前端不自己拼），
 * 其余字段全部结构化供分支与 i18n。
 */
export interface PortfolioSignalPayload {
  /** `bullish` / `bearish` / `neutral`（后端 `SignalDirection` snake_case） */
  direction: string;
  /** 触发该组合信号的股票代码集合 */
  stocks: string[];
  /** 主导信号类型（后端 `SignalType` snake_case），值域随聚合器扩展 */
  dominantSignal: string;
  /** 组合信号强度 [0.0, 1.0] */
  strength: number;
  snapshot?: PortfolioSnapshotPayload | null;
  /** 后端给的建议文案（非结构化，前端不再拆句） */
  suggestedAction: string;
  /** Unix 秒 */
  timestamp: number;
}

/** 后端 `PortfolioSnapshot`：信号触发时的持仓上下文（各项都可能拿不到）。 */
export interface PortfolioSnapshotPayload {
  positionCount: number;
  topConcentrationPct?: number | null;
  maxSectorPct?: number | null;
  riskLevel?: string | null;
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
  /**
   * 2026-10-09（v148）新增：**逐档**诊断，面板「分析师数据缺口详情」据此把每个分析师摊成四行。
   *
   * 形态：`{ abbr: { 档名(snake): DataQualityDiagItem & { tier } } }`。
   *   · 与 `diagnostics` 的关系：`diagnostics` 仍是**代表实例**（各 base 的 mid 优先那一档，
   *     10 维），是 grade/score/quality-gate 的**唯一**依据；本字段只喂面板、不进任何评分。
   *   · 某分析师的逐档键**全部未注入**时该 abbr **整体缺席** ⇒ 前端回落 `diagnostics` 单行旧视图。
   *   · 旧快照无此字段 ⇒ 可选；面板须能降级展示（不显示逐档行）。
   */
  diagnostics_by_tier?: Record<string, Record<string, DataQualityDiagItem & { tier?: string }>>;
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
  /**
   * 分析完成时刻（ms）；null = 未知（本列引入前的历史行 / interrupted）。
   * 与 updatedAt 分家：后者会被交易意图审核、结果回写等后续写库顶掉。
   */
  completedAt: number | null;
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

export type AnalysisStatus =
  | "idle"
  | "loading"
  | "running"
  | "paused"
  | "completed"
  | "error"
  | "cancelled"
  /**
   * 记录**没有结论地消失**：进程在分析跑完前退出，终态从未写入。
   *
   * 由后端启动回收写入（`src-tauri/src/scheduler/restore.rs`）。与 `error` 互斥 ——
   * `error` 是「跑过了并失败」（有失败原因），本态是「连原因都没留下」。
   */
  | "interrupted";

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
  /**
   * v4（2026-10-18）：置信度语义 = **证据强度**（策略条件满足度 × 风格权重），
   * 个股维度、明确不是「上涨概率」（旧伪概率链已撤——同档所有票曾塌成档位常数）。
   * 真概率将来自闭环分桶校准（`PLAN-capital-long-evidence.md` §六），样本足够才输出。
   */
  confidence: number;
  /**
   * v4: 策略原始证据强度（0-100，风格权重调制前）——分桶校准的输入。
   * 缺字段 = v4 之前的存量行（无证据分概念，校准筛样排除）。
   * ⚠ 与分析链 V65 的 `evidenceScore`（evidence_cited 维度分）同名不同义。
   */
  evidenceScore?: number;
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
  /** 来自**用户自己的标的**（自选股 ∪ 本机手工加入/分析过的票） */
  user: number;
  /**
   * 来自**当日涨幅榜**（收盘快照按当日涨幅取前 50）。
   *
   * 口径边界：快照枚举域是 `market_stock_universe`（现网约 681 只），不是全市场；
   * 域外个股这条来源也发现不了 —— 面板不得把它读成「已覆盖全市场」。
   */
  mover: number;
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
  /**
   * mid/long 档被出票门拦截的候选(P0 财报门 / P1 分析回环门,2026-10-18)。
   * 后端 `RecoResponse.gate_blocked`(camelCase)。空数组 = 本批无拦截,字段可能缺省。
   * 展示建议:与 picks 并列呈现「已拦截」分组,理由码 first_loss / two_period_decline /
   * analysis_veto_<档>:<动作> 直接透出,不再靠日志反查「为什么候选少了」。
   */
  gateBlocked?: Array<{
    stockCode: string;
    stockName: string;
    style: StyleKey;
    confidence: number;
    reasons: string[];
  }>;
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
  /** mover 漏检线乘数；缺席 = 该格证据不足，本线未参与合成 */
  moverPenalty?: number;
  /** 数据质量熔断线乘数；缺席 = 本格可判定样本不足，本线未参与合成 */
  dqiPenalty?: number;
  /** 核查窗口内「算出了却被 top-N 淘汰」的事件数（留痕缺失时恒 0） */
  scoredOut?: number;
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
  /** 窗口起点 ISO 日期（P2 漏检补分析的 as-of 锚点） */
  windowStart: string;
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
  /**
   * 本视图属于哪条链：`reco` | `serenity` | `unlabeled`。
   * 值域唯一权威在后端 `RecoChain` / `CHAIN_UNLABELED`（门 `check-reco-horizon-parity` o 段
   * 逐值核对 11 语言的 `chain_*` 键），前端不得自己再列一份链名目。
   */
  chain: string;
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
  /**
   * 逐档达标事件数（`period -> events`）。三态依据：本档已满 + 0 = 「本档真无达标票」，
   * 必须与「本档有达标」分开说；`null` = 本轮未记录（旧轮次）⇒ 面板第三态，不得显示成 0。
   */
  eventsByTier: Record<string, number> | null;
  misses: MoverMissRow[];
}

// ── 妖股漏检 → as-of 历史补分析批次（P2，`run_mover_recall_analysis`）──────────

/**
 * 一轮**已落库**核查的元信息（`list_mover_recall_runs`）。
 * 目前只有 `success` 会落轮次行 ⇒ 「库里没有这一轮」与「那一轮没有达标事件」是两种缺席，
 * 面板不得把前者渲染成后者。
 */
export interface MoverRecallRunMeta {
  id: string;
  /** 本轮所属链；null = A-2 之前的混池轮次（面板单独成句，不冒充某一链的读数） */
  chain: string | null;
  dateFrom: string;
  dateTo: string;
  dataSince: string;
  collectedDays: number;
  universeSize: number;
  universeConfirmed: number;
  eventsTotal: number;
  missCount: number;
  status: string;
  triggerSource: string;
  createdAtMs: number;
}

/**
 * `get_mover_recall_run_view` 响应：轮次元信息 + 与实时核查**同形**的载荷。
 * 后端把 `MoverRecallView` 摊平在同一层，故此处直接继承 —— 面板复用同一套渲染与形状守卫。
 */
export interface MoverRecallRunView extends MoverRecallView {
  run: MoverRecallRunMeta;
}

/** 发起补分析批次请求（字段与 Rust RunMoverRecallAnalysisRequest 一一对应）。 */
export interface MoverRecallAnalysisRequest {
  /** 单批最多补分析事件数（1-100，默认 20） */
  batchLimit: number;
  /** 所属核查轮次 unexplained 占比 ≥ 该阈值时整轮不取（0-1，默认 0.5） */
  maxUnexplainedShare: number;
  /** 限定核查轮次（mover_recall_runs.id）；null/缺省 = 跨全部轮次选样 */
  runId?: string | null;
}

export type MoverRecallBatchStatus =
  | "running"
  | "completed"
  | "partial"
  | "failed"
  | "interrupted";

/** 批次内逐票状态（mover_recall_events 行的前端投影）。 */
export interface MoverRecallEventItem {
  eventId: string;
  runId: string;
  stockCode: string;
  stockName: string;
  period: string;
  /** as-of 锚点（窗口起点） */
  windowStart: string;
  /** 窗口右端（达标确认日） */
  anchorDate: string;
  cumGainPct: number;
  /** 归因层：not_in_pool / l3_scored_out / other_period / unexplained ... */
  missLayer: string;
  /** running / analyzed / failed（excluded/reflected 不进批次） */
  status: string;
  analysisId: string | null;
  errorDetail: string | null;
}

// ── 妖股漏检 → 补分析报告 + 逐档反思复盘视图（P3，`get_mover_recall_event_review`）──

/** 漏检复盘视图中的单档反思行（投影与 `list_reflections` 单一来源对齐）。 */
export interface MoverRecallReviewReflection {
  id: string;
  stockCode: string;
  stockName: string;
  originalAnalysisId: string | null;
  asOfDate: string;
  hindsightDate: string;
  /** 本反思行复盘的档位（ultra_short/short/mid/long） */
  horizon: string | null;
  actualOutcome: string;
  whatWentWrong: string | null;
  /** 漏检信号，落库为 JSON 数组字符串（展示前需 parse；失败按原文展示） */
  missedSignals: string | null;
  fixForFuture: string | null;
  verdict: string | null;
  lessonSummary: string | null;
  rawReturn: number | null;
  alphaReturn: number | null;
  /** 来源桶：`mover_recall` = 漏检回放；其余（含空串）= 常规到期 due */
  source: string;
  /** 系统机械盖章的权威归因层（仅 mover_recall 行非空） */
  missLayer: string | null;
  moverLabel: string | null;
  /** pending / running / completed / failed: ... */
  status: string;
  createdAt: number;
}

/**
 * `get_mover_recall_event_review` 响应：事件事实 + as-of 补分析整行 + 逐档反思。
 * 三键（stockCode + anchorDate + period）定位；查无事件时命令返回 `null`。
 */
export interface MoverRecallEventReview {
  eventId: string;
  runId: string;
  /** pending / excluded / running / analyzed / failed / reflected */
  status: string;
  missLayer: string;
  period: string;
  stockCode: string;
  stockName: string;
  windowStart: string;
  anchorDate: string;
  windowDays: number;
  cumGainPct: number;
  maxDailyPct: number;
  batchId: string | null;
  analysisId: string | null;
  /** 首个 completed 反思档的行 id（status=reflected 时非空） */
  reflectionId: string | null;
  errorDetail: string | null;
  /** as-of 补分析整行（stock_analyses 全字段 camelCase）；未跑到成功为 null */
  analysis: Record<string, unknown> | null;
  reflections: MoverRecallReviewReflection[];
}

/**
 * `get_mover_recall_analysis_batch` 响应；
 * `list_mover_recall_analysis_batches` 返回同型但 `items` 为空数组（摘要）。
 */
export interface MoverRecallBatchState {
  batchId: string;
  status: MoverRecallBatchStatus;
  message: string | null;
  total: number;
  ok: number;
  fail: number;
  items: MoverRecallEventItem[];
  errorDetail: string | null;
  createdAtMs: number;
  updatedAtMs: number;
  finishedAtMs: number | null;
  /** 触发来源：manual / schedule；null = 本列引入前的旧批次（未知） */
  trigger?: string | null;
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
/** 一格失败模式计数（PLAN P2-b）。`share` 的分母是**已标注反思行**，不是全部反思行。 */
export interface FailureModeCell {
  mode: string;
  count: number;
  share: number;
}

/**
 * 失败模式看板（随 `reflection_stats` 一起出 IPC，面板零新增 invoke）。
 * 三种缺席必须各占一句，不得收敛成一个数：
 * `unlabelled`（模板没产出该字段）／`rejectedTokens`（模型自报了词表外的名字）／
 * `cells` 为空（已标注但都落在词表外）。
 */
export interface FailureModeBoard {
  /** 参与统计的 completed 反思行数 */
  totalRows: number;
  labelled: number;
  unlabelled: number;
  cells: FailureModeCell[];
  rejectedTokens: string[];
}

/**
 * 阶段 3 / 决-3：结构层待裁项的一行明细（反思判为 L2/L3，即「根因不在参数层」）。
 * 文本已在后端按**字符**截断，前端不再二次裁剪。
 */
export interface StructuralReflectionRow {
  reflectionId: string;
  stockCode: string;
  stockName: string;
  /** 该反思复盘的档；null = 未记录（不是「所有档通用」） */
  horizon: string | null;
  /** "L2" = 改 .rhai 公式；"L3" = 改 .md 提示词 */
  tier: string;
  /** 反思给的目标与最小改动描述；null = 模板未产出 */
  proposal: string | null;
  whatWentWrong: string | null;
  updatedAt: number;
}

/**
 * 反思落地层级看板。四类计数**互不折算**：
 * `unlabelled` = 本次反思没判层级（含本列引入前的存量行），不得读成 `l1`。
 */
export interface TierBoard {
  totalRows: number;
  l1: number;
  l2: number;
  l3: number;
  unlabelled: number;
  structuralRows: StructuralReflectionRow[];
}

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
   * 逐档结局 2×2（B4）：公式判对否 × LLM 判对否，同窗口、同判定函数（产端
   * `reflection_stats::build_horizon_confusion`）。四档恒各一行 —— 没样本的档是
   * `judged === 0`，不是「没有这一档」。
   * ⚠ 计数域与 `byHorizon`（只统计公式侧）不同，两者不得并成同一个分母读。
   */
  horizonConfusion: HorizonConfusionRow[];
  /**
   * 预测半衰期（交易日）：由逐档 rank IC 的指数衰减拟合导出（IC(h)≈IC₀·e^(−λh)）。
   * null = 无法拟合（有效档 < 3 / IC 非正 / 拟合优度不达标），不是「无限长」。
   * Phase E 仅观测，不回写任何权重。
   */
  signalHalfLifeDays: number | null;
  /** 参与半衰期拟合的档数（有持有期且 IC 有值） */
  usableIcTiers: number;
  /**
   * 失败模式分布（P2-b）：按 completed 反思行的 `failure_modes` 列聚合。
   * 计数域与 `missLayer`（漏检链的机械归因层）不同，两者不得并排成可比数字。
   */
  failureModes: FailureModeBoard;
  /** 阶段 3 / 决-3：落地层级看板（结构层待裁清单）。与 failureModes 计数域不同，不得并列成可比数字。 */
  tiers: TierBoard;
  /**
   * 本次统计采用的**起算代际**下限（`HORIZON_BRANCH_GENERATION_FLOOR`）。
   * 语义是「这一代及之后才算同一套判据」，**不是**「等于当前代」—— 用等号的话每次换代
   * 分母都会被清零（见常量注释）。回传它是为了让面板说清「分母为什么变小」。
   */
  generationFloor?: number;
  /** 因「早于起算代」被排除出统计分母的样本数（旧代样本永久不可比） */
  excludedPreFloorGeneration?: number;
  /**
   * 代际未知（NULL）的样本数 —— P2-f 起这些样本**按收缩纳入**所有统计的分母
   * （不再整批排除；见 `filter_by_generation_floor`），本字段 = 其中代际未知的条数，
   * 供面板如实报出。
   */
  unknownGenerationSamples?: number;
}

/**
 * 逐档结局 2×2 的一行（B4，产端 `reflection_stats.rs::HorizonConfusionRow`）。
 *
 * 每一格 = 「公式这一档判对/判错 × LLM 这一档判对/判错」；两侧同一份 `returnPct`、
 * 同一个判定函数、同一个成熟窗口 ⇒ 四格可直接对比。
 * ⚠ `llmUndecided` **不进任何一格**：LLM 缺席 / 窗口未满 / 动作非方向性（观望、持有）
 * 都不构成「LLM 判错」；把它折进 `bothWrong`/`formulaOnly` 会让 LLM 胜率凭空变差。
 * ⚠ `sufficient === false` ⇒ 四格只是计数、不是结论（门槛 = 产端 `MIN_SAMPLE`）。
 */
export interface HorizonConfusionRow {
  /** 档位键（snake，与 `horizonResults` 的键族一致） */
  tier: string;
  /** 档位中文名（与 `HorizonAgreement.tierLabel` 同口径；显示名仍走本项目 i18n） */
  tierLabel: string;
  bothCorrect: number;
  formulaOnly: number;
  llmOnly: number;
  bothWrong: number;
  llmUndecided: number;
  /** 四格合计 = 这档 2×2 的分母 */
  judged: number;
  /** 该档三维分均值（只收有值的行）；一条都没有 → null（不是 0 分） */
  avgHorizonAgreement: number | null;
  sufficient: boolean;
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
  /**
   * B3：该档 **LLM 侧**的三个键，与公式侧（`decision`/`evaluation`）同层并列 ——
   * 产端 = `reflection.rs::build_horizon_results_json`，落库列 = `strategy_performance.{llm_action,llm_was_correct,horizon_agreement}`。
   * 三者的缺席语义各不同，都不要当 0 读：`llmAction=null` = 该档 LLM 没给（或该记录产自逐档契约上线前）；
   * `llmWasCorrect=null` = 窗口未满**或**该动作不是方向判断（观望/持有）；
   * `horizonAgreement=null` = 该档无从比对。⚠ 它是**三维 65 制**归一分，与主档五维分不同计数域。
   */
  llmAction?: string | null;
  llmWasCorrect?: 0 | 1 | null;
  horizonAgreement?: number | null;
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
