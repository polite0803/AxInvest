// SPDX-License-Identifier: AGPL-3.0-only

import { InvestHub } from "@/components/invest/InvestHub";
import i18n from "@/i18n";
import { useStockAnalysisStore, useWorkspaceStore } from "@/stores";
import type { RecoPick, RecoResponse, StyleKey } from "@/types/stock-analysis";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isUnsafe: () => false,
  isTauri: () => false,
}));

const STYLES: StyleKey[] = ["trend", "value", "capital", "reversion", "watchlist", "serenity"];

function makePick(style: StyleKey, code: string, name: string): RecoPick {
  return {
    stockCode: code,
    stockName: name,
    style,
    period: "ultra_short",
    price: 100,
    entryLow: 98,
    entryHigh: 102,
    stopLoss: 95,
    targetPrice: 110,
    positionPct: 10,
    holdingDays: 5,
    confidence: 80,
    reasons: [],
    riskNotes: [],
  };
}

/** 每个风格一条真实候选，trend 用 600519 作为可点目标 */
function makeReco(): RecoResponse {
  const picks: Partial<Record<StyleKey, RecoPick[]>> = {};
  for (const s of STYLES) {
    picks[s] = [makePick(s, s === "trend" ? "600519" : "000001", s === "trend" ? "贵州茅台" : "平安银行")];
  }
  return {
    period: "ultra_short",
    picks,
    disabledStyles: [],
    degradedStyles: [],
    generatedAt: Date.now(),
    rawSeedPoolSize: 10,
    mode: "live",
  } as unknown as RecoResponse;
}

function installInvokeStub() {
  invokeMock.mockImplementation((command: string, args?: Record<string, unknown>) => {
    switch (command) {
      case "get_cached_recommendation":
      case "recommend_stocks":
        return Promise.resolve(makeReco());
      case "backtest_reco_strategies":
        return Promise.resolve(null);
      case "get_latest_analyses_for_stocks":
      case "get_reco_strategy_weights":
        return Promise.resolve({});
      case "get_market_status":
        return Promise.resolve({ status: "closed" });
      case "get_stock_quote": {
        const code = String(args?.stockCode ?? "");
        return Promise.resolve({
          code,
          name: code === "600519" ? "贵州茅台" : "平安银行",
          price: 100,
          change: 0,
          changePct: 0,
        });
      }
      case "get_stock_kline":
      case "get_vendor_health_all":
        return Promise.resolve([]);
      default:
        return Promise.resolve(null);
    }
  });
}

/** 暴露当前 URL 供断言 */
function LocationProbe() {
  const loc = useLocation();
  return <div data-testid="location-search">{loc.pathname + loc.search}</div>;
}

function renderHub(url: string) {
  return render(
    <MemoryRouter initialEntries={[url]}>
      <I18nextProvider i18n={i18n}>
        <Routes>
          <Route
            path="/invest"
            element={
              <>
                <InvestHub />
                <LocationProbe />
              </>
            }
          />
        </Routes>
      </I18nextProvider>
    </MemoryRouter>,
  );
}

beforeEach(() => {
  invokeMock.mockReset();
  installInvokeStub();
  useStockAnalysisStore.getState().reset();
});

describe("InvestHub — 智能荐股候选点击 → 工作区预填", () => {
  it("超短线 tab 点股票后 URL 带 stockCode，且分析页搜索栏预填「名称 (代码)」", async () => {
    renderHub("/invest?tab=screener");

    // 选股 tab 与其中的荐股面板都是 lazy 组件，全量跑测时首次 import 明显变慢
    // 切到超短线周期 tab
    const ultraTab = await screen.findByText("超短线", { selector: "div" }, { timeout: 20000 });
    fireEvent.click(ultraTab);

    // 候选行出现（贵州茅台）
    const row = await screen.findByText("贵州茅台", {}, { timeout: 20000 });
    fireEvent.click(row);

    // ① 写入端：URL 必须带上 workspace + stockCode + view
    await waitFor(() => {
      const search = screen.getByTestId("location-search").textContent ?? "";
      expect(search).toContain("tab=workspace");
      expect(search).toContain("stockCode=600519");
    });

    // ② 消费端：分析页预填「名称 (代码)」，且工作区拿到名称（不是把代码当名称）
    //    workspace 是 lazy 组件，jsdom 下首次 import 较慢，需放宽等待
    await waitFor(() => {
      expect(useStockAnalysisStore.getState().searchKeyword).toBe("贵州茅台 (600519)");
    }, { timeout: 20000 });
    expect(useWorkspaceStore.getState().currentStockCode).toBe("600519");
    expect(useWorkspaceStore.getState().currentStockName).toBe("贵州茅台");
    expect(useWorkspaceStore.getState().currentView).toBe("analysis");
    // 工作区头部不得出现「代码 (代码)」这种名称解析失败的退化渲染
    await waitFor(() => {
      expect(document.body.textContent).toContain("贵州茅台 (600519)");
    });
    expect(document.body.textContent).not.toContain("600519 (600519)");
  }, 30000);

  it("工作区左栏换股改写 URL（不再被 URL 旧值弹回），且保留当前视图", async () => {
    useWorkspaceStore.setState({
      leftSidebarCollapsed: false,
      currentStockCode: "600519",
      currentStockName: "贵州茅台",
      currentView: "monitor",
      recentStocks: [{ code: "000001", name: "平安银行", visitedAt: Date.now() }],
    });
    renderHub("/invest?tab=workspace&stockCode=600519&stockName=%E8%B4%B5%E5%B7%9E%E8%8C%85%E5%8F%B0&view=monitor");

    // 等 workspace 这个 lazy 组件真正挂载（全量跑测时首次 import 会明显变慢）
    await waitFor(
      () => expect(document.querySelector(".ax-stock-workspace")).toBeTruthy(),
      { timeout: 20000 },
    );

    // 等左栏挂载出「平安银行」条目
    const target = await screen.findByText("平安银行", {}, { timeout: 20000 });
    fireEvent.click(target);

    await waitFor(() => {
      const search = screen.getByTestId("location-search").textContent ?? "";
      expect(search).toContain("stockCode=000001");
      expect(search).toContain("view=monitor");
    });

    // 静默弹回的反向断言：URL 与 store 必须一致指向新股票
    await new Promise((r) => setTimeout(r, 200));
    const search = screen.getByTestId("location-search").textContent ?? "";
    expect(search).not.toContain("stockCode=600519");
    expect(useWorkspaceStore.getState().currentStockCode).toBe("000001");
    expect(useWorkspaceStore.getState().currentStockName).toBe("平安银行");
  }, 30000);
});
