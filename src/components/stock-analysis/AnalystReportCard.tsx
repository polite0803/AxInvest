// i18n-exempt: 业务逻辑判断字符串，非 UI 展示文本
import { classifyDirectionText, getSignalColor, normalizeVerdictScore } from "@/lib/stock-analysis-utils";
import { useSettingsStore } from "@/stores";
import { ExpandOutlined, SafetyCertificateOutlined } from "@ant-design/icons";
import { Button, Card, Collapse, Empty, Modal, Tag, Tooltip } from "antd";
import { setCustomComponents } from "markstream-react";
import type { RenderContext, RenderNodeFn } from "markstream-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { AnalystDataQualityModal } from "./AnalystDataQualityModal";
import { ReportMarkdown } from "./ReportMarkdown";
import { cleanToolCallTags, tryBeautifyJson } from "./utils";

// ── 自定义段落渲染：用 <div> 替代 <p> ─────────────────────────
// 解决 React DOM 嵌套警告：<div> cannot be a descendant of <p>
// markstream-react FallbackComponent 对未知内联节点类型渲染 <div>，
// 在 <p> 内渲染 <div> 违反 HTML 规范。
// 这里用官方 API 全局注册一个段落组件，用 <div> 包裹内容而非 <p>，
// 同时复用库自身的子节点渲染管线，无需修改 node_modules。
let customParagraphRegistered = false;

function registerCustomParagraph() {
  if (customParagraphRegistered) { return; }
  customParagraphRegistered = true;

  const CustomParagraph: React.FC<{
    node: Record<string, unknown> & { children?: unknown[] };
    ctx?: RenderContext;
    renderNode?: RenderNodeFn;
    indexKey?: React.Key;
  }> = ({ node, ctx, renderNode, indexKey }) => {
    if (!ctx || !renderNode || !node?.children?.length) {
      return <div dir="auto" className="paragraph-node" />;
    }

    const children = (node.children as Array<Record<string, unknown>>).map(
      (child: Record<string, unknown>, i: number) =>
        renderNode(child as Parameters<RenderNodeFn>[0], `${String(indexKey ?? "paragraph")}-${i}`, ctx),
    );

    return <div dir="auto" className="paragraph-node">{children}</div>;
  };

  setCustomComponents({ paragraph: CustomParagraph });
}

registerCustomParagraph();

interface Props {
  expertId: string;
  report: string;
  /** v133：逐档实例的档位标签（如「中线」）；裸 base 卡不传。 */
  tierLabel?: string;
  /**
   * v133：本卡对应的持有期 snake id（`mid` / `long` / …）。
   * 只用于把数据质量弹窗指向 `diagnostics_by_tier` 的**本档那一行**；
   * `tierLabel` 是人话、本字段是键，两者不可互换（译文变了查表就落空）。
   */
  tier?: string | null;
}

/** 宽松解析：支持多种后端 JSON 格式 */
interface ParsedReport {
  // 标准格式
  type?: string;
  summary?: string;
  signals?: string[];
  risk_flags?: string[];
  argument?: string;
  key_points?: string[];
  confidence?: number;
  // 辩论格式
  core_arguments?: string[];
  resonance_points?: string[];
  preempted_counter_attacks?: string[];
  bull_strength_score?: number;
  bear_strength_score?: number;
  data_gaps?: string[];
  // 资金面/筹码面格式
  main_flow_state?: string;
  active_player?: string;
  dragon_tiger_signal?: string;
  limit_up_sustainability?: string;
  bull_score?: number;
  bear_score?: number;
  trigger_bull?: string;
  trigger_bear?: string;
  evidence?: Array<{ point?: string; data?: string; weight?: number }>;
  // 估值格式
  expert?: string;
  business_model?: string;
  moat_rating?: string;
  moat_reasoning?: string;
  financial_health?: string;
  intrinsic_value_range?: string | null;
  margin_of_safety?: string;
  buffett_verdict?: string;
  ideal_buy_price?: string | null;
  catalyst_detail?: string;
  catalyst_level?: string;
  narrative_completeness?: string;
  narrative_missing?: string[];
  institutional_trace?: string;
  concept_risk?: string;
  /** VERDICT 标签里的多空论据（无正文时的唯一可见内容，必须渲染）。 */
  bull_points?: string[];
  bear_points?: string[];
  /** 逐档契约的「该档不适用」说明。 */
  notApplicable?: string[];
  key_events?: Array<{ event?: string; source?: string; stance?: string; weight?: number }>;
  // 通用分析
  analysis?: string;
  assessment?: string;
  verdict?: string;
  reasoning?: string;
  stance?: string;
  /**
   * 逐档 VERDICT 契约的方向字段（harness `holding_period.rs::VERDICT_COMMON_FIELDS`），
   * 与 `verdict` 同义。LLM 直接产出裸因子对象（无 report 正文）时只有 `direction`，
   * 不提升为 `verdict` 会导致结论徽章与置信度整段不渲染。
   */
  direction?: string;
  positionPct?: number;
  action?: string;
  [key: string]: unknown;
}

/**
 * VERDICT 标签 JSON 容错解析。
 *
 * 严格 parse 失败时，唯一允许的修复是**剥离尾随多余 `}`**：2026-10-10 002475 快照实证
 * （a-hot-money--mid / a-research--long）LLM 在完整对象后多吐了 1 个闭合括号
 * （`..."absorbed_in_trend"}}`），后端 agent_executor 的 strip_stray_closers 只对
 * **新跑**的分析生效，历史快照里标签已损坏。逐次剥 1 个尾随 `}` 重试（至多 3 次）——
 * 合法 JSON 在首次严格 parse 即通过，不受影响；损坏串剥掉多余括号后是完整 JSON。
 * 不修缺括号 / 引号类损坏（无法在不臆造内容的前提下安全补全）。
 */
function parseVerdictMetaLenient(seg: string): Record<string, unknown> | null {
  const tryParse = (s: string) => {
    try {
      const obj = JSON.parse(s);
      return obj && typeof obj === "object" && !Array.isArray(obj)
        ? obj as Record<string, unknown>
        : null;
    } catch {
      return null;
    }
  };
  const strict = tryParse(seg);
  if (strict) { return strict; }
  let s = seg.trim();
  for (let i = 0; i < 3; i++) {
    if (!s.endsWith("}")) { break; }
    s = s.slice(0, -1).trim();
    const repaired = tryParse(s);
    if (repaired) { return repaired; }
  }
  return null;
}

/** 解析 `<!-- VERDICT: {...} -->` 格式（分析师自由文本 + 末尾 verdict 标签）*/
function tryParseVerdictFormat(report: string): ParsedReport | null {
  const trimmed = report.trim();
  const verdictIdx = trimmed.indexOf("<!-- VERDICT:");
  if (verdictIdx === -1) { return null; }
  const jsonStr = trimmed.slice(verdictIdx + "<!-- VERDICT:".length);
  const jsonEnd = jsonStr.indexOf("-->");
  if (jsonEnd === -1) {
    // 无闭合标签：不猜测 JSON 边界，只保留正文。
    const bodyOnly = trimmed.slice(0, verdictIdx).trim();
    return bodyOnly.length > 20 ? { summary: bodyOnly, analysis: bodyOnly } as ParsedReport : null;
  }
  const meta = parseVerdictMetaLenient(jsonStr.slice(0, jsonEnd));
  const summary = trimmed.slice(0, verdictIdx).trim();
  if (!meta) {
    // 标签 JSON 无法修复：正文仍然渲染（旧实现行为），但结构化字段不可用。
    if (summary.length > 20) {
      return { summary, analysis: summary } as ParsedReport;
    }
    return null;
  }
  // 兼容三种字段命名：
  //   分析师节点: verdict / bull_score / bear_score / confidence
  //   辩论节点:   stance / strength_score / confidence（无 bear_score）
  //   逐档契约:   direction（harness VERDICT_COMMON_FIELDS 的方向字段，与 verdict 同义）
  const bull = meta.bull_score ?? meta.strength_score ?? undefined;
  const bear = meta.bear_score ?? undefined;
  // 辩论节点的 stance 映射到 verdict
  let verdict = meta.verdict ?? undefined;
  if (!verdict && typeof meta.stance === "string") {
    // 2026-09-21: 方向判据收敛到 `classifyDirectionText`（单一真相源）。
    //   原为内联 `bull|看多` / `bear|看空`，值域比 Grid 窄 ⇒ 同一份
    //   `stance: "买入"` 的研报此处判不出方向、Grid 判为看多。
    const dir = classifyDirectionText(meta.stance);
    const s = String(meta.stance).toLowerCase();
    if (dir === "bull") { verdict = "看多"; }
    else if (dir === "bear") { verdict = "看空"; }
    else if (s.includes("neutral") || s.includes("中性")) { verdict = "中性"; }
    else { verdict = meta.stance; }
  }
  // 标签里只给 direction（"偏多"/"bear"/"中性"）时同样作为结论；徽章层负责本地化/配色，
  // 无法归类的自定义结论如实显示原文。
  if (!verdict && typeof meta.direction === "string" && meta.direction.trim().length > 0) {
    verdict = meta.direction;
  }
  return {
    ...meta,
    verdict,
    bull_score: bull,
    bear_score: bear,
    confidence: meta.confidence ?? undefined,
    summary: summary.length > 0 ? summary : undefined,
    analysis: summary.length > 0 ? summary : undefined,
  } as ParsedReport;
}

function tryParse(report: string): ParsedReport | null {
  try {
    const trimmed = report.trim();
    // 1) 直接是 JSON
    if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
      try {
        return JSON.parse(trimmed);
      } catch { /* try below */ }
    }
    // 2) ```json ``` 代码块
    const m = trimmed.match(/```(?:json)?\s*([\s\S]*?)\s*```/);
    if (m) {
      try {
        return JSON.parse(m[1].trim());
      } catch { /* try below */ }
    }
    // 3) 复用 tryBeautifyJson 的容错提取（处理前面有解释文字、trailing comma 等）
    const beautified = tryBeautifyJson(report);
    if (beautified !== report) {
      // tryBeautifyJson 返回了不同的字符串，说明它做了提取/修复
      try {
        return JSON.parse(beautified);
      } catch { /* fallthrough */ }
    }
    // 4) 手动找第一个 { 到最后一个 }
    const fb = trimmed.indexOf("{");
    const lb = trimmed.lastIndexOf("}");
    if (fb !== -1 && lb !== -1 && lb > fb) {
      try {
        return JSON.parse(trimmed.slice(fb, lb + 1));
      } catch { /* ignore */ }
    }
  } catch { /* not JSON */ }
  return null;
}

/**
 * 剥除正文开头 LLM 写的「文档式大标题」（如 `# 国泰海通（601211）基本面估值分析`）。
 *
 * 背景（2026-10-10 用户要求）：卡片头部已显示「分析师名 + 档位」，LLM 又在正文首行
 * 用 H1/H2 重复「股票名（代码）+ 分析类型」，每张卡都顶着同一个大标题 ⇒ 全部去掉。
 * 仅剥**开头窗口内**（前 5 个非空行，允许一行短引导语，如「已收到——工具数据已足够…」）
 * 第一个满足文档标题特征的 ATX 标题：
 *   ① 含 6 位 A 股代码（`\d{6}`，如 601211 / 002475）；
 *   ② 或以 报告 / 分析 / 评估 / 研判 / 追踪 / 点评 / 研报 结尾。
 * 章节标题（`## 数据概览`、`### 1. 趋势状态判定`）不命中 ⇒ 原样保留，不伤正文结构。
 */
function stripReportTitle(markdown: string): string {
  const lines = markdown.split("\n");
  let nonEmpty = 0;
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].trim().length === 0) { continue; }
    nonEmpty++;
    if (nonEmpty > 5) { break; }
    const m = /^\s{0,3}#{1,6}\s+(.+?)\s*#*\s*$/.exec(lines[i]);
    if (!m) { continue; } // 窗口内允许先出现一句引导语
    const title = m[1].trim();
    const isDocTitle = /\d{6}/.test(title)
      || /(报告|分析|评估|研判|追踪|点评|研报)$/.test(title);
    if (!isDocTitle) { break; } // 第一个标题是章节标题 ⇒ 不存在文档标题，停止
    lines.splice(i, 1);
    return lines.join("\n").replace(/^(?:[ \t]*\n)+/, "");
  }
  return markdown;
}

function extractSummary(parsed: ParsedReport): string {
  // candidate 顺序：先 prose 字段（reasoning / summary / analysis ...），再 report 兜底。
  // P2 修复(2026-07-24): trader 节点的 `report` 字段是内层 JSON 字符串而非 markdown，
  // 排在第一位会导致整张卡片显示 JSON 原文。把 report 放到末尾并跳过 JSON-like 字符串。
  const candidates = [
    parsed.summary,
    parsed.argument,
    parsed.analysis,
    parsed.assessment,
    parsed.buffett_verdict,
    parsed.verdict,
    parsed.reasoning,
    parsed.business_model,
    parsed.moat_reasoning,
    parsed.financial_health,
    parsed.margin_of_safety,
    parsed.catalyst_detail,
    parsed.report,
  ];
  for (const c of candidates) {
    if (typeof c === "string" && c.length > 10) {
      // 跳过看起来像 JSON 的字符串（trader / 包装对象的 inner JSON）
      const trimmed = c.trimStart();
      if (trimmed.startsWith("{") || trimmed.startsWith("[")) { continue; }
      return stripReportTitle(c);
    }
  }
  return "";
}

/** 从 ParsedReport 中提取标签列表 */
function extractTags(parsed: ParsedReport, t: (key: string) => string): string[] {
  if (Array.isArray(parsed.signals) && parsed.signals.length > 0) {
    return parsed.signals;
  }
  const tags: string[] = [];
  if (parsed.stance) { tags.push(parsed.stance); }
  if (parsed.action) { tags.push(parsed.action); }
  if (parsed.main_flow_state) {
    tags.push(`${t("stockAnalysis.analystReport.moneyFlow")}:${parsed.main_flow_state}`);
  }
  if (parsed.dragon_tiger_signal) { tags.push(parsed.dragon_tiger_signal); }
  if (parsed.moat_rating) {
    tags.push(
      `${t("stockAnalysis.analystReport.moat")}:${parsed.moat_rating}`,
    );
  }
  if (parsed.catalyst_level) { tags.push(parsed.catalyst_level); }
  if (parsed.narrative_completeness) {
    tags.push(`${t("stockAnalysis.analystReport.narrative")}:${parsed.narrative_completeness}`);
  }
  if (parsed.institutional_trace) {
    tags.push(`${t("stockAnalysis.analystReport.capital")}:${parsed.institutional_trace}`);
  }
  if (parsed.concept_risk) {
    tags.push(`${t("stockAnalysis.analystReport.conceptRisk")}:${parsed.concept_risk}`);
  }
  // 注：bull_score / bear_score 不再进 tags —— 徽章区已用进度条同时呈现多空分数，
  // 标签里再放一遍「看涨:62」属于同值重复（2026-10-10 格式统一时移除）。
  return tags;
}

/** 标签里的多空论据（仅接受字符串元素）；红绿分区渲染用。 */
function extractStancePoints(parsed: ParsedReport): { bull: string[]; bear: string[] } {
  const pick = (v: unknown): string[] =>
    Array.isArray(v) ? v.filter((p): p is string => typeof p === "string" && p.trim().length > 0) : [];
  return { bull: pick(parsed.bull_points), bear: pick(parsed.bear_points) };
}

/**
 * 判断因子值是否为「纯机器枚举」（如 `k_sigma_band` / `not_assessed` / `downward`）：
 * 仅 ASCII 标识符（可含 _ 与少量分隔的同类 token）、不含中文 ⇒ 等宽字体呈现，
 * 与自然语言中文叙述区分开（机器口径与人工叙述的视觉分界，项目既有惯例）。
 */
function isMachineEnumValue(value: string): boolean {
  const stripped = value.trim();
  if (/[一-鿿]/.test(stripped)) { return false; }
  return /^[a-zA-Z][a-zA-Z0-9]*(?:[_\s,/、-]*[a-zA-Z0-9]+)*$/.test(stripped);
}

/** 从 ParsedReport 中提取要点列表 */
function extractKeyPoints(parsed: ParsedReport): string[] {
  if (Array.isArray(parsed.key_points) && parsed.key_points.length > 0) {
    return parsed.key_points;
  }
  if (Array.isArray(parsed.core_arguments) && parsed.core_arguments.length > 0) {
    return parsed.core_arguments;
  }
  if (Array.isArray(parsed.resonance_points) && parsed.resonance_points.length > 0) {
    return parsed.resonance_points;
  }
  if (Array.isArray(parsed.evidence) && parsed.evidence.length > 0) {
    return parsed.evidence
      .filter((e) => e && typeof e.point === "string")
      .map((e) => `${e.point}${e.data ? ` (${e.data})` : ""}`);
  }
  if (Array.isArray(parsed.data_gaps) && parsed.data_gaps.length > 0) {
    return parsed.data_gaps.slice(0, 3);
  }
  if (Array.isArray(parsed.narrative_missing) && parsed.narrative_missing.length > 0) {
    return parsed.narrative_missing;
  }
  if (Array.isArray(parsed.key_events) && parsed.key_events.length > 0) {
    return parsed.key_events
      .filter((e) => e && typeof e.event === "string")
      .map((e) => `${e.event}${e.source ? ` (${e.source})` : ""}`);
  }
  // 注：
  // ① `bull_points` / `bear_points` 不再并入要点 —— 组件内有独立的红绿分区
  //   （extractStancePoints），合并成一个混合 ul 会丢掉多空归属；
  // ② 逐档契约的 `notApplicable`（因子名数组）不进要点 —— 元素是 camelCase 因子键，
  //   混进要点会露出英文契约名，由 notApplicable / qualified 专门区块中文化呈现。
  return [];
}

/** 从 ParsedReport 中提取风险标志 */
function extractRiskFlags(parsed: ParsedReport): string[] {
  if (Array.isArray(parsed.risk_flags) && parsed.risk_flags.length > 0) {
    return parsed.risk_flags;
  }
  return [];
}

/**
 * 展示层措辞判据：`classifyDirectionText` 对「非多空」一律返回 null，但 null 里混着两类
 * 语义 —— 「中性/观望」应译作本地化中性，而「低估/高估」这类估值结论必须如实显示原文，
 * 不能统一染灰标成「中性」（value-investor 的 `valuation_conclusion.action` 即是此形态）。
 */
const NEUTRAL_VERDICT_PATTERN = /中性|中立|震荡|横盘|观望|持平|不确定|neutral|range[- ]?bound|hold/i;

/**
 * 逐档 VERDICT 契约因子键 —— 值域严格对齐后端唯一权威
 * `src-tauri/crates/harness/src/holding_period.rs` 的 `VERDICT_COMMON_FIELDS`
 * （去掉 direction/confidence 两个结论字段）+ `VERDICT_FACTORS`（11 个逐档因子）。
 *
 * 此处只做「契约键 → 展示标签」映射，**不新造因子定义**（禁区 12）；后端调整因子集时
 * 需同步本表。LLM 直接产出裸因子对象（无 report 正文）时，这些字段是卡片唯一可见内容。
 * 必须声明在 `CONSUMED_KEYS` 之前（后者初始化时展开本表，受 TDZ 约束）。
 */
const VERDICT_FACTOR_KEYS = [
  // VERDICT_COMMON_FIELDS
  "eventCatalyst",
  "exitRule",
  "stopSource",
  "priorSource",
  "positionSource",
  // VERDICT_FACTORS
  "momentumSignal",
  "microstructure",
  "breadthState",
  "flowPersistence",
  "trendStrength",
  "valuationBand",
  "earningsQuality",
  "expectationRevision",
  "macroRegime",
  "supplyShock",
  "sectorRotation",
] as const;

/**
 * 后端「缺席文案键」（harness `holding_period.rs::na_key`）→ 因子键的反查表。
 *
 * 契约要求 `notApplicable` 数组放**因子名**，但实测 LLM 会误放 na_key
 * （如 `["notApplicableMomentum",…]`，2026-10-10 a-hot-money--mid 实证）⇒ 不反查
 * 就会在卡片上裸显英文 camelCase。值域严格对齐后端 `na_key` match 全表（11 项）。
 */
const NA_KEY_TO_FACTOR: Record<string, string> = {
  notApplicableMomentum: "momentumSignal",
  notApplicableMicrostructure: "microstructure",
  notApplicableBreadth: "breadthState",
  notApplicableFlow: "flowPersistence",
  notApplicableTrend: "trendStrength",
  notApplicableValuation: "valuationBand",
  notApplicableQuality: "earningsQuality",
  notApplicableExpectation: "expectationRevision",
  notApplicableMacro: "macroRegime",
  notApplicableSupply: "supplyShock",
  notApplicableSector: "sectorRotation",
};

/** 因子键（或其 na_key 文案键）→ 中文展示标签；无法识别时返回 null（调用方保留原文）。 */
function factorLabelOf(key: string, t: (key: string) => string): string | null {
  const factor = VERDICT_FACTOR_KEYS.includes(key as (typeof VERDICT_FACTOR_KEYS)[number])
    ? key
    : NA_KEY_TO_FACTOR[key];
  if (!factor) { return null; }
  return t(`stockAnalysis.analystReport.factors.${factor}`);
}

/** 已在卡片其他位置消费过的键（兜底平铺时排除，避免重复展示）。 */
const CONSUMED_KEYS = new Set<string>([
  // extractSummary
  "summary",
  "argument",
  "analysis",
  "assessment",
  "buffett_verdict",
  "verdict",
  "reasoning",
  "business_model",
  "moat_reasoning",
  "financial_health",
  "margin_of_safety",
  "catalyst_detail",
  "report",
  // extractTags
  "signals",
  "stance",
  "action",
  "main_flow_state",
  "dragon_tiger_signal",
  "moat_rating",
  "catalyst_level",
  "narrative_completeness",
  "institutional_trace",
  "concept_risk",
  "bull_score",
  "bear_score",
  // extractKeyPoints
  "key_points",
  "core_arguments",
  "resonance_points",
  "evidence",
  "data_gaps",
  "narrative_missing",
  "key_events",
  "bull_points",
  "bear_points",
  "notApplicable",
  // 头部 / 其他
  "type",
  "confidence",
  "positionPct",
  "risk_flags",
  // 逐档 VERDICT 契约因子（extractFactorFields / 专门区块消费，禁止裸 camelCase 平铺）
  "direction",
  "qualified",
  "valuation_conclusion",
  "valuationAudit",
  ...VERDICT_FACTOR_KEYS,
  // 非内容元数据
  "expert",
  "node_id",
  "nodeId",
  "raw",
  "_raw",
  "params",
  "input_params",
  "language",
  "status",
]);

/**
 * 提取逐档契约因子为「中文标签 + 值」的结构化行。
 *
 * 值常带「标记键：中文说明」前缀（如 `qualityRealizationBeyondHorizon：营收高增…`，
 * marker 表示该因子为何只作过滤 / 打折），marker 本身是 camelCase 噪音，仅在紧贴冒号时剥离。
 */
function extractFactorFields(
  parsed: ParsedReport,
  t: (key: string) => string,
): Array<{ key: string; label: string; value: string }> {
  const out: Array<{ key: string; label: string; value: string }> = [];
  for (const key of VERDICT_FACTOR_KEYS) {
    const raw = parsed[key];
    if (raw == null) { continue; }
    let value = "";
    if (typeof raw === "string") { value = raw.trim(); }
    else if (typeof raw === "number" || typeof raw === "boolean") { value = String(raw); }
    else { continue; } // 复杂结构不走因子行
    if (value.length === 0) { continue; }
    value = value.replace(/^[a-z][A-Za-z]*\s*[：:]\s*/, "");
    if (value.length === 0) { continue; }
    out.push({ key, label: t(`stockAnalysis.analystReport.factors.${key}`), value });
  }
  return out;
}

/**
 * `notApplicable` 数组 → 去重后的中文标签列表。
 * 元素两种真实形态都吃：因子键（`momentumSignal`）或后端 na_key（`notApplicableMomentum`）。
 */
function extractNotApplicableLabels(
  parsed: ParsedReport,
  t: (key: string) => string,
): string[] {
  if (!Array.isArray(parsed.notApplicable)) { return []; }
  const labels: string[] = [];
  for (const item of parsed.notApplicable) {
    if (typeof item !== "string" || item.length === 0) { continue; }
    const label = factorLabelOf(item, t) ?? item;
    if (!labels.includes(label)) { labels.push(label); }
  }
  return labels;
}

/**
 * `qualified`（条件采用声明）→ 去重后的因子中文标签列表。
 *
 * 实测两种写法：`qualified: {momentumSignal:"...说明"}`（对象，因子键 → 说明 marker）
 * 与 `qualified: ["momentumSignal"]`（数组，002475 快照为对象）。键无法识别时保留原文。
 * 与 notApplicable 对称：这两个区块是同一套因子集的「缺席 / 条件采用」两种状态，
 * 此前 qualified 被 CONSUMED_KEYS 静默吞掉，用户看不到该信息。
 */
function extractQualifiedLabels(
  parsed: ParsedReport,
  t: (key: string) => string,
): string[] {
  const q = parsed.qualified;
  if (q == null) { return []; }
  const keys: string[] = Array.isArray(q)
    ? q.filter((x): x is string => typeof x === "string")
    : typeof q === "object"
    ? Object.keys(q)
    : [];
  const labels: string[] = [];
  for (const key of keys) {
    if (key.length === 0) { continue; }
    const label = factorLabelOf(key, t) ?? key;
    if (!labels.includes(label)) { labels.push(label); }
  }
  return labels;
}

/**
 * 解析成功、但正文与要点都提不出来时，把其余可读字段平铺出来。
 *
 * 背景：LLM 违规或被截断时会产出「形状未知」的 JSON —— 此前既有分支对它只显示
 * 「分析完成，但未返回结构化内容」，用户完全看不到当次分析到底返回了什么。
 * 此处把未消费的标量 / 字符串数组 / 嵌套对象平铺（最多 8 条），属降级展示。
 */
function extractMiscFields(parsed: ParsedReport): Array<{ k: string; v: string }> {
  const out: Array<{ k: string; v: string }> = [];
  for (const [k, v] of Object.entries(parsed)) {
    if (CONSUMED_KEYS.has(k) || k.startsWith("__") || v == null) { continue; }
    let text = "";
    if (typeof v === "string") {
      text = v.trim();
    } else if (typeof v === "number" || typeof v === "boolean") {
      text = String(v);
    } else if (Array.isArray(v)) {
      text = v.filter((x) => typeof x === "string" || typeof x === "number").join("；");
    } else if (typeof v === "object") {
      text = JSON.stringify(v);
    }
    if (text.length === 0 || text === "{}" || text === "[]") { continue; }
    out.push({ k, v: text.length > 300 ? `${text.slice(0, 300)}…` : text });
    if (out.length >= 8) { break; }
  }
  return out;
}

/** 判断是否为"空分析"（全是 data_gaps 或空字段） */
function isEmptyAnalysis(parsed: ParsedReport): boolean {
  const summary = extractSummary(parsed);
  if (summary.length > 20) { return false; }
  const points = extractKeyPoints(parsed);
  if (points.length > 0) { return false; }
  // 如果只有 data_gaps 且没有实质内容，认为是空的
  const hasContent = Object.keys(parsed).some((k) => {
    const v = parsed[k];
    if (k === "data_gaps") { return false; }
    if (Array.isArray(v)) { return v.length > 0; }
    if (typeof v === "string") { return v.length > 5 && !v.includes("无法"); }
    if (typeof v === "number") { return v !== 0; }
    return v != null;
  });
  return !hasContent;
}

/**
 * 多空论据红绿分区（卡片与展开 Modal 共用，保证两处结构一致）。
 * A 股配色：看涨红、看跌绿，与结论徽章 / 分数同色。
 */
function StancePointsBlock(
  { bull, bear, t, modal = false }: {
    bull: string[];
    bear: string[];
    t: (key: string) => string;
    modal?: boolean;
  },
) {
  if (bull.length === 0 && bear.length === 0) { return null; }
  const ulCls = modal
    ? "list-disc pl-6 my-1 text-sm"
    : "list-disc pl-4 mb-1 text-xs";
  const renderSide = (points: string[], label: string, color: string) => (
    <div className="mt-2">
      <div className={modal ? "text-sm mb-1" : "text-xs mb-1"} style={{ color, fontWeight: 600 }}>
        {label}
      </div>
      <ul className={ulCls} style={{ color: "var(--muted)" }}>
        {points.map((p, i) => <li key={i}>{p}</li>)}
      </ul>
    </div>
  );
  return (
    <>
      {bull.length > 0 && renderSide(bull, t("stockAnalysis.analystReport.bullArgument"), "#f5222d")}
      {bear.length > 0 && renderSide(bear, t("stockAnalysis.analystReport.bearArgument"), "#52c41a")}
    </>
  );
}

export function AnalystReportCard({ expertId, report, tierLabel, tier = null }: Props) {
  const { t } = useTranslation();
  const themeMode = useSettingsStore((s) => s.settings.themeMode);
  const isDark = themeMode === "dark"
    || (themeMode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  let name = t(`stockAnalysis.workflow.analyst.${expertId}`, expertId);
  // 安全兜底：如果 i18n key miss（比如 expertId 含 a- 前缀），剥离后重试
  if (name === expertId && expertId.startsWith("a-")) {
    const fallback = t(`stockAnalysis.workflow.analyst.${expertId.slice(2)}`, expertId.slice(2));
    if (fallback !== expertId.slice(2)) {
      name = fallback;
    }
  }
  const cleanedReport = cleanToolCallTags(report);
  // 优先解析 <!-- VERDICT: {...} --> 格式（分析师自由文本 + 末尾 verdict 标签）
  let parsed = tryParseVerdictFormat(cleanedReport);
  let displayContent: string = "";
  if (parsed) {
    displayContent = cleanedReport;
  } else {
    const beautified = tryBeautifyJson(cleanedReport);
    parsed = tryParse(beautified);
    displayContent = beautified || report || "";
  }
  const [expanded, setExpanded] = useState(false);
  const [dqOpen, setDqOpen] = useState(false);
  const hasContent = !!displayContent || !!parsed;

  const extraBtns = hasContent
    ? (
      <>
        <Button
          type="text"
          size="small"
          icon={<SafetyCertificateOutlined />}
          onClick={() => setDqOpen(true)}
          title={t("stockAnalysis.analystReport.dataQuality")}
        />
        <Button
          type="text"
          size="small"
          icon={<ExpandOutlined />}
          onClick={() => setExpanded(true)}
          title={t("stockAnalysis.expandView")}
        />
      </>
    )
    : null;

  // 有解析结果：尝试结构化渲染
  if (parsed) {
    // 后端占位陈述（"该节点仅给出结论标签…"）面向下游 LLM，不是分析师正文；
    // 卡片改由 __verdict_only 标记渲染本地化提示，故在此摘掉，避免中文串漏进非中文界面。
    if (parsed.__verdict_only === true) { parsed.report = undefined; }
    // strict_mode 下 LLM 输出被重构成 {"report":"...","verdict":{"verdict":"看多","bull_score":7,...}}
    // verdict/bull_score/bear_score/confidence 在嵌套的 verdict 对象里，需要提取到顶层
    if (parsed.verdict && typeof parsed.verdict === "object" && !Array.isArray(parsed.verdict)) {
      const v = parsed.verdict as Record<string, unknown>;
      if (typeof v.verdict === "string") { parsed.verdict = v.verdict; }
      else if (typeof v.stance === "string") { parsed.verdict = v.stance; }
      else {
        // 尝试其他常见字段名
        for (const key of ["direction", "recommendation", "decision"] as const) {
          if (typeof v[key] === "string") {
            parsed.verdict = v[key];
            break;
          }
        }
      }
      if (parsed.bull_score == null && typeof v.bull_score === "number") { parsed.bull_score = v.bull_score as number; }
      if (parsed.bear_score == null && typeof v.bear_score === "number") { parsed.bear_score = v.bear_score as number; }
      if (parsed.confidence == null && typeof v.confidence === "number") { parsed.confidence = v.confidence as number; }
      // report 文本不在 summary 里，在 report 字段
      if (!parsed.summary && typeof parsed.report === "string") { parsed.summary = parsed.report; }
      // 提取其他可能的嵌套字段
      if (parsed.summary == null && typeof v.report === "string") { parsed.summary = v.report; }
      if (parsed.analysis == null && typeof v.analysis === "string") { parsed.analysis = v.analysis; }
      if (parsed.summary == null && typeof v.summary === "string") { parsed.summary = v.summary; }
    }
    // 顶层方向兜底（2026-10-10 002475 实测的两种「结论徽章消失」形态）：
    // ① 裸因子对象 `{direction:"偏多", confidence:72, 因子…}`：无 verdict 字段、无 report
    //    正文（a-fundamentals--mid / a-lockup--mid 等 4 个节点）—— `direction` 是契约里
    //    与 verdict 同义的方向字段（harness VERDICT_COMMON_FIELDS）。
    // ② value-investor 校验外壳：verdict 为空对象 `{}`，结论落在
    //    `valuation_conclusion.action`（如「低估」）。
    // 不兜底则「看多/看空 + 置信度」整段不渲染（徽章区只认字符串 verdict / bull / bear）。
    if (typeof parsed.verdict !== "string") {
      if (typeof parsed.direction === "string" && parsed.direction.trim().length > 0) {
        parsed.verdict = parsed.direction;
      } else {
        const vc = parsed.valuation_conclusion;
        if (vc && typeof vc === "object" && !Array.isArray(vc)) {
          const action = (vc as Record<string, unknown>).action;
          if (typeof action === "string" && action.trim().length > 0) {
            parsed.verdict = action;
          }
        }
      }
    }
    const summary = extractSummary(parsed);
    const tags = extractTags(parsed, t);
    const points = extractKeyPoints(parsed);
    const stancePoints = extractStancePoints(parsed);
    const stanceCount = stancePoints.bull.length + stancePoints.bear.length;
    const riskFlags = extractRiskFlags(parsed);
    const factorFields = extractFactorFields(parsed, t);
    const notApplicableLabels = extractNotApplicableLabels(parsed, t);
    const qualifiedLabels = extractQualifiedLabels(parsed, t);
    const empty = isEmptyAnalysis(parsed);
    // 正文与要点都提不出来时，退而平铺其余可读字段（形状未知的输出至少能看见内容）。
    // 逐档契约因子已在 factorFields 消费，不会再裸平铺。
    const miscFields = summary || points.length > 0 || factorFields.length > 0 || stanceCount > 0
      ? []
      : extractMiscFields(parsed);
    const confidence = typeof parsed.confidence === "number"
      ? normalizeVerdictScore(parsed.confidence)
      : (typeof parsed.positionPct === "number" ? parsed.positionPct : null);

    // 计算看多/看空/中性显眼展示
    const bullScore = typeof parsed.bull_score === "number"
      ? normalizeVerdictScore(parsed.bull_score)
      : null;
    const bearScore = typeof parsed.bear_score === "number"
      ? normalizeVerdictScore(parsed.bear_score)
      : null;
    // 2026-09-21: 方向判据收敛到 `classifyDirectionText`（单一真相源）。
    //   本函数内此前有**两份**判据（配色与下方 i18n key 各一份，相隔仅 4 行），
    //   且值域都比 Grid 窄（仅 `看多|bull`）⇒ 同一份 `verdict: "买入"` 的研报
    //   在 Grid 判为看多（红），此处却落到「中性」并染灰。
    const verdictDir = classifyDirectionText(parsed.verdict);
    const verdictColor = verdictDir === "bull"
      ? "red" // A股红=涨
      : verdictDir === "bear"
      ? "green" // A股绿=跌
      : "default";
    // 徽章文案：明确多空走本地化「看多/看空」；中性词走本地化「中性」；
    // 其余自定义结论（低估 / 高估 / 观望 …）如实显示后端原文，不臆造成中性。
    const verdictLabel = verdictDir === "bull"
      ? t("stockAnalysis.analystVerdict.bullish")
      : verdictDir === "bear"
      ? t("stockAnalysis.analystVerdict.bearish")
      : typeof parsed.verdict === "string" && NEUTRAL_VERDICT_PATTERN.test(parsed.verdict)
      ? t("stockAnalysis.analystVerdict.neutral")
      : (typeof parsed.verdict === "string" ? parsed.verdict : "");
    return (
      <>
        <Card
          size="small"
          className="h-full flex flex-col"
          title={
            <span className="flex items-center gap-2 flex-wrap">
              {name}
              {tierLabel && <Tag style={{ marginLeft: 8 }}>{tierLabel}</Tag>}
              {parsed.type && <Tag style={{ marginLeft: 8 }}>{parsed.type}</Tag>}
              {empty && <Tag color="orange">{t("quant.common.empty")}</Tag>}
            </span>
          }
          extra={extraBtns}
          styles={{ body: { flex: 1, maxHeight: 400, overflow: "auto" } }}
        >
          {/* 看多/看空/置信度 显眼展示 */}
          {(bullScore != null || bearScore != null || (typeof parsed.verdict === "string")) && (
            <div className="flex items-center gap-3 mb-2 flex-wrap">
              {verdictLabel && (
                <Tag color={verdictColor} style={{ fontSize: 13, padding: "2px 10px", fontWeight: 600 }}>
                  {verdictLabel}
                </Tag>
              )}
              {bullScore != null && (
                <span className="text-xs" style={{ color: "var(--red)", fontWeight: 600 }}>
                  {t("stockAnalysis.sentimentBullish")} {Math.round(bullScore)}
                </span>
              )}
              {bearScore != null && (
                <span className="text-xs" style={{ color: "var(--green)", fontWeight: 600 }}>
                  {t("stockAnalysis.sentimentBearish")} {Math.round(bearScore)}
                </span>
              )}
              {confidence != null && (
                /* 自报口径必须标明：它是分析师给自己结论的把握度，没跟后续涨跌对过账，
                   也不是胜率；同屏的「决策置信度」是公式后验，两者不同口径不可互比。 */
                <Tooltip title={t("stockAnalysis.analystConfidenceSelfReported")}>
                  <span className="text-xs" style={{ color: "var(--muted)" }}>
                    {t("stockAnalysis.confidence")} {confidence.toFixed(0)}%
                  </span>
                </Tooltip>
              )}
            </div>
          )}

          {
            /* 只有结论标签、没有正文：后端把「本维度没有分析」那句中文留给了下游 LLM，
              呈现层在此按 __verdict_only 标记出本地化文案（见 agentOutput.reconstructVerdictTag）*/
          }
          {parsed.__verdict_only === true && (
            <div className="text-xs mb-2" style={{ color: "var(--muted)" }}>
              {t("stockAnalysis.analystReport.verdictOnlyBody")}
            </div>
          )}

          {summary && (
            <div className="sa-markdown-content" style={{ marginTop: 4 }}>
              <ReportMarkdown content={summary} isDark={isDark} />
            </div>
          )}
          {points.length > 0 && (
            <ul className="text-xs list-disc pl-4 mb-1 mt-2" style={{ color: "var(--muted)" }}>
              {points.map((p, i) => <li key={i}>{p}</li>)}
            </ul>
          )}
          <StancePointsBlock bull={stancePoints.bull} bear={stancePoints.bear} t={t} />
          {
            /* 逐档契约因子：裸因子 JSON（无 report 正文）时这是卡片唯一的证据内容；
               即使有正文，也把因子按「中文标签：值」结构化呈现，避免 camelCase 裸键平铺。
               纯机器枚举（k_sigma_band / not_assessed 等，无中文）走等宽字体。 */
          }
          {factorFields.length > 0 && (
            <dl className="text-xs mt-2 flex flex-col gap-1 sa-factor-list" style={{ color: "var(--muted)" }}>
              {factorFields.map((f) => (
                <div key={f.key} className="flex gap-1.5">
                  <dt style={{ fontWeight: 600, flexShrink: 0 }}>{f.label}</dt>
                  <dd className={`m-0 min-w-0${isMachineEnumValue(f.value) ? " font-mono" : ""}`}>{f.value}</dd>
                </div>
              ))}
            </dl>
          )}
          {notApplicableLabels.length > 0 && (
            <div className="text-xs mt-2 sa-factor-na" style={{ color: "var(--muted)" }}>
              <span style={{ fontWeight: 600 }}>
                {t("stockAnalysis.analystReport.factorNotApplicable")}
                ：
              </span>
              {notApplicableLabels.join("、")}
            </div>
          )}
          {qualifiedLabels.length > 0 && (
            <div className="text-xs mt-2 sa-factor-qualified" style={{ color: "var(--muted)" }}>
              <span style={{ fontWeight: 600 }}>
                {t("stockAnalysis.analystReport.factorQualified")}
                ：
              </span>
              {qualifiedLabels.join("、")}
            </div>
          )}
          {tags.length > 0 && (
            <div className="flex gap-1 flex-wrap mt-2">
              {tags.map((s, i) => (
                <Tag key={i} color={getSignalColor(s)}>
                  {s}
                </Tag>
              ))}
            </div>
          )}
          {riskFlags.length > 0 && (
            <div className="flex gap-1 flex-wrap mt-2">
              {riskFlags.map((r, i) => <Tag key={i} color="orange">{r}</Tag>)}
            </div>
          )}
          {miscFields.length > 0 && (
            <ul className="text-xs list-disc pl-4 mb-1 mt-2" style={{ color: "var(--muted)" }}>
              {miscFields.map((f, i) => (
                <li key={i}>
                  <span style={{ fontWeight: 600 }}>{f.k}</span> {f.v}
                </li>
              ))}
            </ul>
          )}
          {!parsed.verdict && bullScore == null && bearScore == null && !summary && points.length === 0
            && stanceCount === 0 && factorFields.length === 0
            && notApplicableLabels.length === 0 && qualifiedLabels.length === 0
            && miscFields.length === 0 && (
            <div className="text-xs" style={{ color: "var(--muted)" }}>
              {t("stockAnalysis.analystReport.completedNoStructure")}
            </div>
          )}
        </Card>
        <Modal
          title={name}
          open={expanded}
          onCancel={() => setExpanded(false)}
          footer={null}
          width="80vw"
          style={{ top: 20 }}
          styles={{ body: { maxHeight: "80vh", overflow: "auto" } }}
        >
          <div className="sa-markdown-content">
            {summary && <ReportMarkdown content={summary} isDark={isDark} />}
            {points.length > 0 && (
              <ul className="list-disc pl-6 my-2">
                {points.map((p, i) => <li key={i}>{p}</li>)}
              </ul>
            )}
            <StancePointsBlock bull={stancePoints.bull} bear={stancePoints.bear} t={t} modal />
            {factorFields.length > 0 && (
              <dl className="flex flex-col gap-1 my-2 text-sm">
                {factorFields.map((f) => (
                  <div key={f.key} className="flex gap-2">
                    <dt style={{ fontWeight: 600, flexShrink: 0 }}>{f.label}</dt>
                    <dd className={`m-0 min-w-0${isMachineEnumValue(f.value) ? " font-mono" : ""}`}>{f.value}</dd>
                  </div>
                ))}
              </dl>
            )}
            {notApplicableLabels.length > 0 && (
              <div className="text-xs my-2" style={{ color: "var(--muted)" }}>
                <span style={{ fontWeight: 600 }}>
                  {t("stockAnalysis.analystReport.factorNotApplicable")}
                  ：
                </span>
                {notApplicableLabels.join("、")}
              </div>
            )}
            {qualifiedLabels.length > 0 && (
              <div className="text-xs my-2" style={{ color: "var(--muted)" }}>
                <span style={{ fontWeight: 600 }}>
                  {t("stockAnalysis.analystReport.factorQualified")}
                  ：
                </span>
                {qualifiedLabels.join("、")}
              </div>
            )}
            <Collapse
              defaultActiveKey={[]}
              items={[{
                key: "raw",
                label: t("stockAnalysis.analystReport.viewRawData"),
                children: (
                  <pre
                    style={{
                      background: "var(--surface-2)",
                      padding: 12,
                      borderRadius: 6,
                      fontSize: 12,
                      overflow: "auto",
                      maxHeight: "50vh",
                      margin: 0,
                    }}
                  >
                    <code>{displayContent}</code>
                  </pre>
                ),
              }]}
              style={{ marginTop: 12 }}
            />
          </div>
        </Modal>
        <AnalystDataQualityModal
          name={name}
          expertId={expertId}
          open={dqOpen}
          onClose={() => setDqOpen(false)}
          tier={tier}
        />
      </>
    );
  }

  // 无解析结果：尝试从原始文本中"模糊"提取关键信息
  const fuzzy = ((): { points: string[]; tags: string[]; summary: string; empty: boolean } => {
    const text = displayContent || "";
    const points: string[] = [];
    const tags: string[] = [];
    let summary = "";

    // 提取 evidence[].point
    const evMatch = text.match(/"evidence"\s*:\s*\[([\s\S]*?)\]/);
    if (evMatch) {
      const items = evMatch[1].match(/"point"\s*:\s*"([^"]*)"/g);
      if (items) {
        items.forEach((item) => {
          const m = item.match(/"point"\s*:\s*"([^"]*)"/);
          if (m && m[1].length > 3) { points.push(m[1]); }
        });
      }
    }

    // 提取 core_arguments[]
    const coreMatch = text.match(/"core_arguments"\s*:\s*\[([\s\S]*?)\]/);
    if (coreMatch) {
      const raw = coreMatch[1];
      // 匹配 "string" 或 { "point": "..." }
      const strItems = raw.match(/"([^"\n]{5,})"/g);
      if (strItems) {
        strItems.forEach((item) => {
          const clean = item.replace(/^"|"$/g, "");
          if (clean.length > 5 && !points.includes(clean)) { points.push(clean); }
        });
      }
      const objItems = raw.match(/"point"\s*:\s*"([^"]*)"/g);
      if (objItems) {
        objItems.forEach((item) => {
          const m = item.match(/"point"\s*:\s*"([^"]*)"/);
          if (m && m[1].length > 3 && !points.includes(m[1])) { points.push(m[1]); }
        });
      }
    }

    // 提取 key_points[]
    const kpMatch = text.match(/"key_points"\s*:\s*\[([\s\S]*?)\]/);
    if (kpMatch) {
      const items = kpMatch[1].match(/"([^"\n]{5,})"/g);
      if (items) {
        items.forEach((item) => {
          const clean = item.replace(/^"|"$/g, "");
          if (clean.length > 5 && !points.includes(clean)) { points.push(clean); }
        });
      }
    }

    // 提取 data_gaps（作为数据不足提示）
    const gapMatch = text.match(/"data_gaps"\s*:\s*\[([\s\S]*?)\]/);
    if (gapMatch && points.length === 0) {
      const items = gapMatch[1].match(/"([^"\n]{5,})"/g);
      if (items && items.length > 0) {
        const first = items[0].replace(/^"|"$/g, "");
        if (first.length > 5) { summary = `${t("stockAnalysis.analystReport.dataLimited")}:${first}`; }
      }
    }

    // 提取 bull_score / bear_score（归一化到百分制）
    const bullMatch = text.match(/"bull_score"\s*:\s*(\d+)/);
    const bearMatch = text.match(/"bear_score"\s*:\s*(\d+)/);
    if (bullMatch) {
      const raw = parseInt(bullMatch[1], 10);
      const normalized = normalizeVerdictScore(raw);
      tags.push(`${t("stockAnalysis.sentimentBullish")}:${Math.round(normalized)}`);
    }
    if (bearMatch) {
      const raw = parseInt(bearMatch[1], 10);
      const normalized = normalizeVerdictScore(raw);
      tags.push(`${t("stockAnalysis.sentimentBearish")}:${Math.round(normalized)}`);
    }

    // 提取 stance / action
    const stanceMatch = text.match(/"stance"\s*:\s*"([^"]*)"/);
    if (stanceMatch) { tags.push(stanceMatch[1]); }
    const actionMatch = text.match(/"action"\s*:\s*"([^"]*)"/);
    if (actionMatch) { tags.push(actionMatch[1]); }

    // 提取 summary
    const summaryMatch = text.match(/"summary"\s*:\s*"([^"]*)"/);
    if (summaryMatch && summaryMatch[1].length > 10) {
      summary = summaryMatch[1];
    }
    // 提取 argument
    if (!summary) {
      const argMatch = text.match(/"argument"\s*:\s*"([^"]*)"/);
      if (argMatch && argMatch[1].length > 10) { summary = argMatch[1]; }
    }
    // 提取 analysis
    if (!summary) {
      const anaMatch = text.match(/"analysis"\s*:\s*"([^"]*)"/);
      if (anaMatch && anaMatch[1].length > 10) { summary = anaMatch[1]; }
    }

    // 纯文本 fallback：内容不像 JSON 但有实质文字，提取前 200 字作为摘要
    if (!summary && points.length === 0 && tags.length === 0) {
      // 压缩空白前先按多行形态剥掉文档式大标题（剥成单行后标题会与正文粘在一起无法分辨）
      const detitled = stripReportTitle(text);
      const cleaned = detitled
        .replace(/由于上游工具调用返回了.*?的错误[，。]/g, "")
        .replace(/根据系统指令.*?[，。]/g, "")
        .replace(/我的职责是.*?[，。]/g, "")
        .replace(/在上游数据缺失.*?[，。]/g, "")
        .replace(/我无法获取.*?[，。]/g, "")
        .replace(/我必须诚实反映.*?[，。]/g, "")
        .replace(/以下是基于当前可用上下文.*?[，。]/g, "")
        .replace(/请注意，由于缺乏.*?[，。]/g, "")
        .replace(/我需要调用.*?[，。]/g, "")
        .replace(/让我先调用.*?[，。]/g, "")
        .replace(/首先我需要.*?[。]/g, "")
        .replace(/用户要求.*?进行.*?[。]/g, "")
        .replace(/用户需要.*?[。]/g, "")
        .replace(/^\s*[-*]\s+/gm, "") // markdown list
        .replace(/\s+/g, " ")
        .trim();
      if (cleaned.length > 30) {
        summary = cleaned.slice(0, 200) + (cleaned.length > 200 ? "..." : "");
      } else if (!detitled.trim().startsWith("{") && !detitled.trim().startsWith("[") && detitled.length > 30) {
        summary = detitled.slice(0, 200) + "...";
      }
    }

    const empty = points.length === 0 && !summary && tags.length === 0;
    // JSON 字段路径抠出的 summary 也统一剥文档标题
    return { points, tags, summary: summary ? stripReportTitle(summary) : "", empty };
  })();

  return (
    <>
      <Card
        size="small"
        className="h-full flex flex-col"
        title={
          <span>
            {name}
            {fuzzy.empty && <Tag color="orange" style={{ marginLeft: 8 }}>{t("quant.common.empty")}</Tag>}
          </span>
        }
        extra={extraBtns}
        styles={{ body: { flex: 1, maxHeight: 400, overflow: "auto" } }}
      >
        {fuzzy.summary && (
          <div className="sa-markdown-content">
            <ReportMarkdown content={fuzzy.summary} isDark={isDark} />
          </div>
        )}
        {fuzzy.points.length > 0 && (
          <ul className="text-xs list-disc pl-4 mb-1 mt-1" style={{ color: "var(--muted)" }}>
            {fuzzy.points.map((p, i) => <li key={i}>{p}</li>)}
          </ul>
        )}
        {fuzzy.tags.length > 0 && (
          <div className="flex gap-1 flex-wrap mt-1">
            {fuzzy.tags.map((s, i) => (
              <Tag key={i} color={getSignalColor(s)}>
                {s}
              </Tag>
            ))}
          </div>
        )}
        {fuzzy.empty && displayContent && (
          <div className="text-xs" style={{ color: "var(--muted)" }}>
            {t("quant.common.empty")}
          </div>
        )}
        {!displayContent && (
          <Empty
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description={
              <span style={{ color: "var(--muted)", fontSize: 12 }}>
                {t("stockAnalysis.noReport")}
              </span>
            }
          />
        )}
      </Card>
      <Modal
        title={name}
        open={expanded}
        onCancel={() => setExpanded(false)}
        footer={null}
        width="80vw"
        style={{ top: 20 }}
        styles={{ body: { maxHeight: "80vh", overflow: "auto" } }}
      >
        <div className="sa-markdown-content">
          {fuzzy.summary && <ReportMarkdown content={fuzzy.summary} isDark={isDark} />}
          {fuzzy.points.length > 0 && (
            <ul className="list-disc pl-6 my-2">
              {fuzzy.points.map((p, i) => <li key={i}>{p}</li>)}
            </ul>
          )}
          <Collapse
            defaultActiveKey={[]}
            items={[{
              key: "raw",
              label: t("stockAnalysis.analystReport.viewRawData"),
              children: (
                <pre
                  style={{
                    background: "var(--surface-2)",
                    padding: 12,
                    borderRadius: 6,
                    fontSize: 12,
                    overflow: "auto",
                    maxHeight: "50vh",
                    margin: 0,
                  }}
                >
                  <code>{displayContent}</code>
                </pre>
              ),
            }]}
            style={{ marginTop: 12 }}
          />
        </div>
      </Modal>
      <AnalystDataQualityModal
        name={name}
        expertId={expertId}
        open={dqOpen}
        onClose={() => setDqOpen(false)}
        tier={tier}
      />
    </>
  );
}
