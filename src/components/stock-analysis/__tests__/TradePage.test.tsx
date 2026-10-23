import { act, render } from "@testing-library/react";
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

import { TradePage } from "../TradePage";

const renderWithRouter = () => render(<TradePage />, { wrapper: MemoryRouter });

/**
 * 收尾：等挂载副作用（数据请求）落地。
 *
 * 为什么必须有：页面挂载后会异步拉数据，测试同步结束 ⇒ setState 落在测试之外 ⇒
 * React 报 "not wrapped in act"。为什么必须放在**断言之后**：这些用例断言的是
 * **初始渲染态**，提前 flush 会把组件推进"加载完成"态，断言随之失效（实测会让 3 条用例变红）。
 * 为什么不能用 afterEach：React 是在 setState 那一刻检查的，而那一刻在测试函数返回后、
 * afterEach 之前 —— 收尾钩子永远太晚。
 */
const settle = async () => {
  await act(async () => {});
};

describe("TradePage", () => {
  it("renders without crashing", async () => {
    const { container } = renderWithRouter();
    expect(container).toBeTruthy();
    await settle();
  });

  it("renders back button with chat label", async () => {
    const { container } = renderWithRouter();
    const backButton = container.querySelector(".sa-header-back");
    expect(backButton).toBeTruthy();
    expect(backButton?.textContent).toContain("nav.chat");
    await settle();
  });

  it("renders the page title via i18n key", async () => {
    const { container } = renderWithRouter();
    const title = container.querySelector(".sa-header-title");
    expect(title).toBeTruthy();
    expect(title?.textContent).toBe("trade.title");
    await settle();
  });

  it("does not render its own live/as-of anchor (Hub title bar is the single mode layer)", async () => {
    const { container } = renderWithRouter();
    expect(container.querySelector('[data-testid="page-time-anchor"]')).toBeNull();
    await settle();
  });

  it("renders PortfolioMonitorPanel, TradePanel, TradeImportPanel and ExecutionReplayPanel in vertical stack", async () => {
    const { container } = renderWithRouter();
    const body = container.querySelector(".space-y-4");
    expect(body).toBeTruthy();
    expect(body?.children.length).toBe(4);
    await settle();
  });
});
