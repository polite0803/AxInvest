import { render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { HorizonAgreementList } from "../HorizonAgreementList";

vi.mock("react-i18next", () => ({
  // 回吐 `key|插值值` 形态：既能按 key 断言「说的是哪件事」，又能断插值真的落进去了
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) => opts ? `${key}|${JSON.stringify(opts)}` : key,
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

const comparable = (tier: string, total: number, pct: number | null) => ({
  tier,
  tierLabel: tier,
  comparable: true,
  total,
  absence: null,
  formulaAction: "买入",
  llmAction: "观望",
  actionScore: 6,
  actionNote: "lean_vs_wait",
  actionOk: true,
  positionScore: 20,
  positionGap: 5,
  confidenceScore: 15,
  confidenceGap: 8,
  conflictType: "action_divergence",
  sharedEvidenceWeightPct: pct,
  dimsUsed: ["action", "positionPct", "confidence"],
  dimsMissing: ["riskLevel", "evidenceCited"],
});

const absent = (tier: string, why: string) => ({
  tier,
  tierLabel: tier,
  comparable: false,
  total: null,
  absence: why,
  dimsUsed: ["action", "positionPct", "confidence"],
  dimsMissing: ["riskLevel", "evidenceCited"],
});

describe("HorizonAgreementList", () => {
  it("产端没给这个字段：一句「未产出」，不渲染行、也不出现分数", () => {
    const { container } = render(<HorizonAgreementList agreements={undefined} testId="hal" />);
    expect(container.textContent).toBe("stockAnalysis.horizonAgreementNotProduced");
    expect(container.querySelector('[data-testid="horizon-agreement-row-mid"]')).toBeNull();
  });

  it("可比的行显示两侧动作与分数，并必带「不可并排比较」那句声明", () => {
    const { container } = render(
      <HorizonAgreementList agreements={[comparable("mid", 63, null)]} testId="hal" />,
    );
    const row = container.querySelector('[data-testid="horizon-agreement-row-mid"]');
    expect(row?.getAttribute("data-comparable")).toBe("true");
    expect(row?.textContent).toContain("买入");
    expect(row?.textContent).toContain("观望");
    expect(row?.textContent).toContain("63");
    expect(container.textContent).toContain("stockAnalysis.horizonAgreementDimNote");
  });

  /**
   * 核心一格：`comparable=false` 的行**只说成因**，不得把 null 分数渲染成 0。
   * 0 与「无从比对」是两件事 —— 前者会被统计与用户读成「一致得零分」。
   */
  it("无从比对的行显示成因，且不得出现 0 分", () => {
    const why = "LLM 未给出该档决策（逐档点名见 data_gaps）";
    const { container } = render(
      <HorizonAgreementList agreements={[absent("short", why)]} testId="hal" />,
    );
    const row = container.querySelector('[data-testid="horizon-agreement-row-short"]');
    // 行 = 档位名 + 成因句（断 toContain 而非 toBe：整行本来带档名标签）
    expect(row?.textContent).toContain(why);
    expect(row?.textContent).not.toMatch(/(^|[^\d])0([^\d]|$)/);
    expect(container.querySelector(`[data-testid="horizon-agreement-absent-short"]`)).not.toBeNull();
  });

  it("独立性折扣有值才说，且插值真的落进 t()", () => {
    const withPct = render(
      <HorizonAgreementList agreements={[comparable("ultraShort", 63, 25.4)]} testId="hal" />,
    );
    expect(withPct.container.textContent).toContain("stockAnalysis.horizonAgreementIndependenceNote");
    expect(withPct.container.textContent).toContain('"pct":25');

    const without = render(
      <HorizonAgreementList agreements={[comparable("long", 63, null)]} testId="hal" />,
    );
    // 负控：取不到 ⇒ 不显示折扣句（显示成 0% 会把「无从判断」伪装成「两条链完全独立」）
    expect(without.container.textContent).not.toContain("stockAnalysis.horizonAgreementIndependenceNote");
  });

  it("四档各占一行，缺一档不缩行", () => {
    const { container } = render(
      <HorizonAgreementList
        agreements={[
          comparable("ultraShort", 63, null),
          absent("short", "公式侧该档分支未产出"),
          comparable("mid", 100, null),
          comparable("long", 46, null),
        ]}
        testId="hal"
      />,
    );
    expect(container.querySelectorAll("[data-testid^='horizon-agreement-row-']")).toHaveLength(4);
  });
});

/**
 * B2 归因渲染（`divergences`）。判据码是机器可读的，**叙事全在 i18n**，所以这里按 key 断言
 * 「说的是哪件事」，再按参数袋断「插值真的带上了产端给的那个数」—— 两条缺一都不算钉住：
 * 只断 key ⇒ 参数接错也绿；只断参数 ⇒ 七码渲染成同一句也绿。
 */
const div = (tier: string, over: Record<string, unknown> = {}) => ({
  tier,
  tierLabel: tier,
  drivers: [],
  ...over,
});

const SEVEN_CODES = [
  "tier_risk_veto",
  "entry_gate_failed",
  "llm_tier_absent",
  "negative_legs",
  "absent_legs",
  "data_gap",
  "formula_tier_absent",
] as const;

describe("HorizonAgreementList · 逐档分歧归因", () => {
  it("归因整列缺席：不渲染归因块，也不说「无驱动项」", () => {
    const { container } = render(
      <HorizonAgreementList agreements={[comparable("mid", 63, null)]} testId="hal" />,
    );
    // 「该记录产自归因上线前」与「归因跑过、没点出驱动项」是两件事，后者才用 none 那句
    expect(container.querySelector('[data-testid="horizon-divergence-mid"]')).toBeNull();
    expect(container.textContent).not.toContain("stockAnalysis.horizonDivergence.none");
  });

  it("七条驱动码各占一句，且每条拿到自己那个插值槽", () => {
    const d = div("mid", {
      drivers: [...SEVEN_CODES],
      entryGate: "breadth_required",
      entryGatePassed: false,
      gateBasis: "宽度 0.31 < 0.5",
      veto: ["买入", "观望"],
      negativeLegs: [["flowPersistence", -0.42, 12.5]],
      absentLegs: ["eventCatalyst"],
      dataGaps: ["龙虎榜"],
      llmAbsence: "LLM 未给出该档决策（逐档点名见 data_gaps）",
    });
    const { container } = render(
      <HorizonAgreementList
        agreements={[comparable("mid", 63, null)]}
        divergences={[d]}
        testId="hal"
      />,
    );
    const block = container.querySelector('[data-testid="horizon-divergence-mid"]');
    expect(block).not.toBeNull();
    expect(block?.querySelectorAll('[data-testid^="horizon-driver-mid-"]')).toHaveLength(7);
    for (const code of SEVEN_CODES) {
      const chip = block?.querySelector(`[data-testid="horizon-driver-mid-${code}"]`);
      expect(chip?.textContent, `驱动码 ${code} 没渲染`).toContain(
        `stockAnalysis.horizonDivergence.driver.${code}`,
      );
    }
    // 逐码参数：把「参数接错到别的码上」这一类假绿钉死
    expect(block?.querySelector('[data-testid="horizon-driver-mid-tier_risk_veto"]')?.textContent)
      .toContain('"from":"买入"');
    expect(block?.querySelector('[data-testid="horizon-driver-mid-entry_gate_failed"]')?.textContent)
      .toContain('"gate":"breadth_required"');
    expect(block?.querySelector('[data-testid="horizon-driver-mid-data_gap"]')?.textContent)
      .toContain('"n":1');
    expect(block?.querySelector('[data-testid="horizon-driver-mid-absent_legs"]')?.textContent)
      .toContain('"absent":"eventCatalyst"');
    expect(block?.querySelector('[data-testid="horizon-driver-mid-llm_tier_absent"]')?.textContent)
      .toContain('"why":"LLM 未给出该档决策');
  });

  it("负腿文案走 legContribution，数值按两位小数落地", () => {
    const { container } = render(
      <HorizonAgreementList
        agreements={[comparable("mid", 63, null)]}
        divergences={[div("mid", {
          drivers: ["negative_legs"],
          negativeLegs: [["momentumSignal", -0.5, 10], ["flowPersistence", -0.25, 20]],
        })]}
        testId="hal"
      />,
    );
    const chip = container.querySelector('[data-testid="horizon-driver-mid-negative_legs"]');
    const txt = chip?.textContent ?? "";
    expect(txt).toContain("stockAnalysis.horizonDivergence.legContribution");
    // 只断数值不断引号形态：`legs` 是嵌在参数袋里的字符串，t() 替身把它 JSON.stringify
    // 后引号被转义成 \"，断 `"bias":"-0.50"` 会因转义而恒不命中（本测试头一次红就是这样）
    expect(txt.match(/legContribution/g)?.length).toBe(2);
    expect(txt).toContain("-0.50");
    expect(txt).toContain("-0.25");
    expect(txt).toContain("10.00");
    expect(txt).toContain("20.00");
    // 两条腿都要在（截成一条会把「最负的三条」写成「最负的一条」）
    expect(txt).toContain("flowPersistence");
  });

  it("产端给了空驱动项：说「未点出驱动项」，不得说成两侧一致", () => {
    const { container } = render(
      <HorizonAgreementList
        agreements={[comparable("long", 100, null)]}
        divergences={[div("long", { drivers: [] })]}
        testId="hal"
      />,
    );
    expect(container.querySelector('[data-testid="horizon-divergence-none-long"]')).not.toBeNull();
    expect(container.textContent).toContain("stockAnalysis.horizonDivergence.none");
  });

  it("产端新增而前端未译的码：直出原码，不静默丢格", () => {
    const { container } = render(
      <HorizonAgreementList
        agreements={[comparable("mid", 63, null)]}
        divergences={[div("mid", { drivers: ["brand_new_code"] })]}
        testId="hal"
      />,
    );
    const chip = container.querySelector('[data-testid="horizon-driver-mid-brand_new_code"]');
    expect(chip?.textContent).toContain('"defaultValue":"brand_new_code"');
  });

  /**
   * 档位对齐：归因按 `tier` 找行，不按数组下标 —— 产端两列都按 Period::ALL 出四行，
   * 但前端只渲染有对照行的档；下标对齐会在「某档没有对照行」时整体错位一格。
   */
  it("归因按档位键对齐：顺序不同也不挂错档", () => {
    const { container } = render(
      <HorizonAgreementList
        agreements={[comparable("mid", 63, null), comparable("ultraShort", 46, null)]}
        divergences={[div("ultraShort", { drivers: ["data_gap"], dataGaps: ["北向"] }), div("mid")]}
        testId="hal"
      />,
    );
    expect(
      container.querySelector('[data-testid="horizon-divergence-ultraShort"]')?.textContent,
    ).toContain("stockAnalysis.horizonDivergence.driver.data_gap");
    expect(container.querySelector('[data-testid="horizon-divergence-mid"]')?.textContent)
      .toContain("stockAnalysis.horizonDivergence.none");
  });
});
