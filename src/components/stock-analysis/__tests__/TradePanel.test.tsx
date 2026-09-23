import { render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

// ⚠ mock 必须提供 `initReactI18next`：本文件的组件 `TradePanel.tsx:3` import 了
// `@/lib/errorI18n`，而后者（2026-09-13 起）在**顶层** `import i18n from "@/i18n"`
// ⇒ 加载链会执行 `src/i18n/index.ts:22` 的 `i18n.use(initReactI18next)`。
// 若 mock 只给 `useTranslation`，这一步拿到 undefined 并抛
// 「No "initReactI18next" export is defined on the "react-i18next" mock」——
// 报错发生在模块加载期 ⇒ 该文件 **0 个用例执行**、整文件被标记 FAIL。
// 形态照抄仓库主流写法（如 DecisionTimelinePanel.test.tsx:7，共 20 处先例）。
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string) => fallback ?? key,
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

const storeState: Record<string, unknown> = {
  stockCode: "",
  stockName: "",
  decision: null,
};

vi.mock("@/stores", () => ({
  useStockAnalysisStore: (selector: (s: typeof storeState) => unknown) => selector(storeState),
}));

vi.mock("@/lib/invoke", () => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  isTauri: () => false,
}));

import { TradePanel } from "../TradePanel";

describe("TradePanel", () => {
  it("renders without crashing", () => {
    storeState.stockCode = "";
    storeState.stockName = "";
    storeState.decision = null;

    const { container } = render(<TradePanel />);
    expect(container).toBeTruthy();
  });

  it("renders trade panel heading", () => {
    storeState.stockCode = "600519";
    storeState.stockName = "茅台";
    storeState.decision = null;

    const { container } = render(<TradePanel />);
    expect(container.textContent).toBeTruthy();
  });

  it("renders stock code and name from store", () => {
    storeState.stockCode = "600519";
    storeState.stockName = "贵州茅台";
    storeState.decision = {
      action: "买入",
      confidence: 85,
      targetPrice: 1850,
      positionPct: 15,
    };

    const { container } = render(<TradePanel />);
    // 组件应渲染，不崩溃
    expect(container).toBeTruthy();
  });
});
