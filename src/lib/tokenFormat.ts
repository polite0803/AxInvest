// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Token / 时长 / 速度的紧凑格式化（纯函数，无 React / 无 store 依赖）。
 *
 * 2026-11-07（任务 #193，组件中立性 B 类）由 `src/components/gateway/tokenFormat.ts` 下沉至此：
 * 原位置属 gateway 特性目录，却被 `src/components/chat/**` 5 处跨域引用
 * （AssistantFooter / ChatMarkdownNodes / ChatViewToolbar / ContextBar / ContextClassificationBar），
 * 触发组件中立性门禁。函数只做数值格式化，不读任何 feature store ⇒ 归 `src/lib` 共用基元。
 *
 * ⚠ 与 `src/lib/format.ts` 的 `formatTokenCount` / `formatDuration` **不是同一套输出口径**
 * （本文件用小写 k/m 与 `1m2s`，format.ts 用大写 K/M 与 `1m 2s`，且 formatDuration 的入参语义不同）。
 * 本批是**搬家不改语义**，两套口径的合流需单独评估展示回归，未在本批做。
 */

function formatCompact(value: number, divisor: number, suffix: string): string {
  const compact = Math.round((value / divisor) * 10) / 10;
  const formatted = Number.isInteger(compact)
    ? compact.toFixed(0)
    : compact.toFixed(1);
  return `${formatted}${suffix}`;
}

export function formatTokenCount(value: number): string {
  const abs = Math.abs(value);
  if (abs >= 1_000_000) {
    return formatCompact(value, 1_000_000, "m");
  }
  if (abs >= 1_000) {
    return formatCompact(value, 1_000, "k");
  }
  return value.toLocaleString();
}

/** Format milliseconds into human-readable duration: 100ms, 1.1s, 1m2s */
export function formatDuration(ms: number): string {
  if (ms < 1000) {
    return `${Math.round(ms)}ms`;
  }
  if (ms < 60_000) {
    const s = ms / 1000;
    return `${Number.isInteger(s) ? s.toFixed(0) : s.toFixed(1)}s`;
  }
  const m = Math.floor(ms / 60_000);
  const s = Math.round((ms % 60_000) / 1000);
  return s > 0 ? `${m}m${s}s` : `${m}m`;
}

/** Format tokens per second: 28.5 tok/s */
export function formatSpeed(tps: number): string {
  return `${tps < 10 ? tps.toFixed(1) : Math.round(tps)} tok/s`;
}
