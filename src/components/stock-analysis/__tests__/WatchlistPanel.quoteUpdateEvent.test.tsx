// 运行门：`stock-quote-update` 的两端形状必须对得上。
//
// 回归对象（2026-10-09 实机）：wiring 层手写 `json!` 把 `current` 摊平成顶层
// `price` / `name` / …，而消费端按 `payload.current.*` 取值 ⇒ `current` 恒 undefined。
// 抛出点位于 `setQuotes((prev) => ...)` 的 updater 内，React 在 render 阶段执行 updater，
// 于是整页被 PageErrorBoundary 兜成「页面错误：Cannot read properties of undefined
// (reading 'price')」，且行情监视器每 2s 推一次 ⇒ 刷新必现。
// 现已改为直接序列化 `QuoteChangeEvent`
// （`src-tauri/crates/astock-data/src/realtime_quote.rs`），本测试锁的就是那份 camelCase 投影。

import { act, render, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: unknown) => (typeof opts === "string" ? opts : key),
    i18n: { language: "zh-CN" },
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

const invokeMock = vi.fn();
const handlers = new Map<string, (event: { payload: unknown }) => void>();

vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: (name: string, handler: (event: { payload: unknown }) => void) => {
    handlers.set(name, handler);
    return Promise.resolve(() => {});
  },
  isTauri: () => true,
}));

vi.mock("@/stores/feature/timeAnchorStore", () => ({
  useTimeAnchorStore: (selector: (s: Record<string, unknown>) => unknown) => selector({ asOfDate: null, mode: "live" }),
}));

vi.mock("@/stores", () => ({
  useStockAnalysisStore: (selector?: (s: Record<string, unknown>) => unknown) => {
    const state = {
      stockCode: "600000",
      stockName: "浦发银行",
      getStockQuote: async () => {},
      getStockKline: async () => {},
      startAnalysis: async () => {},
      watchlistVersion: 0,
    };
    return selector ? selector(state) : state;
  },
}));

import { WatchlistPanel } from "../WatchlistPanel";

/** `StockQuote` 的 camelCase 投影（只列到消费端会用到的键所在的整份行情）。 */
function currentQuote(overrides: Record<string, unknown> = {}) {
  return {
    code: "600000",
    name: "浦发银行",
    price: 8.12,
    preClose: 8,
    open: 8.02,
    high: 8.2,
    low: 8,
    volume: 1000000,
    amount: 8120000,
    changePct: 1.5,
    turnoverRate: 0.34,
    pe: 4.21,
    pb: 0.5,
    totalMv: 240000000000,
    isSt: false,
    timestamp: "2026-10-09 20:36:50",
    ...overrides,
  };
}

/** `QuoteChangeEvent` 经 serde（`rename_all = "camelCase"`）投影出的 payload。 */
function eventPayload(currentOverrides: Record<string, unknown> = {}) {
  return {
    stockCode: "600000",
    previous: null,
    current: currentQuote(currentOverrides),
    changePct: 1.5,
    trigger: "tick",
  };
}

const renderPanel = () =>
  render(
    <MemoryRouter>
      <WatchlistPanel />
    </MemoryRouter>,
  );

const handlerOf = async () => {
  await waitFor(() => expect(handlers.has("stock-quote-update")).toBe(true));
  const handler = handlers.get("stock-quote-update");
  if (!handler) { throw new Error("监听器未注册"); }
  return handler;
};

beforeEach(() => {
  handlers.clear();
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (cmd: string) => {
    if (cmd === "list_watchlist") {
      return [{ id: "w1", stockCode: "600000", stockName: "浦发银行", createdAt: 0, updatedAt: 0 }];
    }
    if (cmd === "watchlist_list_groups") { return []; }
    return null;
  });
});

describe("WatchlistPanel — stock-quote-update 事件", () => {
  it("推送一次即上屏：价格、涨跌幅、PE 三项都取自 current", async () => {
    renderPanel();
    const handler = await handlerOf();

    act(() => handler({ payload: eventPayload() }));

    expect(document.body.textContent).toContain("8.12");
    expect(document.body.textContent).toContain("+1.50%");
    // PE 此前永远接不上：摊平的 payload 里根本没有 `pe` 这个键
    expect(document.body.textContent).toContain("PE 4.2");
  });

  it("非有限值序列化成 null 时退回 0 且不抛（f64 的 NaN/±Inf 到前端即 null）", async () => {
    renderPanel();
    const handler = await handlerOf();

    const payload = eventPayload({ price: null, changePct: null, pe: null });
    expect(() => act(() => handler({ payload }))).not.toThrow();

    expect(document.body.textContent).toContain("0.00");
    expect(document.body.textContent).not.toContain("PE ");
  });
});
