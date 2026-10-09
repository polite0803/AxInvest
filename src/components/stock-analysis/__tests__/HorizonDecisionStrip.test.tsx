import type { DecisionsByHorizon, HorizonDecision } from "@/types";
import { render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { HorizonDecisionStrip } from "../HorizonDecisionStrip";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
  // `@/lib/stock-analysis-utils` 会经 browserMock 拉起 `src/i18n/index.ts`，
  // 那里要 `i18n.use(initReactI18next)` ⇒ 只 mock useTranslation 会整档失败。
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

function mkHorizon(over: Partial<HorizonDecision>): HorizonDecision {
  return {
    action: "观望",
    verdict: "决策=观望",
    positionPct: 0,
    confidence: 40,
    posterior: 40,
    stopLossPct: 0,
    takeProfitPct: 0,
    expectedHoldingDays: 5,
    targetPrice: null,
    stopLoss: null,
    ...over,
  };
}

const FOUR: DecisionsByHorizon = {
  ultraShort: mkHorizon({ action: "买入", positionPct: 20, riskCategory: "高风险" }),
  short: mkHorizon({ action: "观望", positionPct: 0, riskCategory: "中风险" }),
  mid: mkHorizon({ action: "持有", positionPct: 10, riskCategory: "低风险" }),
  long: mkHorizon({ action: "买入", positionPct: 15, riskCategory: "极高风险" }),
};

const TIER_KEYS = [
  "stockAnalysis.timeHorizonUltraShort",
  "stockAnalysis.timeHorizonShort",
  "stockAnalysis.timeHorizonMid",
  "stockAnalysis.timeHorizonLong",
];

/**
 * 四周期逐档信息条的门。
 *
 * 要挡住的复发形态不是「少渲染一格」，而是这两类：
 *   ① **缺席被补成占位格** —— 某档没有该轴数据时补「—」或拿主档顶替；
 *   ② **该轴没有数据也整条渲染**（读者以为「四档都看过」）。
 * 故用「有数据的档才出现、其余档名一个都不出现」双向断言。
 */
describe("HorizonDecisionStrip", () => {
  it("decision 模式：四档齐备时逐档给出档名 + 行动 + 仓位", () => {
    const { getByTestId } = render(
      <HorizonDecisionStrip decisions={FOUR} mode="decision" testId="strip" />,
    );
    const bar = getByTestId("strip");
    for (const key of TIER_KEYS) {
      expect(bar.textContent, `应列出 ${key}`).toContain(key);
    }
    expect(bar.textContent).toContain("stockAnalysis.actionBuy");
    expect(bar.textContent).toContain("stockAnalysis.actionWait");
    expect(bar.textContent).toContain("stockAnalysis.actionHold");
    expect(bar.textContent).toContain("20%");
    expect(bar.textContent).toContain("15%");
    // decision 轴不渲染风险档
    expect(bar.textContent).not.toContain("stockAnalysis.riskHigh");
  });

  it("risk 模式：逐档给出按档风险档，且不渲染 action/仓位", () => {
    const { getByTestId } = render(
      <HorizonDecisionStrip decisions={FOUR} mode="risk" testId="strip" />,
    );
    const bar = getByTestId("strip");
    expect(bar.textContent).toContain("stockAnalysis.riskHigh");
    expect(bar.textContent).toContain("stockAnalysis.riskMid");
    expect(bar.textContent).toContain("stockAnalysis.riskLow");
    expect(bar.textContent).toContain("stockAnalysis.riskExtreme");
    expect(bar.textContent).not.toContain("stockAnalysis.actionBuy");
    expect(bar.textContent).not.toContain("20%");
  });

  it("缺席不补格：缺该轴数据的档名一个都不出现", () => {
    const partial: DecisionsByHorizon = {
      ultraShort: mkHorizon({ action: "买入", positionPct: 20, riskCategory: "高风险" }),
      // mid 只有 action、没有 riskCategory ⇒ risk 模式下必须整格缺席
      mid: mkHorizon({ action: "持有", positionPct: 10 }),
      // long 只有 riskCategory、没有 action ⇒ decision 模式下必须整格缺席
      long: mkHorizon({ action: "", positionPct: 15, riskCategory: "低风险" }),
    };

    const decision = render(
      <HorizonDecisionStrip decisions={partial} mode="decision" testId="strip" />,
    ).getByTestId("strip");
    expect(decision.textContent).toContain("stockAnalysis.timeHorizonUltraShort");
    expect(decision.textContent).toContain("stockAnalysis.timeHorizonMid");
    expect(decision.textContent).not.toContain("stockAnalysis.timeHorizonLong");

    const risk = render(
      <HorizonDecisionStrip decisions={partial} mode="risk" testId="strip-risk" />,
    ).getByTestId("strip-risk");
    expect(risk.textContent).toContain("stockAnalysis.timeHorizonUltraShort");
    expect(risk.textContent).toContain("stockAnalysis.timeHorizonLong");
    expect(risk.textContent).not.toContain("stockAnalysis.timeHorizonMid");
  });

  it("无数据（null / 空对象）时整条不渲染", () => {
    expect(
      render(<HorizonDecisionStrip decisions={null} mode="decision" testId="strip" />)
        .container.firstChild,
    ).toBeNull();
    expect(
      render(<HorizonDecisionStrip decisions={{}} mode="risk" testId="strip" />)
        .container.firstChild,
    ).toBeNull();
  });
});
