// 回归（2026-10-16 换判据后）三件事：
// ① 决策未产出时仍显式声明「不适用」，不得把「没有可审计对象」渲染成任何 0 值读数；
// ② 面板打开只发**一次** `audit_decision_evidence`（旧回归：`t` 进 useCallback 依赖 ⇒ 自激）；
// ③ 三种缺席**各占一句**：静默缺席 / 无法核查 / 未交代 —— 每一格都有自己的文案，
//    破坏样本只应让对应那一条红。
// ⚠ `t` 刻意每次渲染新建，还原 react-i18next 的真实形态。
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: () => {} },
  useTranslation: () => ({ t: (key: string) => key }),
}));

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import {
  type Closure,
  closureCoverage,
  closureLabel,
  closureTooltip,
  EvidenceCitationPanel,
  fmtNum,
  notCheckedReasonKey,
  sigmaLabelKey,
  sigmaTagColor,
  sigmaTooltip,
} from "../EvidenceCitationPanel";

/** 夹具按 600406 · 2026-10-15 真记录的**形状**裁剪（腿名/角色/档位取真值）。 */
const BASE = {
  stockCode: "600406",
  stockName: "国电南瑞",
  analysisDate: "2026-10-15",
  decisionAction: "持有",
  decisionConfidence: 51.7,
  decisionDegraded: false,
  tier: "ultra_short",
  expectedBranchLegs: 2,
  participatingBranchLegs: 2,
  upstreamPresentLegs: 1,
  upstreamNotCheckedLegs: 1,
  sigmaReconciledLegs: 1,
  participatingFactorLegs: 2,
  factorLegsTotal: 3,
  closure: { state: "closed", recomputed: 0.084, stored: 0.08, tolerance: 0.0057, unverifiedWeightRatio: 0.4 },
};

function leg(over: Record<string, unknown>) {
  return {
    factor: "momentumSignal",
    owner: "a-market-analyst",
    ownerName: "技术面分析师",
    role: "direction",
    signal: 0.5,
    weight: 0.565217,
    participates: true,
    expectedToParticipate: true,
    upstream: { state: "present" },
    absenceDeclared: false,
    ...over,
  };
}

const FACTOR_LEGS = [
  {
    legId: "f4",
    name: "risk",
    sigma: -0.174,
    weight: 0.15,
    owner: null,
    // ⚠ 刻意构造成**独立上游**基准：真记录里 f4 的基准与 σ 同变量（sameVariableCopy），
    //   但本用例需要一条「真的核对过」的绿腿来和下面 f11 的同源副本做区分对照。
    participates: true,
    reconcile: {
      state: "consistent",
      delta: 0.004,
      tolerance: 0.0055,
      basis: "/risk_quant/f4_adjusted",
      source: "independentUpstream",
    },
  },
  {
    legId: "f1",
    name: "trend",
    sigma: 0.56,
    weight: 0.105,
    owner: "a-market-analyst",
    participates: true,
    reconcile: { state: "noBasis", reason: "NO_PERSISTED_BASIS" },
  },
  {
    legId: "f13",
    name: "bottleneck",
    sigma: 0.0,
    weight: 0.0,
    owner: "a-sector",
    participates: false,
    reconcile: { state: "noBasis", reason: "NO_PERSISTED_BASIS" },
  },
  {
    // 真记录同形：基准 `/evidence/f11_pace_signal` 与 σ 都是 `f11_signal` 的落库副本
    legId: "f11",
    name: "pace",
    sigma: 0.31,
    weight: 0.052,
    owner: "a-sentiment",
    participates: true,
    reconcile: {
      state: "consistent",
      delta: 0.0,
      tolerance: 0.0055,
      basis: "/evidence/f11_pace_signal",
      source: "sameVariableCopy",
    },
  },
];

beforeEach(() => invokeMock.mockReset());

describe("不适用原因码 → 文案键（纯映射，不走渲染）", () => {
  it("每个码有专属文案，未知码走兜底而不是猜原因", () => {
    // 后端现役四个码（`evidence_audit.rs` 的 UpstreamState::NotChecked）
    expect(notCheckedReasonKey("TIER_SCORE_NOT_IN_SNAPSHOT")).toBe("tierScoreNotInSnapshot");
    expect(notCheckedReasonKey("NO_CHANNEL_MAPPING")).toBe("noChannelMapping");
    expect(notCheckedReasonKey("NO_OWNER")).toBe("noOwner");
    expect(notCheckedReasonKey("UNREGISTERED_FACTOR")).toBe("unregisteredFactor");
    // 负控：退役码与编造码都必须落到兜底，不许复用某一具体原因（那会把未知说成已知）
    expect(notCheckedReasonKey("NO_SNAPSHOT_KEY")).toBe("unknownReason");
    expect(notCheckedReasonKey("")).toBe("unknownReason");
  });
});

describe("EvidenceCitationPanel 判据换新后的三态", () => {
  it("decisionDegraded=true 时显示不适用提示，且不渲染任何计数与状态标签", async () => {
    invokeMock.mockResolvedValue({ ...BASE, decisionDegraded: true, branchLegs: [], factorLegs: [] });
    render(<EvidenceCitationPanel analysisId="a-1" />);

    expect(await screen.findByText("stockAnalysis.evidenceCitation.notApplicable")).toBeTruthy();
    expect(screen.queryByText("stockAnalysis.evidenceCitation.branchSection")).toBeNull();
    expect(screen.queryByText("stockAnalysis.evidenceCitation.factorSection")).toBeNull();
    expect(screen.queryByText("stockAnalysis.evidenceCitation.upstream.present")).toBeNull();
  });

  it("一次挂载只取一次数（回归：t 进 useCallback 依赖导致 effect 自激）", async () => {
    invokeMock.mockResolvedValue({ ...BASE, branchLegs: [leg({})], factorLegs: [] });
    render(<EvidenceCitationPanel analysisId="a-3" />);
    await waitFor(() => expect(screen.queryByText("stockAnalysis.evidenceCitation.loading")).toBeNull());
    await new Promise((r) => setTimeout(r, 120));
    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith("audit_decision_evidence", { analysisId: "a-3" });
  });

  it("三种缺席各占一句，互不合并", async () => {
    invokeMock.mockResolvedValue({
      ...BASE,
      branchLegs: [
        leg({}),
        leg({
          factor: "eventCatalyst",
          ownerName: "催化剂分析师",
          role: "filter",
          weight: 0,
          participates: false,
          absenceDeclared: true,
          upstream: { state: "declaredAbsent" },
        }),
        leg({
          factor: "breadthState",
          ownerName: "情绪分析师",
          role: "direction",
          weight: 0.2,
          participates: true,
          absenceDeclared: false,
          upstream: { state: "silentAbsent" },
        }),
        leg({
          factor: "valuationBand",
          ownerName: "价值投资者",
          role: "direction",
          weight: 0.1,
          participates: true,
          absenceDeclared: false,
          upstream: { state: "notChecked", reason: "TIER_SCORE_NOT_IN_SNAPSHOT" },
        }),
        leg({
          factor: "supplyShock",
          ownerName: "解禁观察",
          role: "filter",
          weight: 0,
          participates: false,
          absenceDeclared: false,
          upstream: { state: "notChecked", reason: "TIER_SCORE_NOT_IN_SNAPSHOT" },
        }),
      ],
      factorLegs: FACTOR_LEGS,
    });
    render(<EvidenceCitationPanel analysisId="a-4" />);

    // 四态标签各自成句
    expect(await screen.findByText("stockAnalysis.evidenceCitation.upstream.present")).toBeTruthy();
    expect(screen.getByText("stockAnalysis.evidenceCitation.upstream.declaredAbsent")).toBeTruthy();
    expect(screen.getByText("stockAnalysis.evidenceCitation.upstream.silentAbsent")).toBeTruthy();
    expect(screen.getAllByText("stockAnalysis.evidenceCitation.upstream.notChecked").length).toBe(2);
    // 「无法核查」与「静默缺席」必须分开数（前者不是缺席）
    expect(screen.queryByText("stockAnalysis.evidenceCitation.upstream.silentAbsent")).toBeTruthy();
    // 「未交代」只在「不参与且无缺席声明」的那一腿上出现 ⇒ 恰好一条
    expect(screen.getAllByText("stockAnalysis.evidenceCitation.absenceUndeclared").length).toBe(1);
    // A 面：无基准与一致各自成句，闭合带覆盖率读数
    expect(screen.getAllByText("stockAnalysis.evidenceCitation.sigma.noBasis").length).toBe(2);
    // 「真的核对过」与「同源副本复算」必须分成两句话 —— 修前二者共用一个 `consistent` 绿标
    expect(screen.getByText("stockAnalysis.evidenceCitation.sigma.consistent")).toBeTruthy();
    expect(screen.getByText("stockAnalysis.evidenceCitation.sigma.sameSourceCopy")).toBeTruthy();
    // 闭合必须带覆盖率（`unverifiedWeightRatio=0.4` ⇒ 覆盖 60%），不能只说「闭合」
    expect(screen.getByText(/evidenceCitation\.closure\.closed.*closure\.coverage 60%/))
      .toBeTruthy();
    // 元约束腿（owner=null）不得显示空白，必须有专门文案
    expect(screen.getByText("stockAnalysis.evidenceCitation.metaLeg")).toBeTruthy();
  });

  it("闭合越界时只红闭合一格，不牵连各腿状态", async () => {
    invokeMock.mockResolvedValue({
      ...BASE,
      closure: { state: "broken", recomputed: 0.5, stored: 0.08, tolerance: 0.0057 },
      branchLegs: [leg({})],
      factorLegs: FACTOR_LEGS,
    });
    render(<EvidenceCitationPanel analysisId="a-5" />);

    expect(await screen.findByText("stockAnalysis.evidenceCitation.closure.broken")).toBeTruthy();
    expect(screen.queryByText("stockAnalysis.evidenceCitation.closure.closed")).toBeNull();
    // 腿状态不受闭合影响：仍是一致 / 无基准
    expect(screen.getByText("stockAnalysis.evidenceCitation.sigma.consistent")).toBeTruthy();
  });

  it("没有任何腿时停在空态，不渲染两面区块", async () => {
    invokeMock.mockResolvedValue({ ...BASE, branchLegs: [], factorLegs: [] });
    render(<EvidenceCitationPanel analysisId="a-6" />);
    expect(await screen.findByText("stockAnalysis.evidenceCitation.noData")).toBeTruthy();
    expect(screen.queryByText("stockAnalysis.evidenceCitation.branchSection")).toBeNull();
  });
});

describe("判据分档：同源副本 / 覆盖率 / 应参与未计入（2026-10-18 假绿修复）", () => {
  const tk = (key: string) => key;

  it("σ 标签：只有独立上游才算「一致」，同源副本单独成档且不染绿", () => {
    const independent = {
      state: "consistent",
      delta: 0,
      tolerance: 0.0055,
      basis: "/result.t-scoring/content/totalScore",
      source: "independentUpstream",
    } as const;
    const copy = {
      state: "consistent",
      delta: 0,
      tolerance: 0.0055,
      basis: "/evidence/f11_pace_signal",
      source: "sameVariableCopy",
    } as const;

    expect(sigmaLabelKey(independent)).toBe("consistent");
    expect(sigmaLabelKey(copy)).toBe("sameSourceCopy");
    expect(sigmaLabelKey({ state: "inconsistent", delta: 1, tolerance: 0.0055, basis: "b" }))
      .toBe("inconsistent");
    expect(sigmaLabelKey({ state: "noBasis", reason: "NO_PERSISTED_BASIS" })).toBe("noBasis");

    // 负控：两档不得共用同一个颜色，否则只改文案仍看不出区别
    expect(sigmaTagColor(independent)).toBe("green");
    expect(sigmaTagColor(copy)).not.toBe("green");
    // 同源副本的悬浮说明必须解释「为什么这个一致不算证据」
    expect(sigmaTooltip(copy, tk)).toContain("sigma.sameSourceCopyHint");
  });

  it("闭合必须带覆盖率；broken / notComputable 不得凭空给百分比", () => {
    const low: Closure = {
      state: "closed",
      recomputed: 0.084,
      stored: 0.08,
      tolerance: 0.0057,
      unverifiedWeightRatio: 0.63,
    };
    expect(closureCoverage(low)).toBeCloseTo(0.37, 10);
    expect(closureLabel(low, tk)).toBe(
      "stockAnalysis.evidenceCitation.closure.closed · stockAnalysis.evidenceCitation.closure.coverage 37%",
    );

    const full: Closure = { ...low, unverifiedWeightRatio: 0 };
    expect(closureCoverage(full)).toBe(1);

    expect(closureCoverage({ state: "broken", recomputed: 0, stored: 0, tolerance: 0 })).toBeNull();
    expect(closureLabel({ state: "notComputable", reason: "NO_WEIGHT_SUM_OR_FACTORS" }, tk)).toBe(
      "stockAnalysis.evidenceCitation.closure.notComputable",
    );
  });

  it("闭合失败时必须给出差值（修前 broken 连 tooltip 都没有）", () => {
    const tip = closureTooltip({ state: "broken", recomputed: 0.5, stored: 0.08, tolerance: 0.0057 }, tk);
    expect(tip).toContain("Δ=0.4200");
    expect(tip).toContain("> tol=0.0057");
    expect(tip).not.toContain("unverifiedWeight");
  });

  it("应参与却未计入的腿必须显式成标，否则「应参与 N / 实际参与 N-1」无法归因", async () => {
    invokeMock.mockResolvedValue({
      ...BASE,
      branchLegs: [
        leg({ factor: "earningsQuality", expectedToParticipate: true, participates: true }),
        // 应参与、有缺席声明、但没进加权 —— 修前这一行与「正常参与」在 UI 上同形
        leg({
          factor: "macroRegime",
          ownerName: "宏观分析师",
          role: "absent",
          weight: 0,
          participates: false,
          expectedToParticipate: true,
          absenceDeclared: true,
          upstream: { state: "declaredAbsent" },
        }),
        // 对照：本就不该参与的过滤腿不得挂这个标（证明它判的是 expectedToParticipate）
        leg({
          factor: "valuationBand",
          ownerName: "价值投资者",
          role: "filter",
          weight: 0,
          participates: false,
          expectedToParticipate: false,
          absenceDeclared: true,
          upstream: { state: "present" },
        }),
      ],
      factorLegs: FACTOR_LEGS,
    });
    render(<EvidenceCitationPanel analysisId="a-7" />);
    expect(await screen.findByText("stockAnalysis.evidenceCitation.expectedButIdle")).toBeTruthy();
    expect(screen.getAllByText("stockAnalysis.evidenceCitation.expectedButIdle").length).toBe(1);
  });
});

describe("输入缺失与「真的是 0」可分（2026-10-18 去掉 unwrap_or(0.0)）", () => {
  it("fmtNum：null 渲染成 — 而不是 0.000", () => {
    expect(fmtNum(null)).toBe("—");
    // 负控：显式 0 仍要显示成数字，否则「真零」会被误读成「没给」
    expect(fmtNum(0)).toBe("0.000");
    expect(fmtNum(0.105)).toBe("0.105");
  });

  it("σ 缺失与无基准必须分成两句（归因方向相反：缺被检对象 vs 缺参照物）", () => {
    expect(sigmaLabelKey({ state: "inputMissing", field: "sigma" })).toBe("inputMissing");
    expect(sigmaLabelKey({ state: "noBasis", reason: "NO_PERSISTED_BASIS" })).toBe("noBasis");
    // 缺失比「无基准」更需要被看见，不得共用默认灰
    expect(sigmaTagColor({ state: "inputMissing", field: "sigma" })).not.toBe("default");
    expect(sigmaTooltip({ state: "inputMissing", field: "sigma" }, (k) => k)).toContain(
      "sigma.inputMissingHint",
    );
  });

  it("两面都不得把缺失值渲染成 0", async () => {
    invokeMock.mockResolvedValue({
      ...BASE,
      branchLegs: [
        leg({
          factor: "earningsQuality",
          weight: null,
          signal: null,
          participates: false,
          expectedToParticipate: true,
        }),
      ],
      factorLegs: [{
        legId: "f4",
        name: "risk",
        sigma: null,
        weight: 0.15,
        owner: null,
        participates: true,
        reconcile: { state: "inputMissing", field: "sigma" },
      }],
      closure: { state: "notComputable", reason: "MISSING_FACTOR_INPUT" },
    });
    render(<EvidenceCitationPanel analysisId="a-8" />);

    expect(await screen.findByText("stockAnalysis.evidenceCitation.sigma.inputMissing")).toBeTruthy();
    // A 面：σ 缺失显示 —，权重照常有值
    expect(screen.getByText(/σ=— w=0\.150/)).toBeTruthy();
    // B 面：权重缺失同样显示 —，不出现 0.000
    expect(screen.getByText(/evidenceCitation\.weight=—/)).toBeTruthy();
    expect(screen.queryByText(/weight=0\.000/)).toBeNull();
  });
});
