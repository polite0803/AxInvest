// SPDX-License-Identifier: AGPL-3.0-only

import { NODE_KIND_HUE } from "@/components/workflow/types/workflow.types";
import { HUE_COLORS, hueColor } from "@/theme/categoricalColors";
import { describe, expect, it } from "vitest";

describe("hueColor", () => {
  it("按 isDark 返回两档不同取值", () => {
    expect(hueColor("green", false)).toBe(HUE_COLORS.light.green);
    expect(hueColor("green", true)).toBe(HUE_COLORS.dark.green);
    expect(hueColor("green", false)).not.toBe(hueColor("green", true));
  });

  it("每个色相在两档里都有值", () => {
    for (const hue of Object.keys(HUE_COLORS.light) as Array<keyof typeof HUE_COLORS.light>) {
      expect(HUE_COLORS.light[hue]).toMatch(/^#[0-9a-f]{6}$/i);
      expect(HUE_COLORS.dark[hue]).toMatch(/^#[0-9a-f]{6}$/i);
    }
  });
});

describe("节点分类色相", () => {
  it("同一档内 8 个节点分类的颜色互不相同", () => {
    const light = Object.values(NODE_KIND_HUE).map((hue) => hueColor(hue, false));
    const dark = Object.values(NODE_KIND_HUE).map((hue) => hueColor(hue, true));

    // 类目色的意义就是「可区分」；两个分类撞色等于丢信息
    expect(new Set(light).size).toBe(light.length);
    expect(new Set(dark).size).toBe(dark.length);
  });
});
