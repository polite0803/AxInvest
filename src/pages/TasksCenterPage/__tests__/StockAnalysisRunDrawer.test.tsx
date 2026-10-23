import i18n from "@/i18n";
import type { AnalysisSummary } from "@/types";
import { render, screen } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";
import { StockAnalysisRunDrawer } from "../jobs/StockAnalysisRunDrawer";
import type { UnifiedRun } from "../model";

const RAW: AnalysisSummary = {
  id: "a1",
  stockCode: "300308",
  stockName: "中际旭创",
  analysisDate: "2026-10-21",
  status: "completed",
  decisionAction: "BUY",
  decisionPositionPct: 20,
  decisionPositionState: null,
  decisionJson: null,
  createdAt: 1_000,
  updatedAt: 4_000,
  completedAt: 4_000,
  analysisKind: "live",
  asOfDate: null,
  parentAnalysisId: null,
  templateId: "stock-analysis",
};

function makeRun(over: Partial<UnifiedRun> = {}, rawOver: Partial<AnalysisSummary> = {}): UnifiedRun {
  return {
    id: "stock-analysis:a1",
    source: "stock-analysis",
    nativeId: "a1",
    name: "x · 300308 中际旭创",
    trigger: "manual",
    state: "succeeded",
    attempts: 0,
    startedAt: 1_000,
    finishedAt: 4_000,
    durationMs: 3_000,
    deepLink: "/stock-analysis/a1",
    capabilities: { openSource: true },
    raw: { ...RAW, ...rawOver },
    ...over,
  };
}

function renderDrawer(run: UnifiedRun | null) {
  return render(
    <I18nextProvider i18n={i18n}>
      <MemoryRouter>
        <StockAnalysisRunDrawer run={run} onClose={() => {}} />
      </MemoryRouter>
    </I18nextProvider>,
  );
}

describe("StockAnalysisRunDrawer", () => {
  it("渲染标的、操作建议与「打开」按钮", async () => {
    renderDrawer(makeRun());
    expect(await screen.findByText("300308 中际旭创")).toBeInTheDocument();
    // 操作建议必须是权威档位文案，不是后端 token（原断言 `getByText("BUY")` 钉的是缺陷）
    expect(screen.getByText("买入")).toBeInTheDocument();
    expect(screen.queryByText("BUY")).not.toBeInTheDocument();
    // antd 会在两个汉字之间自动插空格（"打开" → "打 开"）
    expect(screen.getByRole("button", { name: /打\s*开/ })).toBeInTheDocument();
  });

  it("后端下发 UNAVAILABLE 哨兵 ⇒ 渲染「数据缺失」，不把内部 token 给用户", async () => {
    renderDrawer(makeRun({}, { decisionAction: "UNAVAILABLE" }));
    expect(await screen.findByText("数据缺失")).toBeInTheDocument();
    expect(screen.queryByText("UNAVAILABLE")).not.toBeInTheDocument();
  });

  it("库里档位存的是中文 ⇒ 与英文 token 渲染同一个档名", async () => {
    renderDrawer(makeRun({}, { decisionAction: "减持" }));
    expect(await screen.findByText("减持")).toBeInTheDocument();
  });

  it("interrupted 渲染「已中断」徽章，且不显示伪造耗时", async () => {
    renderDrawer(
      makeRun(
        { state: "canceled", badges: ["interrupted"], durationMs: undefined, finishedAt: undefined },
        { status: "interrupted", decisionAction: null },
      ),
    );
    expect(await screen.findByText(i18n.t("tasksCenter.badge.interrupted"))).toBeInTheDocument();
    // 结束时刻未知 ⇒ 耗时列必须是「-」，不能拿回收时刻算出超长耗时
    expect(screen.getAllByText("-").length).toBeGreaterThanOrEqual(2);
  });
});
