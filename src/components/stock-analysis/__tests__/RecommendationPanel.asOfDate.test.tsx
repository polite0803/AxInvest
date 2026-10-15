import i18n from "@/i18n";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RecommendationPanel } from "../RecommendationPanel";

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

function renderWithProviders() {
  return render(
    <MemoryRouter>
      <I18nextProvider i18n={i18n}>
        <RecommendationPanel />
      </I18nextProvider>
    </MemoryRouter>,
  );
}

/** 在 Card extra 中寻找"刷新"按钮（避免匹配到空状态文案中的"刷新"） */
function findRefreshButton() {
  const card = document.querySelector(".ant-card-extra");
  if (!card) { throw new Error("card extra not found"); }
  const btn = card.querySelector("button");
  if (!btn) { throw new Error("refresh button not found"); }
  return btn;
}

beforeEach(() => {
  invokeMock.mockReset();
  // 默认所有 invoke 都返回"无数据"响应,避免测试间相互污染
  invokeMock.mockResolvedValue({
    period: "short",
    picks: {},
    disabledStyles: [],
    generatedAt: Date.now(),
    rawSeedPoolSize: 0,
  });
  useTimeAnchorStore.setState({
    asOfDate: null,
    mode: "live",
    tourSeen: true,
    pendingLiveConfirm: false,
  });
});

/**
 * 收尾：重置时间锚点 store，回到 live。
 *
 * 为什么包在 act 里：本钩子在 RTL 的 auto-cleanup **之前**执行（vitest 逆序跑
 * afterEach），此时上一条用例的组件树仍挂载；replay 用例的组件（ReplayBadge /
 * ReplayWatermark 等）订阅了该 store，重置会触发它们重渲染 —— 不包 act 就是
 * "not wrapped in act" 警告（2026-10-14 实测：本文件 68 条警告全部源自此处）。
 * 为什么用 async 形态：重置会把 `loadCache` 推到 live 分支重新拉一次缓存，
 * `await invoke(...)` 之后的 setState 是微任务续链，sync act 返回后才落地仍会告警；
 * async act 会按宏任务轮询到队列稳定，把这条续链也收进 act 窗口。
 * 重置语义与原实现完全一致，只是把 setState 挪进 act。
 */
afterEach(async () => {
  await act(async () => {
    useTimeAnchorStore.setState({ asOfDate: null, mode: "live" });
  });
});

/**
 * 找到 invoke 第一次被以 `command` 名称调用的参数
 */
function findCall(command: string): unknown[] | undefined {
  return invokeMock.mock.calls.find((c) => c[0] === command);
}

/**
 * 收尾：等挂载副作用（数据请求）落地。
 *
 * 为什么要有：面板挂载后会异步拉数据，续链若落在测试收尾窗口，setState 就在
 * act 之外 ⇒ React 报 "not wrapped in act"。为什么必须放在**断言之后**：这些用例
 * 断言的是**挂载初始态**（发过哪些命令、没发哪些命令），提前 flush 会把组件推进
 * "加载完成"态，断言随之失效。为什么不能只靠 afterEach 兜底：React 是在 setState
 * 那一刻检查的，而那一刻在测试函数返回后、afterEach 之前 —— 收尾钩子永远太晚。
 * 注：本文件改前的 68 条警告实际产自 afterEach 的 store 重置（见上），此处 settle
 * 兜住的是用例体内可能残留的续链（防回归）。
 */
const settle = async () => {
  await act(async () => {});
};

describe("RecommendationPanel — as-of propagation", () => {
  it("live 模式: 挂载时调用 get_cached_recommendation(period),不带 asOfDate", async () => {
    renderWithProviders();
    // 等待特定的命令被调用,避免只等任意 call(可能只等到 backtest)
    await waitFor(() => expect(findCall("get_cached_recommendation")).toBeDefined());
    const call = findCall("get_cached_recommendation");
    expect(call).toBeDefined();
    // args[1] 是 { period } 对象,确认不含 asOfDate
    expect(call?.[1]).toEqual({ period: "short" });
  });

  it("replay 模式: 挂载时调用 recommend_stocks_all_periods(asOfDate)，带 asOfDate", async () => {
    useTimeAnchorStore.setState({ asOfDate: "2026-06-01", mode: "replay" });
    renderWithProviders();
    // R-0：replay 也走「一次拿四档」批量命令（候选池后端只建一次）
    await waitFor(() => expect(findCall("recommend_stocks_all_periods")).toBeDefined());
    const call = findCall("recommend_stocks_all_periods");
    expect(call).toBeDefined();
    expect(call?.[1]).toMatchObject({ asOfDate: "2026-06-01" });
    expect(findCall("recommend_stocks")).toBeUndefined();
    await settle();
  });

  it("live 模式: 不调用扫描命令 (用户没点刷新)", async () => {
    renderWithProviders();
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    expect(findCall("recommend_stocks")).toBeUndefined();
    expect(findCall("recommend_stocks_all_periods")).toBeUndefined();
  });

  it("replay 模式: 不调用 get_cached_recommendation (缓存永远是 live 产物)", async () => {
    useTimeAnchorStore.setState({ asOfDate: "2026-06-01", mode: "replay" });
    renderWithProviders();
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    expect(findCall("get_cached_recommendation")).toBeUndefined();
    await settle();
  });

  it("shows a replay banner when in replay mode", async () => {
    useTimeAnchorStore.setState({ asOfDate: "2026-06-01", mode: "replay" });
    renderWithProviders();
    await waitFor(() => {
      expect(
        screen.getByText(
          i18n.t("stockAnalysis.recommendation.bannerAsOf", { date: "2026-06-01" }),
        ),
      ).toBeInTheDocument();
    });
    await settle();
  });

  it("does NOT show a replay banner in live mode", async () => {
    renderWithProviders();
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    expect(
      screen.queryByText(
        i18n.t("stockAnalysis.recommendation.bannerAsOf", { date: "" }),
      ),
    ).toBeNull();
  });

  it("缓存命中时显示「缓存」灰底标签", async () => {
    // 给 get_cached_recommendation 返回有数据的响应
    const cachedTs = Date.now();
    invokeMock.mockReset();
    invokeMock.mockResolvedValue({
      period: "short",
      picks: {
        trend: [{
          stockCode: "000001",
          stockName: "平安银行",
          style: "trend",
          period: "short",
          price: 10,
          entryLow: 9,
          entryHigh: 11,
          stopLoss: 8.5,
          targetPrice: 12,
          positionPct: 10,
          holdingDays: 5,
          confidence: 80,
          reasons: ["测试"],
          riskNotes: [],
          secondaryStyles: [],
          synthetic: false,
        }],
      },
      disabledStyles: [],
      generatedAt: cachedTs,
      rawSeedPoolSize: 10,
      mode: "cached",
    });
    renderWithProviders();
    await waitFor(() => {
      expect(screen.getByTestId("reco-cached-badge")).toBeInTheDocument();
    });
  });

  it("缓存为空时显示 emptyNoCache 文案", async () => {
    // get_cached_recommendation 返回 null → 无缓存
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(null);
    renderWithProviders();
    // 缓存为空 → loadCache 设 emptyKind=noData → 显示 emptyNoCache
    await waitFor(() => {
      expect(
        screen.getByText(i18n.t("stockAnalysis.recommendation.emptyNoCache")),
      ).toBeInTheDocument();
    });
  });

  it("live 模式下点击刷新按钮调用 recommend_stocks_all_periods", async () => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(null);
    renderWithProviders();
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    // 等待 loading 结束（antd 中 "刷新" 按钮含空格,用 card-extra 范围避免匹配空文案）
    await waitFor(() => {
      const btn = findRefreshButton();
      expect(btn).not.toHaveClass("ant-btn-loading");
    });
    invokeMock.mockClear();
    invokeMock.mockResolvedValue({ byHorizon: {}, failedHorizons: {} });
    fireEvent.click(findRefreshButton());
    await waitFor(() => expect(findCall("recommend_stocks_all_periods")).toBeDefined());
    // 批量命令不带 period：四档一次取（R-0）
    expect(findCall("recommend_stocks_all_periods")?.[1]).toEqual({ asOfDate: null });
    expect(findCall("recommend_stocks")).toBeUndefined();
  });

  it("切换 period 重新调用 get_cached_recommendation(newPeriod)", async () => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(null);
    renderWithProviders();
    await waitFor(() => expect(findCall("get_cached_recommendation")).toBeDefined());
    await waitFor(() => {
      const btn = findRefreshButton();
      expect(btn).not.toHaveClass("ant-btn-loading");
    });
    invokeMock.mockClear();
    invokeMock.mockResolvedValue(null);
    fireEvent.click(screen.getByText(i18n.t("stockAnalysis.recommendation.periodMid")));
    await waitFor(() => expect(findCall("get_cached_recommendation")).toBeDefined());
    expect(findCall("get_cached_recommendation")?.[1]).toEqual({ period: "mid" });
  });

  it("replay 模式下点击刷新按钮调用 recommend_stocks_all_periods 带 asOfDate", async () => {
    useTimeAnchorStore.setState({ asOfDate: "2026-06-01", mode: "replay" });
    invokeMock.mockReset();
    invokeMock.mockResolvedValue({ byHorizon: {}, failedHorizons: {} });
    renderWithProviders();
    await waitFor(() => expect(findCall("recommend_stocks_all_periods")).toBeDefined());
    await waitFor(() => {
      const btn = findRefreshButton();
      expect(btn).not.toHaveClass("ant-btn-loading");
    });
    invokeMock.mockClear();
    invokeMock.mockResolvedValue({ byHorizon: {}, failedHorizons: {} });
    fireEvent.click(findRefreshButton());
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    expect(findCall("recommend_stocks_all_periods")?.[1]).toMatchObject({ asOfDate: "2026-06-01" });
    expect(findCall("recommend_stocks")).toBeUndefined();
    await settle();
  });
});
