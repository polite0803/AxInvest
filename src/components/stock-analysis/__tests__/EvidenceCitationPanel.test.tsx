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

import { EvidenceCitationPanel, notCheckedReasonKey } from "../EvidenceCitationPanel";

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
  closure: { state: "closed", recomputed: 0.084, stored: 0.08, tolerance: 0.0057 },
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
    participates: true,
    reconcile: { state: "consistent", delta: 0.004, tolerance: 0.0055, basis: "/risk_quant/f4_adjusted" },
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
    // A 面：无基准与一致各自成句，闭合有读数
    expect(screen.getAllByText("stockAnalysis.evidenceCitation.sigma.noBasis").length).toBe(2);
    expect(screen.getByText("stockAnalysis.evidenceCitation.sigma.consistent")).toBeTruthy();
    expect(screen.getByText("stockAnalysis.evidenceCitation.closure.closed")).toBeTruthy();
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
