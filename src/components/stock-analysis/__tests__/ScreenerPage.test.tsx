import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import { render } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: unknown) => {
      if (typeof opts === "string") { return opts; }
      if (opts && typeof opts === "object" && "defaultValue" in opts) {
        return String((opts as { defaultValue: string }).defaultValue);
      }
      return key;
    },
    // RecommendationPanel accesses i18n.language in useCallback deps.
    // Provide a stub so the panel can mount under the test mock.
    i18n: { language: "en-US" },
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

vi.mock("@/lib/invoke", () => ({
  invoke: vi.fn().mockResolvedValue([]),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

import { ScreenerPage } from "../ScreenerPage";

const renderWithRouter = () => render(<ScreenerPage />, { wrapper: MemoryRouter });

afterEach(() => {
  useTimeAnchorStore.setState({ asOfDate: null, mode: "live" });
});

describe("ScreenerPage", () => {
  it("renders without crashing", () => {
    const { container } = renderWithRouter();
    expect(container).toBeTruthy();
  });

  it("renders no legacy page header (embedded in InvestHub)", () => {
    const { container } = renderWithRouter();
    // 内嵌 InvestHub 后不再渲染 PageHeader 的返回按钮和标题
    expect(container.querySelector(".sa-header-back")).toBeNull();
    expect(container.querySelector(".sa-header-title")).toBeNull();
  });

  it("does not render its own live/as-of anchor (Hub title bar is the single mode layer)", () => {
    const { container } = renderWithRouter();
    // 世界观切换已提升为投资中心标题栏的顶层模式控件，选股页内不得再有锚点
    expect(container.querySelector('[data-testid="page-time-anchor"]')).toBeNull();
  });

  it("renders a single Tabs container with smart-reco as default", () => {
    const { container } = renderWithRouter();
    const tabs = container.querySelector(".ant-tabs");
    expect(tabs).toBeTruthy();
    // Only count the page-level Tabs header (not the nested period Tabs
    // inside RecommendationPanel, which has 3 short/mid/long entries).
    const navHeaders = tabs?.querySelectorAll(":scope > .ant-tabs-nav .ant-tabs-tab");
    // 三个顶层平级 tab：智能荐股 / 趋势智选 / 我的筛选
    expect(navHeaders?.length).toBe(3);
    // Default active tab should be the smart-reco tab.
    const activeHeader = tabs?.querySelector(".ant-tabs-nav .ant-tabs-tab-active");
    expect(activeHeader?.textContent).toContain("screener.tab.smartReco");
  });

  it("renders all three tab labels via i18n", () => {
    const { container } = renderWithRouter();
    expect(container.textContent).toContain("screener.tab.smartReco");
    expect(container.textContent).toContain("screener.tab.myFilter");
    expect(container.textContent).toContain("screener.tab.serenity");
  });

  it("renders 3 accordion items for HotStocks / LimitUp / DragonTiger", () => {
    const { container } = renderWithRouter();
    // 底部市场氛围折叠用 testid 精确定位（批次卡片内部也含 Collapse，不能裸取 .ant-collapse）
    const collapse = container.querySelector('[data-testid="screener-market-atmosphere"]');
    expect(collapse).toBeTruthy();
    const items = collapse?.querySelectorAll(".ant-collapse-item");
    expect(items?.length).toBe(3);
  });

  it("defaults to all accordion items collapsed", () => {
    const { container } = renderWithRouter();
    const collapse = container.querySelector('[data-testid="screener-market-atmosphere"]');
    const expanded = collapse?.querySelectorAll(".ant-collapse-item-active");
    expect(expanded?.length).toBe(0);
  });

  it("does not host the time-travel batch replay card (belongs to /replay-workbench)", () => {
    // 归属边界：批次复盘是历史验证逻辑，与当日荐股生产不同时不同逻辑，
    // 无论 live 还是 replay 模式都不出现在选股页
    useTimeAnchorStore.setState({ asOfDate: null, mode: "live" });
    {
      const { container, unmount } = renderWithRouter();
      expect(container.querySelector('[data-testid="ttb-start-btn"]')).toBeNull();
      unmount();
    }
    useTimeAnchorStore.setState({ asOfDate: "2026-05-04", mode: "replay" });
    {
      const { container } = renderWithRouter();
      expect(container.querySelector('[data-testid="ttb-start-btn"]')).toBeNull();
    }
  });
});
