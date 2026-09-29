import { describe, expect, it } from "vitest";
import { normalizeDecision } from "../agentOutput";
import {
  HORIZON_CAMEL_TO_SNAKE,
  HORIZON_T_SUFFIX,
  horizonSourceLabelKey,
  horizonSuffix,
  readHorizonActions,
} from "../stock-analysis-utils";

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

/**
 * Phase F 同源标注：`sharesPosteriorWith` 里装的是 `decisionsByHorizon` 的**键名**
 * （camelCase），而 i18n 后缀表按 snake_case 建模 ⇒ 互转只有 `horizonSuffix` 这一处。
 * 它一旦退化（比如直接查 camel 键查不到就显示原始键），注脚会显示成「与 ultraShort 相同」。
 */
describe("horizonSuffix（两族键名的单点互转）", () => {
  it("camelCase 与 snake_case 都映射到同一后缀", () => {
    expect(horizonSuffix("ultraShort")).toBe("UltraShort");
    expect(horizonSuffix("ultra_short")).toBe("UltraShort");
    for (const [camel, snake] of Object.entries(HORIZON_CAMEL_TO_SNAKE)) {
      expect(horizonSuffix(camel)).toBe(HORIZON_T_SUFFIX[snake]);
    }
  });

  it("认不出的档名返回 null，不猜档位", () => {
    expect(horizonSuffix("medium")).toBeNull();
    expect(horizonSuffix("")).toBeNull();
  });
});

describe("normalizeDecision 透传逐档口径标注", () => {
  it("scoreSource / sharesPosteriorWith / stopSource / positionSource 原样到达展示层", () => {
    const raw = {
      action: "BUY",
      confidence: 62,
      decisionsByHorizon: {
        ultraShort: {
          action: "观望",
          posterior: 55.0,
          scoreSource: "daily_fallback",
          sharesPosteriorWith: ["short"],
          stopSource: "fallback_pct",
          positionSource: "kelly_only",
        },
        short: { action: "观望", posterior: 55.0, scoreSource: "tier_native", sharesPosteriorWith: ["ultraShort"] },
      },
    };
    const parsed = normalizeDecision(raw);
    expect(parsed).not.toBeNull();
    const ultra = parsed!.decisionsByHorizon?.ultraShort;
    // 这些键是**结构性缺席声明**本身：normalizeDecision 走白名单构造返回体，
    // 漏加一个键就等于把「该档按日线退化」这条信息在 IPC 之后静默丢弃。
    expect(ultra?.scoreSource).toBe("daily_fallback");
    expect(ultra?.sharesPosteriorWith).toEqual(["short"]);
    expect(ultra?.stopSource).toBe("fallback_pct");
    expect(ultra?.positionSource).toBe("kelly_only");
    expect(parsed!.decisionsByHorizon?.short?.scoreSource).toBe("tier_native");
  });
});
