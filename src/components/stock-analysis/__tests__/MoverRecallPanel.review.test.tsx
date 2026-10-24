import i18n from "@/i18n";
import type { MoverRecallEventReview, MoverRecallView } from "@/types/stock-analysis";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MoverRecallPanel } from "../MoverRecallPanel";

/** 让 invoke 的 Promise 链（loading → data 两次 setState）在 act 内 flush 干净。 */
const flushPromises = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

const STOCK = "600519";
const ANCHOR = "2026-09-20";
const PERIOD = "ultra_short";
/** 与 missKey 三键拼法一致（票×右端日×档）。 */
const ROW_KEY = `${STOCK}-${ANCHOR}-${PERIOD}`;

/** 带 1 条漏检行的核查视图（展开入口所在）。 */
function makeViewWithMiss(): MoverRecallView {
  return {
    chain: "reco",
    from: "2026-09-01",
    to: "2026-09-30",
    dataSince: "2026-01-01",
    collectedDays: 200,
    eventsByTier: null,
    universeSize: 46,
    universeConfirmed: 46,
    rules: [
      { period: "ultra_short", gainPct: 10, windowDays: 2, varName: "mover_gain_ultra_short" },
      { period: "short", gainPct: 20, windowDays: 5, varName: "mover_gain_short" },
      { period: "mid", gainPct: 30, windowDays: 28, varName: "mover_gain_mid" },
      { period: "long", gainPct: 40, windowDays: 90, varName: "mover_gain_long" },
    ],
    rates: {
      reachability: 50,
      coverage: 50,
      unexplainedShare: 0,
      events: 1,
      misses: 1,
      judgeableEvents: 1,
    },
    codesWithClose: 684,
    judgeableCodesByTier: { ultra_short: 680 },
    gateVersion: "mover-layers-v2",
    layers: [{ layer: "not_in_pool", count: 1, byMarketType: [["main_sh", 1]] }],
    misses: [
      {
        event: {
          stockCode: STOCK,
          stockName: "测试股",
          period: PERIOD,
          anchorDate: ANCHOR,
          windowStart: "2026-09-18",
          windowDays: 2,
          cumGainPct: 11.2,
          maxDailyPct: 6.1,
          thresholdPct: 10,
          marketType: "main_sh",
          recommended: false,
        },
        layer: "not_in_pool",
        pickedAnyPeriod: false,
        scoredOutStyles: [],
      },
    ],
  };
}

/** 构造复盘视图；默认 reflected + 补分析 + 两档反思（completed / pending）。 */
function makeReview(over: Partial<MoverRecallEventReview> = {}): MoverRecallEventReview {
  return {
    eventId: "evt-1",
    runId: "run-1",
    status: "reflected",
    missLayer: "not_in_pool",
    asofLayer: null,
    asofRecoGeneratedAt: null,
    asofDiffJson: null,
    analysisSignalAtStart: null,
    period: PERIOD,
    stockCode: STOCK,
    stockName: "测试股",
    windowStart: "2026-09-18",
    anchorDate: ANCHOR,
    windowDays: 2,
    cumGainPct: 11.2,
    maxDailyPct: 6.1,
    batchId: "batch-1",
    analysisId: "an-1",
    reflectionId: "rf-1",
    errorDetail: null,
    analysis: {
      id: "an-1",
      asOfDate: "2026-09-18",
      analysisDate: "2026-09-18",
      status: "completed",
      decisionAction: "watch",
      decisionReasoning: "as-of 时点信号偏弱，建议观望。",
    },
    reflections: [
      // 后端按 horizon 字母序返回（short 在 ultra_short 前），前端须重排。
      {
        id: "rf-short",
        stockCode: STOCK,
        stockName: "测试股",
        originalAnalysisId: "an-1",
        asOfDate: "2026-09-18",
        hindsightDate: "2026-09-25",
        horizon: "short",
        actualOutcome: "5天涨+12%",
        whatWentWrong: "把资金流出误判为短期波动",
        missedSignals: JSON.stringify(["北向连续 3 日净流出", "竞价量能未放大"]),
        fixForFuture: "下次净流出连续 2 日即应下调评级阈值",
        verdict: "wrong",
        lessonSummary: "建议：连续两日北向净流出时下调一档评级",
        rawReturn: 12.0,
        alphaReturn: 9.5,
        source: "mover_recall",
        missLayer: "not_in_pool",
        moverLabel: "miss",
        status: "completed",
        createdAt: 1,
      },
      {
        id: "rf-mid",
        stockCode: STOCK,
        stockName: "测试股",
        originalAnalysisId: "an-1",
        asOfDate: "2026-09-18",
        hindsightDate: "2026-10-16",
        horizon: "mid",
        actualOutcome: "",
        whatWentWrong: null,
        missedSignals: null,
        fixForFuture: null,
        verdict: null,
        lessonSummary: null,
        rawReturn: null,
        alphaReturn: null,
        source: "mover_recall",
        missLayer: "not_in_pool",
        moverLabel: null,
        status: "pending",
        createdAt: 1,
      },
    ],
    ...over,
  };
}

function renderWithProviders() {
  return render(
    <I18nextProvider i18n={i18n}>
      <MemoryRouter>
        <MoverRecallPanel />
      </MemoryRouter>
    </I18nextProvider>,
  );
}

/** 核查视图渲染后点开漏检行；review 为 null/对象/throw 三选一。 */
async function expandRow(review: MoverRecallEventReview | null | Error) {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "analyze_mover_recall") { return Promise.resolve([makeViewWithMiss()]); }
    if (cmd === "get_mover_recall_event_review") {
      return review instanceof Error ? Promise.reject(review) : Promise.resolve(review);
    }
    return Promise.resolve(null);
  });
  renderWithProviders();
  // 首次挂载的 analyze_mover_recall Promise 链在 act 内 settle。
  await act(async () => {
    await flushPromises();
  });
  const toggle = await screen.findByTestId(`mover-review-toggle-${ROW_KEY}`);
  await act(async () => {
    fireEvent.click(toggle);
    await flushPromises();
  });
}

beforeEach(() => {
  invokeMock.mockReset();
});

describe("MoverRecallPanel 漏检行展开复盘（P3）", () => {
  it("命令返回 null：实时视图未落库提示，不渲染空壳", async () => {
    await expandRow(null);
    await waitFor(() => expect(screen.getByTestId("mover-review-not-tracked")).toBeTruthy());
    expect(screen.queryByTestId("mover-review-detail")).toBeNull();
  });

  it("加载失败：错误成句展示", async () => {
    await expandRow(new Error(JSON.stringify({ code: "WF_INTERNAL", detail: "复盘记录加载失败：库忙" })));
    const box = await screen.findByTestId("mover-review-error");
    expect(box.textContent ?? "").toContain("库忙");
  });

  it("pending 事件：状态标签 + 不展示分析/反思块", async () => {
    await expandRow(makeReview({
      status: "pending",
      analysis: null,
      analysisId: null,
      reflections: [],
    }));
    const detail = await screen.findByTestId("mover-review-detail");
    expect(detail.textContent ?? "").toContain("待补分析");
    expect(screen.queryByTestId("mover-review-analysis")).toBeNull();
    expect(screen.queryByTestId("mover-review-reflections")).toBeNull();
  });

  it("failed 事件：展示失败原因", async () => {
    await expandRow(makeReview({
      status: "failed",
      analysis: null,
      analysisId: null,
      errorDetail: "as-of 行情缺口",
      reflections: [],
    }));
    const box = await screen.findByTestId("mover-review-failed");
    expect(box.textContent ?? "").toContain("as-of 行情缺口");
  });

  it("reflected：分析摘要 + 逐档反思卡（completed 三问/信号解析，pending 只出排队标签，档位按固定序）", async () => {
    await expandRow(makeReview());
    const detail = await screen.findByTestId("mover-review-detail");
    const detailText = detail.textContent ?? "";

    // 分析摘要：as-of 日期 + 决策动作 + reasoning（label 与正文混排在同一元素，按 textContent 断言）
    const analysis = screen.getByTestId("mover-review-analysis");
    expect(analysis.textContent ?? "").toContain("2026-09-18");
    expect(analysis.textContent ?? "").toContain("watch");
    expect(analysis.textContent ?? "").toContain("as-of 时点信号偏弱");

    // 反思块：信号 JSON 数组被解析为两条 li 纯文本列表项
    expect(screen.getByText("北向连续 3 日净流出")).toBeTruthy();
    expect(screen.getByText("竞价量能未放大")).toBeTruthy();
    // 三问正文与 label 混排，按整卡 textContent 断言
    expect(detailText).toContain("把资金流出误判为短期波动");
    expect(detailText).toContain("下次净流出连续 2 日即应下调评级阈值");
    // wrong 裁决中文标签 + 入模教训 + 实际收益
    const shortCard = screen.getByTestId("mover-review-rf-short");
    expect(shortCard.textContent ?? "").toContain("错误");
    expect(shortCard.textContent ?? "").toContain("+12.0%");
    expect(shortCard.textContent ?? "").toContain("连续两日北向净流出时下调一档评级");
    // pending 档只出排队标签，不出三问
    const midCard = screen.getByTestId("mover-review-rf-mid");
    expect(midCard.textContent ?? "").toContain("反思排队中");
    expect(midCard.textContent ?? "").not.toContain("把资金流出");

    // 档位固定序：超短…短…中…长（本夹具只有 short/mid，short 卡必须在 mid 卡之前）
    const cards = screen
      .getAllByTestId(/^mover-review-rf-/)
      .map((el) => el.getAttribute("data-testid"));
    expect(cards).toEqual(["mover-review-rf-short", "mover-review-rf-mid"]);
  });

  it("已补分析但反思行全空：提示等待各窗口到期", async () => {
    await expandRow(makeReview({ status: "analyzed", reflections: [] }));
    const box = await screen.findByTestId("mover-review-reflections");
    expect(box.textContent ?? "").toContain("各持有窗口到期");
  });

  it("再次点击收起：详情卸载，且不重复请求（缓存命中）", async () => {
    await expandRow(makeReview());
    await screen.findByTestId("mover-review-detail");
    const callsAfterOpen = invokeMock.mock.calls.filter((c) => c[0] === "get_mover_recall_event_review").length;
    expect(callsAfterOpen).toBe(1);

    await act(async () => {
      fireEvent.click(screen.getByTestId(`mover-review-toggle-${ROW_KEY}`));
      await flushPromises();
    });
    expect(screen.queryByTestId("mover-review-detail")).toBeNull();

    // 重新展开：走缓存，不再发请求
    await act(async () => {
      fireEvent.click(screen.getByTestId(`mover-review-toggle-${ROW_KEY}`));
      await flushPromises();
    });
    expect(screen.getByTestId("mover-review-detail")).toBeTruthy();
    const callsFinal = invokeMock.mock.calls.filter((c) => c[0] === "get_mover_recall_event_review").length;
    expect(callsFinal).toBe(1);
  });
});
