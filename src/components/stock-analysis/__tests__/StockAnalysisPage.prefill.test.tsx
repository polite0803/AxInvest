import i18n from "@/i18n";
import { useStockAnalysisStore } from "@/stores";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { MemoryRouter, Route, Routes, useSearchParams } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { StockAnalysisPage } from "../StockAnalysisPage";

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

/**
 * 行情桩：name 与 code 不同，这样才能分辨"名称 (代码)"格式的预填是否真的发生。
 */
function quoteFor(code: string, name: string) {
  return { code, name, price: 10, change: 0, changePct: 0 };
}

function installInvokeStub() {
  invokeMock.mockImplementation((command: string, args?: Record<string, unknown>) => {
    switch (command) {
      case "get_market_status":
        return Promise.resolve({ status: "closed" });
      case "get_stock_quote": {
        const code = String(args?.stockCode ?? "");
        return Promise.resolve(quoteFor(code, code === "600519" ? "贵州茅台" : code));
      }
      case "get_stock_kline":
        return Promise.resolve([]);
      case "get_vendor_health_all":
        return Promise.resolve([]);
      default:
        return Promise.resolve(null);
    }
  });
}

function quoteCalls(): string[] {
  return invokeMock.mock.calls
    .filter((c) => c[0] === "get_stock_quote")
    .map((c) => String((c[1] as Record<string, unknown> | undefined)?.stockCode));
}

/**
 * 模拟工作区壳层切视图时补写 ?view= 的行为：会改 searchParams 引用，
 * 但股票代码不变 —— 不应触发重复拉取。
 */
function ViewBumper() {
  const [, setSearchParams] = useSearchParams();
  return (
    <button
      type="button"
      data-testid="bump-view"
      onClick={() =>
        setSearchParams(
          new URLSearchParams({ tab: "workspace", stockCode: "600519", view: "monitor" }),
          { replace: true },
        )}
    >
      bump
    </button>
  );
}

/** 渲染分析页；入口参数与 StockWorkspaceShell / InvestHub 的路由约定一致 */
function renderAt(url: string, withBumper = false) {
  return render(
    <MemoryRouter initialEntries={[url]}>
      <I18nextProvider i18n={i18n}>
        <Routes>
          <Route
            path="/invest"
            element={
              <>
                <StockAnalysisPage embeddedInWorkspace />
                {withBumper ? <ViewBumper /> : null}
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

describe("StockAnalysisPage — URL 股票预填", () => {
  it("?stockCode= 入口（InvestHub 选股中心点股跳转）预填「名称 (代码)」", async () => {
    renderAt("/invest?tab=workspace&stockCode=600519&view=analysis");
    await waitFor(() => {
      expect(useStockAnalysisStore.getState().searchKeyword).toBe("贵州茅台 (600519)");
    });
    expect(quoteCalls()).toContain("600519");
  });

  it("?code= 入口（/stock-analysis?code= 经 RedirectToInvest 保留 query）同样预填", async () => {
    renderAt("/invest?tab=workspace&code=600519&stockCode=600519&view=analysis");
    await waitFor(() => {
      expect(useStockAnalysisStore.getState().searchKeyword).toBe("贵州茅台 (600519)");
    });
  });

  it("无股票参数时不预填、不拉行情（反向断言）", async () => {
    renderAt("/invest?tab=workspace&view=analysis");
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_market_status");
    });
    expect(useStockAnalysisStore.getState().searchKeyword).toBe("");
    expect(quoteCalls()).toHaveLength(0);
  });

  it("同一代码下壳层改写 ?view= 不重复拉行情", async () => {
    renderAt("/invest?tab=workspace&stockCode=600519&view=analysis", true);
    await waitFor(() => {
      expect(useStockAnalysisStore.getState().searchKeyword).toBe("贵州茅台 (600519)");
    });
    const before = quoteCalls().length;
    fireEvent.click(screen.getByTestId("bump-view"));
    // 若守卫失效，这里会多出一次 get_stock_quote(600519)
    await waitFor(() => {
      expect(quoteCalls().length).toBe(before);
    });
  });
});
