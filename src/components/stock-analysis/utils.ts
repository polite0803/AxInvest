/**
 * 股票分析组件共享工具函数
 */

/** 清理 LLM 原始输出中的工具调用 XML 标签（如 minimax:tool_call、openai:tool_call 等）*/
export function cleanToolCallTags(text: string): string {
  if (!text) { return ""; }
  // 移除 <provider:tool_call ...>...</provider:tool_call> 整块内容（含多行）
  let cleaned = text.replace(/<[a-z][\w-]*:tool_call[^>]*>[\s\S]*?<\/[a-z][\w-]*:tool_call>/gi, "");
  // 移除可能残留的自闭合标签
  cleaned = cleaned.replace(/<[a-z][\w-]*:tool_call[^>]*\/?>/gi, "");
  // 清理多余空行
  return cleaned.replace(/\n{3,}/g, "\n\n").trim();
}
