import { render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { F7ReferenceComparison } from "../F7ReferenceComparison";

vi.mock("react-i18next", () => ({
  // 回吐 `key|插值值` 形态：既能按 key 断言「说的是哪件事」，又能断插值真的落进去了
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) => opts ? `${key}|${JSON.stringify(opts)}` : key,
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

const P = "stockAnalysis.f7Reference";

/** 齐备态：f7 取风险收益比、四值齐全、无缺口。值按生产形态给（方向档是中文原值） */
const ready = {
  action: "买入",
  positionPct: 12.5,
  confidence: 68,
  posterior: 61.5,
  riskLevel: "中风险",
  f7Source: "risk_reward",
  f7WeightPct: 9.2,
  weightsCollapsed: false,
  dataGaps: [],
};

const prod = {
  productionF7Source: "analyst_votes",
  productionAction: "持有",
  productionConfidence: 62,
  productionPositionPct: 8,
};

/** f7 来历四态：一次只应当命中一句，其余三句必须不出现（四句同屏=一条判据没分格） */
describe("F7ReferenceComparison · f7 输入来历四态", () => {
  const CASES: Array<[string, string]> = [
    ["analyst_votes", `${P}.f7.analystVotes`],
    ["trader", `${P}.f7.trader`],
    ["absent", `${P}.f7.absent`],
    ["", `${P}.f7.unknown`],
  ];

  for (const [source, expectedKey] of CASES) {
    it(`参照侧 f7Source="${source}" 只命中 ${expectedKey} 那一句`, () => {
      const { container } = render(
        <F7ReferenceComparison
          snapshot={{ ...ready, f7Source: source || null }}
          {...prod}
          testId="ifx"
        />,
      );
      // 按**行**断言，不按整块：生产侧那一行永远在说自己的口径，整块 toContain 会互相污染
      const row = container.querySelector('[data-testid="ifx-basis-reference"]');
      expect(row?.textContent).toBe(`${P}.referenceLabel${expectedKey}`);
      const others = CASES.map(([, k]) => k).filter((k) => k !== expectedKey);
      for (const absent of others) {
        expect(row?.textContent ?? "").not.toContain(absent);
      }
      expect(container.querySelector('[data-testid="ifx"]')?.getAttribute("data-state")).toBe("ok");
    });
  }

  it("生产侧那一行读的是主决策自己的 f7_source，不是参照侧的", () => {
    const { container } = render(
      <F7ReferenceComparison
        snapshot={{ ...ready, f7Source: "trader" }}
        {...prod}
        testId="ifx"
      />,
    );
    expect(container.querySelector('[data-testid="ifx-basis-production"]')?.textContent)
      .toBe(`${P}.productionLabel${P}.f7.analystVotes`);
    expect(container.querySelector('[data-testid="ifx-basis-reference"]')?.textContent)
      .toBe(`${P}.referenceLabel${P}.f7.trader`);
    expect(container.querySelector('[data-testid="ifx-same-basis"]')).toBeNull();
  });

  /**
   * 关键负控：两侧口径相同（例如回滚位配错）时对照本身不成立 ⇒ 必须出「同口径」那句。
   * 缺了这句，一个 0 差会被读成「交易观点不影响结论」——那是本块最容易造成的假结论。
   */
  it("两侧同口径 ⇒ 出「对照不成立」句并置 data-same-basis", () => {
    const same = render(
      <F7ReferenceComparison
        snapshot={{ ...ready, f7Source: "analyst_votes" }}
        {...prod}
        testId="ifx"
      />,
    );
    expect(same.container.querySelector('[data-testid="ifx"]')?.getAttribute("data-same-basis")).toBe("true");
    expect(same.container.querySelector('[data-testid="ifx-same-basis"]')?.textContent)
      .toBe(`${P}.sameBasis`);

    const diff = render(
      <F7ReferenceComparison snapshot={{ ...ready, f7Source: "trader" }} {...prod} testId="ifx" />,
    );
    expect(diff.container.querySelector('[data-testid="ifx"]')?.getAttribute("data-same-basis")).toBe("false");
    expect(diff.container.querySelector('[data-testid="ifx-same-basis"]')).toBeNull();
  });

  it("键整体缺席（v180 之前的记录）：只说「未产出」，不出数值格、也不出差值句", () => {
    for (const missing of [undefined, null]) {
      const { container } = render(
        <F7ReferenceComparison snapshot={missing} {...prod} testId="ifx" />,
      );
      expect(container.textContent).toBe(`${P}.notProduced`);
      expect(container.querySelector('[data-testid="ifx"]')?.getAttribute("data-state")).toBe("not-produced");
      expect(container.querySelector('[data-testid="ifx-confidence"]')).toBeNull();
      expect(container.querySelector('[data-testid="ifx-delta"]')).toBeNull();
      expect(container.querySelector('[data-testid="ifx-scale-note"]')).toBeNull();
    }
  });
});

describe("F7ReferenceComparison · 与生产公式的差值", () => {
  it("同档时给「同档」句，不同档时把两侧档位都插进句子里", () => {
    const same = render(
      <F7ReferenceComparison
        snapshot={{ ...ready, action: "买入" }}
        productionAction="BUY"
        productionConfidence={62}
        productionPositionPct={8}
        testId="ifx"
      />,
    );
    expect(same.container.querySelector('[data-testid="ifx-action-delta"]')?.textContent)
      .toBe(`${P}.actionSame`);

    const differ = render(
      <F7ReferenceComparison snapshot={ready} {...prod} testId="ifx" />,
    );
    const txt = differ.container.querySelector('[data-testid="ifx-action-delta"]')?.textContent ?? "";
    expect(txt).toContain(`${P}.actionDiffer`);
    // 两侧都拿到**翻译后**的档位（不是原始中文串），插错顺序会露出来
    expect(txt).toContain('"production":"stockAnalysis.actionHold"');
    expect(txt).toContain('"reference":"stockAnalysis.actionBuy"');
  });

  it("数值差带符号：本侧高于生产为正、低于为负，且两侧原值都进句子", () => {
    const up = render(<F7ReferenceComparison snapshot={ready} {...prod} testId="ifx" />);
    const conf = up.container.querySelector('[data-testid="ifx-conf-delta"]')?.textContent ?? "";
    expect(conf).toContain(`${P}.confDelta`);
    expect(conf).toContain('"delta":"+6.0"');
    expect(conf).toContain('"production":62');
    expect(conf).toContain('"reference":68');
    const pos = up.container.querySelector('[data-testid="ifx-pos-delta"]')?.textContent ?? "";
    expect(pos).toContain('"delta":"+4.5"');

    const down = render(
      <F7ReferenceComparison
        snapshot={{ ...ready, confidence: 40, positionPct: 3 }}
        {...prod}
        testId="ifx"
      />,
    );
    expect(down.container.querySelector('[data-testid="ifx-conf-delta"]')?.textContent)
      .toContain('"delta":"-22.0"');
    expect(down.container.querySelector('[data-testid="ifx-pos-delta"]')?.textContent)
      .toContain('"delta":"-5.0"');
  });

  /**
   * 关键负控：生产侧数值缺位时**不得**算出差值句。
   * 把「不知道」显示成 0 差，等于宣称两侧一致 —— 与主决策卡那族假绿同一判据。
   */
  it("任一侧数值缺位 ⇒ 说「不做数值差值」，不出 confDelta / posDelta", () => {
    const { container } = render(
      <F7ReferenceComparison
        snapshot={{ ...ready, confidence: null, positionPct: null }}
        {...prod}
        testId="ifx"
      />,
    );
    expect(container.querySelector('[data-testid="ifx-conf-delta"]')).toBeNull();
    expect(container.querySelector('[data-testid="ifx-pos-delta"]')).toBeNull();
    expect(container.querySelector('[data-testid="ifx-value-not-comparable"]')).not.toBeNull();
    expect(container.querySelector('[data-testid="ifx-confidence"]')?.textContent).toBe("—");
    expect(container.querySelector('[data-testid="ifx-position"]')?.textContent).toBe("—");
  });

  it("本侧方向是权威哨兵 ⇒ 单列「未给出方向结论」，不套档位标签、也不说两侧一致", () => {
    const { container } = render(
      <F7ReferenceComparison
        snapshot={{ ...ready, action: "数据缺失" }}
        {...prod}
        testId="ifx"
      />,
    );
    expect(container.querySelector('[data-testid="ifx-action"]')).toBeNull();
    expect(container.querySelector('[data-testid="ifx-action-absent"]')?.textContent)
      .toBe(`${P}.actionAbsent`);
    expect(container.querySelector('[data-testid="ifx-action-delta"]')?.textContent)
      .toBe(`${P}.actionNotComparable`);
    expect(container.textContent).not.toContain(`${P}.actionSame`);
  });
});

describe("F7ReferenceComparison · 其余三项", () => {
  it("权重坍缩为 true 才出警示句（false 不得显示，显示即谎报本侧不可用）", () => {
    const on = render(
      <F7ReferenceComparison snapshot={{ ...ready, weightsCollapsed: true }} {...prod} testId="ifx" />,
    );
    expect(on.container.querySelector('[data-testid="ifx-collapsed"]')?.textContent)
      .toBe(`${P}.weightsCollapsed`);
    expect(on.container.querySelector('[data-testid="ifx"]')?.getAttribute("data-collapsed")).toBe("true");

    const off = render(<F7ReferenceComparison snapshot={ready} {...prod} testId="ifx" />);
    expect(off.container.querySelector('[data-testid="ifx-collapsed"]')).toBeNull();
    expect(off.container.querySelector('[data-testid="ifx"]')?.getAttribute("data-collapsed")).toBe("false");
  });

  it("缺口按条数成句并带上原文；零缺口说「齐备」，不说「缺 0 项」", () => {
    const withGaps = render(
      <F7ReferenceComparison
        snapshot={{ ...ready, dataGaps: ["资金流向(t-hotmoney-data)", "龙虎榜"] }}
        {...prod}
        testId="ifx"
      />,
    );
    const txt = withGaps.container.querySelector('[data-testid="ifx-gaps"]')?.textContent ?? "";
    expect(txt).toContain(`${P}.gapsPresent`);
    expect(txt).toContain('"n":2');
    expect(txt).toContain("资金流向(t-hotmoney-data)");
    expect(withGaps.container.querySelector('[data-testid="ifx-gaps-none"]')).toBeNull();

    const none = render(<F7ReferenceComparison snapshot={ready} {...prod} testId="ifx" />);
    expect(none.container.querySelector('[data-testid="ifx-gaps-none"]')?.textContent)
      .toBe(`${P}.gapsNone`);
    expect(none.container.textContent).not.toContain(`${P}.gapsPresent`);
  });

  it("后验/风险档/f7 占比取不到时显示破折号，风险档走权威 tKey 表", () => {
    const full = render(<F7ReferenceComparison snapshot={ready} {...prod} testId="ifx" />);
    expect(full.container.querySelector('[data-testid="ifx-posterior"]')?.textContent).toBe("61.5%");
    expect(full.container.querySelector('[data-testid="ifx-risk-level"]')?.textContent)
      .toBe("stockAnalysis.riskMid");
    expect(full.container.querySelector('[data-testid="ifx-f7-weight"]')?.textContent)
      .toContain(`${P}.f7WeightValue`);

    const bare = render(
      <F7ReferenceComparison
        snapshot={{ ...ready, posterior: null, riskLevel: null, f7WeightPct: null }}
        {...prod}
        testId="ifx"
      />,
    );
    expect(bare.container.querySelector('[data-testid="ifx-posterior"]')?.textContent).toBe("—");
    expect(bare.container.querySelector('[data-testid="ifx-risk-level"]')?.textContent).toBe("—");
    expect(bare.container.querySelector('[data-testid="ifx-f7-weight"]')?.textContent).toBe("—");
  });

  it("齐备态必带「同刻度可直接比」那句声明（缺了就会被当成第二套评分）", () => {
    const { container } = render(
      <F7ReferenceComparison snapshot={ready} {...prod} testId="ifx" />,
    );
    expect(container.querySelector('[data-testid="ifx-scale-note"]')?.textContent)
      .toBe(`${P}.scaleNote`);
  });
});
