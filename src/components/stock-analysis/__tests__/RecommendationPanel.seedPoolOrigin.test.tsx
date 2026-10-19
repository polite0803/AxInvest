import i18n from "@/i18n";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { RecommendationPanel } from "../RecommendationPanel";

/**
 * 候选池「只剩内置样本池」的可见性防回归测试。
 *
 * 背景（2026-09-27）：`FALLBACK_STOCKS` 是**无条件**混进种子池的 ⇒ 候选池在任何
 * 数据源状态下都非空。as-of 回放的截止日若没有热股榜/行业榜快照，astock-data 按设计
 * 返回 `Ok(vec![])`，池子 100% 是硬编码样本，而策略照样产出一堆 `synthetic === false`
 * 的"真实" pick —— 面板呈现出一次完全正常的荐股，用户据此以为「那天真的没有热门股」。
 *
 * 这条歧义与 `synthetic`（**结论**层的占位兜底）不是一回事，本测试锁定的是**输入**层：
 *   A. 真实榜一条都没取到、只有内置样本时，必须显式声明；
 *   B. 取到了任一真实榜时，不得报警（内置补全本来就一直在）；
 *   C. preseed 路径（三源全 0 = 来源未知）不得误报。
 */
const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

function makePick(code: string, name: string) {
  return {
    stockCode: code,
    stockName: name,
    style: "trend",
    period: "short",
    price: 10,
    entryLow: 9.8,
    entryHigh: 10.2,
    stopLoss: 9.5,
    targetPrice: 11,
    positionPct: 5,
    holdingDays: 5,
    confidence: 70,
    reasons: [],
    riskNotes: [],
    synthetic: false,
  };
}

function mockResponse(
  // `user`/`mover` 是后加的第四、第五来源（自选股 ∪ 本机标的 / 当日涨幅榜）。夹具缺它们
  // 会造出「undefined === 0 为假 ⇒ 预警不显示」的假失败，所以这里按生产形态补默认 0。
  seedPoolOrigin: { hot: number; industry: number; fallback: number; user?: number; mover?: number },
  asofDegradations: Array<{ vendor: string; method: string; reason: string; kind: string }> = [],
) {
  const origin = { user: 0, mover: 0, ...seedPoolOrigin };
  invokeMock.mockResolvedValue({
    period: "short",
    picks: { trend: [makePick("600001", "候选甲")] },
    disabledStyles: [],
    degradedStyles: [],
    generatedAt: Date.now(),
    rawSeedPoolSize: origin.hot + origin.industry + origin.fallback + origin.user + origin.mover,
    seedPoolOrigin: origin,
    asofDegradations,
    mode: "replay",
  });
}

function renderPanel() {
  return render(
    <MemoryRouter>
      <I18nextProvider i18n={i18n}>
        <RecommendationPanel />
      </I18nextProvider>
    </MemoryRouter>,
  );
}

beforeEach(() => {
  invokeMock.mockReset();
  useTimeAnchorStore.setState({
    asOfDate: null,
    mode: "live",
    tourSeen: true,
    pendingLiveConfirm: false,
  });
});

describe("RecommendationPanel — 候选池来源声明", () => {
  it("A. 真实榜全空、仅内置样本池时必须显式声明", async () => {
    mockResponse({ hot: 0, industry: 0, fallback: 80 });
    renderPanel();

    await waitFor(() => expect(screen.getAllByText("候选甲").length).toBeGreaterThan(0));
    await waitFor(() => expect(screen.getByTestId("reco-pool-fallback-only").textContent).toContain("内置样本池"));
  });

  it("B. 取到任一真实榜来源时不得报警", async () => {
    mockResponse({ hot: 30, industry: 12, fallback: 80 });
    renderPanel();

    await waitFor(() => expect(screen.getAllByText("候选甲").length).toBeGreaterThan(0));
    expect(screen.queryByTestId("reco-pool-fallback-only")).toBeNull();
  });

  it("D. 扫描板块范围：读到持久化值就显示，切换后写回后端", async () => {
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_reco_board_scope") { return "star"; }
      return null;
    });
    renderPanel();
    await waitFor(() => expect(screen.getByTestId("reco-board-scope").textContent).toContain("科创板"));

    // antd Select 的标准交互：先 mouseDown 选择器，再点弹层里的选项
    fireEvent.mouseDown(screen.getByTestId("reco-board-scope"));
    fireEvent.click(await screen.findByTitle("创业板"));
    await waitFor(() => {
      const call = invokeMock.mock.calls.find((c) => c[0] === "set_reco_board_scope");
      expect(call?.[1]).toEqual({ scope: "chinext" });
    });
  });

  it("B2. 有用户自己的标的时不得报「只剩内置样本」（第四来源计入判据）", async () => {
    // 2026-10-19：用户标的（自选股 ∪ 本机标的）并入候选池后，池子不再只由内置样本构成。
    // 若判据漏掉 user，这条警告会常亮，把「注入了但没被选中」误报成「池子退化」。
    mockResponse({ hot: 0, industry: 0, fallback: 80, user: 5 });
    renderPanel();

    await waitFor(() => expect(screen.getAllByText("候选甲").length).toBeGreaterThan(0));
    expect(screen.queryByTestId("reco-pool-fallback-only")).toBeNull();
  });

  it("C. preseed（三源全 0，来源未知）不得误报", async () => {
    mockResponse({ hot: 0, industry: 0, fallback: 0 });
    renderPanel();

    await waitFor(() => expect(screen.getAllByText("候选甲").length).toBeGreaterThan(0));
    expect(screen.queryByTestId("reco-pool-fallback-only")).toBeNull();
  });

  it("D. 声明条必须带上本次运行的真实归因，而不是只有一句套话", async () => {
    mockResponse({ hot: 0, industry: 0, fallback: 80 }, [
      {
        vendor: "astock-data",
        method: "get_hot_stocks",
        reason: "as-of 热门股榜单 无 as-of 历史通道：3 个源均未申报 as-of 能力",
        kind: "structuralGap",
      },
      {
        vendor: "astock-data",
        method: "get_stock_announcements",
        reason: "与候选池无关的降级，不得混进这条提示",
        kind: "failure",
      },
    ]);
    renderPanel();

    await waitFor(() => expect(screen.getAllByText("候选甲").length).toBeGreaterThan(0));
    const alert = screen.getByTestId("reco-pool-fallback-only");
    // 只取热股榜 / 行业榜两条：其它维度混进来会让用户以为它们也是候选池缺口的原因
    await waitFor(() => expect(alert.textContent).toContain("无 as-of 历史通道"));
    expect(alert.textContent).not.toContain("与候选池无关");
  });
});
