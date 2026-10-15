// 运行门：组合面板必须把「几只持仓没有现价」说出来（PLAN 阶段 G 的 G-1）。
//
// 回归对象：集中度与分散度的只数口径从「表里的行数」改成「拿到现价的只数」之后，
// 如果面板不显示缺价只数，就会出现「按 5 只算出的分散度 70」配着 8 行持仓显示 ——
// 同一张面板两套口径，而读者无从知道分数是按几只算的。
//
// 生产侧（后端 DTO 的字段名）由 `scripts/check-monitor-event-authority.mjs` 的 R13 逐键对账；
// 本文件只管渲染侧：出不出这一句、参数带没带对、与集中度告警是否各占一句。

import { render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: unknown) => opts && typeof opts === "object" ? `${key}|${JSON.stringify(opts)}` : key,
    i18n: { language: "zh-CN" },
  }),
  // store → browserMock → src/i18n 那条链在模块图上就会 `i18n.use(initReactI18next)`，
  // 缺这个导出会让整个测试文件在导入期崩掉（不是被测代码的问题）
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

/** 可变的 store 替身：每条测试只改自己那一格（`state.dash`）。 */
const state: { dash: Record<string, unknown> | null } = { dash: null };

vi.mock("@/stores/feature/stockAnalysisStore", () => ({
  useStockAnalysisStore: (sel: (s: Record<string, unknown>) => unknown) =>
    sel({
      portfolioDashboard: state.dash,
      portfolioCorrelations: [],
      portfolioRefreshing: false,
      portfolioLastError: null,
      fetchPortfolioDashboard: vi.fn(),
      fetchPortfolioCorrelations: vi.fn(),
      refreshPortfolioMetrics: vi.fn(),
    }),
}));

vi.mock("@/stores/feature/timeAnchorStore", () => ({
  useTimeAnchorStore: (sel: (s: Record<string, unknown>) => unknown) => sel({ asOfDate: null, mode: "live" }),
}));

import { PortfolioMonitorPanel } from "../PortfolioMonitorPanel";

/** 与后端 `PortfolioDashboard` 的 19 个键逐项对齐（少一键就会被 R13 抓到）。 */
function dash(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    isHistorical: false,
    asOfDate: undefined,
    totalMarketValue: 9000,
    totalPnl: -1000,
    totalPnlPct: -10,
    unpricedPositions: 0,
    cashPct: 0,
    maxDrawdownPct: 0,
    beta: undefined,
    sharpe30d: undefined,
    correlationAvg: undefined,
    topConcentrationPct: 66.7,
    sectorExposure: { 科技: 100 },
    concentrationWarning: undefined,
    riskLevel: "高风险",
    diversificationScore: 30,
    stressTest: { m10: undefined, m20: undefined, blackSwan: undefined },
    positions: [],
    snapshotAt: 1_760_000_000_000,
    ...overrides,
  };
}

const body = () => document.body.textContent ?? "";

describe("PortfolioMonitorPanel —— 缺价只数上屏", () => {
  it("有缺价 ⇒ 独占一句并把只数带进文案", () => {
    state.dash = dash({ unpricedPositions: 3 });
    render(<PortfolioMonitorPanel />);
    expect(body()).toContain("stockAnalysis.portfolioMonitor.unpricedNotice");
    expect(body()).toContain('"count":3');
    expect(body()).toContain("stockAnalysis.portfolioMonitor.unpricedTitle");
  });

  it("没有缺价 ⇒ 这一句不出（不许常驻一条「0 只」的噪声）", () => {
    state.dash = dash({ unpricedPositions: 0 });
    render(<PortfolioMonitorPanel />);
    expect(body()).not.toContain("stockAnalysis.portfolioMonitor.unpricedNotice");
  });

  it("缺价句与集中度告警各占一句，不相并", () => {
    state.dash = dash({
      unpricedPositions: 2,
      concentrationWarning: "单股集中度 67% 过高，建议 ≤30%",
    });
    render(<PortfolioMonitorPanel />);
    expect(body()).toContain("单股集中度 67% 过高，建议 ≤30%");
    expect(body()).toContain('"count":2');
    expect(body()).not.toContain("0 只持仓没有现价");
  });

  it("只有集中度告警时不得顺手冒出缺价句", () => {
    state.dash = dash({
      unpricedPositions: 0,
      concentrationWarning: "行业暴露 100% 过高，建议 ≤40%",
    });
    render(<PortfolioMonitorPanel />);
    expect(body()).toContain("stockAnalysis.portfolioMonitor.warningTitle");
    expect(body()).not.toContain("stockAnalysis.portfolioMonitor.unpricedNotice");
  });
});
