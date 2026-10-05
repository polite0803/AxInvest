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
import i18n from "@/i18n";
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
 * 大数 → 按语言数制的紧凑计数串。
 *
 * 单位与**换算因子绑定**（`亿`=1e8、`B`=1e9、`करोड़`=1e7），所以这里按语言分**数制族**，
 * 不是逐语言翻译一个词 —— 把 `亿/万` 直译成 en 的 "B" 而不改因子，读数就错三个数量级。
 * 三族依据：中（简繁）/ 日 / 韩同用 1e8/1e4 万字族；欧洲与阿语用短 scale（1e9/1e6）；
 * 印地语用自己的 1e7/1e5 族（把 करोड़ 当 1e6 塞是数制错误，不是翻译问题）。
 *
 * ⚠ `zh-CN` / `zh-TW` 的因子与位数**必须与旧实现逐字相同**（现网行情读数依赖它）；
 * 由 `src/lib/__tests__/format.test.ts` 的快照锁住， Intl compact 实测已排除（zh-CN 下
 * `1234.57万` 与本函数 `1235万` 不一致 ⇒ 换过去会改现网数字）。
 */
type ScaleTier = { divisor: number; digits: number; unitKey: string };
/** 数制族 + 该族单位词的取用语种：**因子与单位必须成对取**，见下面 formatYi 的注释。 */
type NumberScale = { tiers: ScaleTier[]; unitLng: string };

const WAN_YI: ScaleTier[] = [
  { divisor: 1e8, digits: 2, unitKey: "unitYi" },
  { divisor: 1e4, digits: 0, unitKey: "unitWan" },
];
const SHORT: ScaleTier[] = [
  { divisor: 1e9, digits: 2, unitKey: "unitBillion" },
  { divisor: 1e6, digits: 2, unitKey: "unitMillion" },
];
const INDIAN: ScaleTier[] = [
  { divisor: 1e7, digits: 2, unitKey: "unitCrore" },
  { divisor: 1e5, digits: 2, unitKey: "unitLakh" },
];
const WAN_SCALE: NumberScale = { tiers: WAN_YI, unitLng: "zh-CN" };

const SCALE_BY_LOCALE: Record<string, NumberScale> = {
  "zh-CN": { tiers: WAN_YI, unitLng: "zh-CN" },
  "zh-TW": { tiers: WAN_YI, unitLng: "zh-TW" },
  ja: { tiers: WAN_YI, unitLng: "ja" },
  ko: { tiers: WAN_YI, unitLng: "ko" },
  "en-US": { tiers: SHORT, unitLng: "en-US" },
  de: { tiers: SHORT, unitLng: "de" },
  es: { tiers: SHORT, unitLng: "es" },
  fr: { tiers: SHORT, unitLng: "fr" },
  ru: { tiers: SHORT, unitLng: "ru" },
  ar: { tiers: SHORT, unitLng: "ar" },
  hi: { tiers: INDIAN, unitLng: "hi" },
};

export function formatYi(v: number, locale: string = i18n.language ?? "zh-CN"): string {
  // ⚠ 数制族与单位语种**必须成对取**：i18next 找不到语种时会回退到 `fallbackLng` 的单位词，
  //   于是「按 A 族换算数字 + 显示 B 语单位」= 错三到四个数量级，而数字看着仍像话
  //   （实测：未知 locale `zz-ZZ` 拿到万字族的 1.23 却配了 en 的 "hundred million"）。
  //   所以未登记的 locale 连单位一起回落到本族代表语种，而不是把 locale 原样交给 i18next。
  const scale = SCALE_BY_LOCALE[locale] ?? WAN_SCALE;
  for (const t of scale.tiers) {
    if (Math.abs(v) >= t.divisor) {
      const unit = String(i18n.t(`stockAnalysis.${t.unitKey}`, { lng: scale.unitLng }));
      return `${(v / t.divisor).toFixed(t.digits)}${unit}`;
    }
  }
  return v.toFixed(0);
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
