import { render, screen } from "@testing-library/react";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it, vi } from "vitest";
import { DecisionComparisonPanel } from "../DecisionComparisonPanel";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) => opts ? `${key}|${JSON.stringify(opts)}` : key,
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

const P = "stockAnalysis.f7Reference";

const snapshot = {
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

/**
 * 面板接线判据（不是渲染判据）：
 * 对照块必须拿到**生产公式侧**的那三个数（`decisionAction` / `confidence` /
 * `decisionPositionPct`）来算差值。接错成 LLM 侧的 `llmConfidence` /
 * `llmDecisionPositionPct`，界面照样出一句「差 +x」，但那个 x 说的是另一件事 ——
 * 这类错法在组件自己的测试里看不出来，只有从面板这一层喂一套不对称的值才暴露。
 */
describe("DecisionComparisonPanel · 独立公式对照的接线", () => {
  it("生产侧数值来自公式列，不是 LLM 列", () => {
    render(
      <DecisionComparisonPanel
        data={{
          decisionAction: "持有",
          confidence: 62,
          decisionPositionPct: 8,
          llmDecisionAction: "买入",
          llmConfidence: 999,
          llmDecisionPositionPct: 999,
          decisionAgreementScore: 70,
          f7ReferenceFormula: snapshot,
        }}
      />,
    );
    const conf = screen.getByTestId("f7-reference-conf-delta").textContent ?? "";
    expect(conf).toContain('"production":62');
    expect(conf).not.toContain("999");
    const pos = screen.getByTestId("f7-reference-pos-delta").textContent ?? "";
    expect(pos).toContain('"production":8');
    expect(screen.getByTestId("f7-reference-action-delta").textContent).toContain(
      '"production":"stockAnalysis.actionHold"',
    );
  });

  it("生产侧数值缺位 ⇒ 说「不做数值差值」，不拿 LLM 的数顶替", () => {
    render(
      <DecisionComparisonPanel
        data={{
          decisionAction: "持有",
          llmDecisionAction: "买入",
          llmConfidence: 55,
          llmDecisionPositionPct: 10,
          decisionAgreementScore: 70,
          f7ReferenceFormula: snapshot,
        }}
      />,
    );
    expect(screen.getByTestId("f7-reference-value-not-comparable")).not.toBeNull();
    expect(screen.queryByTestId("f7-reference-conf-delta")).toBeNull();
  });

  it("记录里没有该字段 ⇒ 面板内说「未产出」，双列对比照常渲染", () => {
    render(
      <DecisionComparisonPanel
        data={{
          decisionAction: "持有",
          confidence: 62,
          decisionPositionPct: 8,
          llmDecisionAction: "买入",
          llmConfidence: 70,
          decisionAgreementScore: 70,
        }}
      />,
    );
    expect(screen.getByTestId("f7-reference").getAttribute("data-state")).toBe("not-produced");
    expect(screen.getByTestId("f7-reference").textContent).toBe(`${P}.notProduced`);
    // 上面那一对「公式 vs LLM」不受影响（本块是加法，不是替换）
    expect(document.querySelector(".decision-comparison-panel")).not.toBeNull();
  });

  /**
   * 回退分支（LLM 整体不可用 + 无一致性分）不摆对照块：那时光侧 f7 也没有 trader 输入，
   * 「两侧之差 = LLM 观点的净贡献」这句解释不成立。钉住它，免得将来有人顺手加进去
   * 造出一句无法兑现的因果声明。
   */
  it("回退分支（LLM 不可用且无一致性分）：不渲染对照块", () => {
    render(<DecisionComparisonPanel data={{ decisionAction: "持有", confidence: 62 }} />);
    expect(screen.queryByTestId("f7-reference")).toBeNull();
    expect(screen.getByText("stockAnalysis.llmViewUnavailable")).toBeTruthy();
  });

  /**
   * 只在**主分支**渲染：当 LLM 侧整体不可用且没有一致性分时，面板退化成一列
   * 「LLM 不可用」的公式卡（`DecisionComparisonPanel.tsx` 的回退分支）—— 那时
   * 生产侧的 f7 本身也没有 trader 输入，两侧之差解释不了任何东西，故不摆对照块。
   * 本例因此把 LLM 侧与一致性分补齐，测的是主分支里的三种计数域互不借用。
   */
  it("两侧都齐备时，声明句与逐档对照互不借用（同屏三种计数域要各自成句）", () => {
    render(
      <DecisionComparisonPanel
        data={{
          decisionAction: "持有",
          confidence: 62,
          llmDecisionAction: "买入",
          llmConfidence: 70,
          decisionAgreementScore: 70,
          f7ReferenceFormula: snapshot,
        }}
      />,
    );
    expect(screen.getByTestId("f7-reference-scale-note").textContent).toBe(`${P}.scaleNote`);
    expect(screen.getByTestId("horizon-agreement-list").textContent)
      .toBe("stockAnalysis.horizonAgreementNotProduced");
  });
});

/**
 * 链路第 4 环的静态门（2026-10-25）。
 *
 * 上面几条测的是「面板 → 对照块」。`StockAnalysisPage` 把 `store.decision.f7ReferenceFormula`
 * 塞进 `dualViewData` 的那一行**不在**组件测试的覆盖面上 —— 它被删掉后本文件照样全绿，
 * 界面却永远显示「未产出」。这与 `normalizeDecision` 白名单漏字段是同一族静默失效，
 * 所以照 `crossCheckDrivers.test.ts` 的办法：从真实源文件按内容抽一遍再断。
 *
 * 判据写成整行字面量而不是正则：dprint 若重排这一行，本门**响亮地**红，
 * 而不是静默失去命中面。
 */
const PAGE = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
  "StockAnalysisPage.tsx",
);
const WIRE = "f7ReferenceFormula: store.decision?.f7ReferenceFormula ?? null";
const WIRE_SOURCE = "f7Source: store.decision?.f7Source ?? null";

describe("StockAnalysisPage · dualViewData 的 f7ReferenceFormula 接线", () => {
  const page = fs.readFileSync(PAGE, "utf8");

  it("页面把对照快照与主决策 f7 口径都透传进 dualViewData", () => {
    expect(page).toContain(WIRE);
    expect(page).toContain(WIRE_SOURCE);
  });

  // 负控：换成 LLM 侧右值（看着也像那么回事）后必须不再命中，否则这条门恒真
  it("负控：换成别的右值就命中不了", () => {
    const mutated = page.replace(WIRE, "f7ReferenceFormula: store.llmDecisionJson ?? null");
    expect(mutated).not.toBe(page);
    expect(mutated).not.toContain(WIRE);
    // 同一条负控也要覆盖 f7 口径那一行：它被删 ⇒ 面板把「前端没接」显示成「产端没声明」
    const mutated2 = page.replace(WIRE_SOURCE, "f7Source: null,");
    expect(mutated2).not.toBe(page);
    expect(mutated2).not.toContain(WIRE_SOURCE);
  });
});
