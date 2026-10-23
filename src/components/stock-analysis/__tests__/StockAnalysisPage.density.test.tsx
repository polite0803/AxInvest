import i18n from "@/i18n";
import { useStockAnalysisStore, useUIStore, useWorkspaceStore } from "@/stores";
import type { StockDecision } from "@/types";
import { act, render, screen } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { MemoryRouter } from "react-router-dom";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { StockAnalysisPage } from "../StockAnalysisPage";

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

const DECISION = {
  action: "BUY",
  positionPct: 20,
  targetPrice: 12,
  stopLoss: 9,
  riskLevel: "MID",
  confidence: 70,
  reasoning: "测试决策",
} as unknown as StockDecision;

function renderPage(embeddedInWorkspace: boolean) {
  return render(
    <MemoryRouter initialEntries={["/invest?tab=workspace&stockCode=600519"]}>
      <I18nextProvider i18n={i18n}>
        <StockAnalysisPage embeddedInWorkspace={embeddedInWorkspace} />
      </I18nextProvider>
    </MemoryRouter>,
  );
}

beforeAll(async () => {
  await i18n.changeLanguage("zh-CN");
});

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation((command: string) => {
    if (command === "get_market_status") { return Promise.resolve({ status: "closed" }); }
    if (command === "get_stock_quote") {
      return Promise.resolve({
        code: "600519",
        name: "贵州茅台",
        price: 10,
        change: 0.1,
        changePct: 1,
        open: 10,
        high: 11,
        low: 9,
        volume: 100000,
      });
    }
    if (command === "get_stock_kline") { return Promise.resolve([]); }
    return Promise.resolve(null);
  });
  useStockAnalysisStore.getState().reset();
  useStockAnalysisStore.setState({
    status: "completed",
    decision: DECISION,
    analysisId: null,
    stockCode: "600519",
    stockName: "贵州茅台",
  });
  useUIStore.setState({ deviceLayout: "desktop" });
  useWorkspaceStore.setState({ userMode: "simple" });
});

describe("StockAnalysisPage — 简洁/专业信息密度", () => {
  it("工作区简洁模式隐藏次级信息与调试入口", async () => {
    renderPage(true);
    expect(screen.queryByTestId("sa-advanced-summary")).toBeNull();
    expect(screen.queryByTestId("sa-debug-toggle")).toBeNull();
    await flushAsyncUpdates();
  });

  it("工作区专业模式展示次级信息与调试入口", async () => {
    useWorkspaceStore.setState({ userMode: "professional" });
    renderPage(true);
    expect(screen.getByTestId("sa-advanced-summary")).toBeInTheDocument();
    expect(screen.getByTestId("sa-debug-toggle")).toBeInTheDocument();
    await flushAsyncUpdates();
  });

  it("独立整页没有模式开关，恒按全量渲染", async () => {
    renderPage(false);
    expect(screen.getByTestId("sa-advanced-summary")).toBeInTheDocument();
    expect(screen.getByTestId("sa-debug-toggle")).toBeInTheDocument();
    await flushAsyncUpdates();
  });

  // 移动端布局不渲染模式开关 ⇒ 即使 store 里是 simple 也不得藏内容，否则用户无处切回
  it("工作区移动端没有开关，次级信息不得被藏起来", async () => {
    useUIStore.setState({ deviceLayout: "mobile" });
    renderPage(true);
    expect(screen.getByTestId("sa-advanced-summary")).toBeInTheDocument();
    expect(screen.getByTestId("sa-debug-toggle")).toBeInTheDocument();
    await flushAsyncUpdates();
  });
});

/** 断言之后再清空行情/K 线的异步 setState，避免 act 警告（不得提到断言之前） */
async function flushAsyncUpdates() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}
