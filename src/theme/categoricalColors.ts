// SPDX-License-Identifier: AGPL-3.0-only

import { cssVar } from "@/lib/cssVar";

/**
 * 类目色板 —— 用于「必须彼此可区分」的语义分类（工作流节点类型、活动流角色等）。
 *
 * 与状态色（--green / --red / --amber）的分工：
 *   · 状态色只有一个含义，走 CSS 变量，随主题自动切换；
 *   · 类目色需要 N 个互不相同的色相，且消费方常做 `${color}15` 这种十六进制
 *     透明度拼接、或把值塞进 SVG fill（MiniMap / edge marker）——这两处 var()
 *     都不生效，所以只能按 isDark 解析成具体色值。
 *
 * 取色口径：亮色档取 antd 7 号色（压在浅底上可读），暗色档取 4–5 号色
 * （压在近黑底上可读）。调色只改这一张表。
 */
export type CategoricalHue =
  | "purple"
  | "green"
  | "lime"
  | "blue"
  | "geekblue"
  | "cyan"
  | "orange"
  | "volcano"
  | "pink"
  | "yellow"
  | "violet"
  | "teal"
  | "emerald"
  | "sky"
  | "red"
  | "gray";

export const HUE_COLORS: Record<"light" | "dark", Record<CategoricalHue, string>> = {
  light: {
    purple: "#722ed1",
    green: "#389e0d",
    lime: "#7cb305",
    blue: "#1677ff",
    geekblue: "#2f54eb",
    cyan: "#08979c",
    orange: "#d46b08",
    volcano: "#d4380d",
    pink: "#c41d7f",
    yellow: "#d48806",
    violet: "#5b21b6",
    teal: "#0f766e",
    emerald: "#047857",
    sky: "#0369a1",
    red: "#cf1322",
    gray: "#8c8c8c",
  },
  dark: {
    purple: "#b37feb",
    green: "#73d13d",
    lime: "#bae637",
    blue: "#4096ff",
    geekblue: "#597ef7",
    cyan: "#36cfc9",
    orange: "#ffa940",
    volcano: "#ff7a45",
    pink: "#f759ab",
    yellow: "#ffd666",
    violet: "#a78bfa",
    teal: "#2dd4bf",
    emerald: "#34d399",
    sky: "#38bdf8",
    red: "#ff7875",
    gray: "#a6a6a6",
  },
};

export function hueColor(hue: CategoricalHue, isDark: boolean): string {
  return HUE_COLORS[isDark ? "dark" : "light"][hue];
}

/**
 * 把色板投影成 `--cat-*` CSS 变量。
 *
 * 供只能走 CSS 的消费方使用（SVG 的 `stroke` 属性不解析 var()，但 svg 的
 * `stroke="currentColor"` 会继承 CSS 的 `color`）。权威仍是 `HUE_COLORS`，
 * 这里只是它在 DOM 上的运行时投影，不构成第二份定义。
 */
export function applyCategoricalCssVars(isDark: boolean): void {
  const root = document.documentElement;
  const palette = HUE_COLORS[isDark ? "dark" : "light"];
  for (const [hue, color] of Object.entries(palette)) {
    root.style.setProperty(`--cat-${hue}`, color);
  }
}

/** 取某个色相的 CSS 变量引用，带浅色兜底（投影未执行时也不至于变透明）。 */
export function hueVar(hue: CategoricalHue): string {
  return `var(--cat-${hue}, ${HUE_COLORS.light[hue]})`;
}

/** 多系列图表的取色顺序：尽量相邻不撞色相。 */
export const SERIES_HUES: CategoricalHue[] = [
  "blue",
  "green",
  "yellow",
  "red",
  "purple",
  "pink",
  "teal",
  "volcano",
  "geekblue",
  "lime",
  "cyan",
  "violet",
  "emerald",
  "sky",
  "gray",
];

/** 取第 i 个系列的颜色（画布可用的具体值，随主题切换）。 */
export function seriesColor(index: number): string {
  return hueVarValue(SERIES_HUES[index % SERIES_HUES.length]);
}

/**
 * 类目色相 → AntD Tag 的预设色名（AntD 只有 13 个预设名，且不与我们一一对应）。
 * 没有对应预设名的色相退到最近的同名预设 —— 退档关系在此登记，不在调用方各抄一份。
 */
const ANTD_TAG_PRESET: Partial<Record<CategoricalHue, string>> = {
  red: "red",
  blue: "blue",
  orange: "orange",
  purple: "purple",
  cyan: "cyan",
  pink: "magenta",
  geekblue: "geekblue",
  lime: "lime",
  yellow: "gold",
  volcano: "volcano",
  green: "green",
  violet: "purple",
  teal: "cyan",
  emerald: "green",
  sky: "blue",
  gray: "default",
};

export function antdTagPreset(hue: CategoricalHue): string {
  return ANTD_TAG_PRESET[hue] ?? "default";
}

/** 同 {@link hueVar}，但返回 CSS 变量解析后的具体颜色，供 canvas 使用。 */
export function hueVarValue(hue: CategoricalHue): string {
  return cssVar(`--cat-${hue}`, HUE_COLORS.light[hue]);
}
