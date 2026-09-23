/**
 * 公式决策 ↔ LLM（trader）决策一致性打分 —— 前端**唯一**实现。
 *
 * 后端权威实现：`src-tauri/src/commands/stock_workflow/decision.rs`
 * 的 `compute_decision_agreement`（V65 起 6 维、满分 100）。
 * 本模块是它的前端镜像，用于**后端未产出 `formulaLlmAgreement` 时的降级路径**
 * （旧记录、后端解析失败、rerun 重算）。
 *
 * ⚠️ 为什么必须收敛到单点（2026-09-21 修复）
 *
 * `stockAnalysisStore.ts` 里原有**三份近乎逐字的副本**
 * （loadAnalysis / rerun / workflow-completed），三份都停留在 V45 的 **3 维刻度**
 * （action 50 / positionPct 30 / confidence 20），而后端 V65 早已改为
 * **6 维**（action 30 / positionPct 20 / confidence 15 / riskLevel 15 /
 * data_gaps 10 / evidence 10）。后果：同一个 `decisionAgreementScore` 字段
 * 在两条路径上是**两把尺子**，而全部消费端都按后端刻度解释它 ——
 *   - `agreementTier` / `agreementColor` 的 60/40 档位；
 *   - `DecisionBanner` / `DecisionComparisonPanel` / `CompactDecisionComparison`
 *     的 `agreement < 80`、`< disagreementThreshold(40)` 判定；
 *   ⇒ 降级路径给出的分值与它触发的档位文案互相矛盾。
 *
 * 三份副本还各自带着**后端已修、前端未跟**的三类同源缺陷：
 *   ① **方向判据只认中文字面量**（`s.includes("买")` / `s === "持有"`）：
 *      链路上 action 一旦是英文 `BUY` / `HOLD` / `WAIT`，或 dashboard 的
 *      `强烈买入` 值域短语，全部不命中 ⇒ 落入 `else { actionScore = 0 }`，
 *      把**同向**双方判成「对立方向 0 分」。后端已用 `normalize_action` 修掉
 *      （见 decision.rs 中 P1-6(2026-09-14) 注释），前端未跟。
 *   ② **不可达分支**：`(isHold || isWatch) vs isUncertain = 5` 排在
 *      `isWatch vs isUncertain = 10` 之前，后者**永远命中不到**；且无「缺失哨兵」
 *      分支（后端 `ActionKind::Unavailable`）⇒ 哨兵被当英文串落到「对立 0 分」。
 *   ③ **阈值量纲错**：`confidence` 用 `diff <= 0.1 / 0.2 / 0.4` 判定，而
 *      confidence 两侧都是 **0~100**（后端注释明确）⇒ 该维度除完全相等外几乎恒 0。
 *
 * 本模块按后端**声明分值**逐维复刻，方向判据交给权威归一化器 `parseActionStrict`。
 */
import { extractLlmField, parseJsonLoose } from "./agentOutput";
import { parseActionStrict, parseRiskLevel, StockAction, StockRiskLevel } from "./stock-analysis-utils";
import type { StockActionType } from "./stock-analysis-utils";

/**
 * 各维度满分 —— 与后端 `compute_decision_agreement` 的声明分值**逐一对应**。
 * 改这里必须同步改 `decision.rs`，否则两条路径再次分叉。
 */
export const AGREEMENT_WEIGHTS = {
  action: 30,
  positionPct: 20,
  confidence: 15,
  riskLevel: 15,
  dataGaps: 10,
  evidence: 10,
} as const;

/** 一致性总分上限（各维满分之和，恒为 100） */
export const AGREEMENT_TOTAL_MAX: number = AGREEMENT_WEIGHTS.action
  + AGREEMENT_WEIGHTS.positionPct
  + AGREEMENT_WEIGHTS.confidence
  + AGREEMENT_WEIGHTS.riskLevel
  + AGREEMENT_WEIGHTS.dataGaps
  + AGREEMENT_WEIGHTS.evidence;

/** 单侧（公式 / LLM）参与比对的字段 */
export interface AgreementSide {
  action?: unknown;
  positionPct?: unknown;
  confidence?: unknown;
  riskLevel?: unknown;
  /** 数据缺口清单，字符串数组（字段名 `data_gaps`） */
  dataGaps?: unknown;
  /** 证据引用条数（字段名 `evidence_cited` 的数组长度） */
  evidenceCitedCount?: unknown;
}

/** 逐维得分 + 总分（对应后端 `AgreementBreakdown` 的数值部分） */
export interface AgreementScore {
  action: number;
  positionPct: number;
  confidence: number;
  riskLevel: number;
  dataGaps: number;
  evidence: number;
  total: number;
}

// ── 单维打分（每维都能单独测试）──

function num(v: unknown): number | null {
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

const BULL: readonly StockActionType[] = [StockAction.BUY, StockAction.INCREASE];
const BEAR: readonly StockActionType[] = [StockAction.SELL, StockAction.REDUCE];

function isBull(a: StockActionType): boolean {
  return BULL.includes(a);
}

function isBear(a: StockActionType): boolean {
  return BEAR.includes(a);
}

function isPair(a: StockActionType, b: StockActionType, x: StockActionType, y: StockActionType): boolean {
  return (a === x && b === y) || (a === y && b === x);
}

/**
 * action 一致性（满分 30）。
 *
 * 与后端分支**同序**，逐条对应：
 * 全等 30 > 同向同类 20 > 持有↔观望 10 > 观望↔不确定 6 > 持有↔不确定 3 >
 * 缺失哨兵 15 > 对立 0 > 单侧缺失 / 未识别值域 15。
 */
export function scoreAgreementAction(formulaAction: unknown, llmAction: unknown): number {
  const a = parseActionStrict(formulaAction);
  const b = parseActionStrict(llmAction);
  // 未识别值域（含 null / 自由文本）—— 后端 `normalize_action` 返回 None 时同档
  if (a === null || b === null) { return 15; }
  if (a === b) { return AGREEMENT_WEIGHTS.action; }
  if (isBull(a) && isBull(b)) { return 20; }
  if (isBear(a) && isBear(b)) { return 20; }
  if (isPair(a, b, StockAction.HOLD, StockAction.WAIT)) { return 10; }
  if (isPair(a, b, StockAction.WAIT, StockAction.UNCERTAIN)) { return 6; }
  if (isPair(a, b, StockAction.HOLD, StockAction.UNCERTAIN)) { return 3; }
  // 缺失哨兵是「数据本身没有」，不是方向判断 ⇒ 与「单侧缺失」同档
  if (a === StockAction.UNAVAILABLE || b === StockAction.UNAVAILABLE) { return 15; }
  return 0;
}

/** positionPct 一致性（满分 20）。单侧缺失记 0（后端 V65：不给兜底分，避免虚高）。 */
export function scoreAgreementPosition(formulaPct: unknown, llmPct: unknown): number {
  const a = num(formulaPct);
  const b = num(llmPct);
  if (a === null || b === null) { return 0; }
  const diff = Math.abs(a - b);
  if (diff <= 10) { return AGREEMENT_WEIGHTS.positionPct; }
  if (diff <= 20) { return 10; }
  return 0;
}

/**
 * confidence 一致性（满分 15）。
 *
 * ⚠️ confidence 两侧都是 **0~100**。旧副本用 `diff <= 0.1 / 0.2 / 0.4`
 * （按 0~1 量纲）判定 ⇒ 该维度几乎恒 0。
 */
export function scoreAgreementConfidence(formulaConf: unknown, llmConf: unknown): number {
  const a = num(formulaConf);
  const b = num(llmConf);
  if (a === null || b === null) { return 0; }
  const diff = Math.abs(a - b);
  if (diff <= 10) { return AGREEMENT_WEIGHTS.confidence; }
  if (diff <= 20) { return 10; }
  if (diff <= 40) { return 5; }
  return 0;
}

const RISK_RANK: Record<string, number> = {
  [StockRiskLevel.LOW]: 0,
  [StockRiskLevel.MID]: 1,
  [StockRiskLevel.HIGH]: 2,
  [StockRiskLevel.EXTREME]: 3,
};

/**
 * riskLevel 一致性（满分 15）：精确 15 / 相邻 8 / 跨级 0。
 *
 * 档位值域交给权威归一化器 `parseRiskLevel`（未识别 → MID），
 * 与后端 `risk_rank` 的 `_ => 1`（默认中风险）同档。
 */
export function scoreAgreementRisk(formulaRisk: unknown, llmRisk: unknown): number {
  const a = RISK_RANK[parseRiskLevel(formulaRisk)] ?? 1;
  const b = RISK_RANK[parseRiskLevel(llmRisk)] ?? 1;
  const diff = Math.abs(a - b);
  if (diff === 0) { return AGREEMENT_WEIGHTS.riskLevel; }
  if (diff === 1) { return 8; }
  return 0;
}

function normGap(s: string): string {
  return s.trim().toLowerCase().replace(/[\s/_\u3000]+/g, "");
}

function toGapSet(raw: unknown): Set<string> {
  if (!Array.isArray(raw)) { return new Set(); }
  const out = new Set<string>();
  for (const item of raw) {
    if (typeof item !== "string") { continue; }
    const n = normGap(item);
    if (n !== "") { out.add(n); }
  }
  return out;
}

/**
 * data_gaps 一致性（满分 10）—— 与后端 V75 口径一致：
 * 双方都空 → 满分（确实一致）；仅一方为空 → 不可比，取中性半分 5；
 * 双方非空 → Jaccard × 10。
 */
export function scoreAgreementDataGaps(formulaGaps: unknown, llmGaps: unknown): number {
  const a = toGapSet(formulaGaps);
  const b = toGapSet(llmGaps);
  if (a.size === 0 && b.size === 0) { return AGREEMENT_WEIGHTS.dataGaps; }
  if (a.size === 0 || b.size === 0) { return 5; }
  let intersection = 0;
  for (const g of a) { if (b.has(g)) { intersection += 1; } }
  const union = a.size + b.size - intersection;
  return union > 0 ? (intersection / union) * AGREEMENT_WEIGHTS.dataGaps : AGREEMENT_WEIGHTS.dataGaps;
}

/** evidence_cited 条数一致性（满分 10）：≥3 条满分 / 2 条 5 / 其余 0。 */
export function scoreAgreementEvidence(llmEvidenceCount: unknown): number {
  const n = num(llmEvidenceCount) ?? 0;
  if (n >= 3) { return AGREEMENT_WEIGHTS.evidence; }
  if (n === 2) { return 5; }
  return 0;
}

// ── 入参解析 ──

function pick(obj: Record<string, unknown>, keys: readonly string[]): unknown {
  for (const k of keys) {
    if (obj[k] !== undefined && obj[k] !== null) { return obj[k]; }
  }
  return undefined;
}

/**
 * 从**已解析的对象**构造一侧入参。
 *
 * 字段名容错：`data_gaps` / `dataGaps`、`risk_level` / `riskLevel`、
 * `position_pct` / `positionPct` 均接受（历史落库两种命名都出现过）。
 */
export function agreementSideFromRaw(raw: unknown): AgreementSide | null {
  if (raw === null || typeof raw !== "object" || Array.isArray(raw)) { return null; }
  const o = raw as Record<string, unknown>;
  return {
    action: pick(o, ["action"]),
    positionPct: pick(o, ["positionPct", "position_pct"]),
    confidence: pick(o, ["confidence"]),
    riskLevel: pick(o, ["riskLevel", "risk_level"]),
    dataGaps: pick(o, ["data_gaps", "dataGaps"]),
    evidenceCitedCount: pick(o, ["evidence_cited", "evidenceCited"]),
  };
}

/**
 * LLM（trader）侧入参 —— 直接吃原始 `llm_decision_json` 字符串。
 *
 * 用 `extractLlmField` 取值，与既有三处副本的取值方式一致：可穿透
 * AgentNode 包装 `{role, content: "<json>"}`、markdown 围栏、`{report: "<json>"}` 嵌套。
 * `action` 缺失时回退读 `stance`（历史字段名）。
 *
 * ⚠️ 整段无法解析时返回 `null`（而不是按「全字段缺失」硬算一个分）——
 * 与后端 `serde_json::from_str(...).ok()?` 返回 `None`（不产出分数）同语义。
 */
export function llmAgreementSideFromJson(llmDecisionJson: string | null | undefined): AgreementSide | null {
  if (!llmDecisionJson) { return null; }
  if (parseJsonLoose(llmDecisionJson) === null) { return null; }
  const evidence = extractLlmField(llmDecisionJson, "evidence_cited")
    ?? extractLlmField(llmDecisionJson, "evidenceCited");
  return {
    action: extractLlmField(llmDecisionJson, "action") ?? extractLlmField(llmDecisionJson, "stance"),
    positionPct: extractLlmField(llmDecisionJson, "positionPct"),
    confidence: extractLlmField(llmDecisionJson, "confidence"),
    riskLevel: extractLlmField(llmDecisionJson, "riskLevel"),
    dataGaps: extractLlmField(llmDecisionJson, "data_gaps") ?? extractLlmField(llmDecisionJson, "dataGaps"),
    evidenceCitedCount: Array.isArray(evidence) ? evidence.length : (num(evidence) ?? 0),
  };
}

// ── 对外主入口 ──

/** 逐维计算一致性分解（任一侧缺失时返回 null） */
export function computeAgreement(formula: AgreementSide | null, llm: AgreementSide | null): AgreementScore | null {
  if (!formula || !llm) { return null; }
  const action = scoreAgreementAction(formula.action, llm.action);
  const positionPct = scoreAgreementPosition(formula.positionPct, llm.positionPct);
  const confidence = scoreAgreementConfidence(formula.confidence, llm.confidence);
  const riskLevel = scoreAgreementRisk(formula.riskLevel, llm.riskLevel);
  const dataGaps = scoreAgreementDataGaps(formula.dataGaps, llm.dataGaps);
  const evidence = scoreAgreementEvidence(llm.evidenceCitedCount);
  return {
    action,
    positionPct,
    confidence,
    riskLevel,
    dataGaps,
    evidence,
    total: Math.round(action + positionPct + confidence + riskLevel + dataGaps + evidence),
  };
}

/**
 * 降级路径主入口：公式侧（已解析对象）+ LLM 侧（原始 JSON 字符串）→ 一致性总分。
 *
 * 任一侧无法解析 ⇒ 返回 `null`（不伪造分数）。调用方应在此情形下
 * 保持 `decisionAgreementScore = null`，让 UI 走「无数据」分支而不是显示一个假分数。
 */
export function computeAgreementScore(
  formulaRaw: unknown,
  llmDecisionJson: string | null | undefined,
): number | null {
  const score = computeAgreement(
    agreementSideFromRaw(formulaRaw),
    llmAgreementSideFromJson(llmDecisionJson),
  );
  return score === null ? null : score.total;
}
