/**
 * 股票分析组件共享工具函数
 *
 * 统一导出自 @/lib/agentOutput，避免重复实现。
 * 所有组件通过 `./utils` 导入保持向后兼容。
 */
export { cleanToolCallTags, tryBeautifyJson } from "@/lib/agentOutput";
