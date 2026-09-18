// SPDX-License-Identifier: AGPL-3.0-only

// 统一的大小 / 时长 / 时间格式化工具。
// 替换各组件中重复实现且口径不一致的 formatBytes / formatDuration / formatTime。

/** 字节数 → 人类可读大小（如 "0 B"、"1.5 MB"）。 */
export function formatBytes(n: number | null | undefined): string {
  if (n == null || n === 0) {
    return "0 B";
  }
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let v = n;
  while (Math.abs(v) >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  const digits = i === 0 ? 0 : Math.abs(v) >= 100 ? 0 : 1;
  return `${v.toFixed(digits)} ${units[i]}`;
}

/** 毫秒 → 人类可读时长（如 "0ms"、"12.3s"、"3m 5s"、"1h 20m"）。 */
export function formatDuration(ms: number | null | undefined): string {
  if (ms == null || ms < 1) {
    return "0ms";
  }
  if (ms < 1000) {
    return `${Math.round(ms)}ms`;
  }
  const s = ms / 1000;
  if (s < 60) {
    return `${s.toFixed(s < 10 ? 1 : 0)}s`;
  }
  const m = Math.floor(s / 60);
  const rs = Math.round(s % 60);
  if (m < 60) {
    return rs === 0 ? `${m}m` : `${m}m ${rs}s`;
  }
  const h = Math.floor(m / 60);
  const rm = Math.round(m % 60);
  return `${h}h ${rm}m`;
}

/** 时间戳 / 日期 → "HH:mm"。非法输入返回 "-"。 */
export function formatTime(ts: number | string | Date | null | undefined): string {
  if (ts == null) {
    return "-";
  }
  const d = ts instanceof Date ? ts : new Date(ts);
  if (Number.isNaN(d.getTime())) {
    return "-";
  }
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  return `${hh}:${mm}`;
}

/** Token 数 → 紧凑表示（如 "950"、"12.3K"、"1.5M"）。 */
export function formatTokens(tokens: number): string {
  if (tokens < 1000) {
    return `${tokens}`;
  }
  if (tokens < 1000000) {
    return `${(tokens / 1000).toFixed(1)}K`;
  }
  return `${(tokens / 1000000).toFixed(1)}M`;
}

/** 计数 → 紧凑表示（如 "1.5K"、"2.3M"），小于 1000 时走本地千分位。 */
export function formatNumber(n: number): string {
  if (n >= 1_000_000) { return `${(n / 1_000_000).toFixed(1)}M`; }
  if (n >= 1_000) { return `${(n / 1_000).toFixed(1)}K`; }
  return n.toLocaleString();
}

/** Token 上限 → 紧凑表示，整数档不带小数（如 "2K"、"1.5M"）。 */
export function formatTokenCount(tokens: number): string {
  if (tokens >= 1000000) {
    const m = tokens / 1000000;
    return m % 1 === 0 ? `${m}M` : `${m.toFixed(1)}M`;
  }
  if (tokens >= 1000) {
    const k = tokens / 1000;
    return k % 1 === 0 ? `${k}K` : `${k.toFixed(1)}K`;
  }
  return `${tokens}`;
}

/** 人民币金额 → 本地化货币字符串（如 "¥1,234.00"）。 */
export function formatCNY(v: number): string {
  return v.toLocaleString("zh-CN", { style: "currency", currency: "CNY" });
}

/**
 * 字节数 → 文件大小（如 "—"、"0 B"、"1.5 MB"）。
 *
 * ⚠ 与 `formatBytes` 存在**三处口径差异，不可互换**（合并前须先做口径决策）：
 *   - 未知值：本函数出 `"—"`；`formatBytes` 出 `"0 B"`
 *   - 100~999 区间：本函数保留 1 位小数（`"123.4 KB"`）；`formatBytes` 取整（`"123 KB"`）
 *   - 单位上限：本函数含 `PB`；`formatBytes` 到 `TB`
 *
 * 因此本函数专供「文件」面板（大小未知时必须显示占位符而非 `0 B`），勿与 `formatBytes` 混用。
 * [2026-09-13] 由 `components/files/{FileList,FilePreview}.tsx` 的两份逐字副本收敛而来。
 */
export function formatFileSize(bytes: number | null | undefined): string {
  if (bytes == null) {
    return "—";
  }
  if (bytes === 0) {
    return "0 B";
  }
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  const i = Math.min(
    Math.floor(Math.log(bytes) / Math.log(1024)),
    units.length - 1,
  );
  return `${(bytes / Math.pow(1024, i)).toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
}
