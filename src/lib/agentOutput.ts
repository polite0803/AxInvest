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

/**
 * 从 AgentExecutor 输出中提取纯文本内容。
 *
 * - 字符串直接返回
 * - 对象：优先取 `content` 字段；若 content 是对象则 JSON.stringify
 * - 其他：JSON.stringify 后回退到 String()
 * - 清理 LLM 工具调用 XML 标签（如 `<minimax:tool_call>...</minimax:tool_call>`）
 * - 合并多余空行
 */
export function extractContent(value: unknown): string {
  let text = "";
  if (typeof value === "string") {
    text = value;
  } else if (value && typeof value === "object") {
    const r = value as AgentResult;
    if (typeof r.content === "string" && r.content.length > 0) {
      text = r.content;
    } else if (r.content != null && typeof r.content === "object") {
      text = JSON.stringify(r.content);
    } else {
      text = JSON.stringify(value);
    }
  } else {
    text = String(value ?? "");
  }
  // 清理 LLM 工具调用 XML 标签（如 <minimax:tool_call>...</minimax:tool_call>）
  text = text.replace(/<[a-z][\w-]*:tool_call[^>]*>[\s\S]*?<\/[a-z][\w-]*:tool_call>/gi, "");
  text = text.replace(/<[a-z][\w-]*:tool_call[^>]*\/?>/gi, "");
  return text.replace(/\n{3,}/g, "\n\n").trim();
}
