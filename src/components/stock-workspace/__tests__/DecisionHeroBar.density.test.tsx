import i18n from "@/i18n";
import { useStockAnalysisStore, useUIStore, useWorkspaceStore } from "@/stores";
import type { StockDecision } from "@/types";
import { render, screen } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import { DecisionHeroBar } from "../DecisionHeroBar";

function renderHero() {
  return render(
    <I18nextProvider i18n={i18n}>
      <DecisionHeroBar />
    </I18nextProvider>,
  );
}

const DECISION = {
  action: "BUY",
  positionPct: 20,
  targetPrice: 12,
  stopLoss: 9,
  riskLevel: "MID",
  confidence: 70,
  reasoning: "测试决策",
} as unknown as StockDecision;

beforeAll(async () => {
  await i18n.changeLanguage("zh-CN");
});

beforeEach(() => {
  useStockAnalysisStore.getState().reset();
  useUIStore.setState({ deviceLayout: "desktop" });
  useWorkspaceStore.setState({ userMode: "simple" });
  useStockAnalysisStore.setState({ stockCode: "600519", stockName: "贵州茅台" });
});

describe("DecisionHeroBar — 简洁/专业信息密度", () => {
  it("无决策时简洁模式是单行占位", () => {
    renderHero();
    expect(screen.getByTestId("decision-hero-empty-simple")).toBeInTheDocument();
    expect(screen.queryByTestId("decision-hero-empty-professional")).toBeNull();
  });

  it("无决策时专业模式把占位拆成标题行 + 提示行", () => {
    useWorkspaceStore.setState({ userMode: "professional" });
    renderHero();
    const placeholder = screen.getByTestId("decision-hero-empty-professional");
    expect(placeholder).toBeInTheDocument();
    expect(placeholder.textContent).toContain("暂无决策");
    expect(placeholder.textContent).toContain("请先运行分析");
    expect(screen.queryByTestId("decision-hero-empty-simple")).toBeNull();
  });

  it("有决策时止损/风险只在专业模式出现", () => {
    useStockAnalysisStore.setState({ decision: DECISION });
    const { unmount } = renderHero();
    expect(screen.queryByText("止损")).toBeNull();
    expect(screen.queryByText("风险")).toBeNull();
    unmount();

    useWorkspaceStore.setState({ userMode: "professional" });
    renderHero();
    expect(screen.getByText("止损")).toBeInTheDocument();
    expect(screen.getByText("风险")).toBeInTheDocument();
  });
});
