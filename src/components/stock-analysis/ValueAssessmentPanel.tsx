// i18n-exempt: 业务逻辑/API 描述/日志字符串，非 UI 展示文本
import { invoke } from "@/lib/invoke";
import { analystBaseOf, horizonSuffix } from "@/lib/stock-analysis-utils";
import { useSettingsStore, useStockAnalysisStore } from "@/stores";
import { ExpandOutlined, LineChartOutlined } from "@ant-design/icons";
import { Alert, Button, Card, Collapse, Empty, Modal, Spin, Tag } from "antd";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ReportMarkdown } from "./ReportMarkdown";
import { cleanToolCallTags, tryBeautifyJson } from "./utils";
import { ValuationBandChart, type ValuationBandData } from "./ValuationBandChart";

/* ------------------------------------------------------------------ */
/*  估值报告 JSON 解析                                                  */
/* ------------------------------------------------------------------ */

/** 粗略检测文本是否看起来像 JSON */
function looksLikeJson(text: string): boolean {
  const trimmed = text.trim();
  return (trimmed.startsWith("{") && trimmed.endsWith("}"))
    || (trimmed.startsWith("[") && trimmed.endsWith("]"));
}

interface ValueReportData {
  type?: string;
  expert?: string;
  business_model?: string;
  moat_rating?: string;
  moat_reasoning?: string;
  financial_health?: string;
  // V74 后 verdict 字段可能是 number/null（margin_of_safety=-100、intrinsic_value_range=null 实证），
  // ReportMarkdown/markstream 只接受 string，传 number 会抛 TypeError 炸掉整页（页面错误兜底）
  intrinsic_value_range?: string | number | null;
  margin_of_safety?: string | number | null;
  buffett_verdict?: string;
  ideal_buy_price?: string;
  risk_flags?: string[];
  // V72(2026-09-10): 现值硬数据（value-investor 直接引用 t-valuation 输出）
  pe?: number | string | null;
  pb?: number | string | null;
  current_price?: number | string | null;
  f_score?: number | string | null;
  moat_score?: number | string | null;
  owner_earnings_yield_pct?: number | string | null;
  value_signal?: string;
  graham_upside_pct?: number | string | null;
  // V73: 结构化基本面硬数据（value-investor 从 t-risk 引用）
  roe_pct?: number | string | null;
  debt_ratio_pct?: number | string | null;
  gross_margin_pct?: number | string | null;
  revenue_growth_yoy_pct?: number | string | null;
  // V92(2026-09-28): **本地算法估值结论**（`value-verify` 无条件注入，顶层键）。
  //   存在理由：`intrinsic_value_range` 是 LLM 口径，实测 301269 现价 87.56 元却给出
  //   「2.16-4.42 元 / -97.5%」——正向 DCF 锚定当期 FCF（收益率 0.14%）时结构性失效。
  //   本块是算法（反向 DCF + 相对估值）的权威结论，冲突时以它为准。
  //   字段名与值由 Rhai 手写，故为 camelCase 子键（非 snake_case）。
  valuation_conclusion?: AlgorithmValuationConclusion | null;
  [key: string]: unknown;
}

/** 算法估值结论（`value-verify.rhai` 的 `algo_conclusion()` 产物形状）。 */
interface AlgorithmValuationConclusion {
  /** 结论档位：低估 / 合理偏低 / 合理 / 偏高 / 高估 / 数据不足。 */
  action?: string;
  /** 一句话结论（含证据），由 `astock-data::valuation::build_conclusion` 生成。 */
  headline?: string;
  /** 主口径：`dcf` / `reverse_dcf` / `relative` / `graham` / `none`。 */
  primaryMethod?: string;
  /** 相对估值的判定（如 `cheap` / `fair` / `rich`）。 */
  relativeVerdict?: string;
  /** 相对估值主指标（`PE` / `PS` / `PB`）。 */
  relativePrimary?: string;
  /** 反向 DCF 可行性：`ok` / `Strained` / `Impossible` 等。 */
  reverseFeasibility?: string;
}

/** 结论档位 → Ant Design Tag 颜色（未知档位回落 default）。 */
function conclusionTagColor(action: string): string {
  switch (action) {
    case "低估":
    case "合理偏低":
      return "green";
    case "合理":
      return "blue";
    case "偏高":
      return "orange";
    case "高估":
      return "red";
    default:
      return "default";
  }
}

/** 结论档位 → Alert 类型；只有明确的「高估」才用 error，避免把数据不足渲染成告警。 */
function conclusionAlertType(action: string): "success" | "info" | "warning" | "error" {
  switch (action) {
    case "低估":
    case "合理偏低":
      return "success";
    case "偏高":
      return "warning";
    case "高估":
      return "error";
    default:
      return "info";
  }
}

/**
 * 从 LLM 输出中提取可读文本
 * 策略：
 * 1. 尝试解析 JSON，成功则按字段提取文本
 * 2. 解析失败则去掉 ```json 代码块标记，直接渲染剩余文本
 *
 * ⚠ I1/J1 确定性闸口（2026-09-28，601399 实证 `de6b0594`；J1 扩至代理锚）：
 *   `gateNoticeKey` 非空时 `intrinsic_value_range` / `margin_of_safety` /
 *   `ideal_buy_price` 一律**不展示数值**，结论区改显该键对应文案——
 *   `dcfNotApplicable`（前提不成立）或 `anchorIsFallback`（锚是近 5 年正净利
 *   均值×0.90 的历史代理，30 天审计该形态给出 0.06–0.37×现价的「估值结论」）。
 *   prompt 硬规则要求前者填 null，但 LLM 实测会填「区间 +（仅供参考）」的
 *   免责声明形态；带说明的区间依然是把被判死/系统性偏低的结果当估值结论展示。
 *   闸口必须在此（数据层、结构化布尔），不能依赖 LLM 守规矩。
 */
function extractReadableText(
  report: string,
  t: (key: string) => string,
  gateNoticeKey: string | null = null,
): string {
  const gated = gateNoticeKey != null;
  // 先尝试解析 JSON
  const parsed = tryParseValueReport(report);
  if (parsed) {
    const parts: string[] = [];
    // V92: 算法结论排在最前 —— 它是本地算法（反向 DCF / 相对估值）的权威输出；
    //   文本兜底路径此前只有 LLM 叙述，冲突时用户读到的是 LLM 口径。
    const algo = parsed.valuation_conclusion;
    if (algo && (algo.headline || algo.action)) {
      const meta = [algo.action, algo.primaryMethod && `口径 ${algo.primaryMethod}`]
        .filter(Boolean)
        .join(" · ");
      parts.push(
        `## ${t("stockAnalysis.valueAssessment.algorithmConclusion")}\n\n${meta}\n\n${algo.headline || ""}`,
      );
    }
    if (parsed.buffett_verdict) {
      parts.push(`## ${t("stockAnalysis.valueAssessment.outlookVerdict")}\n\n${parsed.buffett_verdict}`);
    }
    if (parsed.ideal_buy_price && !gated) {
      parts.push(`${t("stockAnalysis.valueAssessment.idealBuyPriceLabel")}: ${parsed.ideal_buy_price}`);
    }
    if (parsed.business_model) {
      parts.push(`## ${t("stockAnalysis.valueAssessment.businessModel")}\n\n${parsed.business_model}`);
    }
    if (parsed.moat_rating) {
      parts.push(
        `## ${t("stockAnalysis.valueAssessment.moatAssessment")}\n\n${
          t("stockAnalysis.valueAssessment.moatLabel")
        }: ${parsed.moat_rating}\n\n${parsed.moat_reasoning || ""}`,
      );
    }
    if (parsed.financial_health) {
      parts.push(`## ${t("stockAnalysis.valueAssessment.financialHealth")}\n\n${parsed.financial_health}`);
    }
    if (parsed.intrinsic_value_range) {
      parts.push(
        `## ${t("stockAnalysis.valueAssessment.valuationConclusion")}\n\n${
          gated ? t(gateNoticeKey as string) : parsed.intrinsic_value_range
        }`,
      );
    }
    if (!gated && parsed.margin_of_safety != null) { parts.push(asMarkdownText(parsed.margin_of_safety)); }
    // V72: 现值硬数据行
    const metricParts: string[] = [];
    if (parsed.current_price != null) { metricParts.push(`现价 ${parsed.current_price}`); }
    if (parsed.pe != null) { metricParts.push(`PE ${parsed.pe}`); }
    if (parsed.pb != null) { metricParts.push(`PB ${parsed.pb}`); }
    if (parsed.f_score != null) { metricParts.push(`F-Score ${parsed.f_score}/9`); }
    if (parsed.moat_score != null) { metricParts.push(`护城河 ${parsed.moat_score}/100`); }
    if (parsed.owner_earnings_yield_pct != null) { metricParts.push(`OE收益率 ${parsed.owner_earnings_yield_pct}%`); }
    if (parsed.graham_upside_pct != null) { metricParts.push(`格雷厄姆上行 ${parsed.graham_upside_pct}%`); }
    if (parsed.value_signal) { metricParts.push(`综合判断: ${parsed.value_signal}`); }
    // V73: 结构化基本面硬数据
    if (parsed.roe_pct != null) { metricParts.push(`ROE ${parsed.roe_pct}%`); }
    if (parsed.debt_ratio_pct != null) { metricParts.push(`负债率 ${parsed.debt_ratio_pct}%`); }
    if (parsed.gross_margin_pct != null) { metricParts.push(`毛利率 ${parsed.gross_margin_pct}%`); }
    if (parsed.revenue_growth_yoy_pct != null) { metricParts.push(`营收增速 ${parsed.revenue_growth_yoy_pct}%`); }
    if (metricParts.length > 0) {
      parts.push(`## ${t("stockAnalysis.valueAssessment.metricsTitle")}\n\n${metricParts.join(" | ")}`);
    }
    if (Array.isArray(parsed.risk_flags) && parsed.risk_flags.length > 0) {
      parts.push(`## ${t("stockAnalysis.valueAssessment.riskFlags")}\n\n${parsed.risk_flags.join("、")}`);
    }
    const knownFieldsText = parts.filter(Boolean).join("\n\n");
    // 已知字段有内容:直接返回
    if (knownFieldsText) { return knownFieldsText; }
    // 已知字段全空(as-of 模式可能产出非标准 schema,或 schema 字段名变了):
    // 不要 return,继续走下面的递归提取 + 原始 JSON 兜底,避免卡片完全空白
  }

  // 解析失败：去掉 ```json ``` 代码块标记，保留解释文字
  let text = report;
  // 去掉 ```json ... ``` 代码块（内容已通过其他方式处理）
  text = text.replace(/```(?:json)?\s*[\s\S]*?\s*```/g, "");
  // 去掉 tool call 标签
  text = cleanToolCallTags(text);
  const trimmed = text.trim();

  // 如果清理后的文本仍然看起来像 JSON，尝试格式化后返回
  if (looksLikeJson(trimmed)) {
    try {
      const parsed = JSON.parse(trimmed);
      // 递归提取字段
      const fieldParts: string[] = [];
      if (typeof parsed === "object" && parsed !== null) {
        for (const [k, v] of Object.entries(parsed)) {
          if (v != null && typeof v === "string" && v.length > 0) {
            fieldParts.push(`**${k}**: ${v}`);
          } else if (Array.isArray(v) && v.length > 0) {
            fieldParts.push(`**${k}**: ${v.join("、")}`);
          }
        }
      }
      if (fieldParts.length > 0) { return fieldParts.join("\n\n"); }
    } catch {
      // 格式化失败，返回原始文本
      return trimmed;
    }
  }

  return trimmed;
}

/**
 * 尝试逐个字段正则提取（当 JSON.parse 全失败时兜底）
 * LLM 输出的 JSON 中 risk_flags 等数组内常有未转义引号，导致整个 JSON 解析失败，
 * 但顶层字段（business_model / moat_rating / buffett_verdict 等）的字符串值通常是完整的。
 */
function extractFieldsByRegex(text: string): ValueReportData | null {
  const data: ValueReportData = {};
  const patterns: Array<{ key: keyof ValueReportData; pattern: RegExp }> = [
    { key: "expert", pattern: /"expert"\s*:\s*"([^"]+)"/ },
    { key: "type", pattern: /"type"\s*:\s*"([^"]+)"/ },
    { key: "business_model", pattern: /"business_model"\s*:\s*"((?:(?!",\s*"|\n").)+)"/ },
    { key: "moat_rating", pattern: /"moat_rating"\s*:\s*"([^"]+)"/ },
    { key: "moat_reasoning", pattern: /"moat_reasoning"\s*:\s*"((?:(?!",\s*"|\n").)+)"/ },
    { key: "financial_health", pattern: /"financial_health"\s*:\s*"((?:(?!",\s*"|\n").)+)"/ },
    { key: "intrinsic_value_range", pattern: /"intrinsic_value_range"\s*:\s*"((?:(?!",\s*"|\n").)+)"/ },
    { key: "margin_of_safety", pattern: /"margin_of_safety"\s*:\s*"((?:(?!",\s*"|\n").)+)"/ },
    { key: "buffett_verdict", pattern: /"buffett_verdict"\s*:\s*"((?:(?!",\s*"|\n").)+)"/ },
    { key: "ideal_buy_price", pattern: /"ideal_buy_pricee?"\s*:\s*"([^"]+)"/ },
    // V72: 现值硬数据（数字或 null；值可能是数字不带引号）
    { key: "pe", pattern: /"pe"\s*:\s*"?([\d.]+)"?/ },
    { key: "pb", pattern: /"pb"\s*:\s*"?([\d.]+)"?/ },
    { key: "current_price", pattern: /"current_price"\s*:\s*"?([\d.]+)"?/ },
    { key: "f_score", pattern: /"f_score"\s*:\s*"?([\d.]+)"?/ },
    { key: "moat_score", pattern: /"moat_score"\s*:\s*"?([\d.]+)"?/ },
    { key: "owner_earnings_yield_pct", pattern: /"owner_earnings_yield_pct"\s*:\s*"?(-?[\d.]+)"?/ },
    { key: "graham_upside_pct", pattern: /"graham_upside_pct"\s*:\s*"?(-?[\d.]+)"?/ },
    { key: "value_signal", pattern: /"value_signal"\s*:\s*"([^"]+)"/ },
    // V73: 结构化基本面硬数据（数字或 null）
    { key: "roe_pct", pattern: /"roe_pct"\s*:\s*"?(-?[\d.]+)"?/ },
    { key: "debt_ratio_pct", pattern: /"debt_ratio_pct"\s*:\s*"?(-?[\d.]+)"?/ },
    { key: "gross_margin_pct", pattern: /"gross_margin_pct"\s*:\s*"?(-?[\d.]+)"?/ },
    { key: "revenue_growth_yoy_pct", pattern: /"revenue_growth_yoy_pct"\s*:\s*"?(-?[\d.]+)"?/ },
  ];
  for (const { key, pattern } of patterns) {
    const m = text.match(pattern);
    if (m) {
      data[key] = m[1].trim();
    }
  }
  // 尝试提取 risk_flags 数组
  const rfMatch = text.match(/"risk_flags"\s*:\s*\[([\s\S]*?)\]/);
  if (rfMatch) {
    const flags: string[] = [];
    const flagRegex = /"((?:(?!",\s*"|"\]|"\s*\]).)+)"/g;
    let fm: RegExpExecArray | null;
    while ((fm = flagRegex.exec(rfMatch[1])) !== null) {
      const val = fm[1].trim();
      if (val.length > 0 && !val.startsWith("\\")) {
        flags.push(val);
      }
    }
    if (flags.length > 0) {
      data.risk_flags = flags;
    }
  }
  // 字段数太少说明提取失败
  const fieldCount = Object.keys(data).filter((k) => data[k] != null && data[k] !== "").length;
  return fieldCount >= 3 ? data : null;
}
function sanitizeJsonString(raw: string): string {
  return raw
    // 移除对象/数组内的尾部逗号
    .replace(/,\s*([}\]])/g, "$1")
    // 移除换行符之间的多余逗号
    .replace(/",\s*,\s*"/g, '","')
    // 处理字符串中的裸换行（JSON 不允许）
    .replace(/(?<!\\)\n/g, "\\n")
    // 处理字符串中的裸制表符
    .replace(/(?<!\\)\t/g, "\\t");
}

function tryParseValueReport(report: string): ValueReportData | null {
  const errors: string[] = [];
  try {
    const trimmed = report.trim();

    // 收集所有可能的 JSON 候选字符串
    const candidates: string[] = [];

    // 1) 整个字符串就是 JSON
    if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
      candidates.push(trimmed);
    }

    // 2) ```json ``` 代码块（支持多个）
    const codeBlockRegex = /```(?:json)?\s*([\s\S]*?)\s*```/g;
    let m: RegExpExecArray | null;
    while ((m = codeBlockRegex.exec(trimmed)) !== null) {
      candidates.push(m[1].trim());
    }

    // 3) 复用 tryBeautifyJson 容错提取
    const beautified = tryBeautifyJson(report);
    if (beautified !== report) {
      candidates.push(beautified);
    }

    // 4) 手动找第一个 { 到最后一个 }
    const fb = trimmed.indexOf("{");
    const lb = trimmed.lastIndexOf("}");
    if (fb !== -1 && lb !== -1 && lb > fb) {
      candidates.push(trimmed.slice(fb, lb + 1));
    }

    // 去重
    const unique = [...new Set(candidates)];

    console.log("[tryParseValueReport] candidates:", unique.length, unique.map(c => c.slice(0, 80)));

    // 依次尝试解析
    for (const candidate of unique) {
      // 直接解析
      try {
        const parsed = JSON.parse(candidate);
        if (parsed && typeof parsed === "object") {
          console.log("[tryParseValueReport] 解析成功（直接）");
          return flattenVerdictReport(parsed as ValueReportData);
        }
      } catch { /* try next */ }

      // 修复后解析
      try {
        const sanitized = sanitizeJsonString(candidate);
        const parsed = JSON.parse(sanitized);
        if (parsed && typeof parsed === "object") {
          console.log("[tryParseValueReport] 解析成功（修复后）");
          return flattenVerdictReport(parsed as ValueReportData);
        }
      } catch (e) {
        errors.push(`candidate(${candidate.slice(0, 50)}...): ${e instanceof Error ? e.message : e}`);
      }
    }
  } catch (e) {
    errors.push(`outer: ${e instanceof Error ? e.message : e}`);
  }

  if (errors.length > 0) {
    console.warn("[tryParseValueReport] all parses failed:", errors);
  }

  // 最后手段：去掉 tool call 标签后重试一次
  try {
    const cleaned = cleanToolCallTags(report);
    if (cleaned !== report.trim()) {
      const fb2 = cleaned.indexOf("{");
      const lb2 = cleaned.lastIndexOf("}");
      if (fb2 !== -1 && lb2 !== -1 && lb2 > fb2) {
        const candidate = cleaned.slice(fb2, lb2 + 1);
        const parsed = JSON.parse(candidate);
        if (parsed && typeof parsed === "object") {
          console.log("[tryParseValueReport] 解析成功（清理 tool call 后）");
          return flattenVerdictReport(parsed as ValueReportData);
        }
      }
    }
  } catch { /* final fallthrough */ }

  // 最后手段：逐个字段正则提取（容忍未转义引号等 LLM 输出问题）
  const cleanedFull = cleanToolCallTags(report);
  const extracted = extractFieldsByRegex(cleanedFull);
  if (extracted) {
    console.log("[tryParseValueReport] 解析成功（正则兜底）");
    return extracted;
  }

  return null;
}

/**
 * 检测并展开 strict_mode 压缩的 {report, verdict} 格式。
 *
 * 后端 strict_mode 将 LLM 完整扁平 JSON（含 buffett_verdict / moat_rating / financial_health 等）
 * 压缩为 {report: string, verdict: object} 两个顶级键，导致 ValueReportData 的顶层字段全部为 undefined。
 *
 * 此函数在解析成功时将 verdict 子字段提升到顶层，使面板组件能正常读取。
 */
function flattenVerdictReport(data: ValueReportData): ValueReportData {
  if (typeof data.report !== "string" || !data.verdict || typeof data.verdict !== "object") {
    return data;
  }
  const verdict = data.verdict as Record<string, unknown>;
  // 把 verdict 对象的字段复制到顶层（不覆盖 report 本身）
  const keys = Object.keys(verdict);
  if (keys.length > 0) {
    const merged: ValueReportData = { ...data };
    for (const k of keys) {
      if (k !== "report" && !(k in data) || data[k as keyof ValueReportData] === undefined) {
        (merged as Record<string, unknown>)[k] = verdict[k];
      }
    }
    return merged;
  }
  return data;
}

/**
 * 把 LLM verdict 中的任意字段值安全转为 markstream 可渲染的 string。
 * V74 后 margin_of_safety / intrinsic_value_range 等字段是 number 或 null，
 * 直接传给 ReportMarkdown（要求 string）会抛 TypeError 使整页落入「页面错误」兜底。
 */
function asMarkdownText(v: unknown): string {
  if (v == null) { return ""; }
  if (typeof v === "string") { return v; }
  if (typeof v === "number" || typeof v === "boolean") { return String(v); }
  return JSON.stringify(v, null, 2);
}

/** 结构化估值报告渲染 —— 风格与 AnalystReportCard 保持一致 */
function ValueReportRenderer({
  data,
  isDark,
  gateNoticeKey = null,
  rangeIsAlgorithmOutput = false,
  rangeBandIsDecline = false,
}: {
  data: ValueReportData;
  isDark: boolean;
  // I1/J1 闸口语义见 extractReadableText 头注释
  gateNoticeKey?: string | null;
  // V92(2026-09-28 归属修正): `intrinsic_value_range` / `margin_of_safety` **不是 LLM 观点**
  //   —— `value-verify.rhai` 已把它们原地覆写为算法 DCF 的 `low-high` / `upsidePct`，
  //   覆写不命中时也只在「LLM 原文已含算法数值」时才放行 ⇒ 两个字段恒为算法输出。
  //   故有算法结论时标题必须写「算法 DCF 估值区间」，绝不能写「LLM 估值区间」
  //   （实测 600887：用户看到标注 LLM 的 40.16-59.63 元，误以为估值仍由 LLM 产生）。
  rangeIsAlgorithmOutput?: boolean;
  // K3(2026-10-02): 三档增速全部 ≤ 0 ⇒ 区间是「持续衰退带」，标题不得再写「保守档—乐观档」。
  //   与 `gateNoticeKey` 那道闸口正交：闸口拦的是「前提不成立 / 锚是代理」，
  //   而本形态下锚是真的（年报 FCF）、前提是成立的，两道闸口全部放行（600276 实证）。
  rangeBandIsDecline?: boolean;
}) {
  const { t } = useTranslation();
  const gated = gateNoticeKey != null;
  // K3：三档全负时换掉「保守档—乐观档」这个自称（写成链式三元，避免嵌套触发 lint）。
  const rangeBandLabelKey = rangeBandIsDecline && rangeIsAlgorithmOutput
    ? "stockAnalysis.valueAssessment.algorithmRangeBandDecline"
    : rangeIsAlgorithmOutput
    ? "stockAnalysis.valueAssessment.algorithmRangeBand"
    : "stockAnalysis.valueAssessment.valuationConclusion";
  return (
    <div className="space-y-3">
      {/* 展望说明 / 巴菲特裁决 */}
      {data.buffett_verdict && (
        <div>
          <div className="text-xs font-medium mb-1 flex items-center gap-2 flex-wrap" style={{ color: "var(--muted)" }}>
            <span>{t("stockAnalysis.valueAssessment.outlookVerdict")}</span>
            {data.ideal_buy_price && !gated && (
              <Tag color="green">{t("stockAnalysis.valueAssessment.idealBuyPriceLabel")}: {data.ideal_buy_price}</Tag>
            )}
          </div>
          <div className={`prose max-w-none text-sm ${isDark ? "prose-invert" : ""}`}>
            <ReportMarkdown content={asMarkdownText(data.buffett_verdict)} isDark={isDark} />
          </div>
        </div>
      )}

      {/* 商业模式 */}
      {data.business_model && (
        <div>
          <div className="text-xs font-medium mb-1" style={{ color: "var(--muted)" }}>
            {t("stockAnalysis.valueAssessment.businessModel")}
          </div>
          <div className={`prose max-w-none text-xs ${isDark ? "prose-invert" : ""}`}>
            <ReportMarkdown content={asMarkdownText(data.business_model)} isDark={isDark} />
          </div>
        </div>
      )}

      {/* 护城河评估 */}
      {data.moat_rating && (
        <div>
          <div className="text-xs font-medium mb-1" style={{ color: "var(--muted)" }}>
            {t("stockAnalysis.valueAssessment.moatAssessment")}
          </div>
          <div className="flex gap-1 flex-wrap mb-1">
            <Tag color="gold">{t("stockAnalysis.valueAssessment.moatLabel")}: {data.moat_rating}</Tag>
          </div>
          {data.moat_reasoning && (
            <div className={`prose max-w-none text-xs ${isDark ? "prose-invert" : ""}`}>
              <ReportMarkdown content={asMarkdownText(data.moat_reasoning)} isDark={isDark} />
            </div>
          )}
        </div>
      )}

      {/* 财务健康 */}
      {data.financial_health && (
        <div>
          <div className="text-xs font-medium mb-1" style={{ color: "var(--muted)" }}>
            {t("stockAnalysis.valueAssessment.financialHealth")}
          </div>
          <div className={`prose max-w-none text-xs ${isDark ? "prose-invert" : ""}`}>
            <ReportMarkdown content={asMarkdownText(data.financial_health)} isDark={isDark} />
          </div>
        </div>
      )}

      {/* 估值结论 —— I1/J1 闸口：不适用或历史代理锚形态下，区间/安全边际不展示，只说口径 */}
      {(data.intrinsic_value_range || data.margin_of_safety) && (
        <div>
          <div className="text-xs font-medium mb-1" style={{ color: "var(--muted)" }}>
            {t(rangeBandLabelKey)}
          </div>
          {gated
            ? (
              <div className={`prose max-w-none text-xs ${isDark ? "prose-invert" : ""}`}>
                <ReportMarkdown content={t(gateNoticeKey as string)} isDark={isDark} />
              </div>
            )
            : (
              <div className="space-y-1">
                {data.intrinsic_value_range && (
                  <div className={`prose max-w-none text-xs ${isDark ? "prose-invert" : ""}`}>
                    <ReportMarkdown content={asMarkdownText(data.intrinsic_value_range)} isDark={isDark} />
                  </div>
                )}
                {data.margin_of_safety && (
                  <div className={`prose max-w-none text-xs ${isDark ? "prose-invert" : ""}`}>
                    <ReportMarkdown content={asMarkdownText(data.margin_of_safety)} isDark={isDark} />
                  </div>
                )}
              </div>
            )}
        </div>
      )}

      {/* V72: 现值硬数据（value-investor 从 t-valuation 引用） */}
      {(data.pe != null || data.pb != null || data.current_price != null || data.f_score != null
        || data.moat_score != null || data.owner_earnings_yield_pct != null
        || data.value_signal || data.graham_upside_pct != null
        || data.roe_pct != null || data.debt_ratio_pct != null
        || data.gross_margin_pct != null || data.revenue_growth_yoy_pct != null) && (
        <div>
          <div className="text-xs font-medium mb-1" style={{ color: "var(--muted)" }}>
            {t("stockAnalysis.valueAssessment.metricsTitle")}
          </div>
          <div className="flex gap-1 flex-wrap">
            {data.current_price != null && (
              <Tag color="blue">{t("stockAnalysis.valueAssessment.currentPriceLabel")}: {data.current_price}</Tag>
            )}
            {data.pe != null && <Tag color="blue">PE: {data.pe}</Tag>}
            {data.pb != null && <Tag color="blue">PB: {data.pb}</Tag>}
            {data.f_score != null && (
              <Tag color={Number(data.f_score) >= 7 ? "green" : "orange"}>
                {t("stockAnalysis.valueAssessment.fScoreLabel")}: {data.f_score}/9
              </Tag>
            )}
            {data.moat_score != null && (
              <Tag color="gold">{t("stockAnalysis.valueAssessment.moatScoreLabel")}: {data.moat_score}/100</Tag>
            )}
            {data.owner_earnings_yield_pct != null && (
              <Tag color="cyan">
                {t("stockAnalysis.valueAssessment.oeYieldLabel")}: {data.owner_earnings_yield_pct}%
              </Tag>
            )}
            {data.graham_upside_pct != null && (
              <Tag color={Number(data.graham_upside_pct) > 0 ? "red" : "green"}>
                {t("stockAnalysis.valueAssessment.grahamUpsideLabel")}: {data.graham_upside_pct}%
              </Tag>
            )}
            {data.value_signal && (
              <Tag color="purple">{t("stockAnalysis.valueAssessment.valueSignalLabel")}: {data.value_signal}</Tag>
            )}
            {/* V73: 结构化基本面硬数据（t-risk） */}
            {data.roe_pct != null && (
              <Tag color={Number(data.roe_pct) >= 15 ? "green" : "orange"}>
                {t("stockAnalysis.valueAssessment.roeLabel")}: {data.roe_pct}%
              </Tag>
            )}
            {data.debt_ratio_pct != null && (
              <Tag
                color={Number(data.debt_ratio_pct) > 70
                  ? "red"
                  : Number(data.debt_ratio_pct) <= 50
                  ? "green"
                  : "orange"}
              >
                {t("stockAnalysis.valueAssessment.debtRatioLabel")}: {data.debt_ratio_pct}%
              </Tag>
            )}
            {data.gross_margin_pct != null && (
              <Tag color="blue">{t("stockAnalysis.valueAssessment.grossMarginLabel")}: {data.gross_margin_pct}%</Tag>
            )}
            {data.revenue_growth_yoy_pct != null && (
              <Tag color={Number(data.revenue_growth_yoy_pct) > 0 ? "red" : "green"}>
                {t("stockAnalysis.valueAssessment.revenueGrowthLabel")}: {data.revenue_growth_yoy_pct}%
              </Tag>
            )}
          </div>
        </div>
      )}

      {/* 风险标志 */}
      {Array.isArray(data.risk_flags) && data.risk_flags.length > 0 && (
        <div>
          <div className="text-xs font-medium mb-1" style={{ color: "var(--muted)" }}>
            {t("stockAnalysis.valueAssessment.riskFlags")}
          </div>
          <div className="flex gap-1 flex-wrap">
            {data.risk_flags.map((r, i) => <Tag key={i} color="orange">{r}</Tag>)}
          </div>
        </div>
      )}
    </div>
  );
}

/* ------------------------------------------------------------------ */
/*  主组件                                                              */
/* ------------------------------------------------------------------ */

/**
 * 价值投资评估面板
 * 显示 value-investor 节点（巴菲特框架）的输出。
 *
 * 数据来源:
 * - valueAssessments["value-investor--<tier>"]: 各档巴菲特框架评估（v133+ 工作流产出）
 * - valueAssessments["value-investor"]: ≤v132 历史快照的单槽形态
 */

/** 逐档 value 槽位（v133 起 value-investor / value-verify 按持有期实例化）。 */
interface ValueEntry {
  /** store 键：`value-investor--<tier>`（带档）或 `value-investor`（≤v132 历史快照）。 */
  key: string;
  /** 档位（snake）；裸键为 null。 */
  tier: string | null;
  /** 归一为字符串的报告原文。 */
  report: string;
}

/** 档位展示序（与 `Period::ALL` 一致：短 → 长）；裸键排最前。 */
const VALUE_TIER_ORDER = ["ultra_short", "short", "mid", "long"] as const;

function toReportText(value: unknown): string {
  return typeof value === "string"
    ? value
    : value != null
    ? JSON.stringify(value, null, 2)
    : "";
}

/**
 * 从 `valueAssessments` 收集「巴菲特估值」各档产物。
 *
 * ⚠ 四周期下 value 链按档产出（`value-investor--mid|long`），而本面板此前**只读裸键
 * `valueAssessments["value-investor"]`** ⇒ 带档产物全部取不到、`hasValue === false`，
 * 「巴菲特估值」主卡恒不渲染（DB 实证 `a3eba895`：该链只产 `value.assessment--mid|long`，
 * 无裸 `value.assessment`）。此处按 base 归一收集，兼顾 ≤v132 的裸键形态。
 *
 * 快照回放会同时写入 `assessment--<tier>`（别名）与 `value-investor--<tier>`，靠 base 过滤去重。
 */
function collectValueEntries(all: Record<string, unknown>): ValueEntry[] {
  const out: ValueEntry[] = [];
  for (const [key, value] of Object.entries(all)) {
    const base = analystBaseOf(key);
    if (base === null) {
      if (key === "value-investor") {
        const report = toReportText(value);
        if (report.trim().length > 0) { out.push({ key, tier: null, report }); }
      }
      continue;
    }
    if (base !== "value-investor") { continue; }
    const report = toReportText(value);
    if (report.trim().length === 0) { continue; }
    out.push({ key, tier: key.slice(key.indexOf("--") + 2), report });
  }
  out.sort((a, b) => tierOrder(a.tier) - tierOrder(b.tier));
  return out;
}

function tierOrder(tier: string | null): number {
  if (tier === null) { return -1; }
  const i = VALUE_TIER_ORDER.indexOf(tier as (typeof VALUE_TIER_ORDER)[number]);
  return i === -1 ? VALUE_TIER_ORDER.length : i;
}

/** 档位切换标签 —— 复用 `DecisionBanner` 的同一批键（`stockAnalysis.timeHorizon*`）。 */
function valueEntryLabel(tier: string | null, t: (key: string) => string): string {
  const suffix = tier ? horizonSuffix(tier) : null;
  return suffix ? t(`stockAnalysis.timeHorizon${suffix}`) : t("stockAnalysis.valueAssessment.title");
}

export function ValueAssessmentPanel() {
  const { t } = useTranslation();
  const themeMode = useSettingsStore((s) => s.settings.themeMode);
  const isDark = themeMode === "dark"
    || (themeMode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  const valueAssessments = useStockAnalysisStore((s) => s.valueAssessments);
  const ruleCheckResults = useStockAnalysisStore((s) => s.ruleCheckResults);
  const dataQualitySummary = useStockAnalysisStore((s) => s.dataQualitySummary);
  const rawData = useStockAnalysisStore((s) => s.rawData);
  // 估值带需要股票代码：权威来源是 store 顶层 stockCode（loadAnalysis / fetchQuote 都会写）。
  // rawData 的 key 是节点 id（raw-data / combined），从来取不到 stockCode——旧版据此取值，
  // 导致 compute_valuation_band 分支实际从未执行（估值带恒不显示）。rawData 仅作兜底。
  const storeStockCode = useStockAnalysisStore((s) => s.stockCode);
  // 2026-09-21: 估值维度的**适用性 / 锚定口径**（`portfolio-mgr` 产物字段）。
  //   面板上的区间是算法值，但锚定前提此前完全不可见 ⇒ 用户会把
  //   「近 5 年正净利均值 ×0.90」的历史代理锚当成内在价值
  //   （300308 实证：面板显示 80.63–156.77 元，现价 926.43 元）。
  const valuationApplicability = useStockAnalysisStore((s) => s.valuationApplicability);
  // 四周期下 value 链按档产出 ⇒ 收集各档产物并给出档位切换（单档/裸键时不显示切换条）。
  const valueEntries = useMemo(() => collectValueEntries(valueAssessments), [valueAssessments]);
  const [activeValueKey, setActiveValueKey] = useState<string | null>(null);
  const activeValueEntry = valueEntries.find((e) => e.key === activeValueKey) ?? valueEntries[0] ?? null;
  const [expanded, setExpanded] = useState(false);

  // R3-C: 估值带
  const [valuationBand, setValuationBand] = useState<ValuationBandData | null>(null);
  const [valuationBandLoading, setValuationBandLoading] = useState(false);

  useEffect(() => {
    const code = storeStockCode
      || ((rawData?.stockCode as string | undefined) ?? (rawData?.code as string | undefined) ?? "");
    let cancelled = false;
    Promise.resolve().then(() => {
      if (cancelled) { return; }
      if (!code) {
        setValuationBand(null);
        return;
      }
      setValuationBandLoading(true);
      invoke<ValuationBandData>("compute_valuation_band", { stockCode: code, years: 5 })
        .then((d) => {
          if (!cancelled) { setValuationBand(d); }
        })
        .catch((err) => {
          console.warn("[ValueAssessmentPanel] compute_valuation_band failed:", err);
          if (!cancelled) { setValuationBand(null); }
        })
        .finally(() => {
          if (!cancelled) { setValuationBandLoading(false); }
        });
    });
    return () => {
      cancelled = true;
    };
  }, [storeStockCode, rawData]);

  // 类型保护：确保 valueReport 始终是字符串（值已由 collectValueEntries 归一）
  const rawValue = activeValueEntry?.report;
  const valueReport: string = rawValue ?? "";
  const hasValue = valueReport.trim().length > 0;
  const hasRuleCheck = Object.keys(ruleCheckResults).length > 0;
  const hasDataQuality = dataQualitySummary.trim().length > 0;
  const hasRawData = Object.keys(rawData).length > 0;

  // ── 估值前提标注（顺序固定，便于阅读与测试）──
  // 只报「用户看数字时会被误导」的情形；`null`（旧模板/未跑）⇒ 一条不报，
  // 且不显示「已确认适用」——「没这个字段」与「字段说适用」是两件事。
  const applicabilityNotices: Array<{ key: string; tone: "warning" | "info" }> = [];
  if (valuationApplicability) {
    const a = valuationApplicability;
    if (!a.dcfApplicable) {
      applicabilityNotices.push({
        key: "stockAnalysis.valuationApplicability.dcfNotApplicable",
        tone: "warning",
      });
    } else if (a.anchorIsFallback) {
      // 仅当 DCF 腿**仍在参与**时提示锚定口径：已被剔除时上面那条已说明原因，
      // 再报「锚定是代理」属重复（且会让人以为剔除是针对锚定做的）。
      applicabilityNotices.push({
        key: "stockAnalysis.valuationApplicability.anchorIsFallback",
        tone: "warning",
      });
    }
    // K3(2026-10-02)：三档增速全负 ⇒ 区间是「持续衰退带」。与上面两条**并列**而非互斥：
    //   本形态下 dcfApplicable=true、anchorIsFallback=false（600276 实证），
    //   即前两道闸口全部放行，这条是唯一能拦住「乐观档」这个自称的出口。
    //   DCF 腿已被剔除时不再报（区间本来就不展示）。
    if (a.dcfApplicable && a.growthBandAllNegative) {
      applicabilityNotices.push({
        key: "stockAnalysis.valuationApplicability.growthBandAllNegative",
        tone: "warning",
      });
    }
    if (a.grahamGrowthClamped) {
      applicabilityNotices.push({
        key: "stockAnalysis.valuationApplicability.grahamGrowthClamped",
        tone: "info",
      });
    }
    if (!a.dcfLegUsed && !a.grahamLegUsed) {
      applicabilityNotices.push({
        key: "stockAnalysis.valuationApplicability.allLegsExcluded",
        tone: "warning",
      });
    }
  }
  const applicabilityReason = valuationApplicability && !valuationApplicability.dcfApplicable
    ? valuationApplicability.reason
    : "";
  const hasApplicability = applicabilityNotices.length > 0;
  // I1/J1 闸口判据（结构化布尔，不解析 LLM 文案）：
  //   前提不成立 ⇒ 屏蔽并说明不适用；前提成立但锚是历史代理 ⇒ 屏蔽并说明口径
  //   （30 天审计：代理锚形态给出 0.06–0.37×现价的「估值结论」，带警告也是误导）。
  const gateNoticeKey: string | null = valuationApplicability == null
    ? null
    : valuationApplicability.dcfApplicable === false
    ? "stockAnalysis.valuationApplicability.dcfNotApplicable"
    : valuationApplicability.anchorIsFallback
    ? "stockAnalysis.valuationApplicability.anchorIsFallback"
    : null;

  const hasAny = hasValue || hasRuleCheck || hasDataQuality || hasRawData || hasApplicability;

  const parsed = hasValue ? tryParseValueReport(valueReport) : null;
  const readableText = hasValue ? extractReadableText(valueReport, t, gateNoticeKey) : "";
  // V92(2026-09-28): 本地算法估值结论（`value-verify` 无条件注入的顶层键）。
  //   面板此前只渲染 LLM 口径的 `intrinsic_value_range`（301269 给出「-97.5%」），
  //   算法侧结论完全不可见 ⇒ 用户以为那就是结论。此处把它提到最上方。
  const algoConclusion = parsed?.valuation_conclusion ?? null;

  // 暴露调试数据到 window，方便 Console 检查
  useEffect(() => {
    if (typeof window !== "undefined" && hasValue && process.env.NODE_ENV === "development") {
      Object.assign(window, {
        __DEBUG_VALUE__: {
          raw: valueReport.slice(0, 2000),
          parsed,
          parsedType: parsed ? typeof parsed : null,
          readablePreview: readableText.slice(0, 500),
          rawValueType: typeof rawValue,
        },
      });
      console.log("[ValueAssessmentPanel] DEBUG 数据已暴露到 window.__DEBUG_VALUE__");
      console.log("[ValueAssessmentPanel] parsed:", parsed);
      console.log("[ValueAssessmentPanel] rawValue type:", typeof rawValue);
      console.log("[ValueAssessmentPanel] rawValue preview:", String(rawValue).slice(0, 500));
      if (parsed) {
        console.log("[ValueAssessmentPanel] buffett_verdict type:", typeof parsed.buffett_verdict);
        console.log("[ValueAssessmentPanel] buffett_verdict preview:", String(parsed.buffett_verdict).slice(0, 200));
        console.log("[ValueAssessmentPanel] all keys:", Object.keys(parsed));
      } else {
        console.log("[ValueAssessmentPanel] parsed = null，将使用可读文本渲染");
        console.log("[ValueAssessmentPanel] readableText preview:", readableText.slice(0, 500));
        console.log("[ValueAssessmentPanel] looksLikeJson(readableText):", looksLikeJson(readableText));
      }
    }
  }, [valueReport, parsed, readableText, rawValue, hasValue]);

  if (!hasAny) {
    return (
      <div className="p-6">
        <Empty
          description={t("stockAnalysis.valueAssessment.empty")}
          image={Empty.PRESENTED_IMAGE_SIMPLE}
        />
      </div>
    );
  }

  // 渲染内容：优先用结构化数据，失败则用可读文本
  const renderContent = () => {
    if (parsed) {
      return (
        <ValueReportRenderer
          data={parsed}
          isDark={isDark}
          gateNoticeKey={gateNoticeKey}
          rangeIsAlgorithmOutput={algoConclusion != null}
          rangeBandIsDecline={valuationApplicability?.growthBandAllNegative === true}
        />
      );
    }
    // 解析失败：渲染提取后的可读文本
    if (readableText) {
      // 如果可读文本看起来像 JSON，用 <pre> 块渲染（比 NodeRenderer 更清晰）
      if (looksLikeJson(readableText)) {
        return (
          <div>
            <div className="text-xs mb-2" style={{ color: "var(--muted)" }}>
              {t("stockAnalysis.valueAssessment.jsonParseFailed")}：
            </div>
            <pre className="bg-gray-50 dark:bg-gray-900 p-3 rounded text-xs overflow-x-auto whitespace-pre-wrap">
              {readableText}
            </pre>
          </div>
        );
      }
      return (
        <div className={`prose max-w-none text-sm ${isDark ? "prose-invert" : ""}`}>
          <ReportMarkdown content={readableText} isDark={isDark} />
        </div>
      );
    }
    // 都失败：回退到原始文本
    const cleaned = cleanToolCallTags(valueReport);
    if (looksLikeJson(cleaned)) {
      return (
        <div>
          <div className="text-xs mb-2" style={{ color: "var(--muted)" }}>
            {t("stockAnalysis.valueAssessment.jsonRawFallback")}：
          </div>
          <pre className="bg-gray-50 dark:bg-gray-900 p-3 rounded text-xs overflow-x-auto whitespace-pre-wrap">
            {cleaned}
          </pre>
        </div>
      );
    }
    return (
      <div className={`prose max-w-none text-sm ${isDark ? "prose-invert" : ""}`}>
        <ReportMarkdown content={cleaned} isDark={isDark} />
      </div>
    );
  };

  return (
    <div className="p-4 space-y-3">
      {/* R3-C 估值带(在估值报告之上) */}
      {(valuationBand || valuationBandLoading) && (
        <Card
          size="small"
          title={
            <div className="flex items-center gap-2">
              <LineChartOutlined style={{ color: "#f97316" }} />
              <span className="text-sm">{t("stockAnalysis.valuationBand.title")}</span>
              <Tag color="orange" className="m-0 text-xs">PE / PB</Tag>
            </div>
          }
        >
          <Spin spinning={valuationBandLoading} size="small">
            <ValuationBandChart data={valuationBand} loading={valuationBandLoading} />
          </Spin>
        </Card>
      )}

      {
        /* V92(2026-09-28): 算法估值结论 —— 本地算法（反向 DCF / 相对估值）的权威输出，
          必须排在最上方（先结论、后依据、最后才是估值区间与前提标注）。 */
      }
      {algoConclusion && (algoConclusion.headline || algoConclusion.action) && (
        <Alert
          type={conclusionAlertType(algoConclusion.action ?? "")}
          showIcon
          message={
            <div className="flex items-center gap-2 flex-wrap">
              <span>{t("stockAnalysis.valueAssessment.algorithmConclusion")}</span>
              {algoConclusion.action && (
                <Tag color={conclusionTagColor(algoConclusion.action)} className="m-0">
                  {algoConclusion.action}
                </Tag>
              )}
              {algoConclusion.primaryMethod && <Tag className="m-0">口径 {algoConclusion.primaryMethod}</Tag>}
              {algoConclusion.relativePrimary && (
                <Tag className="m-0">
                  {algoConclusion.relativePrimary}
                  {algoConclusion.relativeVerdict ? ` ${algoConclusion.relativeVerdict}` : ""}
                </Tag>
              )}
              {algoConclusion.reverseFeasibility && (
                <Tag className="m-0">反向 DCF {algoConclusion.reverseFeasibility}</Tag>
              )}
            </div>
          }
          description={
            <>
              {algoConclusion.headline && <div>{algoConclusion.headline}</div>}
              <div className="mt-1 opacity-80">
                {t("stockAnalysis.valueAssessment.algorithmConclusionNote")}
              </div>
            </>
          }
        />
      )}

      {
        /* 2026-09-21: 估值前提标注 —— 必须排在数字**之上**，
          否则用户要先读完「内在价值 80.63–156.77 元」才会看到
          「该区间锚定于历史代理、并不成立于当期现金流」。 */
      }
      {hasApplicability && (
        <Alert
          type={applicabilityNotices.some((n) => n.tone === "warning") ? "warning" : "info"}
          showIcon
          message={t("stockAnalysis.valuationApplicability.title")}
          description={
            <>
              <ul className="m-0 pl-4 space-y-0.5">
                {applicabilityNotices.map((n) => <li key={n.key}>{t(n.key)}</li>)}
              </ul>
              {applicabilityReason && (
                <div className="mt-1 opacity-80">
                  {t("stockAnalysis.valuationApplicability.reasonLabel")}：{applicabilityReason}
                </div>
              )}
            </>
          }
        />
      )}

      {
        /* 四周期档位切换：仅多档时出现（单档/裸键无切换条，卡片标题里带档标签） */
      }
      {valueEntries.length > 1 && (
        <div
          data-testid="value-tier-switch"
          className="flex items-center gap-1 flex-wrap"
        >
          {valueEntries.map((e) => {
            const isActive = e.key === (activeValueEntry?.key ?? "");
            return (
              <button
                key={e.key}
                onClick={() => setActiveValueKey(e.key)}
                className="text-sm px-2 py-0.5 rounded font-medium transition-colors"
                style={{
                  background: isActive ? "rgba(250,173,20,0.18)" : "var(--surface)",
                  color: isActive ? "#d48806" : "var(--muted)",
                  border: isActive ? "1px solid rgba(250,173,20,0.45)" : "1px solid var(--border)",
                  cursor: "pointer",
                }}
              >
                {valueEntryLabel(e.tier, t)}
              </button>
            );
          })}
        </div>
      )}

      {hasValue && (
        <Card
          size="small"
          title={
            <div className="flex items-center gap-2">
              <Tag color="gold">{t("stockAnalysis.valueAssessment.buffettLabel")}</Tag>
              <span className="text-sm">{t("stockAnalysis.valueAssessment.title")}</span>
              {parsed?.type && <Tag>{parsed.type}</Tag>}
              {activeValueEntry?.tier && <Tag>{valueEntryLabel(activeValueEntry.tier, t)}</Tag>}
            </div>
          }
          extra={
            <Button
              type="text"
              size="small"
              icon={<ExpandOutlined />}
              onClick={() => setExpanded(true)}
            >
              {t("stockAnalysis.valueAssessment.expand")}
            </Button>
          }
        >
          {renderContent()}
        </Card>
      )}

      {(hasRuleCheck || hasDataQuality || hasRawData) && (
        <Collapse
          ghost
          items={[{
            key: "future",
            label: t("stockAnalysis.valueAssessment.futureFields"),
            children: (
              <div className="space-y-2 text-sm">
                {hasRuleCheck && (
                  <FieldBlock
                    title={t("stockAnalysis.valueAssessment.ruleCheck")}
                    content={JSON.stringify(ruleCheckResults, null, 2)}
                  />
                )}
                {hasDataQuality && (
                  <FieldBlock title={t("stockAnalysis.valueAssessment.dataQuality")} content={dataQualitySummary} />
                )}
                {hasRawData && (
                  <FieldBlock
                    title={t("stockAnalysis.valueAssessment.rawData")}
                    content={JSON.stringify(rawData, null, 2)}
                  />
                )}
              </div>
            ),
          }]}
        />
      )}

      <Modal
        open={expanded}
        onCancel={() => setExpanded(false)}
        footer={null}
        width={800}
        title={t("stockAnalysis.valueAssessment.title")}
      >
        {renderContent()}
      </Modal>
    </div>
  );
}

function FieldBlock({ title, content }: { title: string; content: string }) {
  return (
    <div>
      <div className="text-xs text-gray-500 mb-1">{title}</div>
      <pre className="bg-gray-50 dark:bg-gray-900 p-2 rounded text-xs overflow-x-auto whitespace-pre-wrap">
        {content}
      </pre>
    </div>
  );
}
