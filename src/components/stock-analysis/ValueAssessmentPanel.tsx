// i18n-exempt: 业务逻辑/API 描述/日志字符串，非 UI 展示文本
import { invoke } from "@/lib/invoke";
import { useSettingsStore, useStockAnalysisStore } from "@/stores";
import { ExpandOutlined, LineChartOutlined } from "@ant-design/icons";
import { Alert, Button, Card, Collapse, Empty, Modal, Spin, Tag } from "antd";
import { useEffect, useState } from "react";
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
  [key: string]: unknown;
}

/**
 * 从 LLM 输出中提取可读文本
 * 策略：
 * 1. 尝试解析 JSON，成功则按字段提取文本
 * 2. 解析失败则去掉 ```json 代码块标记，直接渲染剩余文本
 */
function extractReadableText(report: string, t: (key: string) => string): string {
  // 先尝试解析 JSON
  const parsed = tryParseValueReport(report);
  if (parsed) {
    const parts: string[] = [];
    if (parsed.buffett_verdict) {
      parts.push(`## ${t("stockAnalysis.valueAssessment.outlookVerdict")}\n\n${parsed.buffett_verdict}`);
    }
    if (parsed.ideal_buy_price) {
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
      parts.push(`## ${t("stockAnalysis.valueAssessment.valuationConclusion")}\n\n${parsed.intrinsic_value_range}`);
    }
    if (parsed.margin_of_safety != null) { parts.push(asMarkdownText(parsed.margin_of_safety)); }
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
function ValueReportRenderer({ data, isDark }: { data: ValueReportData; isDark: boolean }) {
  const { t } = useTranslation();
  return (
    <div className="space-y-3">
      {/* 展望说明 / 巴菲特裁决 */}
      {data.buffett_verdict && (
        <div>
          <div className="text-xs font-medium mb-1 flex items-center gap-2 flex-wrap" style={{ color: "var(--muted)" }}>
            <span>{t("stockAnalysis.valueAssessment.outlookVerdict")}</span>
            {data.ideal_buy_price && (
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

      {/* 估值结论 */}
      {(data.intrinsic_value_range || data.margin_of_safety) && (
        <div>
          <div className="text-xs font-medium mb-1" style={{ color: "var(--muted)" }}>
            {t("stockAnalysis.valueAssessment.valuationConclusion")}
          </div>
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
 * - valueAssessments["value-investor"]: 巴菲特框架评估（工作流产出）
 */
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

  // 类型保护：确保 valueReport 始终是字符串
  const rawValue = valueAssessments["value-investor"];
  const valueReport: string = typeof rawValue === "string"
    ? rawValue
    : rawValue != null
    ? JSON.stringify(rawValue, null, 2)
    : "";
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

  const hasAny = hasValue || hasRuleCheck || hasDataQuality || hasRawData || hasApplicability;

  const parsed = hasValue ? tryParseValueReport(valueReport) : null;
  const readableText = hasValue ? extractReadableText(valueReport, t) : "";

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
      return <ValueReportRenderer data={parsed} isDark={isDark} />;
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

      {hasValue && (
        <Card
          size="small"
          title={
            <div className="flex items-center gap-2">
              <Tag color="gold">{t("stockAnalysis.valueAssessment.buffettLabel")}</Tag>
              <span className="text-sm">{t("stockAnalysis.valueAssessment.title")}</span>
              {parsed?.type && <Tag>{parsed.type}</Tag>}
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
