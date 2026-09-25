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
  if (["低", "低风险", "L"].includes(clean)) { return StockRiskLevel.LOW; }
  if (["中", "中风险", "M"].includes(clean)) { return StockRiskLevel.MID; }
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
  };
  return map[normalized] ?? "stockAnalysis.riskMid";
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
    // 1a) stance 字段 — 中文方向词，覆盖各 agent 不同的命名空间
    const stanceRaw = json["stance"] ?? json["view"] ?? json["sentiment"] ?? json["verdict"];
    const stance = String(stanceRaw ?? "").trim();
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
const ANALYST_TIME_HORIZON_WEIGHT: Record<string, Record<string, number>> = {
  // 中线决策：基本面与技术面均衡，各分析师权重接近
  mid: {
    "a-fundamentals": 1.2,
    "fundamental": 1.2,
    "value-investor": 1.2,
    "a-macro": 1.1,
    "macro": 1.1,
    "a-sector": 1.1,
    "research-mgr": 1.1,
    "a-market": 1.0,
    "a-technical": 1.0,
    "a-sentiment": 1.0,
    "sentiment": 1.0,
    "a-news": 1.0,
    "a-hot-money": 0.9,
    "capital": 0.9,
    default: 1.0,
  },
  // 长线决策：价值投资者和分析师权重最高，技术面被削弱
  long: {
    "fundamental": 1.5,
    "a-fundamentals": 1.5,
    "value-investor": 2.0,
    "a-macro": 1.3,
    "macro": 1.3,
    "a-sector": 1.2,
    "research-mgr": 1.5,
    "a-news": 0.7,
    "sentiment": 0.7,
    "a-sentiment": 0.7,
    "a-hot-money": 0.5,
    "capital": 0.5,
    "a-technical": 0.6,
    "a-market": 0.6,
    default: 1.0,
  },
  // 短线决策：技术面、资金面、情绪权重最高
  short: {
    "a-market": 1.5,
    "a-technical": 1.5,
    "a-hot-money": 1.5,
    "capital": 1.5,
    "a-sentiment": 1.3,
    "sentiment": 1.3,
    "a-news": 1.2,
    "value-investor": 0.5,
    "a-fundamentals": 0.6,
    "fundamental": 0.6,
    "a-macro": 0.7,
    default: 1.0,
  },
  // 超短线：资金面、情绪权重最高，基本面几乎不考虑
  ultra_short: {
    "a-hot-money": 2.0,
    "capital": 2.0,
    "a-sentiment": 1.5,
    "sentiment": 1.5,
    "a-news": 1.5,
    "a-market": 1.3,
    "value-investor": 0.3,
    "a-fundamentals": 0.3,
    "fundamental": 0.3,
    "a-macro": 0.3,
    "research-mgr": 0.5,
    default: 1.0,
  },
};

/** 根据分析师 ID 获取时间维度权重 */
function getAnalystWeight(analystId: string, timeHorizon?: string | null): number {
  const weights = ANALYST_TIME_HORIZON_WEIGHT[timeHorizon || "mid"] || ANALYST_TIME_HORIZON_WEIGHT.mid;
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
