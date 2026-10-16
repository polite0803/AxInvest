import type { HorizonConfusionRow } from "@/types";
import { render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { HorizonConfusionBoard } from "../HorizonConfusionBoard";

vi.mock("react-i18next", () => ({
  // 回吐 `key|插值值` 形态：既能按 key 断言「说的是哪件事」，又能断插值真的落进去了
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) => opts ? `${key}|${JSON.stringify(opts)}` : key,
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

const row = (over: Partial<HorizonConfusionRow> = {}): HorizonConfusionRow => ({
  tier: "short",
  tierLabel: "短线",
  bothCorrect: 3,
  formulaOnly: 2,
  llmOnly: 1,
  bothWrong: 4,
  llmUndecided: 0,
  judged: 10,
  avgHorizonAgreement: null,
  sufficient: true,
  ...over,
});

const S = "stockAnalysis.reflection";

describe("HorizonConfusionBoard（B4 逐档结局 2×2）", () => {
  it("整列缺席：一句「未产出」，不渲染任何档行", () => {
    const { container } = render(<HorizonConfusionBoard rows={undefined} testId="hcb" />);
    expect(container.textContent).toBe(`${S}.horizonConfusionNotProduced`);
    expect(container.querySelector('[data-testid="confusion-row-short"]')).toBeNull();
  });

  it("四格计数按各自的插值槽落地，档名走本项目 i18n（不是产端中文名）", () => {
    const { container } = render(<HorizonConfusionBoard rows={[row()]} testId="hcb" />);
    const cells = container.querySelector(`[data-testid="confusion-row-short"]`)?.textContent ?? "";
    expect(cells).toContain(`${S}.horizonConfusionCells`);
    expect(cells).toContain('"both":3');
    expect(cells).toContain('"formulaOnly":2');
    expect(cells).toContain('"llmOnly":1');
    expect(cells).toContain('"bothWrong":4');
    // 档名经 HORIZON_T_SUFFIX → t(timeHorizonShort)，产端的中文名不进界面
    expect(cells).toContain("stockAnalysis.timeHorizonShort");
    expect(cells).not.toContain("短线");
  });

  it("样本不足：出「样本不足」那句并带上已判条数；够样本时**不**出", () => {
    const few = render(
      <HorizonConfusionBoard rows={[row({ judged: 2, sufficient: false })]} testId="hcb" />,
    );
    expect(few.container.textContent).toContain(`${S}.horizonConfusionInsufficient`);
    expect(few.container.textContent).toContain('"judged":2');

    // 负控：够样本还挂着「样本不足」= 把结论标成不可信，或反之把计数标成结论
    const enough = render(<HorizonConfusionBoard rows={[row({ judged: 10, sufficient: true })]} testId="hcb" />);
    expect(enough.container.textContent).not.toContain(`${S}.horizonConfusionInsufficient`);
  });

  it("无从判定的条数单列：>0 才出，且参数走 undecided（不得混进任何一格）", () => {
    const withU = render(
      <HorizonConfusionBoard rows={[row({ llmUndecided: 5 })]} testId="hcb" />,
    );
    const u = withU.container.querySelector('[data-testid="confusion-undecided-short"]')?.textContent ?? "";
    expect(u).toContain(`${S}.horizonConfusionUndecided`);
    expect(u).toContain('"undecided":5');
    // 负控：0 条时不出这句（出了就等于凭空报了一个不存在的数据缺口）
    const without = render(<HorizonConfusionBoard rows={[row({ llmUndecided: 0 })]} testId="hcb" />);
    expect(without.container.querySelector('[data-testid="confusion-undecided-short"]')).toBeNull();
  });

  it("三维一致率均值：有值才说，null 不得显示成 0", () => {
    const withAvg = render(
      <HorizonConfusionBoard rows={[row({ avgHorizonAgreement: 61.4 })]} testId="hcb" />,
    );
    expect(withAvg.container.textContent).toContain(`${S}.horizonConfusionAvgAgreement`);
    expect(withAvg.container.textContent).toContain('"pct":61');

    const sans = render(<HorizonConfusionBoard rows={[row({ avgHorizonAgreement: null })]} testId="hcb" />);
    expect(sans.container.textContent).not.toContain(`${S}.horizonConfusionAvgAgreement`);
  });

  it("四档恒各一行（产端四行照渲），缺样本的档也占一行", () => {
    const { container } = render(
      <HorizonConfusionBoard
        rows={[
          row({ tier: "ultra_short", judged: 0, sufficient: false }),
          row({ tier: "short" }),
          row({ tier: "mid" }),
          row({ tier: "long" }),
        ]}
        testId="hcb"
      />,
    );
    expect(container.querySelectorAll('[data-testid^="confusion-row-"]')).toHaveLength(4);
    expect(
      container.querySelector('[data-testid="confusion-row-ultra_short"]')?.getAttribute("data-sufficient"),
    ).toBe("false");
  });
});
