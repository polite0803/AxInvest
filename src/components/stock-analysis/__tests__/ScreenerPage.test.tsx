import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import { act, render } from "@testing-library/react";
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

/**
 * 收尾：重置时间锚点 store 回到 live。
 *
 * 为什么包在 act 里：本钩子在 RTL 的 auto-cleanup **之前**执行（vitest 逆序跑 afterEach），
 * 此时上一条用例的组件树仍挂载；replay 用例的组件（ReplayBadge / ReplayWatermark 等）
 * 订阅了该 store，重置会触发它们重渲染 —— 不包 act 就是 "not wrapped in act" 警告。
 * 为什么用 async 形态：重置会把 RecommendationPanel 推回 live 分支重新拉缓存，
 * await invoke(...) 之后的 setState 是微任务续链，sync act 返回后才落地仍会告警；
 * async act 会按宏任务轮询到队列稳定，把这条续链也收进 act 窗口。
 * （同款修法见 RecommendationPanel.asOfDate.test.tsx，2026-10-14 实测）
 */
afterEach(async () => {
  await act(async () => {
    useTimeAnchorStore.setState({ asOfDate: null, mode: "live" });
  });
});

/**
 * 收尾：等挂载副作用（数据请求）落地。
 *
 * 为什么必须有：页面挂载后会异步拉数据，测试同步结束 ⇒ setState 落在测试之外 ⇒
 * React 报 "not wrapped in act"。为什么必须放在**断言之后**：这些用例断言的是
 * **初始渲染态**，提前 flush 会把组件推进"加载完成"态，断言随之失效。
 * 为什么不能用 afterEach：React 是在 setState 那一刻检查的，而那一刻在测试函数返回后、
 * afterEach 之前 —— 收尾钩子永远太晚。
 */
const settle = async () => {
  await act(async () => {});
};

describe("ScreenerPage", () => {
  it("renders without crashing", async () => {
    const { container } = renderWithRouter();
    expect(container).toBeTruthy();
    await settle();
  });

  it("renders no legacy page header (embedded in InvestHub)", async () => {
    const { container } = renderWithRouter();
    // 内嵌 InvestHub 后不再渲染 PageHeader 的返回按钮和标题
    expect(container.querySelector(".sa-header-back")).toBeNull();
    expect(container.querySelector(".sa-header-title")).toBeNull();
    await settle();
  });

  it("does not render its own live/as-of anchor (Hub title bar is the single mode layer)", async () => {
    const { container } = renderWithRouter();
    // 世界观切换已提升为投资中心标题栏的顶层模式控件，选股页内不得再有锚点
    expect(container.querySelector('[data-testid="page-time-anchor"]')).toBeNull();
    await settle();
  });

  it("renders a single Tabs container with smart-reco as default", async () => {
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
    await settle();
  });

  it("renders all three tab labels via i18n", async () => {
    const { container } = renderWithRouter();
    expect(container.textContent).toContain("screener.tab.smartReco");
    expect(container.textContent).toContain("screener.tab.myFilter");
    expect(container.textContent).toContain("screener.tab.serenity");
    await settle();
  });

  it("renders 3 accordion items for HotStocks / LimitUp / DragonTiger", async () => {
    const { container } = renderWithRouter();
    // 底部市场氛围折叠用 testid 精确定位（批次卡片内部也含 Collapse，不能裸取 .ant-collapse）
    const collapse = container.querySelector('[data-testid="screener-market-atmosphere"]');
    expect(collapse).toBeTruthy();
    const items = collapse?.querySelectorAll(".ant-collapse-item");
    expect(items?.length).toBe(3);
    await settle();
  });

  it("defaults to all accordion items collapsed", async () => {
    const { container } = renderWithRouter();
    const collapse = container.querySelector('[data-testid="screener-market-atmosphere"]');
    const expanded = collapse?.querySelectorAll(".ant-collapse-item-active");
    expect(expanded?.length).toBe(0);
    await settle();
  });

  it("does not host the time-travel batch replay card (belongs to /replay-workbench)", async () => {
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
    await settle();
  });
});
