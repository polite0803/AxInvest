// 运行门：条件单触发（第二链）的 payload 必须真上屏，且它不得被当成价格告警。
//
// 回归对象（2026-10-15 阶段 F）：条件单这一层此前「命令在、引擎在、没人装配」——
// `start_risk_inspection_service` 零构造点、行情回调只转发事件不评估规则，触发永远不发生；
// 而列表命令把 `format!("{:?}")` 的 Rust Debug 串当描述端给前端。
// 现在触发走 `IpcEventName::ConditionalOrderTriggered`，描述由后端 `describe()` 产出，
// 本测试锁的是这份投影的消费面 + 两条链的分工（条件单不写 price_alerts ⇒ 不该触发告警重取）。
//
// 两侧键名的逐项对账不在这里，在 `scripts/check-monitor-event-authority.mjs`（R7–R10）。

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

/** 后端 `ConditionTrigger`（`rename_all = "camelCase"`）序列化出的整份形状。 */
function triggerPayload(overrides: Record<string, unknown> = {}) {
  return {
    orderId: "co_2f7c1a4b-0d3e-4a51-9b7c-6e2f1a0b9c8d",
    stockCode: "601899",
    stockName: "紫金矿业",
    condition: "跌破止损价 12.50",
    action: "卖出全部持仓",
    price: 12.34,
    changePct: -8.33,
    triggeredAtMs: 1_760_000_000_000,
    reason: "跌破止损价 12.50, 现价 12.34",
    ...overrides,
  };
}

const handlerOf = async (name: string) => {
  await waitFor(() => expect(handlers.has(name)).toBe(true));
  const handler = handlers.get(name);
  if (!handler) { throw new Error(`监听器未注册：${name}`); }
  return handler;
};

beforeEach(() => {
  handlers.clear();
  invokeMock.mockReset();
  invokeMock.mockResolvedValue([]);
});

describe("PriceAlertPanel — conditional-order-triggered 事件", () => {
  it("挂载即注册监听，两条链各占一个通道", async () => {
    render(
      <App>
        <PriceAlertPanel />
      </App>,
    );
    await handlerOf("conditional-order-triggered");
    expect(handlers.has("price-alert-triggered")).toBe(true);
  });

  it("触发提示把「哪只、什么条件、要做什么」三项都送进文案", async () => {
    render(
      <App>
        <PriceAlertPanel />
      </App>,
    );
    const handler = await handlerOf("conditional-order-triggered");

    act(() => handler({ payload: triggerPayload() }));

    await waitFor(() => expect(document.body.textContent).toContain("stockAnalysis.conditional.triggeredDetail"));
    const text = document.body.textContent ?? "";
    expect(text).toContain('"name":"紫金矿业"');
    expect(text).toContain('"condition":"跌破止损价 12.50"');
    expect(text).toContain('"action":"卖出全部持仓"');
  });

  it("stockName 为空串时回退到代码，不显示成空名字", async () => {
    render(
      <App>
        <PriceAlertPanel />
      </App>,
    );
    const handler = await handlerOf("conditional-order-triggered");

    act(() => handler({ payload: triggerPayload({ stockName: "" }) }));

    await waitFor(() => expect(document.body.textContent).toContain('"name":"601899"'));
  });

  it("条件单触发不重取价格告警列表（两个实体，不共用一条刷新路）", async () => {
    render(
      <App>
        <PriceAlertPanel />
      </App>,
    );
    const handler = await handlerOf("conditional-order-triggered");
    const before = invokeMock.mock.calls.length;

    act(() => handler({ payload: triggerPayload() }));

    expect(invokeMock.mock.calls.length).toBe(before);
  });

  // 破坏样本的反证：价格告警那条链收到 payload 时**必须**重取列表。
  // 上一条若把「不刷新」写成恒真（比如 handler 根本没接上），这条会红。
  it("价格告警仍会重取列表，证明上一条的「不刷新」不是恒真", async () => {
    render(
      <App>
        <PriceAlertPanel />
      </App>,
    );
    const alertHandler = await handlerOf("price-alert-triggered");
    const before = invokeMock.mock.calls.length;

    act(() =>
      alertHandler({
        payload: {
          alertId: "0f3a1c52-6c1f-4f3e-9a7b-2d8e4c6a1b03",
          stockCode: "300750",
          stockName: "宁德时代",
          alertType: "stop_loss",
          alertMessage: "跌破止损价",
          currentPrice: 12.34,
          changePct: -3.21,
          timestamp: "2026-10-15T13:36:50Z",
          suggestedAction: null,
        },
      })
    );

    await waitFor(() => expect(invokeMock.mock.calls.length).toBeGreaterThan(before));
  });
});
