import { fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string) => fallback ?? key,
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

vi.mock("@/lib/invoke", () => ({
  invoke: vi.fn().mockResolvedValue([]),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

import { BacktestPage } from "../BacktestPage";

const renderWithRouter = () => render(<BacktestPage />, { wrapper: MemoryRouter });

describe("BacktestPage", () => {
  it("renders without crashing", () => {
    const { container } = renderWithRouter();
    expect(container).toBeTruthy();
  });

  it("renders back button with chat label", () => {
    const { container } = renderWithRouter();
    const backButton = container.querySelector(".sa-header-back");
    expect(backButton).toBeTruthy();
    expect(backButton?.textContent).toContain("nav.chat");
  });

  it("renders the page title via i18n key", () => {
    const { container } = renderWithRouter();
    const title = container.querySelector(".sa-header-title");
    expect(title).toBeTruthy();
    expect(title?.textContent).toBe("stockAnalysis.backtest.title");
  });

  it("does not render its own live/as-of anchor (Hub title bar is the single mode layer)", () => {
    const { container } = renderWithRouter();
    expect(container.querySelector('[data-testid="page-time-anchor"]')).toBeNull();
  });

  it("renders BacktestPanel and HistoricalAnalysisPanel in vertical stack", () => {
    const { container } = renderWithRouter();
    // 页面使用 Tabs，analysis 标签下的内容被 lazy-render，
    // Tabs 默认激活第一个 tab (quick)，需要切换到 analysis tab 才能看到 space-y-4
    const tabs = container.querySelector(".ant-tabs");
    expect(tabs).toBeTruthy();
    expect(tabs?.querySelector(".ant-tabs-tab")).toBeTruthy();
  });

  /**
   * 采集三连（段扫 → 补收盘 → 补历史）的落位：曾经埋在两层面板之下（矩阵卡尾部的
   * 「漏检核查」面板中部），实测用户「找半天都找不到」⇒ 现为策略页签第一张卡。
   * 断言两件事：三个按钮同屏可达；这张卡是页签里的第一张卡（不再在矩阵之后）。
   */
  it("策略页签第一屏就是采集三连，且排在回测矩阵之前", async () => {
    const { container } = renderWithRouter();
    const strategyTab = [...container.querySelectorAll(".ant-tabs-tab")].find((el) =>
      el.textContent?.includes("stockAnalysis.backtest.tabStrategy")
    );
    expect(strategyTab).toBeTruthy();
    fireEvent.click(strategyTab!);

    const scan = await screen.findByRole("button", { name: "stockAnalysis.moverRecall.fullScan" });
    expect(screen.getByRole("button", { name: "stockAnalysis.moverRecall.sweepClose" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "stockAnalysis.moverRecall.backfillHistory" })).toBeTruthy();

    const pane = scan.closest('[role="tabpanel"]');
    expect(pane).toBeTruthy();
    const firstCard = pane!.querySelector(".ant-card");
    expect(firstCard?.contains(scan)).toBe(true);
  });

  /**
   * 五段业务顺序与「不再套娃」：数据基础 → 策略表现 → 信号追踪 → 漏检核查 → 智选验证。
   * 旧形态把漏检面板嵌在矩阵卡内部（矩阵卡 → 内层面板 → 八块散文），既找不到入口、
   * 也读不出两段是不同业务。这里同时钉住顺序与卡片不嵌套。
   */
  it("策略页签按五段业务排序，且漏检核查是独立卡片", async () => {
    const { container } = renderWithRouter();
    const strategyTab = [...container.querySelectorAll(".ant-tabs-tab")].find((el) =>
      el.textContent?.includes("stockAnalysis.backtest.tabStrategy")
    );
    fireEvent.click(strategyTab!);

    const missTitle = await screen.findByText("stockAnalysis.moverRecall.title");
    const pane = missTitle.closest('[role="tabpanel"]');
    expect(pane).toBeTruthy();
    const text = pane!.textContent ?? "";

    const order = [
      "stockAnalysis.backtest.sectionData",
      "stockAnalysis.backtest.sectionMatrix",
      "stockAnalysis.backtest.sectionSignals",
      "stockAnalysis.backtest.sectionMisses",
      "stockAnalysis.backtest.sectionSerenity",
    ].map((k) => text.indexOf(k));
    expect(order.every((i) => i >= 0)).toBe(true);
    expect([...order].sort((a, b) => a - b)).toEqual(order);

    // 不嵌在任何外层卡里：旧形态是「矩阵卡 → 内层漏检面板 → 八块散文」两层套娃
    const missCard = missTitle.closest(".ant-card");
    expect(missCard).toBeTruthy();
    expect(missCard!.parentElement?.closest(".ant-card")).toBeNull();
  });
});
