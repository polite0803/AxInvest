// i18n-exempt: 断言按 props 透传判（组件被替身打桩），无 UI 文案。
/**
 * 持有期标识的三层接线（2026-10-25）。
 *
 * `resolveAnalystDiagnosis` 这一批多了 `tier` 形参，Grid 里也多了按档切换。
 * 只有「弹窗内部会按档取行」的用例（AnalystDataQualityModal.scope.test.tsx）不够 ——
 * 它证明的是**末节会用**，不证明**上游真把档名传下来了**。漏传时的症状是
 * 「超短卡点开弹窗显示中线的置信度」，同屏两个数字对不上却不报错，正是最难发现的一类。
 *
 * 本判据从 Grid 起手，把 `AnalystDataQualityModal` 打桩成把 props 打进 DOM 的替身，
 * 于是一条用例同时钉住 Grid→Card→Modal 三层：`tier` 必须等于当前档的 snake 名。
 */
import { render } from "@testing-library/react";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AnalystReportGrid } from "../AnalystReportGrid";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) => opts ? `${key}|${JSON.stringify(opts)}` : key,
    i18n: { language: "zh-CN" },
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
  Trans: ({ children }: { children?: ReactNode }) => <>{children}</>,
}));

// 替身：把卡片交给弹窗的标识原样打到 DOM 上，用于断言「传了哪一档」
vi.mock("../AnalystDataQualityModal", () => ({
  AnalystDataQualityModal: (props: { expertId: string; tier?: string | null }) => (
    <div data-testid="dq-modal">
      {`expert=${props.expertId};tier=${props.tier === undefined ? "undefined" : String(props.tier)}`}
    </div>
  ),
}));
vi.mock("../HorizonScopeNotice", () => ({ HorizonScopeNotice: () => null }));
vi.mock("markstream-react", () => ({ setCustomComponents: () => {} }));
vi.mock("../ReportMarkdown", () => ({
  ReportMarkdown: ({ content }: { content: string }) => <div>{content}</div>,
}));

const stateRef: { current: Record<string, unknown> } = { current: {} };

vi.mock("@/stores", () => ({
  useStockAnalysisStore: (sel: (s: Record<string, unknown>) => unknown) => sel(stateRef.current),
  useSettingsStore: (sel: (s: { settings: { themeMode: string } }) => unknown) =>
    sel({ settings: { themeMode: "dark" } }),
}));

const BULL = '<!-- VERDICT: {"verdict":"看多","bull_score":80,"bear_score":20,"confidence":66} -->';

function baseState(tier: string) {
  return {
    // 逐档实例键：只给一档，确保「显示的档」唯一可判
    analystReports: { [`market-analyst--${tier}`]: BULL },
    valueAssessments: {},
    riskAssessments: {},
    ruleCheckResults: {},
    rawData: {},
    decision: { timeHorizon: tier, action: "BUY", confidence: 60 },
    dataQualitySummary: JSON.stringify({
      grade: "A",
      score: 90,
      diagnostics: { mk: { name: "技术面分析师", confidence: 50, status: "normal", gap_reason: "" } },
      diagnostics_by_tier: {
        mk: {
          [tier]: { name: "技术面分析师", confidence: 50, status: "normal", gap_reason: "" },
        },
      },
    }),
    jevJudgments: {},
    streamingPreviews: {},
    failedNodes: [],
    failedNodeErrors: {},
    status: "idle",
  };
}

describe("AnalystReportGrid → 卡 → 数据质量弹窗的档位透传", () => {
  beforeEach(() => {
    stateRef.current = {};
  });

  it("超短档卡片打开的弹窗拿到的是 ultra_short（不是中线的代表实例）", () => {
    stateRef.current = baseState("ultra_short");
    render(<AnalystReportGrid />);
    const modal = document.body.querySelector('[data-testid="dq-modal"]');
    expect(modal, "Grid 未渲染出分析师卡（接线第一层断在哪）").not.toBeNull();
    expect(modal?.textContent).toContain("tier=ultra_short");
    expect(modal?.textContent).toContain("expert=market-analyst");
  });

  it("切到长档 ⇒ 透传跟着变（同一个弹窗组件吃不同档）", () => {
    stateRef.current = baseState("long");
    render(<AnalystReportGrid />);
    const modal = document.body.querySelector('[data-testid="dq-modal"]');
    expect(modal?.textContent).toContain("tier=long");
  });
});
