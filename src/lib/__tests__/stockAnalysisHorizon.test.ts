import { describe, expect, it } from "vitest";
import { HORIZON_T_SUFFIX, horizonSourceLabelKey, readHorizonActions } from "../stock-analysis-utils";

/**
 * 历史列表的档位呈现（2026-09-29）：四档 Action 取自 `decisionJson.decisionsByHorizon`，
 * 缺席必须**渲染为空**而不是回退成「四档同主档」。
 */
describe("readHorizonActions", () => {
  it("取回四档各自的 Action（camelCase，现网产出形态）", () => {
    const json = JSON.stringify({
      decisionsByHorizon: {
        ultraShort: { action: "观望" },
        short: { action: "持有" },
        mid: { action: "买入" },
        long: { action: "买入" },
      },
    });
    expect(readHorizonActions(json)).toEqual([
      { key: "ultra_short", action: "观望" },
      { key: "short", action: "持有" },
      { key: "mid", action: "买入" },
      { key: "long", action: "买入" },
    ]);
  });

  it("兼容 snake_case 旧快照，且只产出实际存在的档", () => {
    const json = JSON.stringify({ decisions_by_horizon: { mid: { action: "持有" }, long: null } });
    expect(readHorizonActions(json)).toEqual([{ key: "mid", action: "持有" }]);
  });

  it("无结构 / 坏 JSON / 空串 ⇒ 空数组（调用方不渲染，不伪造）", () => {
    expect(readHorizonActions(null)).toEqual([]);
    expect(readHorizonActions("{")).toEqual([]);
    expect(readHorizonActions(JSON.stringify({ action: "买入" }))).toEqual([]);
    expect(readHorizonActions(JSON.stringify({ decisionsByHorizon: { mid: {} } }))).toEqual([]);
  });
});

describe("档位标签与来源", () => {
  it("四档各有 i18n 后缀（与 DecisionBanner 同一批键）", () => {
    expect(Object.keys(HORIZON_T_SUFFIX).sort()).toEqual(["long", "mid", "short", "ultra_short"]);
  });

  it("来源只认 formula / model，其余（含缺失）不贴来源标签", () => {
    expect(horizonSourceLabelKey("formula")).toBe("stockAnalysis.horizonSourceFormula");
    expect(horizonSourceLabelKey("model")).toBe("stockAnalysis.horizonSourceModel");
    expect(horizonSourceLabelKey(null)).toBeNull();
    expect(horizonSourceLabelKey(undefined)).toBeNull();
    expect(horizonSourceLabelKey("")).toBeNull();
  });
});
