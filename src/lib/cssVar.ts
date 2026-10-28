// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 读取 `:root` 上的主题变量，返回**可被画布消费的**颜色字符串。
 *
 * 为什么不能直接返回 token 原文：主题 token 是 `oklch(...)`，而 ECharts 底层
 * zrender 的取色器只认 hex / rgb / hsl —— 它会把 oklch 判为非法，派生色
 * （emphasis、渐变、透明度）随之失效。这里用 canvas 的 `fillStyle` 做一次
 * 归一化：浏览器能解析就换成 `#rrggbb` / `rgba(...)`，解析不了就退回内置兜底值
 * （同时覆盖「旧 WebView2 不支持 oklch」这一环境差异）。
 */

/** 各 token 的浅色兜底值，仅在取不到变量或浏览器不认该颜色格式时使用。 */
const FALLBACKS: Record<string, string> = {
  "--fg": "rgba(0,0,0,0.88)",
  "--fg-2": "rgba(0,0,0,0.65)",
  "--muted": "rgba(0,0,0,0.45)",
  "--border": "#e5e7eb",
  "--border-light": "#f0f0f0",
  "--surface": "#ffffff",
  "--surface-2": "#f0f0f0",
  "--accent": "#1677ff",
  "--green": "#389e0d",
  "--red": "#cf1322",
  "--amber": "#d48806",
  "--purple": "#722ed1",
};

let probe: CanvasRenderingContext2D | null | undefined;

/** 用 canvas 把任意合法颜色归一化成 hex/rgba；非法则返回 null。 */
function normalize(color: string): string | null {
  if (typeof document === "undefined") {
    return null;
  }
  if (probe === undefined) {
    const canvas = document.createElement("canvas");
    canvas.width = 1;
    canvas.height = 1;
    probe = canvas.getContext("2d");
  }
  if (!probe) {
    return null;
  }
  const sentinel = "#010203";
  probe.fillStyle = sentinel;
  probe.fillStyle = color;
  const resolved = probe.fillStyle;
  // 赋值被忽略时 fillStyle 仍是哨兵色 ⇒ 该颜色格式不被支持
  return resolved === sentinel ? null : resolved;
}

export function cssVar(name: string, fallback?: string): string {
  const raw = typeof document === "undefined"
    ? ""
    : getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  const value = raw || fallback || FALLBACKS[name] || "";
  if (!value) {
    return "";
  }
  return normalize(value) ?? value;
}
