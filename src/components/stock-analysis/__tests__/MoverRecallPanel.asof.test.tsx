import i18n from "@/i18n";
import type { MoverRecallEventReview, MoverRecallView } from "@/types/stock-analysis";
import { act, fireEvent, render, screen } from "@testing-library/react";
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
const ROW_KEY = `${STOCK}-${ANCHOR}-${PERIOD}`;

function makeView(): MoverRecallView {
  return {
    chain: "reco",
    from: "2026-09-01",
    to: "2026-09-30",
    dataSince: "2026-01-01",
    collectedDays: 200,
    eventsByTier: null,
    universeSize: 46,
    universeConfirmed: 46,
    reportedTotal: 5925,
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
    judgeableCodesByTier: { ultra_short: 680, short: 600, mid: 300, long: 120 },
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

function makeReview(over: Partial<MoverRecallEventReview> = {}): MoverRecallEventReview {
  return {
    eventId: "evt-1",
    runId: "run-1",
    status: "analyzed",
    missLayer: "not_in_pool",
    asofLayer: "l3_scored_out",
    asofRecoGeneratedAt: "2026-10-24T10:00:00.000",
    asofDiffJson: JSON.stringify({
      poolState: "present",
      inPool: true,
      pickedStyles: ["capital"],
      trimmedStyles: ["trend", "reversion"],
      pickedOtherPeriods: ["short"],
      divergedFromLive: true,
      replayFailure: null,
    }),
    analysisSignalAtStart: JSON.stringify({ horizon: PERIOD, action: "hold", confidence: 42 }),
    period: PERIOD,
    stockCode: STOCK,
    stockName: "测试股",
    windowStart: "2026-09-18",
    anchorDate: ANCHOR,
    windowDays: 2,
    cumGainPct: 11.2,
    maxDailyPct: 6.1,
    batchId: null,
    analysisId: null,
    reflectionId: null,
    errorDetail: null,
    analysis: null,
    reflections: [],
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

type Overrides = {
  review?: MoverRecallEventReview;
  asofSummary?: unknown;
  reflowMerge?: string;
  setCalls?: string[];
};

async function mountAndExpand(over: Overrides = {}) {
  invokeMock.mockImplementation((cmd: string, args?: unknown) => {
    switch (cmd) {
      case "analyze_mover_recall":
        return Promise.resolve([makeView()]);
      case "get_mover_recall_event_review":
        return Promise.resolve(over.review ?? makeReview());
      case "get_mover_asof_summary":
        return Promise.resolve(over.asofSummary ?? null);
      case "get_mover_reflow_merge":
        return Promise.resolve(over.reflowMerge ?? "off");
      case "set_mover_reflow_merge":
        over.setCalls?.push(JSON.stringify(args));
        return Promise.resolve((args as { value?: string } | undefined)?.value ?? "off");
      default:
        return Promise.resolve(null);
    }
  });
  renderWithProviders();
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

describe("MoverRecallPanel as-of 回放呈现", () => {
  /** as-of 层与 live 层必须分列；截断风格/别档/分歧逐项可见。 */
  it("renders as-of evidence alongside the live layer", async () => {
    await mountAndExpand();
    const block = await screen.findByTestId("mover-review-asof");
    const text = block.textContent ?? "";
    expect(text).toContain(i18n.t("stockAnalysis.moverRecall.review.asofHeading"));
    // as-of 层以**译文**出现在 as-of 块里（层码本身只进 prompt/落库，不上 UI）
    expect(text).toContain(
      i18n.t("stockAnalysis.moverRecall.layer.l3_scored_out", { defaultValue: "l3_scored_out" }),
    );
    expect(text).toContain("trend");
    expect(text).toContain("short");
    expect(text).toContain(i18n.t("stockAnalysis.moverRecall.review.asofDiverged"));
    // 决策摘要与「未记录」是两态
    const signal = await screen.findByTestId("mover-review-analysis-signal");
    expect(signal.textContent).toContain("hold");
  });

  /** 没有 as-of 证据 ⇒ 显式成句，不得留白或把 live 层伪装成 as-of。 */
  it("declares missing as-of evidence instead of leaving it blank", async () => {
    await mountAndExpand({
      review: makeReview({
        asofLayer: null,
        asofRecoGeneratedAt: null,
        asofDiffJson: null,
        analysisSignalAtStart: null,
      }),
    });
    const block = await screen.findByTestId("mover-review-asof");
    expect(block.textContent).toContain(
      i18n.t("stockAnalysis.moverRecall.review.asofMissing"),
    );
    const signal = await screen.findByTestId("mover-review-analysis-signal");
    expect(signal.textContent).toContain(
      i18n.t("stockAnalysis.moverRecall.review.analysisSignalMissing"),
    );
  });

  /** 轮级摘要：六桶渲染；缺失时「该轮还没跑过回放」单独成句。 */
  it("shows the run-level as-of summary and its missing state", async () => {
    // mock 必须先于 render：React 19 的 effect 在提交阶段就可能发出 invoke，
    // 先 render 后设 mock 会让首轮拿到 undefined（该做法曾让本用例找不到权重行）。
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "analyze_mover_recall") { return Promise.resolve([makeView()]); }
      if (cmd === "get_mover_asof_summary") {
        return Promise.resolve({
          runId: "run-1",
          dateTo: "2026-09-30",
          layers: { l3_scored_out: { ultra_short: 2 } },
          summary: {
            events: 5,
            byPeriod: {
              ultra_short: { inPoolTrimmed: 2, notInPool: 1, unavailable: 2 },
            },
            divergedFromLive: 3,
            failures: [],
          },
          weightChanges: [{ style: "trend", period: "short", old: 0.45, new: 0.26 }],
        });
      }
      return Promise.resolve(null);
    });
    const { unmount } = render(
      <I18nextProvider i18n={i18n}>
        <MemoryRouter>
          <MoverRecallPanel />
        </MemoryRouter>
      </I18nextProvider>,
    );
    await act(async () => {
      await flushPromises();
    });
    const weights = await screen.findByTestId("mover-asof-weights");
    expect(weights.textContent).toContain("0.45→0.26");
    unmount();

    await mountAndExpand({ asofSummary: null });
    const none = await screen.findByTestId("mover-asof-summary-none");
    expect(none.textContent).toContain(
      i18n.t("stockAnalysis.moverRecall.asofSummaryMissing"),
    );
  });

  /** B-7：事件档反思行永远排第一（否则短档先完成会把事件档挤到后面）。 */
  it("puts the event-tier reflection first", async () => {
    await mountAndExpand({
      review: makeReview({
        period: "mid",
        reflections: [
          {
            id: "rf-short",
            stockCode: STOCK,
            stockName: "测试股",
            originalAnalysisId: null,
            asOfDate: "2026-09-18",
            hindsightDate: "2026-09-25",
            horizon: "short",
            actualOutcome: "5天涨+3%",
            whatWentWrong: null,
            missedSignals: null,
            fixForFuture: null,
            verdict: "partial",
            lessonSummary: "短档",
            rawReturn: 3,
            alphaReturn: 1,
            source: "mover_recall",
            missLayer: "not_in_pool",
            moverLabel: "normal",
            status: "completed",
            createdAt: 1,
          },
          {
            id: "rf-mid",
            stockCode: STOCK,
            stockName: "测试股",
            originalAnalysisId: null,
            asOfDate: "2026-09-18",
            hindsightDate: "2026-10-16",
            horizon: "mid",
            actualOutcome: "28天涨+31%",
            whatWentWrong: null,
            missedSignals: null,
            fixForFuture: null,
            verdict: "wrong",
            lessonSummary: "中档",
            rawReturn: 31,
            alphaReturn: 20,
            source: "mover_recall",
            missLayer: "not_in_pool",
            moverLabel: "mover",
            status: "completed",
            createdAt: 1,
          },
        ],
      }),
    });
    const cards = await screen.findAllByTestId(/^mover-review-rf-/);
    expect(cards[0].getAttribute("data-testid")).toBe("mover-review-rf-mid");
  });

  /** 开关写失败要回滚 UI（不得显示一个没生效的状态）。 */
  it("rolls the reflow merge switch back when the write fails", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "analyze_mover_recall") { return Promise.resolve([makeView()]); }
      if (cmd === "get_mover_reflow_merge") { return Promise.resolve("off"); }
      if (cmd === "set_mover_reflow_merge") { return Promise.reject(new Error("boom")); }
      return Promise.resolve(null);
    });
    renderWithProviders();
    await act(async () => {
      await flushPromises();
    });
    const sw = await screen.findByRole("switch");
    expect(sw.getAttribute("aria-checked")).toBe("false");
    await act(async () => {
      fireEvent.click(sw);
      await flushPromises();
    });
    expect(screen.getByRole("switch").getAttribute("aria-checked")).toBe("false");
  });
});
