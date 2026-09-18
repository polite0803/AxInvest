import NodeRenderer from "markstream-react";

interface ReportMarkdownProps {
  content: string;
  isDark?: boolean;
}

/**
 * 把任意形态的 content 收敛为 string。
 *
 * markstream 解析器内部直接对 content 调 `startsWith`，收到非 string（number / object / null）
 * 会抛 `TypeError: initialMarkdown.startsWith is not a function`，把整页渲染树打崩，
 * 由 PageErrorBoundary 兜底成「页面错误」。
 *
 * 实测来源：LLM 结构化输出（value-investor 等 verdict 字段）常出现 number（-100）、
 * null、object 等形态，而下游各处默认按 string 传参。类型标注管不住运行时 JSON，
 * 因此在本组件（stock-analysis 报告渲染的唯一入口）做统一收敛。
 */
function toMarkdownString(content: unknown): string {
  if (typeof content === "string") { return content; }
  if (content == null) { return ""; }
  if (typeof content === "number" || typeof content === "boolean") { return String(content); }
  return JSON.stringify(content, null, 2) ?? "";
}

/**
 * 静态报告 Markdown 渲染封装（分析师卡片 / 辩论卡片 / 风险矩阵 / 估值面板等）。
 *
 * markstream-react 默认面向"流式 token"场景，两个默认值会坑到静态完整内容：
 *   1. deferNodesUntilVisible 默认 true —— 节点进入视口才真正渲染，否则显示占位；
 *   2. final 默认 undefined      —— 解析器认为流未结束，尾部未闭合构造保持 loading 占位。
 *
 * 这些卡片渲染的是**已完整**的报告文本，且外层普遍是 `maxHeight + overflow:auto` 的
 * 滚动容器。内容一长，后半部分节点落在滚动裁剪区、永远不进入视口 →
 * IntersectionObserver 判定"不可见" → 永久停留在"待显示"占位，一直渲染不出来。
 *
 * 因此这里固定：
 *   - final={true}                  告诉解析器内容已完整，别再挂 loading 占位；
 *   - deferNodesUntilVisible={false} 禁用视口懒渲染，完整内容一次性全部渲染。
 */
export function ReportMarkdown({ content, isDark }: ReportMarkdownProps) {
  // 运行时收敛（类型标注挡不住 LLM JSON 的任意形态）
  const text = toMarkdownString(content as unknown);
  return (
    <NodeRenderer
      content={text}
      isDark={isDark}
      final
      deferNodesUntilVisible={false}
    />
  );
}
