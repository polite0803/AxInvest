/** AgentExecutor 输出的 JSON 结构 */
interface AgentResult {
  role?: string;
  model?: string;
  content?: string;
  thinking?: string;
  usage?: { input_tokens?: number; output_tokens?: number };
  node_id?: string;
  tool_calls_made?: unknown[];
}

import { parseAction, parseRiskLevel } from "@/lib/stock-analysis-utils";
import type { StockDecision } from "@/types/stock-analysis";

/** 清理 LLM 原始输出中的工具调用标签、think 标签和乱码 */
export function cleanToolCallTags(text: string): string {
  if (!text) { return ""; }
  let cleaned = text;
  // XML 格式：<provider:tool_call>...</provider:tool_call>
  cleaned = cleaned.replace(/<[a-z][\w-]*:tool_call[^>]*>[\s\S]*?<\/[a-z][\w-]*:tool_call>/gi, "");
  cleaned = cleaned.replace(/<[a-z][\w-]*:tool_call[^>]*\/?>/gi, "");
  // 通用 Hermes/Qwen 风格工具调用：<tool_call>...<function=name>...<parameter=name>val</parameter>...</function>...</tool_call>
  // 注意：<function=name> 与 <parameter=name> 使用 "=" 分隔名称（无空格），需与 HTML 标签区分
  cleaned = cleaned.replace(/<tool_call[^>]*>[\s\S]*?<\/tool_call>/gi, "");
  cleaned = cleaned.replace(/<tool_calls?[^>]*\/?>/gi, "");
  cleaned = cleaned.replace(/<\/tool_calls?>/gi, "");
  cleaned = cleaned.replace(/<function[=\s][^>]*>[\s\S]*?<\/function>/gi, "");
  cleaned = cleaned.replace(/<function[=\s][^>]*\/?>/gi, "");
  cleaned = cleaned.replace(/<\/function>/gi, "");
  cleaned = cleaned.replace(/<parameter[=\s][^>]*>[\s\S]*?<\/parameter>/gi, "");
  cleaned = cleaned.replace(/<parameter[=\s][^>]*\/?>/gi, "");
  // Anthropic/Claude 风格 tool_calls（复数+下划线，invoke 子标签）
  cleaned = cleaned.replace(/<tool_calls>[\s\S]*?<\/tool_calls>/gi, "");
  cleaned = cleaned.replace(/<invoke[^>]*>[\s\S]*?<\/invoke>/gi, "");
  cleaned = cleaned.replace(/<invoke[^>]*\/>/gi, "");
  cleaned = cleaned.replace(/<\/parameter>/gi, "");
  // [PROVIDER|tool_calls]...[PROVIDER|/tool_calls] 格式（如 CHAT2API）
  cleaned = cleaned.replace(/\[[A-Z0-9_]+\|tool_calls\][\s\S]*?\[[A-Z0-9_]+\|\/tool_calls\]/gi, "");
  cleaned = cleaned.replace(/\[[A-Z0-9_]+\|invoke[^\]]*\][\s\S]*?\[[A-Z0-9_]+\|\/invoke\]/gi, "");
  cleaned = cleaned.replace(/\[[A-Z0-9_]+\|parameter[^\]]*\][\s\S]*?\[[A-Z0-9_]+\|\/parameter\]/gi, "");
  // <|PROVIDER|tool_calls|>...<|PROVIDER|/tool_calls|> 格式（CHAT2API 变体，外层用 | 闭合）
  cleaned = cleaned.replace(/<\|[A-Z0-9_]+\|tool_calls\|>[\s\S]*?<\|[A-Z0-9_]+\|\/tool_calls\|>/gi, "");
  // 清理所有残留的 <|PROVIDER|...> / </|PROVIDER|...> 标签（invoke/parameter 等内层标签用 > 闭合）
  cleaned = cleaned.replace(/<\|[A-Z0-9_]+\|[\w-]+[^>]*\/?>[\s\S]*?<\/\|[A-Z0-9_]+\|[\w-]+\|?>/gi, "");
  cleaned = cleaned.replace(/<\|[A-Z0-9_]+\|[\w-]+[^>]*\/?>/gi, "");
  cleaned = cleaned.replace(/<\/\|[A-Z0-9_]+\|[\w-]+\|?>/gi, "");
  // 清理 CDATA 包装
  cleaned = cleaned.replace(/<!\[CDATA\[/g, "");
  cleaned = cleaned.replace(/\]\]>/g, "");
  // 清理 UTF-8 替换字符（乱码）
  cleaned = cleaned.replace(/�+/g, "...");
  // 清理 LLM 推理标签：<think>...</think>
  cleaned = cleaned.replace(/<think>[\s\S]*?<\/think>/gi, "");
  cleaned = cleaned.replace(/<think>[\s\S]*?(?=\n|$)/gi, "");
  // 清理后合并多余空行
  return cleaned.replace(/\n{3,}/g, "\n\n").trim();
}

/**
 * 尝试将内容 beautify 为可读 JSON。
 * 支持三种形式：
 * - 转义 JSON 字符串（"{\"a\":1}"）
 * - 普通 JSON 对象/数组（{"a":1}）
 * - 代码块中的 JSON（```json {"a":1} ```）
 */
export function tryBeautifyJson(text: string): string {
  if (!text) { return text; }
  const trimmed = text.trim();
  // 转义 JSON 字符串 "{\"a\":1}"
  if (
    (trimmed.startsWith('"{') && trimmed.endsWith('"'))
    || (trimmed.startsWith('"[') && trimmed.endsWith('"'))
  ) {
    try {
      const unescaped = JSON.parse(trimmed);
      if (typeof unescaped === "string") {
        const parsed = JSON.parse(unescaped);
        return JSON.stringify(parsed, null, 2);
      }
    } catch { /* ignore */ }
  }
  // 普通 JSON（严格匹配）
  if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
    try {
      return JSON.stringify(JSON.parse(trimmed), null, 2);
    } catch { /* 不完美 JSON：尝试提取中间片段 */ }
  }
  // 不完美 JSON 提取：找第一个 { 到最后一个 }（或 [ 到 ]）
  const firstBrace = trimmed.indexOf("{");
  const lastBrace = trimmed.lastIndexOf("}");
  if (firstBrace !== -1 && lastBrace !== -1 && lastBrace > firstBrace) {
    const candidate = trimmed.slice(firstBrace, lastBrace + 1);
    try {
      return JSON.stringify(JSON.parse(candidate), null, 2);
    } catch {
      // 修复常见错误：trailing comma、多余换行
      const fixed = candidate
        .replace(/,\s*}/g, "}")
        .replace(/,\s*\]/g, "]")
        .replace(/\n/g, " ");
      try {
        return JSON.stringify(JSON.parse(fixed), null, 2);
      } catch { /* fallthrough */ }
    }
  }
  const firstBracket = trimmed.indexOf("[");
  const lastBracket = trimmed.lastIndexOf("]");
  if (firstBracket !== -1 && lastBracket !== -1 && lastBracket > firstBracket) {
    const candidate = trimmed.slice(firstBracket, lastBracket + 1);
    try {
      return JSON.stringify(JSON.parse(candidate), null, 2);
    } catch { /* fallthrough */ }
  }
  // 代码块中的 JSON
  const m = trimmed.match(/```(?:json)?\s*([\s\S]*?)\s*```/);
  if (m) {
    try {
      return JSON.stringify(JSON.parse(m[1].trim()), null, 2);
    } catch { /* ignore */ }
  }
  return text;
}

/**
 * 从 AgentExecutor 输出中提取纯文本内容。
 *
 * - 字符串：尝试 beautify JSON（处理转义字符）后返回
 * - 对象：优先取 `content` 字段；若 content 是对象则 JSON.stringify
 * - 其他：JSON.stringify 后回退到 String()
 * - 清理 LLM 工具调用标签
 * - 合并多余空行
 */
export function extractContent(value: unknown): string {
  let text: string;
  if (typeof value === "string") {
    text = value;
  } else if (value && typeof value === "object") {
    const r = value as AgentResult;
    if (typeof r.content === "string" && r.content.length > 0) {
      text = r.content;
    } else if (r.content != null && typeof r.content === "object") {
      text = JSON.stringify(r.content, null, 2);
    } else if (typeof r.content === "string" && r.content.length === 0) {
      // content 为空字符串：说明 LLM 未产生实质输出，不返回 AgentResult 包装 JSON
      text = "";
    } else {
      text = JSON.stringify(value, null, 2);
    }
  } else {
    text = String(value ?? "");
  }
  // 调用统一的清理 + beautify 函数
  text = cleanToolCallTags(text);
  return tryBeautifyJson(text);
}

/**
 * 规范化 decision 对象：兼容 snake_case/camelCase、置信度 0-100、空值保护
 */
export function normalizeDecision(raw: Record<string, unknown>): StockDecision {
  // CodeNode 输出兼容：若顶层字段是 CodeNode 包装（status/result/params/node_id），
  // 从 result 或 params 中提取
  const source: Record<string, unknown> = (!("action" in raw) && !("confidence" in raw) && !raw.result && !raw.params)
    ? raw
    : (!("action" in raw) && !("confidence" in raw) && typeof raw.result === "object" && raw.result !== null)
    ? (raw.result as Record<string, unknown>)
    : (!("action" in raw) && !("confidence" in raw) && typeof raw.params === "object" && raw.params !== null)
    ? (raw.params as Record<string, unknown>)
    : raw;
  const action = parseAction(source.action ?? source["action"]);
  const positionPct = Number(source.positionPct ?? source.position_pct ?? 0);
  const targetPrice = source.targetPrice != null
    ? Number(source.targetPrice)
    : (source.target_price != null ? Number(source.target_price) : null);
  const stopLoss = source.stopLoss != null
    ? Number(source.stopLoss)
    : (source.stop_loss != null ? Number(source.stop_loss) : null);
  const reasoning = String(source.reasoning ?? "");
  const riskLevel = parseRiskLevel(source.riskLevel ?? source.risk_level);
  const confidence = Math.round(Math.max(0, Math.min(100, Number(source.confidence ?? 0))));
  const timeHorizon = String(source.timeHorizon ?? source.time_horizon ?? "") || null;
  const expectedHoldingDays = source.expectedHoldingDays != null
    ? Number(source.expectedHoldingDays)
    : (source.expected_holding_days != null ? Number(source.expected_holding_days) : null);
  const targetTimeframe = String(source.targetTimeframe ?? source.target_timeframe ?? "") || null;
  return {
    action,
    positionPct: isNaN(positionPct) ? 0 : positionPct,
    targetPrice: targetPrice != null && !isNaN(targetPrice) ? targetPrice : null,
    stopLoss: stopLoss != null && !isNaN(stopLoss) ? stopLoss : null,
    reasoning,
    riskLevel,
    confidence,
    timeHorizon: timeHorizon || null,
    expectedHoldingDays: expectedHoldingDays != null && !isNaN(expectedHoldingDays) ? expectedHoldingDays : null,
    targetTimeframe: targetTimeframe || null,
  };
}

/**
 * 尝试从文本中解析 JSON decision（兼容 markdown 代码块包裹）。
 * 返回规范化的 StockDecision，或 null。
 */
function findJsonCandidate(text: string): string | null {
  const trimmed = text.trim();
  const codeBlockMatch = trimmed.match(/```(?:json)?\s*([\s\S]*?)\s*```/);
  if (codeBlockMatch) {
    return codeBlockMatch[1].trim();
  }

  if ((trimmed.startsWith('"{') && trimmed.endsWith('"')) || (trimmed.startsWith('"[') && trimmed.endsWith('"'))) {
    try {
      const unescaped = JSON.parse(trimmed);
      if (typeof unescaped === "string") {
        return unescaped.trim();
      }
    } catch {
      // ignore
    }
  }

  const firstObjectStart = trimmed.indexOf("{");
  const firstArrayStart = trimmed.indexOf("[");
  const openBraceOrder: Array<"{" | "["> = [];
  if (firstObjectStart !== -1 && (firstArrayStart === -1 || firstObjectStart < firstArrayStart)) {
    openBraceOrder.push("{");
  }
  if (firstArrayStart !== -1 && (firstObjectStart === -1 || firstArrayStart < firstObjectStart)) {
    openBraceOrder.push("[");
  }

  for (const openBrace of openBraceOrder) {
    const start = trimmed.indexOf(openBrace);
    if (start === -1) { continue; }
    let depth = 0;
    let inString = false;
    let escaped = false;
    for (let i = start; i < trimmed.length; i += 1) {
      const char = trimmed[i];
      if (char === "\\" && !escaped) {
        escaped = true;
        continue;
      }
      if (char === '"' && !escaped) {
        inString = !inString;
      }
      if (!inString) {
        if (char === openBrace) {
          depth += 1;
        } else if ((openBrace === "{" && char === "}") || (openBrace === "[" && char === "]")) {
          depth -= 1;
          if (depth === 0) {
            return trimmed.slice(start, i + 1).trim();
          }
        }
      }
      escaped = false;
    }
  }

  return null;
}

export function extractDecision(value: unknown): StockDecision | null {
  if (typeof value === "string") {
    return tryParseDecision(value);
  }
  if (value && typeof value === "object" && !Array.isArray(value)) {
    const record = value as Record<string, unknown>;
    const content = record.content;
    if (typeof content === "string") {
      const parsed = tryParseDecision(content);
      if (parsed) {
        return parsed;
      }
    }
    if (content && typeof content === "object" && !Array.isArray(content)) {
      return normalizeDecision(content as Record<string, unknown>);
    }
    // normalizeDecision 内部已处理 CodeNode 的 result/params 包装
    if (
      "action" in record || "confidence" in record || "positionPct" in record || "position_pct" in record
      || "result" in record || "params" in record
    ) {
      return normalizeDecision(record);
    }
    return null;
  }
  return null;
}

export function tryParseDecision(text: string): StockDecision | null {
  const trimmed = text.trim();
  const candidates = [trimmed];
  const extracted = findJsonCandidate(trimmed);
  if (extracted && extracted !== trimmed) {
    candidates.unshift(extracted);
  }
  for (const candidate of candidates) {
    const candidateTrimmed = candidate.trim();
    if (!candidateTrimmed.startsWith("{") && !candidateTrimmed.startsWith("[")) { continue; }
    try {
      const parsed = JSON.parse(candidateTrimmed);
      if (typeof parsed === "object" && parsed !== null && !Array.isArray(parsed)) {
        return normalizeDecision(parsed as Record<string, unknown>);
      }
    } catch { /* try next */ }
  }
  return null;
}
