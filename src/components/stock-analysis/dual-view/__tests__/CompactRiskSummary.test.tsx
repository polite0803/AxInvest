/**
 * chat 里的三视角风险摘要。
 *
 * 修复前：`risk-agg` 被标成「风险聚合」、`risk-aggregated` 参与平均与排序，
 * 且另用一套关键词词频算分（与主面板 confidence 口径不一致）。
 * 这里锁：标签正确、三视角固定三行、缺票明说、聚合容器不参与。
 */
import i18n from "@/i18n";
import { useStockAnalysisStore } from "@/stores";
import { render, screen } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { beforeEach, describe, expect, it } from "vitest";
import { CompactRiskSummary } from "../CompactRiskSummary";

function assessment(stance: string, confidence: number, positionPct: number): string {
  return JSON.stringify({ report: "正文", verdict: { stance, confidence, position_pct: positionPct } });
}

const DATA: Record<string, string> = {
  "risk-agg": assessment("aggressive", 45, 20),
  "risk-con": assessment("conservative", 55, 0),
  "risk-neu": assessment("neutral", 45, 20),
  "risk-level": JSON.stringify({ category: "极高风险", matched_rules: [], axisScope: {}, metrics: {} }),
};

function renderCompact(data: Record<string, string>) {
  return render(
    <I18nextProvider i18n={i18n}>
      <CompactRiskSummary data={data} />
    </I18nextProvider>,
  );
}

beforeEach(() => {
  useStockAnalysisStore.setState({ untrustedNodes: {} });
});

describe("CompactRiskSummary — chat 三视角摘要", () => {
  it("三视角固定三行，显示 verdict 原值；risk-agg 标签是「激进」不是「风险聚合」", () => {
    renderCompact(DATA);

    expect(screen.getAllByText(/20% · 45/).length).toBe(2);
    expect(screen.getByText(/0% · 55/)).toBeTruthy();
    expect(screen.getByText("极高风险")).toBeTruthy();
    expect(screen.queryByText("风险聚合")).toBeNull();
    expect(screen.queryByText(i18n.t("stockAnalysis.riskMatrix.noVote"))).toBeNull();
  });

  it("缺一票时明说，不拿别的视角顶替", () => {
    const data = { ...DATA };
    delete data["risk-neu"];
    renderCompact(data);

    expect(screen.getByText(i18n.t("stockAnalysis.riskMatrix.noVote"))).toBeTruthy();
    expect(screen.getAllByText(/20% · 45/).length).toBe(1);
  });

  it("聚合容器不参与：只有 risk-aggregated 时按无数据处理", () => {
    renderCompact({ "risk-aggregated": JSON.stringify([{ content: "聚合" }]) });

    expect(screen.getByText(i18n.t("stockAnalysis.noRiskData"))).toBeTruthy();
  });
});
