// i18n-exempt: 断言全部为计票结构与档位键，无 UI 展示文本。
import type { DataQualityReport } from "@/types";
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useAnalystConsensusByTier } from "../useAnalystConsensusByTier";

const bullish = '<!-- VERDICT: {"bull_score":80,"bear_score":20} -->';
const bearish = '<!-- VERDICT: {"bull_score":20,"bear_score":80} -->';

describe("useAnalystConsensusByTier", () => {
  const reports = {
    "market-analyst--ultra_short": bearish,
    "hot-money--ultra_short": bearish,
    "market-analyst--mid": bullish,
    "policy--mid": bullish,
    "sector--mid": bullish,
  };

  it("默认跟随本次决策档，票数只算该档的分析师子集", () => {
    const { result } = renderHook(() => useAnalystConsensusByTier(reports, "ultra_short"));
    expect(result.current.activeTier).toBe("ultra_short");
    expect(result.current.tally).toEqual({ bullish: 0, bearish: 2, neutral: 0, total: 2 });
  });

  it("切档后各档用自己的子集与共识，不合计成一个池", () => {
    const { result } = renderHook(() => useAnalystConsensusByTier(reports, "ultra_short"));
    // 合计池会把 `market-analyst` 计两票（5 票两位分析师）；分组后两档各自 2 / 3 票
    expect(result.current.options.map((o) => o.tally.total)).toEqual([2, 3]);
    act(() => result.current.setActiveTier("mid"));
    expect(result.current.activeTier).toBe("mid");
    expect(result.current.tally.total).toBe(3);
    expect(result.current.consensus).toBe("bullish");
  });

  it("空产出不计票（既不记成中性，也不占档位）", () => {
    const { result } = renderHook(() =>
      useAnalystConsensusByTier({ "market-analyst--mid": "", "policy--mid": "  " }, "mid")
    );
    expect(result.current.tally.total).toBe(0);
    expect(result.current.tally.neutral).toBe(0);
  });

  it("无档位后缀的历史快照单独成组，不冒充某一档", () => {
    const { result } = renderHook(() => useAnalystConsensusByTier({ "market-analyst": bullish }, "mid"));
    expect(result.current.options).toEqual([]);
    expect(result.current.activeTier).toBeNull();
    expect(result.current.tally.bullish).toBe(1);
  });

  it("决策档还没有实例时回退 mid，而不是退回裸键", () => {
    const mixed = { ...reports, "investment-plan": bearish };
    const { result } = renderHook(() => useAnalystConsensusByTier(mixed, "long"));
    expect(result.current.activeTier).toBe("mid");
    expect(result.current.tally.total).toBe(3);
  });

  it("档位切换器按权威档序排，不跟节点完成先后跑", () => {
    const outOfOrder = {
      "sector--long": bullish,
      "market-analyst--ultra_short": bearish,
      "policy--long": bearish,
      "market-analyst--mid": bullish,
    };
    const { result } = renderHook(() => useAnalystConsensusByTier(outOfOrder, null));
    expect(result.current.options.map((o) => o.tier)).toEqual(["ultra_short", "mid", "long"]);
  });

  it("本轮结论与多空评分矛盾的分析师不计入该档票，并回报扣了几票", () => {
    // 判据住在后端 data-quality，这里只喂它的产出：mid 档的 mk 被判矛盾。
    const dq = {
      grade: "A",
      score: 90,
      diagnostics: { mk: { inconsistent_reason: "" } },
      diagnostics_by_tier: {
        mk: {
          mid: { inconsistent_reason: "verdict='看多' 但 bull_score(30) <= bear_score(60)" },
          ultra_short: { inconsistent_reason: "" },
        },
      },
    } as unknown as DataQualityReport;
    const { result } = renderHook(() => useAnalystConsensusByTier(reports, "mid", dq));
    expect(result.current.options.find((o) => o.tier === "mid")?.excluded).toBe(1);
    expect(result.current.tally.total).toBe(2);
    expect(result.current.excludedCount).toBe(1);
    // 另一档没被判矛盾 ⇒ 票照计
    expect(result.current.options.find((o) => o.tier === "ultra_short")?.excluded).toBe(0);
  });

  it("负控：拿不到 data-quality 产出时不扣票（不知道 ≠ 没人矛盾）", () => {
    const { result } = renderHook(() => useAnalystConsensusByTier(reports, "mid", null));
    expect(result.current.tally.total).toBe(3);
    expect(result.current.excludedCount).toBe(0);
  });
});
