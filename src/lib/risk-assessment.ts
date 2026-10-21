// i18n-exempt: 纯解析逻辑，不含 UI 文案
/**
 * 风险产物解析 —— 主面板（RiskMatrix）、chat 卡片与导出报告共用。
 *
 * 只做取值，不做打分：
 *   · 三视角 = 三个评估者各自的 verdict（stance / position_pct / confidence）+ 正文；
 *   · 规则档 = `risk-level` 的结构化 JSON（category / matched_rules / axisScope / metrics）；
 *   · 收敛 / 研究经理 = 正文。
 *
 * 为什么不再有「风险分」：三视角的 confidence 是**评估置信度**，不是风险高低；
 * 把它当风险分并染红绿，会与同一屏的规则档 `category=极高风险` 直接打架
 * （现网 002156：三视角 45/55/45，规则档极高风险）。各视角的差异在仓位与立场上，
 * 结构化字段原样上屏即可，不需要跨视角归一。
 */

import { cleanToolCallTags } from "@/lib/agentOutput";

/** 三视角的固定顺序与标签（主面板与 chat 卡片共用，不跟随事件/JSON 顺序） */
export const RISK_VIEW_ROWS = [
  { key: "risk-agg", labelKey: "stockAnalysis.risk.aggressive" },
  { key: "risk-con", labelKey: "stockAnalysis.risk.conservative" },
  { key: "risk-neu", labelKey: "stockAnalysis.risk.neutral" },
] as const;

export const RISK_VIEW_KEYS: readonly string[] = RISK_VIEW_ROWS.map((row) => row.key);

const STANCE_LABEL_KEYS: Record<string, string> = {
  aggressive: "stockAnalysis.risk.aggressive",
  conservative: "stockAnalysis.risk.conservative",
  neutral: "stockAnalysis.risk.neutral",
};

/** verdict.stance → i18n key；未知值返回 null（调用方原样显示，不猜） */
export function riskStanceLabelKey(stance: string | null): string | null {
  if (!stance) { return null; }
  return STANCE_LABEL_KEYS[stance.toLowerCase()] ?? null;
}

export interface RiskVerdict {
  stance: string | null;
  positionPct: number | null;
  confidence: number | null;
}

export interface RiskAssessmentView {
  verdict: RiskVerdict;
  /** 报告正文；解析成功但没有正文时为空串（不把 JSON 原文当正文） */
  report: string;
}

export interface RiskLevelView {
  category: string | null;
  matchedRules: string[];
  axisScope: Record<string, string>;
  metrics: Record<string, number | null>;
  degraded: boolean;
  warnings: string[];
}

function unwrapAgentResult(raw: string): string {
  let text = raw.trim();
  for (let depth = 0; depth < 2; depth++) {
    if (!text.startsWith("{")) { break; }
    try {
      const obj = JSON.parse(text) as Record<string, unknown>;
      if (typeof obj.content !== "string") { break; }
      const wrapped = typeof obj.role === "string"
        || typeof obj.model === "string"
        || typeof obj.node_id === "string";
      if (!wrapped) { break; }
      text = obj.content.trim();
    } catch {
      break;
    }
  }
  return text;
}

function tryParseObject(text: string): Record<string, unknown> | null {
  const trimmed = text.trim();
  const candidates = [trimmed];
  const fenced = trimmed.match(/```(?:json)?\s*([\s\S]*?)```/);
  if (fenced) { candidates.push(fenced[1].trim()); }
  for (const candidate of candidates) {
    if (!candidate.startsWith("{")) { continue; }
    try {
      const parsed = JSON.parse(candidate);
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
        return parsed as Record<string, unknown>;
      }
    } catch {
      /* 试下一个候选 */
    }
  }
  return null;
}

function readNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function readString(value: unknown): string | null {
  return typeof value === "string" && value.trim().length > 0 ? value.trim() : null;
}

/** 三视角 / 收敛 / 研究经理：verdict 字段 + 正文（verdict 可为顶层或嵌套） */
export function parseRiskAssessment(raw: string): RiskAssessmentView {
  const inner = unwrapAgentResult(cleanToolCallTags(raw));
  const obj = tryParseObject(inner);
  if (!obj) {
    // 纯 Markdown / 自然语言正文
    return {
      verdict: { stance: null, positionPct: null, confidence: null },
      report: inner,
    };
  }
  const verdict = obj.verdict && typeof obj.verdict === "object" && !Array.isArray(obj.verdict)
    ? obj.verdict as Record<string, unknown>
    : obj;
  return {
    verdict: {
      stance: readString(verdict.stance),
      positionPct: readNumber(verdict.position_pct ?? verdict.positionPct),
      confidence: readNumber(verdict.confidence),
    },
    report: readString(obj.report) ?? readString(obj.text) ?? "",
  };
}

/**
 * 规则风险档（`risk-level` / `cls-risk-level`）。
 * 认不出任何结构化字段 ⇒ null（调用方按「不是规则档的普通产物」处理，不静默丢）。
 */
export function parseRiskLevel(raw: string): RiskLevelView | null {
  const obj = tryParseObject(unwrapAgentResult(cleanToolCallTags(raw)));
  if (!obj) { return null; }

  const category = readString(obj.category);
  const matchedRules = Array.isArray(obj.matched_rules)
    ? obj.matched_rules.filter((x): x is string => typeof x === "string")
    : [];

  const axisScope: Record<string, string> = {};
  if (obj.axisScope && typeof obj.axisScope === "object" && !Array.isArray(obj.axisScope)) {
    for (const [key, value] of Object.entries(obj.axisScope as Record<string, unknown>)) {
      if (typeof value === "string") { axisScope[key] = value; }
    }
  }

  const metrics: Record<string, number | null> = {};
  if (obj.metrics && typeof obj.metrics === "object" && !Array.isArray(obj.metrics)) {
    for (const [key, value] of Object.entries(obj.metrics as Record<string, unknown>)) {
      if (value === null || typeof value === "number") { metrics[key] = value as number | null; }
    }
  }

  const warnings = Array.isArray(obj.warnings)
    ? obj.warnings.filter((x): x is string => typeof x === "string")
    : [];

  if (
    category === null
    && matchedRules.length === 0
    && Object.keys(axisScope).length === 0
    && Object.keys(metrics).length === 0
  ) {
    return null;
  }

  return {
    category,
    matchedRules,
    axisScope,
    metrics,
    degraded: obj.degraded === true,
    warnings,
  };
}

/** 聚合容器：live 路径也会把 `risk-aggregated` 塞进 riskAssessments，它不是第四个视角 */
export function isRiskAggregateKey(key: string): boolean {
  return key === "risk-aggregated" || key === "agg-risk";
}
