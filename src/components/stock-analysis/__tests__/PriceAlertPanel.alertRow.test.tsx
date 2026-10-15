// 运行门：告警行的呈现层三件事（PLAN §9 H-1/H-2/H-3）。
//
// 回归对象（2026-10-15）：
//   H-1 建单/删单/读列表三处 `catch { /* 静默 */ }` ⇒ 失败用户无感；
//   H-2 一格 `targetPrice` 同时装价格、涨跌幅 % 与换手率 %，类型列直接显示兼容列英文 token
//       （`above`），且方向按兼容列判（6→5 有损：resistance 与 take_profit 同写 above）；
//   H-3 `isTriggered` 声明成 `boolean` 而后端是 `i32`（0/1）。
// 三件事都发生在**同一行渲染**上，所以合在一个文件里测；每条断言只对应其中一件。

import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { App } from "antd";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: unknown) => opts && typeof opts === "object" ? `${key}|${JSON.stringify(opts)}` : key,
    i18n: { language: "zh-CN" },
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

const invokeMock = vi.fn();

vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: () => Promise.resolve(() => {}),
  isTauri: () => true,
}));

import { useStockAnalysisStore } from "@/stores/feature/stockAnalysisStore";
import { canonicalAlertTypeOf, conditionTypeOf, PriceAlertPanel, thresholdOf } from "../PriceAlertPanel";

/** 老行（v203 之前）：只有兼容列，新三列为空。 */
const LEGACY_ABOVE_ROW = {
  id: "a1",
  stockCode: "601899",
  stockName: "紫金矿业",
  condition: "above",
  targetPrice: 12.5,
  alertType: null,
  conditionType: null,
  threshold: null,
  isTriggered: 0 as const,
  triggeredAt: null,
  createdAt: 1,
};

/** 新行：涨跌幅类（阈值是百分数，不是价格）。 */
const CHANGE_PCT_ROW = {
  id: "a2",
  stockCode: "600519",
  stockName: "贵州茅台",
  condition: "above",
  targetPrice: 5,
  alertType: "change",
  conditionType: "change_pct",
  threshold: 5,
  isTriggered: 0 as const,
  triggeredAt: null,
  createdAt: 2,
};

/** 已触发的价格类行（进历史 tab）。 */
const TRIGGERED_ROW = {
  id: "a3",
  stockCode: "000001",
  stockName: "平安银行",
  condition: "above",
  targetPrice: 11,
  alertType: "resistance",
  conditionType: "price",
  threshold: 11,
  isTriggered: 1 as const,
  triggeredAt: 1_760_000_000_000,
  createdAt: 3,
};

function wireBackend(rows: unknown[], overrides: { createFails?: boolean } = {}) {
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case "list_price_alerts":
        return Promise.resolve(rows);
      case "monitor_status":
        return Promise.resolve({
          assembled: true,
          session: "morning",
          tradingNow: true,
          pollIntervalSecs: 30,
          configsActive: rows.length,
          roundCodesTotal: rows.length,
          roundCodesFailed: 0,
          consecutiveAllFailedRounds: 0,
          roundFinishedAtMs: 1_760_000_000_000,
        });
      case "get_t0_config":
        return Promise.resolve({
          enabled: false,
          changePctThreshold: 3,
          turnoverRateThreshold: 8,
          minIntervalMinutes: 30,
        });
      case "create_price_alert":
        return overrides.createFails
          ? Promise.reject({ code: "STOCK_ALERT_CREATE_FAILED", detail: "写库失败：disk full" })
          : Promise.resolve({});
      default:
        return Promise.resolve(null);
    }
  });
}

async function mount() {
  render(
    <App>
      <PriceAlertPanel />
    </App>,
  );
  await waitFor(() => {
    expect(invokeMock.mock.calls.some((c) => c[0] === "list_price_alerts")).toBe(true);
  });
  await act(async () => {
    await Promise.resolve();
  });
}

describe("纯函数：三个取值口径（老行回落）", () => {
  it("canonicalAlertTypeOf：优先 alertType，老行按兼容列回落", () => {
    expect(canonicalAlertTypeOf(LEGACY_ABOVE_ROW)).toBe("take_profit");
    expect(canonicalAlertTypeOf(CHANGE_PCT_ROW)).toBe("change");
    expect(canonicalAlertTypeOf({ alertType: null, condition: "volume_spike" })).toBe("volume");
    expect(canonicalAlertTypeOf({ alertType: null, condition: "未知名" })).toBeUndefined();
  });

  it("conditionTypeOf / thresholdOf：老行回落成 price + targetPrice", () => {
    expect(conditionTypeOf(LEGACY_ABOVE_ROW)).toBe("price");
    expect(thresholdOf(LEGACY_ABOVE_ROW)).toBe(12.5);
    expect(conditionTypeOf(CHANGE_PCT_ROW)).toBe("change_pct");
    expect(thresholdOf(CHANGE_PCT_ROW)).toBe(5);
  });
});

describe("告警行渲染", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    useStockAnalysisStore.setState({
      t0Config: { enabled: false, changePctThreshold: 3, turnoverRateThreshold: 8, minIntervalMinutes: 30 },
      t0Loading: false,
      _unlisten: null,
    });
  });

  it("H-2：阈值按 conditionType 分列单位 —— 涨幅 5 渲染成 5.00% 而不是 5.00", async () => {
    wireBackend([CHANGE_PCT_ROW, LEGACY_ABOVE_ROW]);
    await mount();
    expect(screen.getByText("5.00%")).toBeTruthy();
    expect(screen.getByText("12.50")).toBeTruthy();
    expect(screen.queryByText("5.00")).toBeNull();
  });

  it("H-2：类型列读权威 alertType（老行回落），不再显示兼容列英文 token", async () => {
    wireBackend([LEGACY_ABOVE_ROW, CHANGE_PCT_ROW]);
    await mount();
    // 老行 above + 空 alertType ⇒ 回落 take_profit ⇒ i18n 键（不是 "above"）
    expect(screen.getByText("stockAnalysis.alert.typeTakeProfit")).toBeTruthy();
    expect(screen.getByText("stockAnalysis.alert.typeChange")).toBeTruthy();
    expect(screen.queryByText("above")).toBeNull();
  });

  it("H-3：isTriggered 是 0/1 数字，活跃/历史两个 tab 按数字筛选", async () => {
    wireBackend([CHANGE_PCT_ROW, TRIGGERED_ROW]);
    await mount();
    // 活跃 tab 计数 1（0 那条）、历史 tab 计数 1（1 那条）
    expect(screen.getByText('stockAnalysis.alert.activeTab|{"count":1}')).toBeTruthy();
    expect(screen.getByText('stockAnalysis.alert.triggeredTab|{"count":1}')).toBeTruthy();
  });

  it("H-1：建单失败必须报出来（此前完全静默）", async () => {
    wireBackend([], { createFails: true });
    await mount();
    // 打开建单表单 → 填最小必填 → 提交
    await act(async () => {
      screen.getAllByRole("button").find((b) => b.querySelector(".anticon-plus"))?.click();
    });
    const byPlaceholder = (key: string) => document.querySelector<HTMLInputElement>(`input[placeholder="${key}"]`);
    const codeInput = byPlaceholder("stockAnalysis.alert.code");
    const nameInput = byPlaceholder("stockAnalysis.alert.name");
    const priceInput = byPlaceholder("stockAnalysis.alert.price");
    expect(codeInput && nameInput && priceInput).toBeTruthy();
    await act(async () => {
      // 三个输入都要真填：`targetPrice` 是 InputNumber（受控），只改 DOM 的 value 不会进表单
      if (codeInput) { fireEvent.change(codeInput, { target: { value: "600000" } }); }
      if (nameInput) { fireEvent.change(nameInput, { target: { value: "浦发银行" } }); }
      if (priceInput) { fireEvent.change(priceInput, { target: { value: "11.5" } }); }
    });
    await act(async () => {
      document.querySelector<HTMLButtonElement>("button[type='submit']")?.click();
    });
    await waitFor(() => {
      const toast = document.querySelector(".ant-message");
      expect(toast?.textContent ?? "").toContain("写库失败：disk full");
    });
  });
});
