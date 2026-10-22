import i18n from "@/i18n";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MoverRecallPanel } from "../MoverRecallPanel";

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

function makeView(chain: string) {
  return {
    chain,
    from: "2026-09-30",
    to: "2026-10-20",
    dataSince: "2026-09-30",
    collectedDays: 9,
    eventsByTier: {},
    universeSize: 681,
    universeConfirmed: 595,
    rules: [
      { period: "ultra_short", gainPct: 10, windowDays: 2, varName: "mover_gain_ultra_short" },
    ],
    rates: {
      reachability: 50,
      coverage: 30,
      unexplainedShare: 3,
      events: 77,
      misses: 65,
      judgeableEvents: 77,
    },
    codesWithClose: 684,
    judgeableCodesByTier: { ultra_short: 680 },
    gateVersion: "mover-layers-v2",
    layers: [{ layer: "not_in_pool", count: 38, byMarketType: [["main_sz", 33]] }],
    misses: chain === "reco"
      ? []
      : [],
  };
}

const cmd = (name: string) => invokeMock.mock.calls.filter((c) => c[0] === name);

function renderPanel(entry = "/backtest") {
  return render(
    <I18nextProvider i18n={i18n}>
      <MemoryRouter initialEntries={[entry]}>
        <MoverRecallPanel />
      </MemoryRouter>
    </I18nextProvider>,
  );
}

beforeEach(() => {
  invokeMock.mockReset();
});

/**
 * A-2 链维度（`PLAN-missed-mover-reflection-closure.md` F-2）：现场重算一次拿回**两份**视图，
 * 面板必须说清当前这份账属于哪条链，且换链只换内存里那一份（同一次核查算出来的两链结果
 * 重新请求会随库里新落的批次漂移）。
 */
describe("MoverRecallPanel 链维度", () => {
  it("A. 两链各一份：默认落在第一份（荐股链），并出现链选择器", async () => {
    invokeMock.mockImplementation(async (name: string) => {
      if (name === "list_mover_recall_runs") { return []; }
      if (name === "analyze_mover_recall") { return [makeView("reco"), makeView("serenity")]; }
      return null;
    });
    renderPanel();
    await waitFor(() => expect(screen.getByTestId("mover-recall-chain")).toBeTruthy());
    expect(cmd("analyze_mover_recall").length).toBe(1);
    expect(screen.getByTestId("mover-recall-chain").textContent).toContain("智能荐股");
    const select = screen.getByTestId("mover-recall-chain-select");
    expect(select).toBeTruthy();
    fireEvent.mouseDown(select);
    expect(await screen.findByTitle("趋势智选")).toBeTruthy();
  });

  it("B. 切链只换内存那一份：不得为换链重发核查命令", async () => {
    invokeMock.mockImplementation(async (name: string) => {
      if (name === "list_mover_recall_runs") { return []; }
      if (name === "analyze_mover_recall") { return [makeView("reco"), makeView("serenity")]; }
      return null;
    });
    renderPanel();
    await waitFor(() => expect(screen.getByTestId("mover-recall-chain")).toBeTruthy());
    fireEvent.mouseDown(screen.getByTestId("mover-recall-chain-select"));
    fireEvent.click(await screen.findByTitle("趋势智选"));
    await waitFor(() => expect(screen.getByTestId("mover-recall-chain").textContent).toContain("趋势智选"));
    expect(cmd("analyze_mover_recall").length).toBe(1);
    // 两链来自同一次核查 ⇒ 换了链仍要能看出这份是「现场重算」而非某一落库轮次
    expect(screen.getByTestId("mover-recall-source").textContent).toContain("现场重算");
  });

  it("C. 载荷不是数组（契约漂移）：走失败支并点名缺的字段，不静默按单份渲染", async () => {
    invokeMock.mockImplementation(async (name: string) => {
      if (name === "list_mover_recall_runs") { return []; }
      if (name === "analyze_mover_recall") { return makeView("reco"); }
      return null;
    });
    renderPanel();
    await waitFor(() => expect(screen.getByTestId("mover-recall-failed")).toBeTruthy());
    expect(screen.queryByTestId("mover-recall-chain")).toBeNull();
    expect(cmd("analyze_mover_recall").length).toBe(1);
  });

  it("D. 读回分链之前的混池轮次：单独成句说明「未分链」，不冒充某一链的账", async () => {
    // 现网真形态：mrr-d5cb333a 落在 A-2 之前 ⇒ chain 为 null，rates 是混池口径
    const oldRun = {
      id: "mrr-d5cb333a",
      chain: null,
      dateFrom: "2026-09-30",
      dateTo: "2026-10-16",
      dataSince: "2026-09-30",
      collectedDays: 9,
      eventsByTier: {},
      universeSize: 681,
      universeConfirmed: 595,
      eventsTotal: 51,
      missCount: 46,
      status: "success",
      triggerSource: "scheduler",
      createdAtMs: 1792199083889,
    };
    invokeMock.mockImplementation(async (name: string) => {
      if (name === "list_mover_recall_runs") { return [oldRun]; }
      if (name === "get_mover_recall_run_view") { return { ...makeView("unlabeled"), run: oldRun }; }
      return null;
    });
    renderPanel();
    await waitFor(() => expect(screen.getByTestId("mover-recall-chain")).toBeTruthy());
    expect(within(screen.getByTestId("mover-recall-chain")).getByTestId("mover-recall-chain-unlabeled")).toBeTruthy();
    expect(screen.getByTestId("mover-recall-chain").textContent).toContain("未分链");
    // 混池轮次只有一份视图 ⇒ 不该长出链选择器（没有可换的另一份）
    expect(screen.queryByTestId("mover-recall-chain-select")).toBeNull();
  });
});

/**
 * 事件域与口径呈现（阶段 3/5）：清单数不等于事件域；只核右端日必须写明；
 * 逐档可判票为 0 时不得把「零达标」当成「真无达标」。
 */
describe("MoverRecallPanel 事件域呈现", () => {
  it("显示有收盘票数、逐档可判数与口径版本，并写明只核右端日", async () => {
    const view = {
      ...makeView("reco"),
      codesWithClose: 684,
      judgeableCodesByTier: { ultra_short: 680 },
      gateVersion: "mover-layers-v2",
    };
    invokeMock.mockImplementation(async (name: string) => {
      if (name === "list_mover_recall_runs") { return []; }
      if (name === "analyze_mover_recall") { return [view]; }
      return null;
    });
    renderPanel();
    await waitFor(() => expect(screen.getByTestId("mover-recall-event-domain")).toBeTruthy());
    expect(screen.getByTestId("mover-recall-event-domain").textContent).toContain("684");
    expect(screen.getByTestId("mover-recall-event-domain").textContent).toContain("680");
    expect(screen.getByTestId("mover-recall-gate-version").textContent).toContain("mover-layers-v2");
    expect(screen.getByTestId("mover-recall-boundary").textContent).toContain("只核查 2026-10-20");
  });

  it("本档无历史可判票时，零事件不得说成「真无达标」", async () => {
    const view = {
      ...makeView("reco"),
      codesWithClose: 684,
      judgeableCodesByTier: { ultra_short: 0 },
      rates: {
        reachability: null,
        coverage: null,
        unexplainedShare: null,
        events: 0,
        misses: 0,
        judgeableEvents: 0,
      },
    };
    invokeMock.mockImplementation(async (name: string) => {
      if (name === "list_mover_recall_runs") { return []; }
      if (name === "analyze_mover_recall") { return [view]; }
      return null;
    });
    renderPanel();
    await waitFor(() => expect(screen.getByTestId("mover-recall-empty")).toBeTruthy());
    expect(screen.getByTestId("mover-recall-empty").textContent).toContain("足够历史");
    expect(screen.getByTestId("mover-tier-judgeable-ultra_short")).toBeTruthy();
  });
});
