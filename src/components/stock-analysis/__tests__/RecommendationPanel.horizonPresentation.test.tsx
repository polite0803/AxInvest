import i18n from "@/i18n";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import type { BacktestComparisonResponse, StrategyStats } from "@/types/stock-analysis";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
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

/** invoke 第一次以 `command` 被调用的参数（同 RecommendationPanel.asOfDate.test） */
function findCall(command: string): unknown[] | undefined {
  return invokeMock.mock.calls.find((c) => c[0] === command);
}

/** 在 Card extra 中寻找「刷新」按钮（避免匹配到空状态文案里的「刷新」字样） */
function findRefreshButton(): HTMLButtonElement {
  const extra = document.querySelector(".ant-card-extra");
  if (!extra) { throw new Error("card extra not found"); }
  const btn = extra.querySelector("button");
  if (!btn) { throw new Error("refresh button not found"); }
  return btn as HTMLButtonElement;
}

/** 等 loading 结束（刷新按钮带 ant-btn-loading 时点击无效） */
async function waitNotLoading() {
  await waitFor(() => expect(findRefreshButton()).not.toHaveClass("ant-btn-loading"));
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

describe("逐档 rank IC 观测面（R-E）", () => {
  it("有 IC 的格报数，够不到门槛的格必须分句报「样本不足」而不是留空", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "reco_ic_stats") {
        return Promise.resolve({
          styles: [{
            style: "trend",
            halfLifeDays: null,
            halfLifeStatus: "insufficient_tiers",
            cells: [
              { style: "trend", period: "ultra_short", rankIc: 0.42, samples: 12, icStatus: "ok", holdingDays: 2 },
              {
                style: "trend",
                period: "short",
                rankIc: null,
                samples: 3,
                icStatus: "insufficient_ic_samples",
                holdingDays: 5,
              },
            ],
          }],
          totalCells: 2,
          usableCells: 1,
          totalSamples: 15,
        });
      }
      return Promise.resolve(comparisonResponse([stats("trend", "ultra_short", 60, 9)]));
    });
    renderWithI18n(
      <RecoStrategyMatrix
        data={comparisonResponse([stats("trend", "ultra_short", 60, 9), stats("trend", "short", 55, 8)])}
      />,
    );
    await waitFor(() => expect(screen.getByText(/IC 0\.420 · n=12/)).toBeTruthy());
    // 样本不够 ≠ 无数据：必须给「需 ≥8」这类可行动的缺席句
    await waitFor(() => expect(screen.getAllByText(/样本/).length).toBeGreaterThan(0));
  });

  it("完全没有已验证样本的格也要显式说明，不显示成空白", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "reco_ic_stats") {
        return Promise.resolve({ styles: [], totalCells: 0, usableCells: 0, totalSamples: 0 });
      }
      return Promise.resolve(comparisonResponse([stats("trend", "short", 55, 8)]));
    });
    renderWithI18n(
      <RecoStrategyMatrix data={comparisonResponse([stats("trend", "short", 55, 8)])} />,
    );
    await waitFor(() => expect(screen.getAllByTestId("reco-ic-none").length).toBeGreaterThan(0));
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
      if (cmd === "recommend_stocks_all_periods") {
        return Promise.resolve({
          byHorizon: {
            short: recoResponse(
              { trend: [makePick()] },
              {
                degradedStyles: ["trend"],
                degradedReasons: { trend: "PE-TTM 仅有当日快照，无截止日历史" },
                asOfDate: "2026-06-01",
                mode: "replay",
              },
            ),
          },
          failedHorizons: {},
        });
      }
      return Promise.resolve({});
    });
    renderWithI18n(<RecommendationPanel />);

    await waitFor(() => expect(screen.getAllByText("测试甲").length).toBeGreaterThan(0));
    await waitFor(() => expect(screen.getByText(/PE-TTM 仅有当日快照，无截止日历史/)).toBeTruthy());
  });
});

describe("口径标注透传与跨档同分（R-C / R-D / R-F）", () => {
  /** 批量响应带上 short + mid 两档同一只票（同分对照用），并让缓存路径也能出票 */
  function mockBatchShort(pick: Record<string, unknown>) {
    const tier = recoResponse({ trend: [pick] });
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "recommend_stocks_all_periods") {
        return Promise.resolve({
          byHorizon: { short: tier, mid: tier },
          failedHorizons: {},
        });
      }
      if (cmd === "get_cached_recommendation") { return Promise.resolve(tier); }
      return Promise.resolve(null);
    });
    return tier;
  }

  /** 挂载 → 等缓存出票 → 点刷新（批量命令只有刷新才发，R-0 契约） */
  async function mountAndRefresh() {
    renderWithI18n(<RecommendationPanel />);
    await waitFor(() => expect(screen.getByText("测试甲")).toBeTruthy());
    await waitNotLoading();
    fireEvent.click(findRefreshButton());
    await waitFor(() => expect(findCall("recommend_stocks_all_periods")).toBeDefined());
  }

  it("σ 不可得时把止损口径标成固定百分比，不冒充波动率推导", async () => {
    mockBatchShort(makePick({ stopSource: "fallback_pct" }));
    await mountAndRefresh();
    expect(screen.getAllByText(/固定百分比/).length).toBeGreaterThan(0);
  });

  it("无历史先验样本时声明「仅由评分得出」，不假装四档各有先验", async () => {
    mockBatchShort(makePick({ priorSource: "absent" }));
    await mountAndRefresh();
    expect(screen.getAllByText(/仅由该风格评分得出/).length).toBeGreaterThan(0);
  });

  it("分位与绝对置信并存：展示分位但**不覆写** confidence", async () => {
    mockBatchShort(makePick({ confidence: 62, confidencePercentile: 30 }));
    await mountAndRefresh();
    expect(screen.getAllByText(/组内当日分位 30/).length).toBeGreaterThan(0);
    // 绝对置信仍是 62（旧实现会把 62 覆写成分位 30 ⇒ 概率与分位混成同一个数）
    expect(screen.getAllByText(/62/).length).toBeGreaterThan(0);
    expect(screen.queryByText(/置信度\s*30/)).toBeNull();
  });

  it("同一票在另一档同分 ⇒ 必须点名（分不清收敛与复制不可接受）", async () => {
    mockBatchShort(makePick({ confidence: 70 }));
    await mountAndRefresh();
    await waitFor(() => expect(screen.getByTestId("reco-same-score-peer")).toBeTruthy());
    expect(screen.getByTestId("reco-same-score-peer").textContent).toContain("中期");
  });
});

describe("一次拿四档（R-0）", () => {
  /** 四档都有真实产出的批量响应 */
  function batchFourTiers() {
    const byHorizon: Record<string, unknown> = {};
    for (const p of ["ultra_short", "short", "mid", "long"]) {
      byHorizon[p] = recoResponse({ trend: [makePick({ period: p, stockName: `票-${p}` })] }, { period: p });
    }
    return { byHorizon, failedHorizons: {} };
  }

  it("刷新走批量命令；切到另一档不再发任何扫描/缓存请求", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "recommend_stocks_all_periods") { return Promise.resolve(batchFourTiers()); }
      if (cmd === "get_cached_recommendation") { return Promise.resolve(null); }
      return Promise.resolve({});
    });
    renderWithI18n(<RecommendationPanel />);
    await waitFor(() => expect(findCall("get_cached_recommendation")).toBeDefined());
    await waitNotLoading();

    // 点刷新 → 一次批量扫描
    const refresh = findRefreshButton();
    fireEvent.click(refresh);
    await waitFor(() => expect(findCall("recommend_stocks_all_periods")).toBeDefined());
    const callsAfterRefresh = invokeMock.mock.calls.length;
    expect(screen.getByText("票-short")).toBeTruthy();

    // 切档：整批已在手，不得再发请求（旧形态是每档各扫一遍 / 各读一次缓存）
    fireEvent.click(screen.getByText("超短线"));
    await waitFor(() => expect(screen.getByText("票-ultra_short")).toBeTruthy());
    expect(invokeMock.mock.calls.length).toBe(callsAfterRefresh);
  });

  it("某一档失败时独立成句，不渲染成「该档没有推荐」", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "recommend_stocks_all_periods") {
        return Promise.resolve({
          byHorizon: { short: recoResponse({ trend: [makePick()] }) },
          failedHorizons: { short: "东财榜单接口超时" },
        });
      }
      return Promise.resolve(null);
    });
    renderWithI18n(<RecommendationPanel />);
    await waitFor(() => expect(findCall("get_cached_recommendation")).toBeDefined());
    await waitNotLoading();
    fireEvent.click(findRefreshButton());

    await waitFor(() => expect(screen.getByTestId("reco-horizon-failed")).toBeTruthy());
    const alertEl = screen.getByTestId("reco-horizon-failed");
    // 归因文本与档位名由 i18n 插值拆成多个文本节点，按整串断言
    expect(alertEl.textContent).toContain("东财榜单接口超时");
    expect(alertEl.textContent).toContain("短线");
  });
});

/**
 * 矩阵的**契约驱动**呈现（S4：缺席不冒充空白）。
 *
 * 反控形态（修复前必红）：
 *  - 行集合来自组件自带的 4 项 `STYLE_KEYS` ⇒ 趋势智选连一行都没有；
 *  - `style === "reversion" && period === "long"` 硬编码一格「—」，其余不成立格（含 serenity
 *    的短/超短两格）静默走「无数据」分支 ⇒ 「按设计不做」与「还没跑出样本」同为空白；
 *  - 一名两写（工作流链 `serenity` / 策略链 `bottleneck`）没有别名表 ⇒ 趋势智选行永远匹配不到数据。
 */
type MatrixCell = {
  style: string;
  period: string;
  active: boolean;
  reasonCode: string;
  misfitCode: string | null;
  dbStyles: string[];
};

/** 24 格契约夹具 —— 逐格与 `recommender/style_matrix.rs` 的 MATRIX / MISFIT_DECLARATIONS 对齐。 */
function contractFixture(): MatrixCell[] {
  const periods = ["ultra_short", "short", "mid", "long"];
  const inactive: Record<string, Record<string, string>> = {
    reversion: {
      ultra_short: "oversold_rebound_needs_at_least_mid_horizon",
      long: "oversold_rebound_not_applied_to_long_horizon",
    },
    serenity: {
      ultra_short: "serenity_needs_week_or_longer_realization",
      short: "serenity_needs_week_or_longer_realization",
    },
  };
  const out: MatrixCell[] = [];
  for (const style of ["trend", "value", "capital", "reversion", "watchlist", "serenity"]) {
    for (const period of periods) {
      const reason = inactive[style]?.[period];
      out.push({
        style,
        period,
        active: reason === undefined,
        reasonCode: reason ?? "cell_is_active",
        misfitCode: style === "value" && period === "ultra_short"
          ? "valuation_needs_weeks_to_realize_kept_by_user_decision"
          : null,
        dbStyles: style === "serenity" ? ["serenity", "bottleneck"] : [style],
      });
    }
  }
  return out;
}

function icStats(matrix: MatrixCell[] | undefined): Record<string, unknown> {
  return {
    styles: [],
    totalCells: 0,
    usableCells: 0,
    totalSamples: 0,
    ...(matrix ? { matrix } : {}),
  };
}

describe("矩阵契约驱动（S4）", () => {
  it("趋势智选有自己的行，四个不成立格各带理由而不是空白", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "reco_ic_stats"
        ? Promise.resolve(icStats(contractFixture()))
        : Promise.resolve(comparisonResponse([]))
    );
    renderWithI18n(<RecoStrategyMatrix data={comparisonResponse([])} />);
    await waitFor(() => expect(screen.getAllByText("趋势智选").length).toBeGreaterThan(0));
    const cells = await waitFor(() => {
      const els = screen.getAllByTestId("matrix-absence");
      expect(els.length).toBe(4);
      return els;
    });
    const text = cells.map((el) => el.textContent).join(" | ");
    expect(text).toContain("瓶颈证据需以周为单位兑现");
    expect(text).toContain("超跌反弹不适用于长线");
    expect(text).toContain("超跌反弹需至少中线兑现");
    // 出票但没样本的格走另一条句（与「按设计不做」分开）
    expect(screen.getAllByTestId("matrix-no-sample").length).toBeGreaterThan(0);
  });

  it("serenity 行的统计经 dbStyles 别名归一（落库写 bottleneck 也算趋势智选）", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "reco_ic_stats"
        ? Promise.resolve(icStats(contractFixture()))
        : Promise.resolve(comparisonResponse([stats("bottleneck", "mid", 58, 7)]))
    );
    renderWithI18n(<RecoStrategyMatrix data={comparisonResponse([stats("bottleneck", "mid", 58, 7)])} />);
    await waitFor(() => expect(screen.getByText("58.0%")).toBeTruthy());
  });

  it("出票但已知档-因子错配的格带错配声明（第三种状态，不混进不成立）", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "reco_ic_stats"
        ? Promise.resolve(icStats(contractFixture()))
        : Promise.resolve(comparisonResponse([stats("value", "ultra_short", 51, 9)]))
    );
    renderWithI18n(
      <RecoStrategyMatrix data={comparisonResponse([stats("value", "ultra_short", 51, 9)])} />,
    );
    const el = await waitFor(() => screen.getByTestId("matrix-misfit"));
    expect(el.textContent).toContain("估值需数周兑现");
  });

  it("观测面取不到时退回兜底行集合，主表不崩也不白屏", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "reco_ic_stats"
        ? Promise.reject(new Error("观测面不可得"))
        : Promise.resolve(comparisonResponse([stats("trend", "short", 55, 8)]))
    );
    renderWithI18n(<RecoStrategyMatrix data={comparisonResponse([stats("trend", "short", 55, 8)])} />);
    await waitFor(() => expect(screen.getByText("趋势跟踪")).toBeTruthy());
    // 兜底集合就是原 4 行：契约缺席时不得凭空多出/少掉行
    expect(screen.queryByText("趋势智选")).toBeNull();
  });
});
