import i18n from "@/i18n";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
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

/** 与 Rust `MoverRecallRunMeta` 同形（键名必须真，值取自现网那一轮）。 */
function makeRun(id = "mrr-aaa") {
  return {
    id,
    dateFrom: "2026-09-30",
    dateTo: "2026-10-20",
    dataSince: "2026-09-30",
    collectedDays: 9,
    eventsByTier: null,
    universeSize: 681,
    universeConfirmed: 595,
    eventsTotal: 77,
    missCount: 65,
    status: "success",
    triggerSource: "scheduler",
    createdAtMs: 1792492500679,
  };
}

function makeView() {
  return {
    chain: "reco",
    from: "2026-09-30",
    to: "2026-10-20",
    dataSince: "2026-09-30",
    collectedDays: 9,
    eventsByTier: null,
    universeSize: 681,
    universeConfirmed: 595,
    rules: [
      { period: "ultra_short", gainPct: 10, windowDays: 2, varName: "mover_gain_ultra_short" },
      { period: "short", gainPct: 20, windowDays: 5, varName: "mover_gain_short" },
      { period: "mid", gainPct: 30, windowDays: 28, varName: "mover_gain_mid" },
      { period: "long", gainPct: 40, windowDays: 90, varName: "mover_gain_long" },
    ],
    rates: { reachability: 50.6, coverage: 30.7, unexplainedShare: 3.07, events: 77, misses: 65 },
    layers: [{ layer: "not_in_pool", count: 38, byMarketType: [["main_sz", 33]] }],
    misses: [],
  };
}

/** `get_mover_recall_run_view` 的响应是「元信息 + 摊平载荷」，与实时核查同形。 */
function makeRunView(id = "mrr-aaa") {
  return { ...makeView(), run: makeRun(id) };
}

const cmd = (name: string) => invokeMock.mock.calls.filter((c) => c[0] === name);

function renderAt(entry = "/backtest") {
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
 * D-5 轮次读回（`PLAN-missed-mover-reflection-closure.md` F-13）：
 * 任务中心点「立即执行」落的那一轮必须能在面板里读到，且「已落库轮次」与「现场重算」
 * 「还没有轮次」「指定轮次不在库内」四种状态各自成句 —— 把后两种说成前一种是伪装。
 */
describe("MoverRecallPanel 轮次来源", () => {
  it("A. 已有落库轮次：默认读那一轮，不发出现场重算命令", async () => {
    invokeMock.mockImplementation(async (name: string) => {
      if (name === "list_mover_recall_runs") { return [makeRun()]; }
      if (name === "get_mover_recall_run_view") { return makeRunView(); }
      return null;
    });
    renderAt();
    await waitFor(() => expect(screen.getByTestId("mover-recall-source")).toBeTruthy());
    const call = cmd("get_mover_recall_run_view")[0];
    expect(call?.[1]).toEqual({ runId: "mrr-aaa" });
    expect(cmd("analyze_mover_recall").length).toBe(0);
    expect(screen.getByTestId("mover-recall-source").textContent).toContain("已落库核查轮次");
    expect(screen.getByTestId("mover-recall-source").textContent).toContain("mrr-aaa");
  });

  it("B. 库里一轮都没有：退回现场重算并单独成句（不得说成「本轮没有达标事件」）", async () => {
    invokeMock.mockImplementation(async (name: string) => {
      if (name === "list_mover_recall_runs") { return []; }
      if (name === "analyze_mover_recall") { return [makeView()]; }
      return null;
    });
    renderAt();
    await waitFor(() => expect(screen.getByTestId("mover-recall-source")).toBeTruthy());
    expect(screen.getByTestId("mover-recall-runs-empty")).toBeTruthy();
    expect(cmd("analyze_mover_recall").length).toBe(1);
    expect(cmd("get_mover_recall_run_view").length).toBe(0);
  });

  it("C. 深链指定的轮次不在库内：点名该轮并退回现场重算", async () => {
    invokeMock.mockImplementation(async (name: string) => {
      if (name === "list_mover_recall_runs") { return [makeRun("mrr-aaa")]; }
      if (name === "analyze_mover_recall") { return [makeView()]; }
      return null;
    });
    renderAt("/backtest?moverRun=mrr-gone");
    await waitFor(() => expect(screen.getByTestId("mover-recall-run-gone")).toBeTruthy());
    expect(screen.getByTestId("mover-recall-run-gone").textContent).toContain("mrr-gone");
    expect(cmd("analyze_mover_recall").length).toBe(1);
    expect(cmd("get_mover_recall_run_view").length).toBe(0);
  });

  it("E. 补分析批次从未发起：单独成句（0 批不得说成「没有待补」）", async () => {
    invokeMock.mockImplementation(async (name: string) => {
      if (name === "list_mover_recall_runs") { return [makeRun()]; }
      if (name === "get_mover_recall_run_view") { return makeRunView(); }
      if (name === "list_mover_recall_analysis_batches") { return []; }
      return null;
    });
    renderAt();
    await waitFor(() => expect(screen.getByTestId("mover-batches-never")).toBeTruthy());
    expect(screen.getByTestId("mover-batches-never").textContent ?? "").toContain("从未发起");
  });

  it("F. 有批次 ⇒ 显示最近一批状态；读不到 ⇒ 第三态（不冒充从未发起）", async () => {
    invokeMock.mockImplementation(async (name: string) => {
      if (name === "list_mover_recall_runs") { return [makeRun()]; }
      if (name === "get_mover_recall_run_view") { return makeRunView(); }
      if (name === "list_mover_recall_analysis_batches") {
        return [{ batchId: "mrb-1", status: "success", message: null }];
      }
      return null;
    });
    renderAt();
    await waitFor(() => expect(screen.getByTestId("mover-batches-latest")).toBeTruthy());
    expect(screen.getByTestId("mover-batches-latest").textContent ?? "").toContain("success");
    expect(screen.queryByTestId("mover-batches-never")).toBeNull();
  });

  it("D. 轮次载荷缺字段：走 failed 支，不把半份载荷当视图（既不崩也不静默空白）", async () => {
    invokeMock.mockImplementation(async (name: string) => {
      if (name === "list_mover_recall_runs") { return [makeRun()]; }
      if (name === "get_mover_recall_run_view") { return {}; }
      return null;
    });
    renderAt();
    await waitFor(() => expect(screen.getByTestId("mover-recall-failed")).toBeTruthy());
    expect(screen.queryByTestId("mover-recall-source")).toBeNull();
    expect(screen.queryByTestId("mover-miss-table")).toBeNull();
  });

  it("F. 命令回来说该轮不在库内：走「轮次不在库内」句并退回现场重算，不占用失败支", async () => {
    invokeMock.mockImplementation(async (name: string) => {
      if (name === "list_mover_recall_runs") { return [makeRun()]; }
      if (name === "get_mover_recall_run_view") { return null; }
      if (name === "analyze_mover_recall") { return [makeView()]; }
      return null;
    });
    renderAt();
    await waitFor(() => expect(screen.getByTestId("mover-recall-run-gone")).toBeTruthy());
    expect(screen.getByTestId("mover-recall-run-gone").textContent).toContain("mrr-aaa");
    expect(screen.queryByTestId("mover-recall-failed")).toBeNull();
    expect(cmd("analyze_mover_recall").length).toBe(1);
  });

  it("E. 下拉切到「现场重算」：必须真的发出重算命令", async () => {
    invokeMock.mockImplementation(async (name: string) => {
      if (name === "list_mover_recall_runs") { return [makeRun()]; }
      if (name === "get_mover_recall_run_view") { return makeRunView(); }
      if (name === "analyze_mover_recall") { return [makeView()]; }
      return null;
    });
    renderAt();
    await waitFor(() => expect(cmd("get_mover_recall_run_view").length).toBe(1));
    fireEvent.mouseDown(screen.getByTestId("mover-recall-run-select"));
    fireEvent.click(await screen.findByTitle("现场重算（不落库）"));
    await waitFor(() => expect(cmd("analyze_mover_recall").length).toBe(1));
  });
});
