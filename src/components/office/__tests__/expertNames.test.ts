// SPDX-License-Identifier: AGPL-3.0-only

import zhLocale from "@/i18n/locales/zh-CN.json";
import { describe, expect, it } from "vitest";
import { expertNameKey } from "../expertNames";

describe("expertNameKey 成员显示名 i18n 键推导（4-③）", () => {
  it("域包播种成员（opc-<expert_key>）推导出 office.experts 键", () => {
    expect(expertNameKey("opc-software-architect")).toBe("office.experts.software-architect");
    expect(expertNameKey("opc-ai-research-director")).toBe("office.experts.ai-research-director");
  });

  it("非域包成员不推导（手工 SubAgent / 前缀残缺）", () => {
    expect(expertNameKey("my-agent")).toBeNull();
    expect(expertNameKey("opc-")).toBeNull();
    expect(expertNameKey("")).toBeNull();
  });

  it("zh-CN locale 的 office.experts 非空且值不含键路径（防误写成原始 key）", () => {
    const experts = (zhLocale as any).office.experts as Record<string, string>;
    expect(Object.keys(experts).length).toBeGreaterThanOrEqual(45);
    for (const [key, value] of Object.entries(experts)) {
      expect(value).toBeTruthy();
      expect(value).not.toContain("office.experts.");
      expect(value).not.toContain(key);
    }
  });
});
