// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";
import { CONFIDENCE_BAR_COLORS, confidenceBar } from "../evidenceConfidenceBar";

describe("confidenceBar —— 契约 [0,1] 的渲染判定", () => {
  it("正常强档：宽度按百分比、绿色", () => {
    expect(confidenceBar(0.85)).toEqual({
      widthPct: 85,
      color: CONFIDENCE_BAR_COLORS.strong,
      outOfRange: false,
    });
  });

  it("正常中档：0.3 为黄色", () => {
    expect(confidenceBar(0.3)).toEqual({
      widthPct: 30,
      color: CONFIDENCE_BAR_COLORS.medium,
      outOfRange: false,
    });
  });

  it("边界 0 与 1 都不算越界", () => {
    expect(confidenceBar(0).outOfRange).toBe(false);
    expect(confidenceBar(0).widthPct).toBe(0);
    expect(confidenceBar(1).outOfRange).toBe(false);
    expect(confidenceBar(1).color).toBe(CONFIDENCE_BAR_COLORS.strong);
  });

  // ── 以下两条是回归锁：它们在「事件发生时的代码」上是红的 ──

  it("★历史事故值 4.5（后端曾产出 0–30.7）必须判越界并标红", () => {
    // 修复前：`Math.min(4.5 * 100, 100)` ⇒ 恒满格，`4.5 > 0.5` ⇒ 恒绿
    // ⇒ 症状与「一个正常的高置信度」完全相同，缺陷因此潜伏。
    const bar = confidenceBar(4.5);
    expect(bar.outOfRange).toBe(true);
    expect(bar.color).toBe(CONFIDENCE_BAR_COLORS.outOfRange);
    // 宽度仍夹到 100（布局不能塌），但颜色已经暴露问题
    expect(bar.widthPct).toBe(100);
  });

  it("★负值与 NaN 同样必须可见，且不得产生 NaN 宽度", () => {
    const negative = confidenceBar(-0.2);
    expect(negative.outOfRange).toBe(true);
    expect(negative.color).toBe(CONFIDENCE_BAR_COLORS.outOfRange);

    const nan = confidenceBar(Number.NaN);
    expect(nan.outOfRange).toBe(true);
    expect(nan.color).toBe(CONFIDENCE_BAR_COLORS.outOfRange);
    expect(Number.isFinite(nan.widthPct)).toBe(true);

    const infinite = confidenceBar(Number.POSITIVE_INFINITY);
    expect(infinite.outOfRange).toBe(true);
    expect(infinite.widthPct).toBe(100);
  });
});
