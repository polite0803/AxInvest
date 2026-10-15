// 运行门：条件单面板（裁定 13 要界面）的四个必须分开的状态。
//
// 锁的形态：
// 1. 列表真从 `list_conditional_orders` 读，显示的是后端 describe() 的人话，不是前端拼的类别+阈值；
// 2. **读失败** 与 **表里确实没有** 是两种状态 —— 把故障渲染成「暂无」就是把缺陷伪装成空集；
// 3. 表里存在但读不回成规则的行单独一句（不参与评估），不与行数并成可比的一个数；
// 4. 两个计数域各用自己的文案：`evaluating`（启用且可读回）≠ `items.length`（含已停用）；
// 5. 建单参数走 camelCase 且整体包在 `req` 里（Tauri v2 的 `#[tauri::command]` 按 camelCase 校键名，
//    传 snake_case 会在 IPC 层就被拒）。
// 键名与后端 DTO 的逐项对账不在这里（TS 拿不到运行期形状），在 check-contracts 项 E。

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { App } from "antd";
import type { TFunction } from "i18next";
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

import { ConditionalOrderPanel, conditionTypeLabel, thresholdPlaceholderKey } from "../ConditionalOrderPanel";

function item(overrides: Record<string, unknown> = {}) {
  return {
    orderId: "co_1",
    stockCode: "601899",
    stockName: "紫金矿业",
    conditionType: "stop_loss",
    conditionValue: 12.5,
    conditionText: "跌破止损价 12.50",
    actionType: "sell",
    actionValue: null,
    actionText: "卖出全部持仓",
    enabled: true,
    coolDownMinutes: 30,
    maxTriggersPerDay: 3,
    triggersToday: 2,
    triggerDate: "2026-10-15",
    lastTriggeredAt: 1_760_000_000_000,
    activeStart: null,
    activeEnd: null,
    ...overrides,
  };
}

function listView(body: Record<string, unknown> = {}) {
  return { items: [item()], unreadable: [], evaluating: 1, ...body };
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(listView());
});

describe("ConditionalOrderPanel — 列表与三种缺席", () => {
  it("列表来自命令，显示后端给的条件句与动作句", async () => {
    render(
      <App>
        <ConditionalOrderPanel />
      </App>,
    );
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("list_conditional_orders"));
    expect(await screen.findByText("跌破止损价 12.50")).toBeTruthy();
    expect(screen.getByText("卖出全部持仓")).toBeTruthy();
    expect(screen.getByText("601899")).toBeTruthy();
  });

  it("读失败显示失败原因，不得同时显示「暂无条件单」", async () => {
    invokeMock.mockRejectedValue(new Error("后端未启动"));
    render(
      <App>
        <ConditionalOrderPanel />
      </App>,
    );
    const alert = await screen.findByText(/stockAnalysis.conditional.loadFailed/);
    expect(alert.textContent).toContain("后端未启动");
    expect(screen.queryByText("stockAnalysis.conditional.empty")).toBeNull();
  });

  it("真的没有行时才说「暂无条件单」", async () => {
    invokeMock.mockResolvedValue(listView({ items: [], evaluating: 0 }));
    render(
      <App>
        <ConditionalOrderPanel />
      </App>,
    );
    expect(await screen.findByText("stockAnalysis.conditional.empty")).toBeTruthy();
    expect(screen.queryByText(/loadFailed/)).toBeNull();
  });

  it("读不回的行单独成一句，带原因与计数", async () => {
    invokeMock.mockResolvedValue(listView({
      unreadable: [{ orderId: "co_bad", reason: '条件类别无法读回（type="ma_cross"）' }],
    }));
    render(
      <App>
        <ConditionalOrderPanel />
      </App>,
    );
    const warn = await screen.findByText(/stockAnalysis.conditional.unreadable/);
    expect(warn.textContent).toContain('"count":1');
    expect(screen.getByText(/co_bad/)).toBeTruthy();
    expect(screen.getByText("跌破止损价 12.50")).toBeTruthy();
  });

  it("两个计数域各用自己的键，不收敛成同一个数", async () => {
    invokeMock.mockResolvedValue(listView({
      items: [item(), item({ orderId: "co_2", enabled: false })],
      evaluating: 1,
    }));
    render(
      <App>
        <ConditionalOrderPanel />
      </App>,
    );
    const badge = await screen.findByText(/stockAnalysis.conditional.evaluating/);
    const rows = await screen.findByText(/stockAnalysis.conditional.totalRows/);
    expect(badge.textContent).toContain('"count":1');
    expect(rows.textContent).toContain('"count":2');
  });
});

describe("ConditionalOrderPanel — 建单与启停的参数面", () => {
  it("填表提交后按 camelCase 包在 req 里调用创建命令", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      Promise.resolve(
        cmd === "list_conditional_orders" ? listView({ items: [], unreadable: [], evaluating: 0 }) : "co_new",
      )
    );
    render(
      <App>
        <ConditionalOrderPanel />
      </App>,
    );
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("list_conditional_orders"));

    // 建单按钮只有图标没有可读名，按图标类定位
    const plus = (await screen.findAllByRole("button")).find((b) => b.querySelector(".anticon-plus"));
    expect(plus).toBeTruthy();
    fireEvent.click(plus as HTMLElement);

    fireEvent.change(screen.getByPlaceholderText("stockAnalysis.alert.code"), { target: { value: "601899" } });
    fireEvent.change(screen.getByPlaceholderText("stockAnalysis.alert.name"), { target: { value: "紫金矿业" } });
    fireEvent.change(screen.getByPlaceholderText("stockAnalysis.alert.price"), { target: { value: "12.5" } });
    fireEvent.click(screen.getByRole("button", { name: "stockAnalysis.conditional.create" }));

    await waitFor(() => {
      const call = invokeMock.mock.calls.find((c) => c[0] === "create_conditional_order");
      if (!call) { throw new Error("没调用 create_conditional_order"); }
      const req = (call[1] as { req: Record<string, unknown> }).req;
      expect(req.stockCode).toBe("601899");
      expect(req.stockName).toBe("紫金矿业");
      expect(req.conditionType).toBe("stop_loss");
      expect(req.conditionValue).toBe(12.5);
      expect(req.actionType).toBe("notify");
    });
  });

  it("停用走本表的 enabled 列，参数名是 orderId", async () => {
    render(
      <App>
        <ConditionalOrderPanel />
      </App>,
    );
    const toggle = await screen.findByRole("switch");
    fireEvent.click(toggle);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "set_conditional_order_enabled",
        { orderId: "co_1", enabled: false },
      )
    );
  });

  it("删除先确认，不在点击当刻落刀", async () => {
    render(
      <App>
        <ConditionalOrderPanel />
      </App>,
    );
    // 先等行渲染出来，否则按钮还不存在（表格是异步读回来的）
    await screen.findByText("跌破止损价 12.50");
    const del = (await screen.findAllByRole("button")).find((b) => b.querySelector(".anticon-delete"));
    expect(del).toBeTruthy();
    fireEvent.click(del as HTMLElement);
    await waitFor(() => expect(document.body.textContent).toContain("stockAnalysis.conditional.confirmDelete"));
    expect(invokeMock).not.toHaveBeenCalledWith("delete_conditional_order", expect.anything());
  });
});

describe("ConditionalOrderPanel — 两处判据的纯函数面", () => {
  it("价格类要「价格」单位、比率类要「%」单位，不共用一格", () => {
    expect(thresholdPlaceholderKey("stop_loss")).toBe("stockAnalysis.alert.price");
    expect(thresholdPlaceholderKey("resistance_break")).toBe("stockAnalysis.alert.price");
    expect(thresholdPlaceholderKey("change_pct")).toBe("stockAnalysis.conditional.thresholdPct");
    expect(thresholdPlaceholderKey("turnover_rate")).toBe("stockAnalysis.conditional.thresholdPct");
  });

  it("认识之外的类别落「未知」，不硬套成六类之一", () => {
    const t = ((key: string) => key) as unknown as TFunction;
    expect(conditionTypeLabel(t, "turnover_rate")).toBe("stockAnalysis.alert.typeVolume");
    expect(conditionTypeLabel(t, "ma_cross")).toBe("stockAnalysis.alert.typeUnknown");
  });
});
