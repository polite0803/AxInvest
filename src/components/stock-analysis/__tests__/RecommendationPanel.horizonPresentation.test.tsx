import i18n from "@/i18n";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import type { BacktestComparisonResponse, StrategyStats } from "@/types/stock-analysis";
import { render, screen, waitFor } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CompactRecommendation } from "../dual-view/CompactRecommendation";
import { RecommendationPanel } from "../RecommendationPanel";
import { RecoStrategyMatrix } from "../RecoStrategyMatrix";

/**
 * 荐股四档的呈现契约（`PLAN-reco-horizon-science-alignment.md` Phase UI-0）。
 *
 * 每条断言都写成「修复前必红」的形态（反控内联），而不是只验新行为：
 *   F1 旧三元 `short/mid/else→long` 会把 ultra_short 显示成「长期」；
 *   F2 矩阵 `PERIOD_KEYS` 缺 ultra_short ⇒ 该档表头不存在；
 *   F3 旧实现把四档胜率做算术平均 ⇒ 60%/40% 会渲染成 50%；
 *   F4 缺失字段旧实现渲染 `0d` / 置信 `0`（把缺席压成读数）；
 *   F5 `degradedReasons` 旧实现完全不消费；
 *   F6 缩略视图旧实现不过滤 synthetic，兜底票可进 Top3。
 */
const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

type Pick = Record<string, unknown>;

function makePick(overrides: Partial<Pick> = {}): Pick {
  return {
    stockCode: "600001",
    stockName: "测试甲",
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
    ...overrides,
  };
}

function recoResponse(picks: Record<string, Pick[]>, extra: Record<string, unknown> = {}) {
  return {
    period: "short",
    picks,
    disabledStyles: [],
    degradedStyles: [],
    generatedAt: Date.now(),
    rawSeedPoolSize: 90,
    mode: "live",
    ...extra,
  };
}

function stats(style: string, period: string, winRatePct: number, totalSignals: number): StrategyStats {
  return {
    strategyId: `${style}_${period}`,
    style,
    period,
    totalSignals,
    winCount: 1,
    lossCount: 1,
    winRatePct,
    avgReturnPct: 1,
    totalReturnPct: 2,
    avgMaxDrawdownPct: 1,
    maxConsecutiveLosses: 1,
    sharpeRatio: null,
    profitFactor: null,
  };
}

function comparisonResponse(strategies: StrategyStats[]): BacktestComparisonResponse {
  const group = { label: "positive", stockCount: 1, strategies: {} as Record<string, StrategyStats> };
  for (const s of strategies) { group.strategies[s.strategyId] = s; }
  const empty = { label: "negative", stockCount: 0, strategies: {} as Record<string, StrategyStats> };
  return {
    positive: group,
    negative: empty,
    positiveStocks: ["600001"],
    negativeStocks: [],
    skipped: [],
  };
}

function renderWithI18n(node: React.ReactNode) {
  return render(
    <MemoryRouter>
      <I18nextProvider i18n={i18n}>{node}</I18nextProvider>
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

describe("档位显示名（F1 / F2 / F8）", () => {
  it("缩略视图 ultra_short 渲染「超短线」，绝不渲染成「长期」", () => {
    renderWithI18n(
      <CompactRecommendation
        data={recoResponse({ trend: [makePick()] }, { period: "ultra_short" })}
      />,
    );
    expect(screen.getByText("超短线")).toBeTruthy();
    // 反控：旧三元走 else ⇒ 这里会出现「长期」
    expect(screen.queryByText("长期")).toBeNull();
  });

  it("缩略视图认不出的档名原样显示键名，不猜档", () => {
    renderWithI18n(
      <CompactRecommendation data={recoResponse({ trend: [makePick()] }, { period: "midterm" })} />,
    );
    expect(screen.getByText("midterm")).toBeTruthy();
  });

  it("回测矩阵表头必须四档齐全（缺 ultra_short 即红）", () => {
    renderWithI18n(
      <RecoStrategyMatrix
        data={comparisonResponse([
          stats("trend", "ultra_short", 50, 8),
          stats("trend", "short", 60, 9),
        ])}
      />,
    );
    for (const label of ["超短线", "短线", "中期", "长期"]) {
      expect(screen.getAllByText(label).length).toBeGreaterThan(0);
    }
  });
});

describe("兜底合成候选（F6）", () => {
  it("缩略视图不渲染兜底票；全是兜底时报数量而不是「暂无推荐」", () => {
    renderWithI18n(
      <CompactRecommendation
        data={recoResponse({
          trend: [
            makePick({ stockName: "兜底乙", synthetic: true }),
            makePick({ stockCode: "600002", stockName: "兜底丙", synthetic: true }),
          ],
        })}
      />,
    );
    expect(screen.queryByText("兜底乙")).toBeNull();
    expect(screen.queryByText("兜底丙")).toBeNull();
    expect(screen.getByText(/0 条真实候选 · 2 条兜底候选已隐藏/)).toBeTruthy();
    expect(screen.queryByText("暂无推荐")).toBeNull();
  });
});

describe("回测徽章按当前档取值，不跨档平均（F3）", () => {
  it("当前档 short=60% 时徽章是 60%，不是四档算术平均的 50%", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_cached_recommendation" || cmd === "recommend_stocks") {
        return Promise.resolve(recoResponse({ trend: [makePick()] }));
      }
      if (cmd === "backtest_reco_strategies") {
        return Promise.resolve(comparisonResponse([
          stats("trend", "ultra_short", 30, 5),
          stats("trend", "short", 60, 9),
          stats("trend", "mid", 40, 7),
          stats("trend", "long", 80, 3),
        ]));
      }
      return Promise.resolve({});
    });
    renderWithI18n(<RecommendationPanel />);

    await waitFor(() => expect(screen.getAllByText("测试甲").length).toBeGreaterThan(0));
    await waitFor(() => expect(screen.getByText("60%")).toBeTruthy());
    // 反控：旧实现 (30+60+40+80)/4 = 52.5 → 渲染 "52.5%"；三档平均 50 也不得出现
    expect(screen.queryByText("52.5%")).toBeNull();
    expect(screen.queryByText("50%")).toBeNull();
  });
});

describe("缺席不冒充读数（F4）", () => {
  it("持有天数与置信度缺失时渲染「—」，不渲染 0d / 置信 0", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_cached_recommendation" || cmd === "recommend_stocks") {
        return Promise.resolve(recoResponse({
          trend: [makePick({ holdingDays: undefined, confidence: undefined })],
        }));
      }
      return Promise.resolve({});
    });
    renderWithI18n(<RecommendationPanel />);

    await waitFor(() => expect(screen.getAllByText("测试甲").length).toBeGreaterThan(0));
    expect(screen.queryByText(/0d/)).toBeNull();
    // 「持有」「置信度」两处各渲染一个 —（文本与标签在同一元素内，按整串匹配）
    expect(screen.getAllByText(/持仓天数\s*—/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/置信度\s*—/).length).toBeGreaterThan(0);
  });
});

describe("降级归因消费（F5）", () => {
  it("replay 下渲染后端逐风格 degradedReasons 文本", async () => {
    useTimeAnchorStore.setState({ asOfDate: "2026-06-01", mode: "replay" });
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "recommend_stocks") {
        return Promise.resolve(recoResponse(
          { trend: [makePick()] },
          {
            degradedStyles: ["trend"],
            degradedReasons: { trend: "PE-TTM 仅有当日快照，无截止日历史" },
            asOfDate: "2026-06-01",
            mode: "replay",
          },
        ));
      }
      return Promise.resolve({});
    });
    renderWithI18n(<RecommendationPanel />);

    await waitFor(() => expect(screen.getAllByText("测试甲").length).toBeGreaterThan(0));
    await waitFor(() => expect(screen.getByText(/PE-TTM 仅有当日快照，无截止日历史/)).toBeTruthy());
  });
});
