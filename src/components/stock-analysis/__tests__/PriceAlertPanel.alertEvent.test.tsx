// 运行门：价格告警事件的 payload 必须真的上屏，且六类之外的值落「未知类型」。
//
// 回归对象（2026-10-14 缺陷评估）：发射点发的是字面量 `stock-monitor-alert`，
// 前端监听的是 `price-alert-triggered`，两头都有名字、中间没有任何东西做主 ⇒
// handler 从未被调用，而它本来也只是 `message.info(t("...triggered"))` 一句无参提示，
// payload 里携带的股票、阈值类型、现价一个都没读。
// 现在事件名收口到 `IpcEventName::PriceAlertTriggered` 单一权威，payload 由
// `MonitorAlert` 直接序列化（不再手写 `json!` 摊平），本测试锁的就是这份投影的消费面。
//
// 两侧键名的逐项对账不在这里，在 `scripts/check-monitor-event-authority.mjs`
// （TS 接口拿不到运行期形状，只有源文扫描能做）。

import { act, render, waitFor } from "@testing-library/react";
import { App } from "antd";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    // 回 key 原文，带插值参数时拼成 `key|{json}` ⇒ 缺席按 key 断言、插值面也能断到
    t: (key: string, opts?: unknown) => opts && typeof opts === "object" ? `${key}|${JSON.stringify(opts)}` : key,
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

import { PriceAlertPanel } from "../PriceAlertPanel";

/** 后端 `MonitorAlert`（`rename_all = "camelCase"`）序列化出的 payload 整份形状。 */
function alertPayload(overrides: Record<string, unknown> = {}) {
  return {
    alertId: "0f3a1c52-6c1f-4f3e-9a7b-2d8e4c6a1b03",
    stockCode: "300750",
    stockName: "宁德时代",
    alertType: "stop_loss",
    alertMessage: "跌破止损价 宁德时代: 现价 12.34 <= 止损 13.00",
    currentPrice: 12.34,
    changePct: -3.21,
    timestamp: "2026-10-14T13:36:50.123456Z",
    suggestedAction: "建议: 考虑减仓50%，现价低于止损",
    ...overrides,
  };
}

const handlerOf = async () => {
  await waitFor(() => expect(handlers.has("price-alert-triggered")).toBe(true));
  const handler = handlers.get("price-alert-triggered");
  if (!handler) { throw new Error("监听器未注册"); }
  return handler;
};

beforeEach(() => {
  handlers.clear();
  invokeMock.mockReset();
  invokeMock.mockResolvedValue([]);
});

describe("PriceAlertPanel — price-alert-triggered 事件", () => {
  it("收到告警即上屏：股票名、阈值类型键、现价两位小数三项都取自 payload", async () => {
    render(
      <App>
        <PriceAlertPanel />
      </App>,
    );
    const handler = await handlerOf();

    act(() => handler({ payload: alertPayload() }));

    await waitFor(() => expect(document.body.textContent).toContain("stockAnalysis.alert.triggeredDetail"));
    const text = document.body.textContent ?? "";
    expect(text).toContain('"name":"宁德时代"');
    expect(text).toContain('"type":"stockAnalysis.alert.typeStopLoss"');
    expect(text).toContain('"price":"12.34"');
  });

  it("六类之外的 alertType 落「未知类型」键，不得硬套成六类中的某一类", async () => {
    render(
      <App>
        <PriceAlertPanel />
      </App>,
    );
    const handler = await handlerOf();

    act(() => handler({ payload: alertPayload({ alertType: "margin_call" }) }));

    await waitFor(() => expect(document.body.textContent).toContain("stockAnalysis.alert.typeUnknown"));
    expect(document.body.textContent).not.toContain("stockAnalysis.alert.typeStopLoss");
  });

  it("stockName 为空串时回退到代码，不显示成空名字", async () => {
    render(
      <App>
        <PriceAlertPanel />
      </App>,
    );
    const handler = await handlerOf();

    act(() => handler({ payload: alertPayload({ stockName: "" }) }));

    await waitFor(() => expect(document.body.textContent).toContain('"name":"300750"'));
  });
});
