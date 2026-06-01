import { render } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";
import { DecisionBanner } from "../DecisionBanner";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, fallback?: string) => fallback ?? _key,
  }),
}));

// Default mock: no decision
const storeState = {
  decision: null as {
    action: string;
    positionPct: number;
    reasoning: string;
    riskLevel: string;
    confidence: number;
    targetPrice?: number;
    stopLoss?: number;
  } | null,
};

vi.mock("@/stores", () => ({
  useStockAnalysisStore: (selector: (s: typeof storeState) => unknown) => selector(storeState),
  useSettingsStore: (selector: (s: { settings: { theme_mode: string } }) => unknown) =>
    selector({ settings: { theme_mode: "system" } }),
}));

describe("DecisionBanner", () => {
  it("renders nothing when no decision", () => {
    storeState.decision = null;
    const { container } = render(
      <MemoryRouter>
        <DecisionBanner />
      </MemoryRouter>,
    );
    expect(container.firstChild).toBeNull();
  });

  it("renders decision info when decision exists", () => {
    storeState.decision = {
      action: "买入",
      positionPct: 10.0,
      reasoning: "技术面突破",
      riskLevel: "中",
      confidence: 0.8,
      targetPrice: 1850.0,
      stopLoss: 1580.0,
    };
    const { container } = render(
      <MemoryRouter>
        <DecisionBanner />
      </MemoryRouter>,
    );
    expect(container.firstChild).not.toBeNull();
    expect(container.textContent).toContain("stockAnalysis.actionBuy");
    expect(container.textContent).toContain("10%");
  });
});
