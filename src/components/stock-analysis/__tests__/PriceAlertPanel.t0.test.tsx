// 运行门：T+0 自动重跑面板开关（PLAN §11 裁定 4 ①「接面板并默认关 + 成本声明」）。
//
// 回归对象（2026-10-15）：这条链路**后端全通**（引擎 `maybe_trigger_t0` 读配置、`services.rs`
// 注入了后端重跑 callback、store 里挂着事件 listener），唯独 store 的 `fetchT0Config` /
// `setT0Config` 零组件调用者 ⇒ 用户看得见的只有「功能不存在」。本文件锁三件事：
//   ① 默认关（不替用户开，与引擎 `TZeroConfig::default().enabled=false` 同口径）；
//   ② 开启/改阈值真的发 `set_t0_config`，且载荷是**合并后**的完整配置（后端收全量结构体）；
//   ③ 保存失败要回滚乐观态并把话吐出来（静默吞掉＝面板显示已开启而后端没收到）。

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
import { PriceAlertPanel } from "../PriceAlertPanel";

const DEFAULT_T0 = {
  enabled: false,
  changePctThreshold: 3,
  turnoverRateThreshold: 8,
  minIntervalMinutes: 30,
};

/** 后端各命令的最小应答；`set_t0_config` 默认成功，个别用例改成 reject。 */
function wireBackend(overrides: { setFails?: boolean; t0?: Partial<typeof DEFAULT_T0> } = {}) {
  const t0 = { ...DEFAULT_T0, ...overrides.t0 };
  invokeMock.mockImplementation((cmd: string, args?: unknown) => {
    switch (cmd) {
      case "get_t0_config":
        return Promise.resolve({ ...t0 });
      case "list_price_alerts":
        return Promise.resolve([]);
      case "monitor_status":
        return Promise.resolve({
          assembled: true,
          session: "morning",
          tradingNow: true,
          pollIntervalSecs: 30,
          configsActive: 0,
          roundCodesTotal: 0,
          roundCodesFailed: 0,
          consecutiveAllFailedRounds: 0,
          roundFinishedAtMs: 0,
        });
      case "set_t0_config":
        return overrides.setFails
          ? Promise.reject(new Error("RealtimeMonitor 未初始化"))
          : Promise.resolve(undefined);
      default:
        void args;
        return Promise.resolve(null);
    }
  });
}

/** 取 `set_t0_config` 最后一次调用带的配置（载荷断言只走这一个来源）。 */
function lastT0Payload(): Record<string, unknown> | undefined {
  const calls = invokeMock.mock.calls.filter((c) => c[0] === "set_t0_config");
  return calls.length === 0 ? undefined : (calls[calls.length - 1][1] as { config: Record<string, unknown> }).config;
}

async function mount() {
  render(
    <App>
      <PriceAlertPanel />
    </App>,
  );
  // 等 `get_t0_config` 落地（默认关也要先读到后端态）
  await waitFor(() => {
    expect(invokeMock.mock.calls.some((c) => c[0] === "get_t0_config")).toBe(true);
  });
  await act(async () => {
    await Promise.resolve();
  });
}

describe("T+0 自动重跑开关（面板层）", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    useStockAnalysisStore.setState({
      t0Config: { ...DEFAULT_T0 },
      t0Loading: false,
      _unlisten: null,
    });
  });

  it("默认关：后端说 enabled=false 时开关就是关的（面板不替用户开）", async () => {
    wireBackend();
    await mount();
    const sw = screen.getByTestId("t0-switch");
    expect(sw.getAttribute("aria-checked")).toBe("false");
    // 成本句上屏常显：关着也要看得见「开启要花多少」
    expect(screen.getByTestId("t0-cost").textContent).toContain("stockAnalysis.alert.t0.cost");
    // 关着时不渲染三个阈值输入（少一屏噪声，且避免误改）
    expect(screen.queryByTestId("t0-change-pct")).toBeNull();
  });

  it("开启：发 set_t0_config 且载荷是合并后的完整配置（后端收全量结构体）", async () => {
    wireBackend();
    await mount();
    await act(async () => {
      screen.getByTestId("t0-switch").click();
    });
    await waitFor(() => {
      expect(lastT0Payload()).toBeTruthy();
    });
    expect(lastT0Payload()).toEqual({
      enabled: true,
      changePctThreshold: 3,
      turnoverRateThreshold: 8,
      minIntervalMinutes: 30,
    });
    // 开着才出现三个阈值输入
    await waitFor(() => {
      expect(screen.getByTestId("t0-change-pct")).toBeTruthy();
    });
  });

  it("保存失败：回滚乐观态 + toast 说出原因（不得显示成「已开启」）", async () => {
    wireBackend({ setFails: true });
    await mount();
    await act(async () => {
      screen.getByTestId("t0-switch").click();
    });
    await waitFor(() => {
      expect(useStockAnalysisStore.getState().t0Config.enabled).toBe(false);
    });
    expect(screen.getByTestId("t0-switch").getAttribute("aria-checked")).toBe("false");
    await waitFor(() => {
      const toast = document.querySelector(".ant-message");
      expect(toast?.textContent ?? "").toContain("stockAnalysis.alert.t0.saveFailed");
      expect(toast?.textContent ?? "").toContain("RealtimeMonitor 未初始化");
    });
  });

  it("改阈值只发被改的那一格（合并语义），且带上 enabled 现值", async () => {
    wireBackend({ t0: { enabled: true } });
    await mount();
    await act(async () => {
      // antd v6 把 `data-testid` 透传到**内层 input 本身**（不是外层 wrapper）
      fireEvent.change(screen.getByTestId("t0-interval"), { target: { value: "45" } });
    });
    await waitFor(() => {
      expect(lastT0Payload()?.minIntervalMinutes).toBe(45);
    });
    expect(lastT0Payload()?.enabled).toBe(true);
    expect(lastT0Payload()?.changePctThreshold).toBe(3);
  });
});
