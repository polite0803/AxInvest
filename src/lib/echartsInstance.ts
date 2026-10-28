// SPDX-License-Identifier: AGPL-3.0-only

import * as echarts from "echarts";

/**
 * ECharts 实例的唯一取用入口 —— 保证实例的 chrome 颜色与当前明暗档一致。
 *
 * 为什么要统一：ECharts 的坐标轴文字、图例、tooltip 底色取自 **init 时传入的主题**，
 * `setOption` 改不动这些。全仓此前有 11 处各自 `echarts.init(el, undefined)`，
 * 暗色主题下就是「深字压深底」（与主题选择器/终端配色同族的第二个颜色来源）。
 *
 * 档位变化时这里会销毁重建实例（`tierByElement` 记住每个容器当初是按哪一档建的），
 * 调用方的 effect 只要把 `isDark` 放进依赖即可。
 */
const tierByElement = new WeakMap<HTMLElement, boolean>();

export function ensureECharts(el: HTMLElement, isDark: boolean): echarts.ECharts {
  const existing = echarts.getInstanceByDom(el);
  if (existing && !existing.isDisposed() && tierByElement.get(el) === isDark) {
    return existing;
  }
  existing?.dispose();
  const inst = echarts.init(el, isDark ? "dark" : undefined, { renderer: "canvas" });
  tierByElement.set(el, isDark);
  return inst;
}
