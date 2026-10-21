// i18n-exempt: 股票分析运行时工具函数与报告模板生成，NLP/技术字符串，非用户可见 UI 文案。
/**
 * stock-analysis 运行时工具函数
 *
 * 与 @/types/stock-analysis 中的纯类型定义分离，避免类型文件混合实现。
 * 保留 @/types/stock-analysis 作为向后兼容的 re-export 入口。
 */

// ── 证据质量驱动权重 (P0-1) ──

import { invoke } from "@/lib/invoke";
import type { CSSProperties } from "react";

/**
 * 快速分析工作流模板 id（Jev 判定链）—— 前端**唯一**定义。
 *
 * 与后端 `stock_analysis_setup::seed_stock_analysis::FAST_TEMPLATE_ID` 必须逐字一致：
 * ① 它是 `workflow_templates.id` 的查找键，写错会直接报「模板不存在」；
 * ② 分析记录落库后，`stock_analyses.template_id` 存的就是这个值，历史列表据此
 *    区分「快速链 vs 完整链」并打标识。
 */
export const FAST_TEMPLATE_ID = "stock-analysis-fast";

/** 市场环境信息（与后端 EvidenceWeightRequest 对应） */
export interface MarketRegimeInfo {
  regime: string;
  confidence: number;
  volatility: string;
  description: string;
  volatilityPct?: number | null;
  consecutiveUp?: number;
  consecutiveDown?: number;
}

/** 分析师输入 */
export interface AnalystInput {
  analystId: string;
  reportText?: string | null;
  stance?: string | null;
  bullScore?: number | null;
  bearScore?: number | null;
  positionPct?: number | null;
}

/** 证据权重计算请求 */
export interface EvidenceWeightRequest {
  marketRegime: MarketRegimeInfo;
  timeHorizon: string;
  analysts: AnalystInput[];
  historicalWeights?: Record<string, number> | null;
}

/** 分析师权重详情 */
export interface AnalystWeight {
  analystId: string;
  domain: string;
  horizonWeight: number;
  regimeModifier: number;
  historyModifier: number;
  finalWeight: number;
  stanceDirection: string;
  stanceConfidence: number;
}

/** 共识结果 */
export interface EvidenceConsensus {
  bullishScore: number;
  bearishScore: number;
  neutralScore: number;
  totalWeight: number;
  netScore: number;
  consensus: string;
  confidence: number;
}

/** HOLD 门控结果 */
export interface HoldGateResult {
  holdAllowed: boolean;
  reason: string;
  technicalHasTrend: boolean;
  moneyflowHasDirection: boolean;
  fundamentalHasCatalyst: boolean;
  suggestedAction: string;
}

/** 完整证据权重报告 */
export interface EvidenceWeightReport {
  marketRegime: MarketRegimeInfo;
  timeHorizon: string;
  analystWeights: AnalystWeight[];
  consensus: EvidenceConsensus;
  holdGate: HoldGateResult;
  recommendedAction: string;
  recommendedPositionPct: number;
  overallConfidence: number;
}

/** 调用后端证据质量驱动权重计算 */
export async function computeEvidenceWeights(
  request: EvidenceWeightRequest,
): Promise<EvidenceWeightReport> {
  return invoke<EvidenceWeightReport>("compute_evidence_weights", { request });
}

// ── 枚举常量 ──

/** 股票操作动作枚举 — 内部统一用英文标识，展示时通过 i18n 翻译 */
export const StockAction = {
  BUY: "BUY",
  INCREASE: "INCREASE",
  HOLD: "HOLD",
  REDUCE: "REDUCE",
  SELL: "SELL",
  WAIT: "WAIT",
  /** 有决策数据但无法判断方向（解析失败 / 证据不足） */
  UNCERTAIN: "UNCERTAIN",
  /** 决策数据本身缺失（后端显式哨兵 `UNAVAILABLE`，与「不确定」不同：后者是判断结论） */
  UNAVAILABLE: "UNAVAILABLE",
} as const;

export type StockActionType = (typeof StockAction)[keyof typeof StockAction];

/**
 * action 值域权威表（中英文**严格全等**匹配，仅容错首尾空白）。
 *
 * ⚠️ 本表只承载「操作档位」语义，**不得**加入方向词（看多 / 看空 / 中性）：
 * 方向词属于 verdict 空间，混入会让「增持」「观望」两档被静默吞掉
 * （`"看多"→BUY` 凭空吃掉 INCREASE 表达）。verdict → action 的降维转换
 * 走 `directionToAction`，两张表职责分离。
 *
 * 也不得加入带空白/换行的变体键 —— 严格匹配已用 `trim()` 覆盖该场景。
 */
export const STOCK_ACTION_LABELS: Record<string, StockActionType> = {
  "买入": StockAction.BUY,
  "增持": StockAction.INCREASE,
  "持有": StockAction.HOLD,
  "减持": StockAction.REDUCE,
  "卖出": StockAction.SELL,
  "观望": StockAction.WAIT,
  "等待": StockAction.WAIT,
  "减仓": StockAction.REDUCE,
  "加仓": StockAction.INCREASE,
  "不确定": StockAction.UNCERTAIN,
  "无法判断": StockAction.UNCERTAIN,
  "数据缺失": StockAction.UNAVAILABLE,
  // ── 兼容 dashboard_report 的「6 档中文」值域（强烈买入…卖出）──
  // 该值域由 `harness/dashboard_report.rs` 产出（`report.action = "强烈买入"`），
  // 与 portfolio-mgr 的 6 档是**两套不同定义**（前者无「观望」档、有「强烈买入」）。
  // 收敛前先把这两个值纳入权威表，否则严格解析会把它们降级为 UNCERTAIN。
  "强烈买入": StockAction.BUY,
  "强烈卖出": StockAction.SELL,
};

/** 股票风险等级枚举 */
export const StockRiskLevel = {
  LOW: "LOW",
  MID: "MID",
  HIGH: "HIGH",
  EXTREME: "EXTREME",
  /**
   * 「无法评级」——不是风险高低的一档，而是压根评不出来。
   * 后端 `compute_risk_level` 在零持仓时返回中文哨兵值「无持仓」，此前它不在值域表内，
   * 会被兜底成 MID ⇒ 空仓在界面上显示成「中风险」（把缺席伪装成正常结论）。
   * ⚠ 本值刻意不进 `decision-agreement` 的 `RISK_RANK`：它没有风险高低序可比。
   */
  UNRATED: "UNRATED",
} as const;

export type StockRiskLevelType = (typeof StockRiskLevel)[keyof typeof StockRiskLevel];

/** 中文风险等级标签映射 */
export const STOCK_RISK_LABELS: Record<string, StockRiskLevelType> = {
  "低风险": StockRiskLevel.LOW,
  "中风险": StockRiskLevel.MID,
  "高风险": StockRiskLevel.HIGH,
  "极高": StockRiskLevel.EXTREME,
  "低": StockRiskLevel.LOW,
  "中": StockRiskLevel.MID,
  "高": StockRiskLevel.HIGH,
};

// ── 解析函数 ──

/**
 * 严格值域解析器 —— 只在输入**本身就是 action 值**时返回结果，否则 null。
 *
 * 接受：英文枚举（大小写不敏感）、中文标签（首尾空白容错）。
 * 不接受：自由文本、句子、带修饰词的短语。
 */
export function parseActionStrict(raw: unknown): StockActionType | null {
  if (typeof raw !== "string") { return null; }
  const clean = raw.trim();
  if (clean === "") { return null; }
  const upper = clean.toUpperCase();
  if (upper in StockAction) { return upper as StockActionType; }
  return STOCK_ACTION_LABELS[clean] ?? null;
}

/**
 * 解析股票操作动作（兼容英文/中文/大小写/前后空格）—— 严格值域解析。
 *
 * ⚠️ 不要改回 `raw.includes(label)` 式的全文扫描：历史实现在自由文本上会
 * **按对象键序**命中（`买入` 键序先于 `看空`、`减持` 先于 `看多`），于是
 * 「方向:看多。…风险点：大股东减持 5%」（trader.md 自带示例文本）被解析成
 * REDUCE —— 只要句中出现否定词或风险词，方向即反转（已实证 3/5 用例不符）。
 * 需要从自由文本推导方向请用 `parseDirectionFromText`。
 *
 * 未识别返回 `UNCERTAIN`（不是 `WAIT`）：解析失败属于「无法判断」，
 * 不是业务语义上的「观望」。把解析失败伪装成「观望」等于把缺失当结论。
 */
export function parseAction(raw: unknown): StockActionType {
  return parseActionStrict(raw) ?? StockAction.UNCERTAIN;
}

/** 方向结论（verdict 空间三值） */
export type DirectionVerdict = "看多" | "看空" | "中性";

/**
 * 从自由文本中读取**显式方向标记** —— 只认 `方向:` / `verdict:` / `stance:`
 * 这类结构化前缀，命中即返回，**不做全文关键词扫描**（全文扫描是方向反转的根因）。
 *
 * 例：`方向:看多。…风险点：大股东减持 5%` → "看多"（不会被「减持」反转）
 *     无标记文本 → null（由调用方决定兜底，不要臆造方向）
 */
export function parseDirectionFromText(text: unknown): DirectionVerdict | null {
  if (typeof text !== "string") { return null; }
  const s = text.trim();
  if (s === "") { return null; }
  // `方向：看多` / `方向: 看多` / `"verdict":"bearish"` / `verdict = neutral`
  const m = s.match(
    /(?:方向|verdict|stance|direction)\s*["']?\s*[:：=]\s*["']?\s*(看多|看空|中性|bullish|bearish|neutral)/i,
  );
  if (!m) { return null; }
  const v = m[1].toLowerCase();
  if (v === "看多" || v === "bullish") { return "看多"; }
  if (v === "看空" || v === "bearish") { return "看空"; }
  return "中性";
}

/**
 * verdict（方向结论）→ action（操作档位）的降维映射。
 *
 * 依据 `trader.md` 的一致性表：「买入 / 增持 → 看多」「卖出 / 减持 → 看空」
 * 「持有 / 观望 → 中性」。反向映射是有损的（一对二），因此**只在没有结构化
 * action 字段时**作为兜底使用，且必须让调用方知道这是一次降维。
 */
export function directionToAction(verdict: unknown): StockActionType | null {
  if (typeof verdict !== "string") { return null; }
  const v = verdict.trim().toLowerCase();
  switch (true) {
    case v === "看多" || v === "bullish" || v === "buy":
      return StockAction.BUY;
    case v === "看空" || v === "bearish" || v === "sell":
      return StockAction.SELL;
    case v === "中性" || v === "neutral" || v === "hold":
      return StockAction.HOLD;
    default:
      return null;
  }
}

/**
 * 多空方向关键词判据 —— 分析师 / 辩论报告自由文本的**单一真相源**。
 *
 * 用途：给定 `verdict` / `stance` / `direction` 这类**自由文本**（如「看多」「偏空」
 * 「bullish」「买入」），判定其多空方向。**不是** action 值域解析 ——
 * action 值请走 `parseActionStrict`；方向词表（看多/看空/中性）请走
 * `parseDirectionFromText`（它只认显式 `方向:` 标记）。
 *
 * ⚠️ 为什么必须是单点（2026-09-21 修复）
 *   同一判据此前在 4 处各写一份，且**值域各不相同**：
 *     - `AnalystReportGrid` 取多空分数兜底：`看多|买入|增持|做多|看涨|bull`
 *     - `AnalystReportGrid` 表格「判断」列：`看多|bull|偏多|买入|增持|正面`
 *     - `AnalystReportCard` stance→verdict：仅 `bull|看多`
 *     - `AnalystReportCard` 徽标配色 / i18n key（同函数内**两份**，相隔 4 行）：仅 `看多|bull`
 *   ⇒ 同一份 `verdict: "买入"` 的研报：Grid 判为看多（红），Card 两份判据都不命中
 *   ⇒ 徽标落到「中性」且染成灰色 —— 同一屏内两套结论。
 *   本表取上述四份值域的**并集**（`做多/看涨/偏多/正面` 与 `减持/负面` 等一并纳入）。
 *
 * 判定顺序：**先多后空**（与四份旧实现一致）。因此
 * 「看多，但需注意减持风险」判为看多 —— 该取舍是既有行为，此处显式固定；
 * 需要更精细的双向判定请另外设计，不要在此叠加分支。
 */
export const DIRECTION_BULL_PATTERN = /看多|做多|看涨|偏多|买入|增持|加仓|正面|bullish|bull/i;
export const DIRECTION_BEAR_PATTERN = /看空|做空|看跌|偏空|卖出|减持|减仓|负面|bearish|bear/i;

/**
 * 自由文本 → 多空方向。无法判定返回 `null`（**不臆造方向**）。
 *
 * @returns `"bull"` | `"bear"` | `null`
 */
export function classifyDirectionText(text: unknown): "bull" | "bear" | null {
  if (typeof text !== "string") { return null; }
  const s = text.trim();
  if (s === "") { return null; }
  if (DIRECTION_BULL_PATTERN.test(s)) { return "bull"; }
  if (DIRECTION_BEAR_PATTERN.test(s)) { return "bear"; }
  return null;
}

/**
 * 从 LLM 决策对象推导 action —— 按「结构化程度」降序取第一个可用来源：
 *   ① 结构化 `action` 字段（严格值域）
 *   ② 结构化 `verdict` / `stance` / `direction` 字段
 *   ③ `reasoning` 里的显式 `方向:` 标记
 * 全部不可用返回 null —— 调用方应保留原 action 或标记缺失，**不得臆造方向**。
 */
export function deriveActionFromLlmDecision(
  llmRaw: Record<string, unknown> | null | undefined,
): StockActionType | null {
  if (!llmRaw) { return null; }
  const direct = parseActionStrict(llmRaw.action);
  if (direct) { return direct; }
  const fromVerdict = directionToAction(llmRaw.verdict ?? llmRaw.stance ?? llmRaw.direction);
  if (fromVerdict) { return fromVerdict; }
  return directionToAction(parseDirectionFromText(llmRaw.reasoning));
}

/**
 * action → 交易方向（下单表单用）。只有方向明确的档位可映射：
 *   BUY / INCREASE → "buy"，SELL / REDUCE → "sell"
 *   HOLD / WAIT / UNCERTAIN / UNAVAILABLE → null
 *
 * ⚠️ 不要写成 `action === SELL ? "sell" : "buy"`：那会把「观望 / 不确定 / 数据缺失」
 * 一并当成**买入方向**填进表单（TradePanel 的历史缺陷，会误导真实下单录入）。
 */
export function actionToDirection(action: unknown): "buy" | "sell" | null {
  switch (parseAction(action)) {
    case StockAction.BUY:
    case StockAction.INCREASE:
      return "buy";
    case StockAction.SELL:
    case StockAction.REDUCE:
      return "sell";
    default:
      return null;
  }
}

// ── 持仓状态轴（与 action 正交，P1-2） ──

/**
 * 持仓状态枚举 —— 与 `action`（方向强度）**正交**的第二轴。
 *
 * 背景：「持有 vs 观望」此前不是两个语义，而是同一中性档因仓位有无被单向互改的
 * 两个名字（后端公式里仓位 > 0 时「观望」升级为「持有」、仓位 ≤ 0 时
 * 「买入/增持/持有」降级为「观望」）。两轴混在一列后，一条「观望」记录
 * 无法区分「判断中性」与「有方向但不可持仓」。后端自 v228 起单独落库本轴。
 */
export const PositionState = {
  /** 空仓（当前不持有该标的） */
  EMPTY: "EMPTY",
  /** 建仓中（本次为买入 / 增持） */
  OPENING: "OPENING",
  /** 持有中（有仓位且不打算变动） */
  HOLDING: "HOLDING",
  /** 减仓中（本次为减持 / 卖出） */
  TRIMMING: "TRIMMING",
} as const;

export type PositionStateType = (typeof PositionState)[keyof typeof PositionState];

/**
 * 解析持仓状态。`null` / 空串 / 未识别 → `null`。
 *
 * ⚠️ `null` 的语义是「该记录产生于本字段引入之前，采集时点无此信息」，
 * **不得**读成 `EMPTY` —— 那会把「不知道」当成「空仓」。
 */
export function parsePositionState(raw: unknown): PositionStateType | null {
  if (typeof raw !== "string") { return null; }
  const clean = raw.trim().toUpperCase();
  return clean in PositionState ? (clean as PositionStateType) : null;
}

/**
 * 把后端决策解析为**展示档**。
 *
 * ⚠️ 自 2026-09-22 起展示档 = 方向档，**不再按仓位派生**（本函数现为恒等变换）。
 *
 * 历史（已被本次修复废除）：V76 起本函数按 `(action, positionState, positionPct)`
 * 派生 —— 中性档（HOLD/WAIT）空仓 ⇒ 观望、有仓位 ⇒ 持有（见 `AUDIT-300642-run-variance-2026-09-22.md`）。
 * 该派生是**循环判据**：`positionState` 来自**本次决策刚算出的建议仓位**，用它反推
 * 本次决策的展示名，于是 LLM trader 一个措辞变化（verdict 看空 → 中性）经
 * 「试探仓守卫 → position_pct 0%→3% → positionState EMPTY→HOLDING」三级旁路，
 * 把同一个后端档「观望」翻成了「持有」——同日两次分析给出两个结论，而
 * `action` 两次完全相同（都是「观望」）。结论名必须只由确定性的后验阶梯决定。
 *
 * 现规则：一律返回 `parseAction(action)`。`positionState` / `positionPct` 仍是
 * 独立可展示的量（见 `positionState` 字段），但**不参与**方向档名的判定。
 * 参数保留是为了兼容既有调用点；判据已收敛到 `portfolio-mgr.rhai` 的
 * `final_action`，改一侧必须同步另一侧。
 *
 * ⚠️ 用户**真实持仓**不在此判据内。若将来要按真实持仓区分「持有 / 观望」文案，
 * 须另接 `portfolio_holdings`，不要复用本次建议仓位（复用即回到循环判据）。
 */
export function resolveDisplayAction(
  action: unknown,
  _positionState?: unknown,
  _positionPct?: number | null,
): StockActionType {
  return parseAction(action);
}

/**
 * 决策档位的**规范中文名** —— 与 `portfolio-mgr.rhai` 实际输出的 6 档一一对应。
 *
 * ⚠️ 与 `STOCK_ACTION_LABELS` 方向相反：那张表是「中文 → 枚举」（含别名，多对一），
 * 本表是「枚举 → 唯一中文名」（一对一）。**不要**用 `Object.keys(STOCK_ACTION_LABELS)`
 * 反查 —— 别名（"等待" / "减仓" / "加仓"）会让结果取决于键序，属不可判定。
 * 用途仅为「改写后端中文文本」对账，**不是** UI 展示文案（UI 走 `getActionTKey` + i18n）。
 */
export const STOCK_ACTION_CANONICAL_LABELS: Record<StockActionType, string> = {
  [StockAction.BUY]: "买入",
  [StockAction.INCREASE]: "增持",
  [StockAction.HOLD]: "持有",
  [StockAction.REDUCE]: "减持",
  [StockAction.SELL]: "卖出",
  [StockAction.WAIT]: "观望",
  [StockAction.UNCERTAIN]: "不确定",
  [StockAction.UNAVAILABLE]: "数据缺失",
};

/** `portfolio-mgr.rhai` reasoning 的结论前缀 —— 唯一允许改写的面，勿扩到全文 */
const REASONING_DECISION_PREFIX = /^决策=(\S+)/;

/**
 * 把 `portfolio-mgr` 的 `reasoning` 开头的 `决策=<档名>` 对齐到**展示档**。
 *
 * 背景（2026-09-21 实证）：V76 把「持有 / 观望」拆成正交两轴后，展示层统一由
 * `resolveDisplayAction` 派生（中性档 + 空仓 ⇒ 观望）。但 `portfolio-mgr.rhai` 拼
 * reasoning 时写的仍是**原始方向档** `final_action` ⇒ **同一条落库记录**里挂角 Tag
 * 显示「观望」、结论文本却写「决策=持有」，用户直接质问二者矛盾。
 * 后端已同步改为展示档（新记录一致），本函数让**修复前落库的历史行**也对齐。
 *
 * ⚠️ 只改开头**第一个** `决策=X` 这一个 token（见 `REASONING_DECISION_PREFIX`）。
 *    同段文本里 `| ⚠️极高风险风控否决:持有→观望`、`action已从X修正为Y` 是
 *    **档位迁移留痕**，用原始档名才表达得对，一律不得改写。
 * ⚠️ 前缀缺失（LLM 侧 reasoning / 其它来源文本）或**档名本身识别不出** ⇒ 原样返回，
 *    不臆造结论（识别失败时改写等于把「看不懂」当「已确认」）。
 */
export function alignReasoningDecisionLabel(
  reasoning: string,
  displayAction: StockActionType,
): string {
  if (!reasoning) { return reasoning; }
  const m = REASONING_DECISION_PREFIX.exec(reasoning);
  if (!m) { return reasoning; }
  const written = parseActionStrict(m[1]);
  // 识别不出 / 本来就一致 ⇒ 零改动（幂等）
  if (written === null || written === displayAction) { return reasoning; }
  const label = STOCK_ACTION_CANONICAL_LABELS[displayAction];
  if (!label) { return reasoning; }
  return `决策=${label}${reasoning.slice(m[0].length)}`;
}

/** 解析股票风险等级（兼容英文/中文/大小写） */
export function parseRiskLevel(raw: unknown): StockRiskLevelType {
  if (raw == null || (typeof raw !== "string" && typeof raw !== "number")) {
    return StockRiskLevel.MID;
  }
  const clean = String(raw).trim().toUpperCase();
  if (clean in StockRiskLevel) { return clean as StockRiskLevelType; }
  // 后端 compute_risk_level 的两个非标准档名，必须走精确匹配：「无持仓」不是风险高低的
  // 一档（评不出来），「中等风险」此前只靠子串表里的「中」巧合命中 MID。放在末尾 MID
  // 兜底之前判定，否则「无持仓」会被显示成「中风险」。
  if (["无持仓", "无评级", "无法评级"].includes(clean)) { return StockRiskLevel.UNRATED; }
  if (["低", "低风险", "L"].includes(clean)) { return StockRiskLevel.LOW; }
  if (["中", "中风险", "中等风险", "M"].includes(clean)) { return StockRiskLevel.MID; }
  if (["高", "高风险", "H"].includes(clean)) { return StockRiskLevel.HIGH; }
  if (["极高", "极高风险", "E"].includes(clean)) { return StockRiskLevel.EXTREME; }
  for (const [label, level] of Object.entries(STOCK_RISK_LABELS)) {
    if (String(raw).includes(label)) { return level; }
  }
  console.warn("[parseRiskLevel] 未匹配的风险等级输入:", raw, "→ 默认返回 MID");
  return StockRiskLevel.MID;
}

// ── 颜色 & i18n 键 ──

/**
 * 返回 Ant Design Tag 预设色名（red/green/blue/orange 等）。
 * 用于详情页等 Ant Design 主题环境下的 Tag color 属性。
 * 列表/下拉面板请使用 getActionTagStyle（CSS 变量样式）。
 */
export function getActionColor(action: string): string {
  // P1-6(2026-09-14): 原先直接 `switch (action)` **不解析** —— 传入中文「买入」或小写
  // `buy` 全部落到 `default`（无配色）。同文件的 getActionTKey 一直是解析后再查表的，
  // 三者判据不一致，导致「标签显示了颜色却没有」。现统一走 parseAction。
  switch (parseAction(action)) {
    case StockAction.BUY:
    case StockAction.INCREASE:
      return "red";
    case StockAction.SELL:
    case StockAction.REDUCE:
      return "green";
    case StockAction.HOLD:
      return "blue";
    case StockAction.WAIT:
      return "orange";
    case StockAction.UNCERTAIN:
    case StockAction.UNAVAILABLE:
    default:
      return "default";
  }
}

/**
 * 返回决策 Tag 的内联样式，使用主题 CSS 变量（非硬编码色名）。
 * 遵循 A 股涨跌色习惯：红=买入/增持，绿=卖出/减持。
 * 背景使用半透明底色 + 实色文字 + 同色细边框，深色/浅色模式自动适配。
 * 适用于自定义面板/下拉列表等非标准 Ant Design 容器。
 */
export function getActionTagStyle(action: string): CSSProperties {
  const base: CSSProperties = {
    margin: 0,
    fontSize: 10,
    lineHeight: "16px",
    padding: "0 5px",
    borderRadius: 3,
    border: "1px solid",
  };
  let fg: string;
  let bg: string;
  let bd: string;
  // P1-6(2026-09-14): 同 getActionColor —— 原 switch 不解析原串，中文/小写值无配色。
  switch (parseAction(action)) {
    case StockAction.BUY:
    case StockAction.INCREASE:
      fg = "var(--color-danger)";
      bg = "color-mix(in oklch, var(--color-danger) 14%, transparent)";
      bd = "color-mix(in oklch, var(--color-danger) 30%, transparent)";
      break;
    case StockAction.SELL:
    case StockAction.REDUCE:
      fg = "var(--color-success)";
      bg = "color-mix(in oklch, var(--color-success) 14%, transparent)";
      bd = "color-mix(in oklch, var(--color-success) 30%, transparent)";
      break;
    case StockAction.HOLD:
      fg = "var(--color-info)";
      bg = "color-mix(in oklch, var(--color-info) 14%, transparent)";
      bd = "color-mix(in oklch, var(--color-info) 30%, transparent)";
      break;
    case StockAction.WAIT:
      fg = "var(--color-warning)";
      bg = "color-mix(in oklch, var(--color-warning) 14%, transparent)";
      bd = "color-mix(in oklch, var(--color-warning) 30%, transparent)";
      break;
    case StockAction.UNCERTAIN:
    case StockAction.UNAVAILABLE:
    default:
      fg = "var(--color-t-tertiary)";
      bg = "color-mix(in oklch, var(--color-t-tertiary) 12%, transparent)";
      bd = "color-mix(in oklch, var(--color-t-tertiary) 25%, transparent)";
      break;
  }
  return { ...base, color: fg, background: bg, borderColor: bd };
}

export function getActionTKey(action: string): string {
  const normalized = parseAction(action); // 中文"卖出"→"SELL"，统一后再查表
  switch (normalized) {
    case StockAction.BUY:
      return "stockAnalysis.actionBuy";
    case StockAction.INCREASE:
      return "stockAnalysis.actionIncrease";
    case StockAction.HOLD:
      return "stockAnalysis.actionHold";
    case StockAction.REDUCE:
      return "stockAnalysis.actionReduce";
    case StockAction.SELL:
      return "stockAnalysis.actionSell";
    case StockAction.WAIT:
      return "stockAnalysis.actionWait";
    case StockAction.UNAVAILABLE:
      return "stockAnalysis.actionUnavailable";
    default:
      return "stockAnalysis.actionUncertain";
  }
}

export function getRiskColor(level: string): string {
  // 2026-09-21: 与相邻的 `getRiskTKey` 统一判据 —— 此前本函数直接 `switch (level)` **不解析**，
  //   而后端 `riskLevel` / `agreementBreakdown.formulaRiskLevel` 等落库值是**原始中文**
  //   （「高风险」「极高风险」，见 portfolio-mgr.rhai 的 overall_risk 赋值处），
  //   于是中文输入全部落到 `default` ⇒ 灰字，而同一入参在 `getRiskTKey` 上却能正确出文案
  //   —— 「标签显示了文字却没有颜色」。
  // 同型先例：本文件 `getActionColor`（见其上方 P1-6 注释）在 2026-09-14 已按此修过 action 侧，
  //   当时**漏了 risk 侧**，故此处补齐。
  // ⚠️ 行为变化：无法识别的输入由 `var(--muted)` 变为 MID 档（`parseRiskLevel` 内部会
  //   `console.warn` 后兜底 MID）。这是**有意**与 `getRiskTKey` 的 `riskMid` 兜底对齐 ——
  //   两者必须同判据，否则又回到「文字一档、颜色另一档」。
  switch (parseRiskLevel(level)) {
    case StockRiskLevel.LOW:
      return "var(--sa-green)";
    case StockRiskLevel.MID:
      return "var(--sa-amber)";
    case StockRiskLevel.HIGH:
      return "var(--sa-red)";
    case StockRiskLevel.EXTREME:
      return "var(--sa-extreme)";
    default:
      return "var(--muted)";
  }
}

export function getRiskTKey(level: string): string {
  const normalized = parseRiskLevel(level); // "高风险"→"HIGH"，统一后再查表
  const map: Record<string, string> = {
    [StockRiskLevel.HIGH]: "stockAnalysis.riskHigh",
    [StockRiskLevel.MID]: "stockAnalysis.riskMid",
    [StockRiskLevel.LOW]: "stockAnalysis.riskLow",
    [StockRiskLevel.EXTREME]: "stockAnalysis.riskExtreme",
    // 无法评级 ≠ 中风险：复用既有的「暂无持仓」文案，不新增键
    [StockRiskLevel.UNRATED]: "stockAnalysis.noHoldings",
  };
  return map[normalized] ?? "stockAnalysis.riskMid";
}

// ── 集中度算法的风险档（与上面的决策链档位是两套独立体系）──
//
// 后端 `portfolio_monitor::compute_risk_level` 按持仓集中度分成「低风险 / 中等风险 /
// 中高风险 / 高风险」四档、零持仓时返回「无持仓」，与决策链（rhai `overall_risk`）的
// 「低 / 中 / 高 / 极高」不是一套划分。硬把前者的值喂进 `parseRiskLevel` 会串味：
// 「中高风险」在决策链的 `contains` 判据里归「中」（后端 decision.rs），在前端的子串
// 表里却归「高」——同一个词两个结论。故这两档各用各自的呈现，判据也各留一份。

/** 集中度风险档 → i18n 键。 */
export function portfolioRiskTKey(raw: unknown): string {
  const map: Record<string, string> = {
    "低风险": "stockAnalysis.riskLow",
    "中等风险": "stockAnalysis.riskMid",
    "中高风险": "stockAnalysis.riskMidHigh",
    "高风险": "stockAnalysis.riskHigh",
    "无持仓": "stockAnalysis.noHoldings",
  };
  return map[String(raw ?? "").trim()] ?? "stockAnalysis.riskMid";
}

/** 集中度风险档 → 颜色变量。判据与 `portfolioRiskTKey` 同源，避免文字一档、颜色另一档。 */
export function portfolioRiskColor(raw: unknown): string {
  switch (String(raw ?? "").trim()) {
    case "低风险":
      return "var(--sa-green)";
    case "中高风险":
    case "高风险":
      return "var(--sa-red)";
    case "无持仓":
      return "var(--muted)";
    default:
      // 「中等风险」及未识别输入
      return "var(--sa-amber)";
  }
}

// ── 决策一致性分（0-100）的展示分档 ──
//
// 2026-09-21: 收敛为单一真相源。此前**四处各存一份档位表**，且高档阈值已漂移：
//   `dual-view/DecisionComparisonPanel.tsx` 用 **80**，另三处
//   （`dual-view/CompactDecisionComparison.tsx`、`DecisionBanner.tsx`、
//   `EvolutionDriftPanel.tsx`）用 **60** ⇒ 分数落在 [60, 80) 时两个面板会给出
//   **相反的颜色**（一个绿、一个琥珀），用户在同一屏看到矛盾结论。
//
// 取 60 为高档阈值：三处使用它（含主决策卡 `DecisionBanner`），且它与后端的
// `compute_decision_agreement` 6 维度满分 100 的刻度一致（「60 分以上算基本一致」）。
// ⚠️ 若产品意图是 80，改这一个常量即可 —— 不要再改回组件内联。
export const AGREEMENT_HIGH_THRESHOLD = 60;
export const AGREEMENT_MID_THRESHOLD = 40;

/** 一致性分 → 分档（high / mid / low） */
export function agreementTier(score: number): "high" | "mid" | "low" {
  if (score >= AGREEMENT_HIGH_THRESHOLD) { return "high"; }
  if (score >= AGREEMENT_MID_THRESHOLD) { return "mid"; }
  return "low";
}

/** 一致性分 → 前景色（绿=高 / 琥珀=中 / 红=低） */
export function agreementColor(score: number): string {
  const tier = agreementTier(score);
  return tier === "high" ? "#10b981" : tier === "mid" ? "#f59e0b" : "#ef4444";
}

/** 一致性分 → 半透明底色（与 `agreementColor` 严格同档，不得各写一份阈值） */
export function agreementBgColor(score: number): string {
  const tier = agreementTier(score);
  return tier === "high"
    ? "rgba(16, 185, 129, 0.12)"
    : tier === "mid"
    ? "rgba(245, 158, 11, 0.12)"
    : "rgba(239, 68, 68, 0.12)";
}

// ── `dashboard_report` 专属值域（与 portfolio-mgr 的 6 档是**两套定义**）──
//
// 根因（2026-09-21）：`harness/dashboard_report.rs` 的 action 是
//   强烈买入 / 买入 / 增持 / 持有 / 减持 / 卖出
// 与 portfolio-mgr 的 6 档（**无「强烈买入」、有「观望」**）并非同一张表。
// `DashboardReportPreview` 长期把 `report.action` 等字段**裸渲染**，靠文件头一行
// `// i18n-exempt` 让硬编码扫描器跳过整个文件 —— 于是 action / trend / severity /
// category / direction+timeline 这 5 处裸展示在 zh-CN 下看不出问题，切到 en-US
// 等语言就整片中文乱入。
//
// 本区块把这 5 个值域的「值 → i18n key」收敛成单点。**不要**再在组件里写值域
// switch：文件本地的 `actionColor` 之类的副本已因此在值域变动时静默走 default。

/**
 * `dashboard_report.action` 中**超出** portfolio-mgr 6 档的两档强度档。
 *
 * `parseActionStrict` 已把「强烈买入 / 强烈卖出」收敛到 `BUY` / `SELL`
 * （见 `STOCK_ACTION_LABELS` 尾部注释），但**收敛会丢掉「强烈」这个强度信息**
 * ⇒ 展示层不能直接复用 `getActionTKey`，否则 UI 上「强烈买入」被降级成「买入」。
 */
const DASHBOARD_ACTION_TKEY_OVERRIDES: Record<string, string> = {
  "强烈买入": "stockAnalysis.dashboard.actionStrongBuy",
  "强烈卖出": "stockAnalysis.dashboard.actionStrongSell",
};

/** 值域查表的公共形态：非字符串 / 未知值一律返回 null（由调用方决定兜底） */
function lookupTKey(map: Record<string, string>, raw: unknown): string | null {
  if (typeof raw !== "string") { return null; }
  return map[raw.trim()] ?? null;
}

/** `dashboard_report.trend` 值域：看多 / 看空 / 震荡（**不是**「中性」） */
const DASHBOARD_TREND_TKEYS: Record<string, string> = {
  "看多": "stockAnalysis.dashboard.trendBullish",
  "看空": "stockAnalysis.dashboard.trendBearish",
  "震荡": "stockAnalysis.dashboard.trendSideways",
};

/** `RiskAlert.severity` 值域：低 / 中 / 高（与 `riskLevel` 的「低风险」措辞不同，故独立成表） */
const DASHBOARD_SEVERITY_TKEYS: Record<string, string> = {
  "低": "stockAnalysis.dashboard.severityLow",
  "中": "stockAnalysis.dashboard.severityMid",
  "高": "stockAnalysis.dashboard.severityHigh",
};

/** `ChecklistItem.category` 值域：入场 / 加仓 / 减仓 / 止损 / 止盈 */
const CHECKLIST_CATEGORY_TKEYS: Record<string, string> = {
  "入场": "stockAnalysis.dashboard.checklistEntry",
  "加仓": "stockAnalysis.dashboard.checklistAdd",
  "减仓": "stockAnalysis.dashboard.checklistReduce",
  "止损": "stockAnalysis.dashboard.checklistStopLoss",
  "止盈": "stockAnalysis.dashboard.checklistTakeProfit",
};

/** `Catalyst.direction` 值域：利好 / 利空 */
const CATALYST_DIRECTION_TKEYS: Record<string, string> = {
  "利好": "stockAnalysis.dashboard.catalystBullish",
  "利空": "stockAnalysis.dashboard.catalystBearish",
};

/** `Catalyst.timeline` 值域：短期 / 中期 / 长期 */
const CATALYST_TIMELINE_TKEYS: Record<string, string> = {
  "短期": "stockAnalysis.dashboard.catalystShortTerm",
  "中期": "stockAnalysis.dashboard.catalystMidTerm",
  "长期": "stockAnalysis.dashboard.catalystLongTerm",
};

/**
 * `dashboard_report.action` 的 i18n key —— **保留「强烈」强度**的展示层解析器。
 *
 * 非强度档完全复用 `getActionTKey`（同一批 UI 文案，避免「买入」在两处各存一份
 * 翻译而逐渐漂移）；仅「强烈买入 / 强烈卖出」走 dashboard 专属 key。
 * 解析不出（自由文本 / 脏数据）⇒ `actionUncertain`，与 `parseAction` 同判据，不臆造档位。
 */
export function getDashboardActionTKey(raw: unknown): string {
  if (typeof raw !== "string" || raw.trim() === "") {
    return "stockAnalysis.actionUnavailable";
  }
  const override = lookupTKey(DASHBOARD_ACTION_TKEY_OVERRIDES, raw);
  if (override !== null) { return override; }
  const kind = parseActionStrict(raw);
  if (kind === null) { return "stockAnalysis.actionUncertain"; }
  return getActionTKey(kind);
}

/** `dashboard_report.trend` 的 i18n key；未知值返回 null（调用方展示原文，不臆造） */
export function getDashboardTrendTKey(raw: unknown): string | null {
  return lookupTKey(DASHBOARD_TREND_TKEYS, raw);
}

/** `RiskAlert.severity` 的 i18n key；未知值返回 null */
export function getDashboardSeverityTKey(raw: unknown): string | null {
  return lookupTKey(DASHBOARD_SEVERITY_TKEYS, raw);
}

/** `ChecklistItem.category` 的 i18n key；未知值返回 null */
export function getChecklistCategoryTKey(raw: unknown): string | null {
  return lookupTKey(CHECKLIST_CATEGORY_TKEYS, raw);
}

/** `Catalyst.direction` 的 i18n key；未知值返回 null */
export function getCatalystDirectionTKey(raw: unknown): string | null {
  return lookupTKey(CATALYST_DIRECTION_TKEYS, raw);
}

/** `Catalyst.timeline` 的 i18n key；未知值返回 null */
export function getCatalystTimelineTKey(raw: unknown): string | null {
  return lookupTKey(CATALYST_TIMELINE_TKEYS, raw);
}

// ── 上述 dashboard 值域的**配色**（同样收敛到单点，组件内不得再写值域 switch）──
//
// ⚠️ 这两个 `action` 表**键集不同**（tKey 表只有 2 个强度档，其余档复用
// `getActionTKey`；配色表覆盖全部 6 档）⇒ 后端增删档位时**两张表都要看**。

/**
 * `dashboard_report.action` 的**强度渐变**配色。
 *
 * ⚠️ 与 `getActionColor()` 的分工：后者按「涨跌方向」二值着色（买入/增持 → red，
 * 卖出/减持 → green），同一方向内不再分强度；本表保留 dashboard 的**强度渐变**
 * （深红 → 橙红 → 橙 → 灰 → 绿 → 青），表达档位的连续强弱。
 * **不要把两者合并** —— 合并即丢失强度信息（视觉降级）。
 */
const DASHBOARD_ACTION_COLORS: Record<string, string> = {
  "强烈买入": "#f5222d",
  "买入": "#fa541c",
  "增持": "#fa8c16",
  "持有": "#8c8c8c",
  "减持": "#52c41a",
  "卖出": "#13c2c2",
};

const DASHBOARD_NEUTRAL_COLOR = "#8c8c8c";

/** `dashboard_report.action` 的配色；未知值取中性灰（不臆造方向色） */
export function getDashboardActionColor(raw: unknown): string {
  return lookupTKey(DASHBOARD_ACTION_COLORS, raw) ?? DASHBOARD_NEUTRAL_COLOR;
}

/** `dashboard_report.trend` 配色：看多红 / 看空绿（A 股涨跌色习惯），其余中性灰 */
const DASHBOARD_TREND_COLORS: Record<string, string> = {
  "看多": "#f5222d",
  "看空": "#52c41a",
};

export function getDashboardTrendColor(raw: unknown): string {
  return lookupTKey(DASHBOARD_TREND_COLORS, raw) ?? DASHBOARD_NEUTRAL_COLOR;
}

/** `RiskAlert.severity` 配色：Ant Design Tag 预设色名，未知值走 default */
const DASHBOARD_SEVERITY_COLORS: Record<string, string> = {
  "高": "red",
  "中": "orange",
  "低": "green",
};

export function getDashboardSeverityColor(raw: unknown): string {
  return lookupTKey(DASHBOARD_SEVERITY_COLORS, raw) ?? "default";
}

/**
 * `Catalyst.direction` 配色：利好红 / 其余绿（A 股涨跌色习惯）。
 *
 * ⚠️ 非「利好」一律按利空着色 —— 后端值域只有「利好 / 利空」二值，未知值由
 * `getCatalystDirectionTKey` 兜底显示原文，此处不额外造「未知」色。
 */
export function getCatalystDirectionColor(raw: unknown): string {
  const key = typeof raw === "string" ? raw.trim() : "";
  return key === "利好" ? "red" : "green";
}

/**
 * 自由文本「信号/方向」关键词 → Ant Design 色名。
 *
 * ⚠️ 配色约定（A 股）：**红=看多/买入，绿=看空/卖出** —— 与
 * [`getActionColor`] / [`getActionTagStyle`] 同源。
 *
 * 2026-09-21 修复：本函数此前返回**相反**配色（看多→green、看空→red，美式习惯），
 * 与同模块的 `getActionColor`（BUY→red）以及 `getActionTagStyle` 的文档注释
 * 「遵循 A 股涨跌色习惯：红=买入/增持，绿=卖出/减持」直接矛盾 ⇒ 同一屏里
 * 「看多」标签是绿的、「买入」标签是红的。相邻的 `getCatalystDirectionColor`
 * （利好→red）亦为 A 股口径。
 *
 * 关键词表本身不可直接复用时序枚举（入参是「买入信号 / 上涨趋势」这类自由文本，
 * 不是 action 值域），故此处保留关键词匹配，只把配色约定对齐权威函数。
 */
export function getSignalColor(signal: string): "green" | "red" | "blue" {
  const s = String(signal ?? "").trim().toLowerCase();
  if (s.includes("买") || s.includes("多") || s.includes("涨") || s.includes("牛")) { return "red"; }
  if (s.includes("卖") || s.includes("空") || s.includes("跌") || s.includes("熊")) { return "green"; }
  return "blue";
}

/** 尝试解析报告字符串为 JSON 对象 */
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

// ── 情感分析 ──

/**
 * 自由文本 → 多空情绪（`sentiment` 二值；无法判定返回 `null`）。
 *
 * `classifySentiment` 的 **VERDICT 分支**与 **stance 分支共用本函数** ——
 * 二者此前各写一份内联正则，值域互不相同（`多头/利好/超配/扫货` 只在一份里，
 * `偏多/正面` 两份都没有），于是**同一段文本走哪条分支结论不同**。
 *
 * 分工：方向词走权威单点 [`classifyDirectionText`]；`情绪词`（利好/乐观/流入/超配/
 * 多头/空头 …）只在本函数拼接 —— 它们不是方向词，仅对分析师自由文本有意义，
 * **不得**混进 `DIRECTION_BULL_PATTERN`（那会让 Card / Grid 对显式 verdict 的判定过宽）。
 */
function directionTextToSentiment(text: string): "bullish" | "bearish" | null {
  const lower = text.toLowerCase();
  const dir = classifyDirectionText(text);
  if (
    dir === "bull"
    || text.includes("多头")
    || text.includes("利好") || text.includes("上涨") || text.includes("乐观")
    || text.includes("上行") || text.includes("流入") || text.includes("扫货")
    || text.includes("强于") || text.includes("超配")
    || lower.includes("buy") || lower.includes("overweight")
  ) {
    return "bullish";
  }
  if (
    dir === "bear"
    || text.includes("空头")
    || text.includes("利空") || text.includes("下跌") || text.includes("悲观")
    || text.includes("下行") || text.includes("流出") || text.includes("出货")
    || text.includes("弱于") || text.includes("低配")
    || lower.includes("sell") || lower.includes("underweight")
  ) {
    return "bearish";
  }
  return null;
}

/**
 * 中性词表 —— 同样被 VERDICT 分支与 stance 分支共用。
 * 两份旧内联表分别为 `中性|观望|持有|震荡|hold|neutral` 与
 * `观望|中性|平衡|震荡|同步|保守|放缓|持有|hold|neutral`（后者是前者的超集）。
 */
const NEUTRAL_SENTIMENT_PATTERN = /中性|观望|持有|震荡|平衡|同步|保守|放缓|hold|neutral/i;

/**
 * 分析师报告情感分类（先解析 JSON 提取结构化字段，再回退子串匹配）
 *
 * 支持:
 * - JSON 中 stance / action / bull_score / positionPct / recommendation 等字段
 * - 纯文本关键词匹配（买入/卖出/持有/看多/看空 等维度）
 */
export function classifySentiment(report: string): "bullish" | "bearish" | "neutral" {
  // 0) 优先解析 <!-- VERDICT: {...} --> 格式（分析师自由文本 + 末尾 verdict 标签）
  const verdictIdx = report.indexOf("<!-- VERDICT:");
  if (verdictIdx !== -1) {
    try {
      const jsonStr = report.slice(verdictIdx + "<!-- VERDICT:".length);
      const jsonEnd = jsonStr.indexOf("-->");
      if (jsonEnd !== -1) {
        const meta = JSON.parse(jsonStr.slice(0, jsonEnd).trim());
        const stance = String(meta.verdict ?? meta.stance ?? "").trim();
        if (stance) {
          // 2026-09-21: 收敛到 `directionTextToSentiment`（原先这里是**第三份**内联
          //   方向词表，且与下方 stance 分支的表值域不同 ⇒ 同一文本走哪条分支结论不同）。
          const s = directionTextToSentiment(stance);
          if (s) { return s; }
          if (NEUTRAL_SENTIMENT_PATTERN.test(stance)) { return "neutral"; }
        }
        // 用 bull_score / bear_score 判断
        const bull = Number(meta.bull_score ?? -1);
        const bear = Number(meta.bear_score ?? -1);
        if (bull >= 0 && bear >= 0) {
          if (bull > bear) { return "bullish"; }
          if (bear > bull) { return "bearish"; }
          return "neutral";
        }
      }
    } catch { /* ignore */ }
  }

  // 1) 尝试从 JSON 结构化字段提取
  const json = tryParseJson(report);
  if (json) {
    // strict_mode 下方向与分数嵌在 `verdict` 对象里（`{"report":…,"verdict":{"verdict":"观望","bull_score":…}}`）。
    // 不提升时 `String(json["verdict"])` 得到 `"[object Object]"` ⇒ 结构化通道整体失效，
    // 退化成对整篇 JSON 做关键词扫描，显式 verdict 被正文措辞覆盖。
    // 提升形态与 `AnalystReportCard.tryParseVerdictFormat` / `readRiskReportText` 一致。
    const inner = json["verdict"];
    const meta: Record<string, unknown> = inner && typeof inner === "object" && !Array.isArray(inner)
      ? { ...json, ...(inner as Record<string, unknown>) }
      : json;
    // 1a) stance 字段 — 中文方向词，覆盖各 agent 不同的命名空间
    const stanceRaw = meta["stance"] ?? meta["view"] ?? meta["sentiment"] ?? meta["verdict"];
    // 只接受标量：`{}` 这类空对象 stringify 成 "[object Object]" 会被下游当成文本参与判定。
    const stance = typeof stanceRaw === "string" || typeof stanceRaw === "number"
      ? String(stanceRaw).trim()
      : "";
    if (stance) {
      // 2026-09-21: 收敛到 `directionTextToSentiment`（原先此处自写一份方向词表，
      //   与 VERDICT 分支的表在 `多头/利好/超配/扫货` 等词上分歧，且两份都缺
      //   `偏多/正面/偏空/负面` ⇒ 同一份 `stance: "偏多"` 的研报，卡片徽标判看多（红），
      //   共识聚合却计入「中性」）。
      const s = directionTextToSentiment(stance);
      if (s) { return s; }
      if (NEUTRAL_SENTIMENT_PATTERN.test(stance)) { return "neutral"; }
    }
    // 1b) action 字段（BUY/INCREASE/SELL/REDUCE/HOLD 或中文 买入/增持/...）
    const action = String(meta["action"] ?? "").trim();
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
    const bullScoreRaw = meta["bull_score"] ?? meta["bullScore"];
    const bearScoreRaw = meta["bear_score"] ?? meta["bearScore"];
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
    const posPctRaw = meta["positionPct"] ?? meta["position_pct"];
    if (posPctRaw != null) {
      const posPct = Number(posPctRaw);
      if (Number.isFinite(posPct)) {
        if (posPct >= 6) { return "bullish"; }
        if (posPct < 0) { return "bearish"; }
        return "neutral";
      }
    }
    // 1e) recommendation / rating 字段
    const rec = String(meta["recommendation"] ?? meta["rating"] ?? "").trim().toLowerCase();
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

  // 2) 纯文本回退：子串级情感匹配
  const text = report.toLowerCase();
  let bullScore = 0;
  let bearScore = 0;

  const bullPatterns = [
    "买入",
    "增持",
    "看多",
    "做多",
    "看涨",
    "多头",
    "利好",
    "上涨",
    "乐观",
    "上行",
    "流入",
    "扫货",
    "强于",
    "超配",
    "加仓",
    "bull",
    "buy",
    "overweight",
    "增长",
    "改善",
    "盈利",
    "回升",
    "反弹",
    "突破",
    "看好",
  ];
  const bearPatterns = [
    "卖出",
    "减持",
    "看空",
    "做空",
    "看跌",
    "空头",
    "利空",
    "下跌",
    "悲观",
    "下行",
    "流出",
    "出货",
    "弱于",
    "低配",
    "减仓",
    "bear",
    "sell",
    "underweight",
    "下滑",
    "恶化",
    "亏损",
    "回落",
    "跌破",
  ];

  // 否定前缀：出现在看涨/看跌关键词前 3 字内时，抵消该关键词的情感贡献
  const NEG_PREFIXES = ["无", "没有", "不会", "不存", "无需", "未", "难", "非", "缺乏", "不见"];
  // 否定后缀：出现在看涨/看跌关键词之后（紧邻），将正向词转为反向语义
  // 例如「增长放缓」「盈利承压」「回升不及」应判为空头或中性，而非多头
  const NEG_SUFFIXES = ["放缓", "承压", "不及", "乏力", "疲软", "回落", "受阻", "不及预期", "低于预期", "下滑", "恶化"];

  for (const p of bullPatterns) {
    const idx = text.indexOf(p);
    if (idx !== -1) {
      const before = text.slice(Math.max(0, idx - 3), idx);
      const after = text.slice(idx + p.length, idx + p.length + 4);
      const negated = NEG_PREFIXES.some((neg) => before.includes(neg))
        || NEG_SUFFIXES.some((neg) => after.includes(neg));
      if (!negated) {
        bullScore++;
      }
    }
  }
  for (const p of bearPatterns) {
    const idx = text.indexOf(p);
    if (idx !== -1) {
      const before = text.slice(Math.max(0, idx - 3), idx);
      const after = text.slice(idx + p.length, idx + p.length + 4);
      const negated = NEG_PREFIXES.some((neg) => before.includes(neg))
        || NEG_SUFFIXES.some((neg) => after.includes(neg));
      if (!negated) {
        bearScore++;
      }
    }
  }

  if (bullScore > bearScore) { return "bullish"; }
  if (bearScore > bullScore) { return "bearish"; }
  return "neutral";
}

// ── 共识聚合 ──

export type Sentiment = "bullish" | "bearish" | "neutral";
export type Consensus = Sentiment | "divided";

export interface StockConsensus {
  consensus: Consensus;
  bullish: number;
  bearish: number;
  neutral: number;
  total: number;
  /** 时间戳（毫秒） */
  updatedAt: number;
}

/** 分析师按 ID 后缀的领域权重：value=价值/长线, technical=技术/短线, sentiment=情绪, macro=宏观 */
/**
 * ⚠ 本表是后端 `evidence_weight::get_horizon_base_weights` 的**离线兜底副本**
 * ⚠ 键必须是后端 `EVIDENCE_ANALYST_IDS` 里的**分析师节点 id** —— 历史上这里混着
 *   `a-market`/`a-technical`/`capital`/`macro`/`fundamental`/`sentiment` 六个图里不存在的名字，
 *   后端精确查表全部落空、逐档权重静默退 1.0（2026-10-03 统一，见 A 落地记录）。
 * （仅在 `compute_evidence_weights` 命令失败时由 `computeStockConsensus` 使用）。
 * 逐字段一致性由 `node scripts/check-horizon-weight-parity.mjs` 把守 —— 缺键会被
 * `getAnalystWeight` 的 default 静默吸收成 1.0（2026-09-29 实测曾缺 6 项）。
 * 新增/改动档位权重：先改后端权威源，再同步本表，然后跑该门。
 */
const ANALYST_TIME_HORIZON_WEIGHT: Record<string, Record<string, number>> = {
  // 中线决策：基本面与技术面均衡，各分析师权重接近
  mid: {
    "a-market-analyst": 1.0,
    "a-fundamentals": 1.2,
    // P0-3（PLAN-stock-decision-reliability）：估值在 28 日窗无方向预测力，
    // 偏置 1.2→0.32（归一化占比 29.3%→9.9%）。必须与后端
    // evidence_weight.rs get_horizon_base_weights("mid") 逐值相等
    // （check-horizon-weight-parity.mjs 判据 drift）。
    "value-investor": 0.32,
    "a-research": 1.0,
    "research-mgr": 1.1,
    "a-catalyst": 1.0,
    "a-sentiment": 1.0,
    "a-hot-money": 0.9,
    "a-lockup": 1.0,
    "a-sector": 1.1,
    "a-policy": 1.0,
    default: 1.0,
  },
  // 长线决策：价值投资者和分析师权重最高，技术面被削弱
  long: {
    "a-market-analyst": 0.6,
    "a-fundamentals": 1.5,
    "value-investor": 2.0,
    "a-research": 1.0,
    "research-mgr": 1.5,
    "a-catalyst": 1.0,
    "a-sentiment": 0.7,
    "a-hot-money": 0.5,
    "a-lockup": 1.0,
    "a-sector": 1.2,
    "a-policy": 1.0,
    default: 1.0,
  },
  // 短线决策：技术面、资金面、情绪权重最高
  short: {
    "a-market-analyst": 1.5,
    "a-fundamentals": 0.6,
    "value-investor": 0.5,
    "a-research": 1.0,
    "research-mgr": 1.0,
    "a-catalyst": 1.0,
    "a-sentiment": 1.3,
    "a-hot-money": 1.5,
    "a-lockup": 1.0,
    "a-sector": 0.8,
    "a-policy": 1.0,
    default: 1.0,
  },
  // 超短线：资金面、情绪权重最高，基本面几乎不考虑
  ultra_short: {
    "a-market-analyst": 1.3,
    "a-fundamentals": 0.3,
    "value-investor": 0.3,
    "a-research": 1.0,
    "research-mgr": 0.5,
    "a-catalyst": 1.0,
    "a-sentiment": 1.5,
    "a-hot-money": 2.0,
    "a-lockup": 1.0,
    "a-sector": 0.5,
    "a-policy": 1.0,
    default: 1.0,
  },
};

/** 根据分析师 ID 获取时间维度权重 */
function getAnalystWeight(rawId: string, timeHorizon?: string | null): number {
  const weights = ANALYST_TIME_HORIZON_WEIGHT[timeHorizon || "mid"] || ANALYST_TIME_HORIZON_WEIGHT.mid;
  // 黑版键带 `report.` 前缀（blackboard.rs 的 report.{节点 id} 规则），本表按裸节点 id 索引
  // —— 与后端 `evidence_weight::analyst_key` 同一归一，两侧才不会一个是 1.0 一个是 0.3。
  const analystId = rawId.replace(/^report\./, "");
  // 精确匹配
  if (weights[analystId] != null) { return weights[analystId]; }
  // 后缀模糊匹配
  for (const [suffix, w] of Object.entries(weights)) {
    if (suffix === "default") { continue; }
    if (analystId.includes(suffix)) { return w; }
  }
  return weights.default ?? 1.0;
}

/**
 * 聚合分析师报告/投票，推最终共识。
 * 支持 timeHorizon 参数：不同时间维度下分析师权重不同
 * （长线重基本面、短线重技术面动量、超短线重资金流）。
 */
export function computeStockConsensus(
  reports: Record<string, string>,
  updatedAt?: number,
  timeHorizon?: string | null,
): StockConsensus {
  let bullish = 0;
  let bearish = 0;
  let neutral = 0;
  for (const [analystId, text] of Object.entries(reports)) {
    const s = classifySentiment(text);
    const w = getAnalystWeight(analystId, timeHorizon);
    if (s === "bullish") { bullish += w; }
    else if (s === "bearish") { bearish += w; }
    else { neutral += w; }
  }
  const total = bullish + bearish + neutral;
  let consensus: Consensus;
  if (total === 0) {
    consensus = "neutral";
  } else {
    const net = bullish - bearish;
    // 加权后阈值：加权 net > 加权 total × 0.3 → bullish（等效 N/3）
    const threshold = total * 0.3;
    if (net > threshold) { consensus = "bullish"; }
    else if (net < -threshold) { consensus = "bearish"; }
    else if (bullish > 0 && bearish > 0) { consensus = "divided"; }
    else { consensus = "neutral"; }
  }
  return {
    consensus,
    bullish: Math.round(bullish * 10) / 10,
    bearish: Math.round(bearish * 10) / 10,
    neutral: Math.round(neutral * 10) / 10,
    total: Math.round(total * 10) / 10,
    updatedAt: updatedAt ?? Date.now(),
  };
}

/**
 * 证据质量驱动的共识计算（P0-1）。
 *
 * 替代简单的阈值投票，结合市场环境(regime)、时间维度、分析师历史表现
 * 动态分配权重，并检查 HOLD 门控条件。
 *
 * @param reports 分析师报告字典
 * @param marketRegime 市场环境信息（从 loadMarketRegime 获取）
 * @param timeHorizon 投资周期
 * @param historicalWeights 可选的历史表现权重
 * @param updatedAt 可选的时间戳
 */
export async function computeEvidenceDrivenConsensus(
  reports: Record<string, string>,
  marketRegime: MarketRegimeInfo,
  timeHorizon?: string | null,
  historicalWeights?: Record<string, number> | null,
  updatedAt?: number,
): Promise<
  StockConsensus & { evidenceReport?: EvidenceWeightReport; evidenceFallback?: boolean }
> {
  // marketRegime 缺失时提供默认值（避免后端校验失败）
  const defaultRegime: MarketRegimeInfo = marketRegime ?? {
    regime: "neutral",
    confidence: 0.5,
    volatility: "medium",
    description: "未知市场环境（默认值）",
  };

  try {
    // 构建分析师输入
    const analysts: AnalystInput[] = Object.entries(reports).map(([analystId, text]) => {
      const json = tryParseJson(text);
      return {
        analystId,
        reportText: text,
        stance: json ? (String(json["stance"] ?? json["view"] ?? json["verdict"] ?? "") || null) : null,
        bullScore: json ? (json["bull_score"] as number ?? json["bullScore"] as number ?? null) : null,
        bearScore: json ? (json["bear_score"] as number ?? json["bearScore"] as number ?? null) : null,
        positionPct: json ? (json["positionPct"] as number ?? json["position_pct"] as number ?? null) : null,
      };
    });

    const request: EvidenceWeightRequest = {
      marketRegime: defaultRegime,
      timeHorizon: timeHorizon ?? "mid",
      analysts,
      historicalWeights: historicalWeights ?? null,
    };

    const evidenceReport = await computeEvidenceWeights(request);

    // 将后端结果映射为前端 StockConsensus 格式
    const consensus = evidenceReport.consensus;
    const consensusMap: Record<string, Consensus> = {
      bullish: "bullish",
      bearish: "bearish",
      divided: "divided",
      neutral: "neutral",
    };

    return {
      consensus: consensusMap[consensus.consensus] ?? "neutral",
      bullish: consensus.bullishScore,
      bearish: consensus.bearishScore,
      neutral: consensus.neutralScore,
      total: consensus.totalWeight,
      updatedAt: updatedAt ?? Date.now(),
      evidenceReport,
    };
  } catch (err) {
    // fallback: 如果后端不可用，回退到前端旧版计算
    console.warn("[computeEvidenceDrivenConsensus] 后端计算失败，回退到前端简单共识:", err);
    const result = computeStockConsensus(reports, updatedAt, timeHorizon);
    // 标记降级：前端可据此提示「证据驱动共识不可用，当前为简化版共识，仅供参考」
    return { ...result, evidenceReport: undefined, evidenceFallback: true };
  }
}

// ── 分析师名称映射（已迁移至 i18n: stockAnalysis.workflow.analyst.*）──

/**
 * 四档周期的 i18n 标签后缀。
 *
 * 与 `DecisionBanner` 的 `HORIZON_KEYS` 共用同一批键（`stockAnalysis.timeHorizon*`）——
 * 档位显示名全仓只此一套，历史列表与时间线不得各写一套（2026-09-29）。
 */
export const HORIZON_T_SUFFIX: Readonly<Record<string, string>> = {
  ultra_short: "UltraShort",
  short: "Short",
  mid: "Mid",
  long: "Long",
};

/** 档位后缀分隔符 —— 与 Rust 侧 `harness::holding_period::ANALYST_TIER_SEP` 同一常量。 */
export const ANALYST_TIER_SEP = "--";

/** 某档分析师实例的节点 id（产端由种子生成，见 Rust 侧 `analyst_node_id`）。 */
export function analystNodeId(base: string, tierSnake: string): string {
  return `${base}${ANALYST_TIER_SEP}${tierSnake}`;
}

/**
 * 从节点 id 剥回 base，供面板按 base 查显示名 / 权重 / 行 key。
 *
 * 不是 `base--已知档` 形状就返回 `null` 而**不猜**：历史分析行与快速链里是裸 base id
 * （改造前产的），后缀不是四档之一则说明接线接错了对象。档位域取自 `HORIZON_T_SUFFIX`
 * ⇒ 值域加档而这里没同步时，症状是「剥不到、那一格显示为未知」，不会把别档的读数顶上来。
 */
export function analystBaseOf(nodeId: string): string | null {
  const i = nodeId.indexOf(ANALYST_TIER_SEP);
  if (i <= 0) { return null; }
  const tier = nodeId.slice(i + ANALYST_TIER_SEP.length);
  if (!Object.prototype.hasOwnProperty.call(HORIZON_T_SUFFIX, tier)) { return null; }
  return nodeId.slice(0, i);
}

/** 辩论节点 id / 黑板快照键的分解结果。`tier === null` = 分档之前的历史记录。 */
export interface DebateRoundRef {
  tier: string | null;
  round: number;
}

/**
 * 解析辩论节点 id（或同形的黑板快照键）的档位与轮次。
 *
 * 认三种并存形态：
 *   · `bull-researcher` —— ≤v160 的单份历史记录 ⇒ `{ tier: null, round: 1 }`
 *   · `bull-r2` —— v161 前的多轮（不分档）⇒ `{ tier: null, round: 2 }`
 *   · `bull-r1--mid` —— v161 起逐档辩论 ⇒ `{ tier: "mid", round: 1 }`
 *
 * 后缀不是四档之一 ⇒ 返回 `null`（接线接错对象，不得猜成某一档）；
 * 调用方据此跳过该节点，而不是把它塞进第 1 轮去覆盖别人。
 */
export function parseDebateRoundRef(nodeId: string): DebateRoundRef | null {
  const m = /^(?:bull|bear)-(researcher|r(\d+))(?:--(.+))?$/.exec(nodeId.trim());
  if (!m) { return null; }
  if (m[1] === "researcher") {
    // 历史单份节点没有档位后缀；带后缀说明接线接错了对象
    return m[3] === undefined ? { tier: null, round: 1 } : null;
  }
  const round = Number(m[2]);
  if (!Number.isInteger(round) || round < 1) { return null; }
  if (m[3] === undefined) { return { tier: null, round }; }
  if (!Object.prototype.hasOwnProperty.call(HORIZON_T_SUFFIX, m[3])) { return null; }
  return { tier: m[3], round };
}

/** 同档同轮的对侧节点 id（从 bull 键取 bear 正文用）；认不出返回 null。 */
export function debatePeerNodeId(nodeId: string, peer: "bull" | "bear"): string | null {
  const ref = parseDebateRoundRef(nodeId);
  if (!ref) { return null; }
  if (/^(?:bull|bear)-researcher$/.test(nodeId.trim())) { return `${peer}-researcher`; }
  return ref.tier === null
    ? `${peer}-r${ref.round}`
    : `${peer}-r${ref.round}${ANALYST_TIER_SEP}${ref.tier}`;
}

const DEBATE_TIER_ORDER = Object.keys(HORIZON_T_SUFFIX);

/** 档位排序权重：四档按权威顺序，历史单份排最后。 */
export function debateTierOrder(tier: string | null): number {
  if (tier === null) { return DEBATE_TIER_ORDER.length; }
  const i = DEBATE_TIER_ORDER.indexOf(tier);
  return i === -1 ? DEBATE_TIER_ORDER.length : i;
}

/** 辩论行稳定排序：先四档顺序、再轮次（历史单份排最后）。 */
export function compareDebateRounds(a: DebateRoundRef, b: DebateRoundRef): number {
  return debateTierOrder(a.tier) - debateTierOrder(b.tier) || a.round - b.round;
}

/**
 * `decisionsByHorizon` 的键是 camelCase，而 i18n 后缀表按 snake_case 建模
 * （`stock_reflections.horizon_results_json`、命中率的 `byHorizon` 都用 snake）⇒
 * 同一档位两套键名并存是既成事实，互转只在此单点做一次。
 */
export const HORIZON_CAMEL_TO_SNAKE: Readonly<Record<string, string>> = {
  ultraShort: "ultra_short",
  short: "short",
  mid: "mid",
  long: "long",
};

/** 任意形态档名（camel 或 snake）→ i18n 后缀；认不出 → null（调用方给原始键，不得瞎猜档位）。 */
export function horizonSuffix(key: string): string | null {
  const snake = HORIZON_CAMEL_TO_SNAKE[key] ?? key;
  return HORIZON_T_SUFFIX[snake] ?? null;
}

/**
 * 任意形态档名（camel 或 snake）→ **全仓唯一一套档名 i18n 键** `stockAnalysis.timeHorizon*`；
 * 认不出 → `null`（调用方自己兜底显示原始键，不得瞎猜档位）。
 *
 * 存在理由（2026-10-16 收敛）：档名此前有 **8 套键族**（`period.*` / `period*` /
 * `recommendation.period*` / `reflection.horizon*` / `scheduledAnalysis.period.*` /
 * `replayWorkbench.batch.period.*` / `settings.scheduled-recommendation.period-*` +
 * 本族），值虽已统一但**定义仍是 8 份** ⇒ 任一族漏改就前后端同屏两名。
 * 收敛后只此一族，取键也走这一个函数：`t(horizonLabelKey(p) ?? p)`。
 */
export function horizonLabelKey(key: string): string | null {
  const suffix = horizonSuffix(key);
  return suffix ? `stockAnalysis.timeHorizon${suffix}` : null;
}

/**
 * rank IC 缺席原因 → i18n 键。**四种缺席各占一句**：把它们并成一句「暂无数据」，
 * 就等于告诉用户「这里没东西」，而真实情况分别是「要补字段」「要攒样本」
 * 「取值无差异（本档算不出相关系数）」、「要等口径换代」——要做的事完全不同。
 *
 * 状态字符串的权威在后端 `reflection_stats.rs::HitrateGroup::ic_status`；
 * 未登记的状态**返回 null**（渲染方回退「暂无数据」），而不是猜一个最接近的翻译。
 */
export function horizonIcAbsenceKey(status?: string | null): string | null {
  const map: Record<string, string> = {
    no_confidence: "stockAnalysis.reflection.hitrateIcNoConfidence",
    insufficient_ic_samples: "stockAnalysis.reflection.hitrateIcInsufficient",
    degenerate_variance: "stockAnalysis.reflection.hitrateIcDegenerate",
    pre_snr_regime: "stockAnalysis.reflection.hitrateIcPreRegime",
  };
  return status ? (map[status] ?? null) : null;
}

/** 妖股标签的呈现分层（与 `horizonIcAbsenceKey` 同族：缺席必须分句）。 */
export type MoverLabelKind = "yes" | "no" | "absence" | "notReflected" | "unknown";

/**
 * 后端 `stock_reflections.mover_label` → 呈现分层（#10 P7）。
 *
 * 值域权威在 `crates/analysis-engine/src/mover_recall.rs::mover_label_for`。四种「无从判定」
 * 与「未达标」必须各占一句 —— 并成一句「不是妖股」等于把**没判定**说成**判定过**，
 * 用户会以为系统看过这只票：
 * · `window_incomplete` = 持有期未满（该档妖股位留空并注明未满，不拿当前价冒充到期价）
 * · `no_market_data` = 该档行情快照不可得
 * · `rule_unavailable` = 阈值判据不可用（变量缺失/非正，或变量表读取失败）
 * · `null` = 该行还没走到反思收尾（pending/running/failed）或本列引入前的存量行
 *
 * 未登记的标签值返回 `unknown` + `i18nKey=null`，由渲染方**原样显示**（不猜翻译、不静默吞掉）。
 */
export function moverLabelPresentation(label?: string | null): {
  kind: MoverLabelKind;
  i18nKey: string | null;
} {
  if (label == null || label === "") {
    return { kind: "notReflected", i18nKey: "stockAnalysis.reflection.moverNotReflected" };
  }
  if (label === "mover") {
    return { kind: "yes", i18nKey: "stockAnalysis.reflection.moverYes" };
  }
  if (label === "normal") {
    return { kind: "no", i18nKey: "stockAnalysis.reflection.moverNo" };
  }
  const absenceKey: Record<string, string> = {
    window_incomplete: "stockAnalysis.reflection.moverWindowIncomplete",
    no_market_data: "stockAnalysis.reflection.moverNoMarketData",
    rule_unavailable: "stockAnalysis.reflection.moverRuleUnavailable",
  };
  const key = absenceKey[label];
  if (key !== undefined) {
    return { kind: "absence", i18nKey: key };
  }
  return { kind: "unknown", i18nKey: null };
}

/**
 * 主档来源标签。值域权威是 `crates/harness/src/holding_period.rs` 的 `HORIZON_SOURCES`
 * （产出方 `portfolio-mgr.rhai`、落库白名单 `decision::extract_horizon_source` 都读那一份），
 * 本函数是它的**第三处载体** ⇒ 必须逐值覆盖：漏一个值不是「少个标签」，
 * 而是那一代记录的「谁定的档」在界面上直接消失（返回 null ⇒ 调用方不渲染）。
 *
 * · `branch_pick` = 主档由四档分支结论选出（现网正常路径）
 * · `formula_no_branch` = 四路分支全部未产出 ⇒ 退回后验阈值定档（兜底，必须与上一值可区分）
 * · `formula` / `model` / `user` = 历史值（旧快照与存量行）
 */
export function horizonSourceLabelKey(
  source?: string | null,
): string | null {
  if (source === "branch_pick") { return "stockAnalysis.horizonSourceBranchPick"; }
  if (source === "formula_no_branch") { return "stockAnalysis.horizonSourceNoBranch"; }
  if (source === "formula") { return "stockAnalysis.horizonSourceFormula"; }
  if (source === "model") { return "stockAnalysis.horizonSourceModel"; }
  return null;
}

/**
 * 主档 action 改写的**值域权威在产出方** `src-tauri/src/commands/portfolio-mgr.rhai`
 * 的 `action_source`（按「最后改写者优先」判定），本数组是第三处载体 ⇒ 必须逐值覆盖，
 * 并由 `stockAnalysisHorizon.test.ts` 的并集门双向核对（漏一个值 = 那一类记录的来历在界面上直接消失）。
 */
export const ACTION_SOURCES: readonly string[] = [
  "branch_pick",
  // 2026-10-10（B 裁定）：四档分支各自按本档风险节点否决自身 action —— 主档取自该档时，
  // 这一格就是它的来历（排名在 branch_pick 之前，因否决发生在分支内部）。
  "tier_risk_veto",
  "risk_veto_downgrade",
  "bearish_veto_downgrade",
  "sim_veto_downgrade",
  "sanity_cap_downgrade",
  "partial_low_confidence_cap",
  // v149(L2/L3)：算法估值结论=高估 ⇒ 买入/增持 封顶为 持有（`valuation_capped` 在产端链首）。
  "valuation_cap",
  "main_chain",
];

/** 主档置信度口径的值域（产出方同上，`confidence_source`）。 */
export const CONFIDENCE_SOURCES: readonly string[] = ["branch_row", "main_chain_posterior"];

/**
 * action 来历 → i18n key。**未命中返回 null**（= 该记录产生于 v127 之前，无此信息），
 * 调用方不渲染，不得回退成「分支选档」那种乐观默认（把旧记录说成新口径就是伪装）。
 */
export function actionSourceLabelKey(source?: string | null): string | null {
  switch (source) {
    case "branch_pick":
      return "stockAnalysis.actionSourceBranchPick";
    case "risk_veto_downgrade":
      return "stockAnalysis.actionSourceRiskVeto";
    case "tier_risk_veto":
      return "stockAnalysis.actionSourceTierRiskVeto";
    case "bearish_veto_downgrade":
      return "stockAnalysis.actionSourceBearishVeto";
    case "sim_veto_downgrade":
      return "stockAnalysis.actionSourceSimVeto";
    case "sanity_cap_downgrade":
      return "stockAnalysis.actionSourceSanityCap";
    case "partial_low_confidence_cap":
      return "stockAnalysis.actionSourcePartialLowConf";
    case "valuation_cap":
      return "stockAnalysis.actionSourceValuationCap";
    case "main_chain":
      return "stockAnalysis.actionSourceMainChain";
    default:
      return null;
  }
}

/** 置信度口径 → i18n key（未命中同样返回 null，语义同上）。 */
export function confidenceSourceLabelKey(source?: string | null): string | null {
  switch (source) {
    case "branch_row":
      return "stockAnalysis.confidenceSourceBranchRow";
    case "main_chain_posterior":
      return "stockAnalysis.confidenceSourceMainChain";
    default:
      return null;
  }
}

/**
 * 从分析记录的 `decisionJson` 里取「四档各自的 Action」。
 *
 * 一次分析产四档决策（`decisionsByHorizon`，portfolio-mgr.rhai），历史行此前只渲染主档那条。
 * 键兼容 camelCase（现网产出）与 snake_case（旧快照 / `agentOutput` 同规矩）；解析不出任何一档
 * 时返回空数组 ⇒ 调用方**不渲染**，不得回退成「四档同主档」。
 */
export function readHorizonActions(decisionJson?: string | null): Array<{ key: string; action: string }> {
  if (!decisionJson) { return []; }
  let parsed: Record<string, unknown>;
  try {
    parsed = JSON.parse(decisionJson) as Record<string, unknown>;
  } catch {
    return [];
  }
  const raw = parsed.decisionsByHorizon ?? parsed.decisions_by_horizon;
  if (raw == null || typeof raw !== "object") { return []; }
  const map = raw as Record<string, { action?: unknown } | null>;
  const out: Array<{ key: string; action: string }> = [];
  // 键表来自 `HORIZON_CAMEL_TO_SNAKE`（单点），不在此重复一份档位表
  for (const [camel, snake] of Object.entries(HORIZON_CAMEL_TO_SNAKE)) {
    const row = map[camel] ?? map[snake];
    const action = row && typeof row.action === "string" ? row.action : "";
    if (action) { out.push({ key: snake, action }); }
  }
  return out;
}

/** 主档 action 被**后置规则改写**的那几类来源（`branch_pick` / `main_chain` 之外的降级格）。 */
const DOWNGRADE_ACTION_SOURCES: readonly string[] = [
  // 档内风控否决（2026-10-10 B 裁定）：四档分支各自按**本档**风险节点否决自身 action。
  // 位置最靠前（发生在分支内部），但它同样是「主档被后置规则降级」格的一员 ——
  // 主档 action 已是否决后的终值，成句要说的「分支原名」取自 `originalAction`。
  "tier_risk_veto",
  "risk_veto_downgrade",
  "bearish_veto_downgrade",
  "sim_veto_downgrade",
  "sanity_cap_downgrade",
  "partial_low_confidence_cap",
  // v149(L3)：算法估值结论=高估 ⇒ 买入/增持 封顶为 持有（分支行仍留着原 action ⇒ 成句用得上）。
  "valuation_cap",
];

/** 主档来历（`stockAnalysis.decisionProvenance*` 两条模板的入参）。 */
export type DecisionProvenance = {
  /** `direct` = 主档就是所选档；`downgraded` = 被后置规则单向降级；`label` = 只报来历不成句 */
  kind: "direct" | "downgraded" | "label";
  actionSource: string;
  confidenceSource?: string;
  /** 所选档档名（snake_case）；主链定档那代没有分支行 ⇒ 可能缺 */
  horizon?: string;
  /** 改写前、所选档分支自己给的 action */
  branchAction?: string;
};

/**
 * 从 `decisionJson` 读「主档这条是怎么来的」（§五十三 ①，v127 起产端才发这两个字段）。
 *
 * ⚠ 返回 `null` 的语义是**无此信息**（该记录产生于 v127 之前），调用方必须不渲染 ——
 *   不得回退成「分支选档」，那是给旧记录编一个它没有的来历（同 `horizonPriceMap` 的
 *   「缺席 ≠ 空」规矩）。降级格但读不到分支原 action 时退成 `label`（只说「风控否决降级」，
 *   不编造「买入→持有」里的那个「买入」）。
 */
export function readDecisionProvenance(
  source?: string | Record<string, unknown> | null,
): DecisionProvenance | null {
  if (!source) { return null; }
  let parsed: Record<string, unknown>;
  if (typeof source === "string") {
    try {
      parsed = JSON.parse(source) as Record<string, unknown>;
    } catch {
      return null;
    }
  } else {
    parsed = source;
  }
  const pick = (camel: string, snake: string): string | undefined => {
    const v = parsed[camel] ?? parsed[snake];
    return typeof v === "string" && v ? v : undefined;
  };
  const actionSource = pick("actionSource", "action_source");
  if (!actionSource) { return null; }
  const confidenceSource = pick("confidenceSource", "confidence_source");
  const horizonRaw = pick("timeHorizon", "time_horizon");
  let horizon: string | undefined;
  let branchAction: string | undefined;
  if (horizonRaw) {
    for (const [camel, snake] of Object.entries(HORIZON_CAMEL_TO_SNAKE)) {
      if (horizonRaw !== camel && horizonRaw !== snake) { continue; }
      horizon = snake;
      const raw = parsed.decisionsByHorizon ?? parsed.decisions_by_horizon;
      const row = raw && typeof raw === "object"
        ? (raw as Record<string, { action?: unknown; originalAction?: unknown }>)[camel]
          ?? (raw as Record<string, { action?: unknown; originalAction?: unknown }>)[snake]
        : undefined;
      // 2026-10-10（B 裁定）：逐档行的 `action` 已是**档内风控否决后**的终值，与主档一致 ⇒
      //   成句「主档取自中期分支「X」，被…降为「Y」」里的 X 必须取 `originalAction`（否决前
      //   的方向值）。否则 X 会等于 Y，成句退化成「…「观望」，被风控降为「观望」」。
      //   缺席 `originalAction`（早于本版的记录 / 未否决）⇒ 退回 `action`（现状口径）。
      if (row && typeof row.originalAction === "string" && row.originalAction) {
        branchAction = row.originalAction;
      } else if (row && typeof row.action === "string" && row.action) {
        branchAction = row.action;
      }
      break;
    }
  }
  if (DOWNGRADE_ACTION_SOURCES.includes(actionSource)) {
    return branchAction
      ? { kind: "downgraded", actionSource, confidenceSource, horizon, branchAction }
      : { kind: "label", actionSource, confidenceSource, horizon };
  }
  if (actionSource === "branch_pick" && horizon) {
    return { kind: "direct", actionSource, confidenceSource, horizon, branchAction };
  }
  return { kind: "label", actionSource, confidenceSource, horizon };
}

// ── 分析师报告多空计票（单一真相源） ──
//
// 2026-10-10 修复：同一份「逐报告 → 看多/看空/中性」计票逻辑此前在三处各写一份副本
// （页面顶部 AnalystConsensusBar、分析师 tab 的 AnalystReportGrid、chat 紧凑版
// CompactAnalystSummary），且副本已经漂移 —— 顶部条缺 VERDICT 分数 0-1 标度归一化、
// 缺 stance-only 回退、缺裸 bull_score/bear_score 正则，连 tool_call 清洗都只剥标签
// 不删内容 ⇒ 同一屏顶部「10 看多 10 看空」、下方「9 看多 11 看空」。
// 此后三处一律走本模块，禁止再复制解析/计票/共识推导逻辑（AGENTS.md 禁区 12）。
//
// 三值/共识类型复用本文件既有的 `Sentiment` / `Consensus`（见「共识聚合」段），不另造。

/** 多空计票结果（`StockConsensus` 去掉时间戳的计票子集）。 */
export type AnalystSentimentTally = Pick<StockConsensus, "bullish" | "bearish" | "neutral" | "total">;

/**
 * 判决分数归一到百分制 —— 全站唯一的标度还原口径。
 *
 * 权威标度是提示词逐字声明的 `bull_score / bear_score / confidence: 0-100`
 * （`agent_executor.rs` 四处 VERDICT 提醒语）。实测最近 400 条分析共 2846 个分数，
 * 2835 个落在 >10 档 ⇒ 0-100 是真实形态；只有 ≤1 的小数（模型把 0-100 写成 0-1）需还原。
 *
 * ⚠ **不做**「≤10 就 ×10」那一族猜测：那会把声明值 7（=7%）编成 70%，并与
 * `classifyAnalystReport` 的计票口径分叉（计票按声明值比 1.2 倍阈值）。
 * 此前 `AnalystReportGrid` 表格列与 `AnalystReportCard` 的标签各有一份 ×10，
 * 同一条报告在卡片上显示 70、在共识计票里按 7 算。
 */
export function normalizeVerdictScore(value: number): number {
  return value > 1 ? value : value * 100;
}

/**
 * 从报告中提取结构化多空分数（与 AnalystReportCard.tryParseVerdictFormat 同源）。
 * 优先解析 `<!-- VERDICT: {bull_score, bear_score} -->` 格式；其次匹配正文裸 JSON 字段。
 * 无任何结构化分数时返回 `null`，调用方应回退到 `classifySentiment`。
 */
export function extractBullBearScores(report: string): { bull: number; bear: number } | null {
  // 1) 尝试 VERDICT JSON 格式（与 AnalystReportCard.tryParseVerdictFormat 一致）
  const verdictIdx = report.indexOf("<!-- VERDICT:");
  if (verdictIdx !== -1) {
    try {
      const jsonStr = report.slice(verdictIdx + "<!-- VERDICT:".length);
      const jsonEnd = jsonStr.indexOf("-->");
      if (jsonEnd !== -1) {
        const meta = JSON.parse(jsonStr.slice(0, jsonEnd).trim());
        const bull = meta.bull_score ?? meta.strength_score ?? null;
        const bear = meta.bear_score ?? null;
        if (typeof bull === "number" || typeof bear === "number") {
          return {
            bull: typeof bull === "number" ? normalizeVerdictScore(bull) : 0,
            bear: typeof bear === "number" ? normalizeVerdictScore(bear) : 0,
          };
        }
        // 有 stance/verdict 字段但没有分数 → 用 stance 判断
        // 2026-09-21: 方向判据收敛到 `classifyDirectionText`（单一真相源）。
        //   原为内联正则 `/看多|买入|增持|做多|看涨|bull/`，与同文件表格「判断」列的
        //   `/看多|bull|偏多|买入|增持|正面/` 值域不同 ⇒ 同一 verdict 两处结论可能相反。
        const dir = classifyDirectionText(meta.verdict ?? meta.stance);
        if (dir === "bull") { return { bull: 60, bear: 0 }; }
        if (dir === "bear") { return { bull: 0, bear: 60 }; }
      }
    } catch { /* ignore */ }
  }

  // 2) 尝试直接从文本正则匹配 bull_score/bear_score
  try {
    const bullMatch = report.match(/"bull_score"\s*:\s*(\d+(?:\.\d+)?)/);
    const bearMatch = report.match(/"bear_score"\s*:\s*(\d+(?:\.\d+)?)/);
    if (bullMatch || bearMatch) {
      return {
        bull: bullMatch ? parseFloat(bullMatch[1]) : 0,
        bear: bearMatch ? parseFloat(bearMatch[1]) : 0,
      };
    }
  } catch { /* ignore */ }

  // 3) 无结构化数据 → 返回 null，调用方用 classifySentiment fallback
  return null;
}

/**
 * 单份**已清洗**分析师报告 → 多空三分类。
 *
 * 口径：结构化分数优先且需领先 20%（1.2 倍阈值，接近即中性）；无结构化分数时回退
 * `classifySentiment`（VERDICT/JSON stance → 自由文本关键词）。
 *
 * 注意：传入前必须经 `cleanToolCallTags`（`@/lib/agentOutput`）清洗。本模块不能反向
 * 依赖 agentOutput（它已依赖本模块），故清洗由调用侧统一完成。
 */
export function classifyAnalystReport(cleanedReport: string): Sentiment | null {
  // 空产出（空串 / 纯空白）＝ 这个分析师没产出，**不是**「它投了中性」。返回 null 让
  // 调用方跳过这一票 —— 记成中性会把「24 个槽位里 5 个没跑出来」显示成「24 份报告、
  // 5 份中性」，与快速链 JEV 分支同一条裁定（未产出判定的维度不计票）。
  if (cleanedReport.trim() === "") { return null; }
  const scores = extractBullBearScores(cleanedReport);
  if (scores) {
    if (scores.bull > scores.bear * 1.2) { return "bullish"; }
    if (scores.bear > scores.bull * 1.2) { return "bearish"; }
    return "neutral"; // 分数接近 → 中性/分歧
  }
  return classifySentiment(cleanedReport);
}

/**
 * 对多份**已清洗**分析师报告计票。
 * 输入顺序不影响票型；调用侧负责 `Object.values(...).map(cleanToolCallTags)`。
 *
 * **空产出（空串 / 纯空白）不计票**：那是「这个分析师没产出」，不是「它投了中性」。
 * 记成中性会让「24 个槽位里 5 个没跑出来」显示成「24 份报告、5 份中性」——
 * 与快速链 JEV 分支同一条裁定。判据住在 `classifyAnalystReport` 的 null 契约里，
 * 调用侧无需各自 filter（此前顶部条剔空、分歧卡不剔，两边 total 可以不等）。
 */
export function tallyAnalystSentiment(reports: Iterable<string>): AnalystSentimentTally {
  let bullish = 0;
  let bearish = 0;
  let neutral = 0;
  for (const cleanedReport of reports) {
    const s = classifyAnalystReport(cleanedReport);
    if (s === null) { continue; }
    if (s === "bullish") { bullish++; }
    else if (s === "bearish") { bearish++; }
    else { neutral++; }
  }
  return { bullish, bearish, neutral, total: bullish + bearish + neutral };
}

/**
 * 票型 → 共识标签：
 *  - 一方占比 > 65% → 共识看多 / 共识看空
 *  - 多空双方都有票且均未过半到 65% → 分歧
 *  - 全部中性 / 单边极小 / 无票 → 中性
 */
export function deriveAnalystConsensus(tally: AnalystSentimentTally): Consensus {
  const { bullish, bearish, total } = tally;
  if (total === 0) { return "neutral"; }
  const bullRatio = bullish / total;
  const bearRatio = bearish / total;
  if (bullRatio > 0.65) { return "bullish"; }
  if (bearRatio > 0.65) { return "bearish"; }
  if (bullRatio > 0 && bearRatio > 0) { return "divided"; }
  return "neutral";
}

/**
 * 本次分析**应有的**辩论轮数，取自 `stock-analysis` 模板变量 `debate_rounds`。
 *
 * 为什么读变量而不是前端自造常量：轮数是**建图参数**（后端按它展开 N 对 `bull-rN`/`bear-rN`
 * 节点），而种子在落库前用 `force_variable_value` 把变量值与 DAG 展开数强制同源
 * （`src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs` 的辩论轮数 force 段）
 * ⇒ 变量值就是图里真跑的轮数。前端此前两处写死 3（`AnalysisProgress.tsx` 的
 * `TOTAL_DEBATE_ROUNDS`、`DebatePanel.tsx` 的 `EXPECTED_ROUNDS`），用户把轮数设成 2 或 5 后
 * 分母仍显示 3、缺席的轮次被报成「失败轮次」。
 *
 * 返回 `null` = 没读到（IPC 失败 / 变量缺失 / 值非正数）。调用方**不得**把 `null` 当 0
 * 或当默认值使用：拿不到期望轮数时就不得断言「某轮失败」—— 那会把「我们不知道」
 * 伪装成「它跑坏了」。
 */
export async function fetchExpectedDebateRounds(): Promise<number | null> {
  try {
    const tmpl = await invoke<Record<string, unknown>>("get_workflow_template", { id: "stock-analysis" });
    const vars = (tmpl?.variables ?? []) as Record<string, unknown>[];
    const found = vars.find((v) => v.name === "debate_rounds");
    const value = found?.value;
    return typeof value === "number" && value > 0 ? value : null;
  } catch {
    return null;
  }
}
