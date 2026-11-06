// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 薄 re-export —— 实现已于 2026-11-07（任务 #193，组件中立性 B 类）下沉到
 * `src/lib/tokenFormat.ts`。
 *
 * 为什么搬：本文件原属 gateway 特性目录，却被 `src/components/chat/**` 5 处跨域引用
 * （AssistantFooter / ChatMarkdownNodes / ChatViewToolbar / ContextBar /
 * ContextClassificationBar），触发组件中立性门禁。函数只做数值格式化、不读任何 feature
 * store ⇒ 归 `src/lib` 共用基元。
 *
 * 为什么保留本文件：`docs/audits/AUDIT-duplication-2026-09-13.md` 仍按
 * `gateway/tokenFormat.ts:23` 记录「formatDuration 定义点」的取证位置；直接删文件会让
 * `check-doc-refs` 判 BROKEN。文档改指属另一批（docs 不在本任务可改范围）。
 *
 * 导出的三个函数（实现见新位置）：
 *   · formatTokenCount(value)  1 234 → "1.2k"（与 `src/lib/format.ts` 的大写 K/M 口径**不同**）
 *   · formatDuration(ms)        62 000 → "1m2s"（与 format.ts 的 "1m 2s" 口径**不同**）
 *   · formatSpeed(tps)          28.5 → "28.5 tok/s"
 *
 * ⚠ 本文件只剩转发，不再是「格式化函数的家」。新增代码请直接 import
 * `@/lib/tokenFormat`，不要从这里转一道（gateway → lib 是对的，chat → gateway 不是）。
 */
export * from "@/lib/tokenFormat";
