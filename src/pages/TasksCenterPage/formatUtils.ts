// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 任务中心时间 / 耗时展示工具（计划任务表格与手动作业历史共用）。
 * 纯函数、无 React 依赖，便于单测。
 */

/** ISO 字符串或 epoch ms → epoch ms；无法解析返回 null。 */
export function toEpochMs(value: string | number | null | undefined): number | null {
  if (value === null || value === undefined || value === "") {
    return null;
  }
  if (typeof value === "number") {
    return Number.isFinite(value) ? value : null;
  }
  const ms = Date.parse(value);
  return Number.isNaN(ms) ? null : ms;
}

/**
 * 相对时间（"3 分钟前"），语言跟随 i18n 当前语言；
 * 超出阈值或不可解析时回退绝对时间（由调用方兜底）。
 */
export function formatRelativeTime(
  value: string | number | null | undefined,
  locale: string,
  now: number = Date.now(),
): string | null {
  const ms = toEpochMs(value);
  if (ms === null) {
    return null;
  }
  const diffSec = Math.round((ms - now) / 1000);
  const absSec = Math.abs(diffSec);
  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
  if (absSec < 60) {
    return rtf.format(diffSec, "second");
  }
  if (absSec < 3600) {
    return rtf.format(Math.round(diffSec / 60), "minute");
  }
  if (absSec < 86400) {
    return rtf.format(Math.round(diffSec / 3600), "hour");
  }
  if (absSec < 86400 * 30) {
    return rtf.format(Math.round(diffSec / 86400), "day");
  }
  return null;
}

/** 本地化绝对时间（YYYY-MM-DD HH:mm:ss 风格，由 Intl 按 locale 排版）。 */
export function formatDateTime(
  value: string | number | null | undefined,
  locale: string,
): string {
  const ms = toEpochMs(value);
  if (ms === null) {
    return "-";
  }
  return new Intl.DateTimeFormat(locale, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(new Date(ms));
}

/** 相对时间优先，超 30 天回退绝对时间。 */
export function formatRelativeOrAbsolute(
  value: string | number | null | undefined,
  locale: string,
  now: number = Date.now(),
): string {
  return formatRelativeTime(value, locale, now) ?? formatDateTime(value, locale);
}

/** 耗时毫秒人话：<1s 显示 ms，<60s 显示 s，否则 m s。 */
export function formatDuration(ms: number | null | undefined, locale: string): string {
  if (ms === null || ms === undefined || ms <= 0) {
    return "-";
  }
  if (ms < 1000) {
    return new Intl.NumberFormat(locale, { style: "unit", unit: "millisecond", unitDisplay: "narrow" }).format(ms);
  }
  if (ms < 60_000) {
    return new Intl.NumberFormat(locale, {
      style: "unit",
      unit: "second",
      unitDisplay: "narrow",
      maximumFractionDigits: 1,
    }).format(ms / 1000);
  }
  const minutes = Math.floor(ms / 60_000);
  const seconds = Math.round((ms % 60_000) / 1000);
  return new Intl.NumberFormat(locale, { style: "unit", unit: "minute", unitDisplay: "narrow" }).format(minutes)
    + " "
    + new Intl.NumberFormat(locale, { style: "unit", unit: "second", unitDisplay: "narrow" }).format(seconds);
}
