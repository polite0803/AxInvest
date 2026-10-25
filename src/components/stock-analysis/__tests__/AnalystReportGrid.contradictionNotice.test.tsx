// i18n-exempt: 断言按 i18n key 判（useTranslation 被 mock 成回原 key + 插值），无 UI 文案。
import { render, screen } from "@testing-library/react";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AnalystReportGrid } from "../AnalystReportGrid";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) => opts ? `${key}|${JSON.stringify(opts)}` : key,
    i18n: { language: "zh-CN" },
  }),
  // import 链上的 `@/i18n` 会 `i18n.use(initReactI18next)` —— 不给这个导出，
  // 整文件在收集阶段就报「No export defined on mock」（一条测试都跑不到）。
  initReactI18next: { type: "3rdParty", init: () => {} },
  Trans: ({ children }: { children?: ReactNode }) => <>{children}</>,
}));
// 子卡片会自行拉 store 与重依赖，本判据只关心顶部共识条那几格
vi.mock("../AnalystReportCard", () => ({
  AnalystReportCard: () => <div data-testid="analyst-card" />,
}));
vi.mock("../HorizonScopeNotice", () => ({ HorizonScopeNotice: () => null }));

const BULL = '<!-- VERDICT: {"bull_score":80,"bear_score":20} -->';

const stateRef: { current: Record<string, unknown> } = { current: {} };

vi.mock("@/stores", () => ({
  useStockAnalysisStore: (sel: (s: Record<string, unknown>) => unknown) => sel(stateRef.current),
}));

function baseState(dataQualitySummary: string) {
  return {
    analystReports: {
      "market-analyst--mid": BULL,
      "fundamentals--mid": BULL,
      "sector--mid": '<!-- VERDICT: {"bull_score":20,"bear_score":80} -->',
    },
    decision: { timeHorizon: "mid" },
    dataQualitySummary,
    jevJudgments: {},
    streamingPreviews: {},
    failedNodes: [],
    failedNodeErrors: {},
    status: "idle",
  };
}

/** 三态各占一句：本档有人矛盾 / 判过且无人矛盾 / 旧快照没这个判据。 */
const dqWith = (reason: string) =>
  JSON.stringify({
    grade: "A",
    score: 90,
    diagnostics: { mk: { inconsistent_reason: "" } },
    diagnostics_by_tier: { mk: { mid: { inconsistent_reason: reason, tier: "mid" } } },
  });

describe("AnalystReportGrid 扣票说明", () => {
  beforeEach(() => {
    stateRef.current = {};
  });

  it("本档有人被判矛盾 ⇒ 共识条出现「未计票」那一格，并带上扣了几票", () => {
    stateRef.current = baseState(dqWith("verdict='看多' 但 bull_score(30.0) <= bear_score(60.0)"));
    render(<AnalystReportGrid />);
    const lines = screen.getAllByText(/stockAnalysis\.analystConsensusExcluded/);
    expect(lines).toHaveLength(1);
    expect(lines[0].textContent).toContain('"count":1');
    // 被扣的那位不得再进票：3 位分析师扣 1 ⇒ 票面总数是 2
    const counter = screen.getAllByText(/stockAnalysis\.tab\.analysts/)[0];
    expect(counter.textContent).toContain("· 2");
    expect(counter.textContent).not.toContain("· 3");
  });

  it("判过且无人矛盾 ⇒ 那一格不出现（不得凭空造出一个警告）", () => {
    stateRef.current = baseState(dqWith(""));
    render(<AnalystReportGrid />);
    expect(screen.queryByText(/stockAnalysis\.analystConsensusExcluded/)).toBeNull();
  });

  it("旧快照没有这个判据 ⇒ 那一格不出现，也不扣票（不知道 ≠ 没人矛盾）", () => {
    const legacy = JSON.stringify({
      grade: "A",
      score: 90,
      diagnostics: { mk: { status: "normal" } },
    });
    stateRef.current = baseState(legacy);
    render(<AnalystReportGrid />);
    expect(screen.queryByText(/stockAnalysis\.analystConsensusExcluded/)).toBeNull();
  });
});
