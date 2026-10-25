// SPDX-License-Identifier: AGPL-3.0-only

import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

function readSource(...segments: string[]) {
  return fs.readFileSync(path.resolve(process.cwd(), ...segments), "utf8");
}

describe("LLM 采样参数默认值单一权威", () => {
  it("标题摘要/压缩默认值前后端一致", () => {
    const rust = readSource("src-tauri/crates/harness/src/constants.rs");
    const ui = readSource("src/components/settings/DefaultModelSettings.tsx");

    expect(rust).toMatch(/TITLE_SUMMARY_MAX_TOKENS: u32 = 256;/);
    expect(rust).toMatch(/COMPRESSION_MAX_TOKENS: u32 = 1024;/);
    expect(ui).toContain("defaultMaxTokens={256}");
    expect(ui).toContain("defaultMaxTokens={1024}");
  });

  it("Provider 模型编辑不再暴露 frequency/force 两个假配置", () => {
    const src = readSource("src/components/settings/ProviderDetail.tsx");

    expect(src).not.toContain("frequencyPenalty");
    expect(src).not.toContain("forceMaxTokens");
    // 上下文窗口与输出上限必须仍是两个明确区分的入口
    expect(src).toContain('t("settings.contextWindow")');
    expect(src).toContain("ModelParamSliders");
  });

  it("Agent 节点不再提供 temperature / maxTokens 入口", () => {
    const profileList = readSource(
      "src/components/settings/AgentProfileList.tsx",
    );
    const propertyPanel = readSource(
      "src/components/workflow/Panels/PropertyPanels/AgentPropertyPanel.tsx",
    );

    expect(profileList).not.toContain("maxTokens");
    expect(propertyPanel).not.toContain("maxTokens");
  });
});
