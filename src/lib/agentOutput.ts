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

/** 清理 LLM 原始输出中的工具调用标签和乱码 */
export function cleanToolCallTags(text: string): string {
  if (!text) { return ""; }
  let cleaned = text;
  // XML 格式：<provider:tool_call>...</provider:tool_call>
  cleaned = cleaned.replace(/<[a-z][\w-]*:tool_call[^>]*>[\s\S]*?<\/[a-z][\w-]*:tool_call>/gi, "");
  cleaned = cleaned.replace(/<[a-z][\w-]*:tool_call[^>]*\/?>/gi, "");
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
  cleaned = cleaned.replace(/\uFFFD+/g, "...");
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
