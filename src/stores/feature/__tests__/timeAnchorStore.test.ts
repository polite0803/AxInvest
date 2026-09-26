import { summarizeDegradations, timeAnchorHelpers, useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { isValidPastDate, todayIso, DATE_RE } = timeAnchorHelpers;

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
}));

// enterReplay 会启动降级轮询并真实 invoke；不 mock 的话失败分支的 console.warn
// 会在 worker RPC teardown 之后才 flush，触发 EnvironmentTeardownError
vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
  listen: vi.fn(() => Promise.resolve(() => {})),
  isTauri: () => false,
  logIpcError: vi.fn(() => vi.fn()),
}));

beforeEach(() => {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "get_asof_degradation_log") { return Promise.resolve([]); }
    if (cmd === "get_asof_degradation_count") { return Promise.resolve(0); }
    return Promise.resolve(undefined);
  });
  // 静默 console.warn，避免 vitest RPC teardown 时序竞争
  vi.spyOn(console, "warn").mockImplementation(() => {});
  // 重置 store + 清除 localStorage 持久化
  useTimeAnchorStore.setState({
    asOfDate: null,
    mode: "live",
    tourSeen: false,
    pendingLiveConfirm: false,
  });
  if (typeof localStorage !== "undefined") {
    localStorage.removeItem("axagent-time-anchor");
  }
});

afterEach(() => {
  useTimeAnchorStore.getState().stopDegradationPolling();
  vi.restoreAllMocks();
  if (typeof localStorage !== "undefined") {
    localStorage.removeItem("axagent-time-anchor");
  }
});

describe("timeAnchorHelpers", () => {
  it("DATE_RE matches YYYY-MM-DD only", () => {
    expect(DATE_RE.test("2026-06-01")).toBe(true);
    expect(DATE_RE.test("2026/06/01")).toBe(false);
    expect(DATE_RE.test("2026-6-1")).toBe(false);
    expect(DATE_RE.test("garbage")).toBe(false);
  });

  it("isValidPastDate accepts past date", () => {
    // 用一个 30 天前的日期
    const past = new Date();
    past.setDate(past.getDate() - 30);
    const s = past.toISOString().slice(0, 10);
    expect(isValidPastDate(s)).toBe(true);
  });

  it("isValidPastDate accepts today", () => {
    const today = todayIso();
    // today <= today 为 true（包括等于），允许用户选择当天
    expect(isValidPastDate(today)).toBe(true);
  });

  it("isValidPastDate rejects future date", () => {
    const future = new Date();
    future.setDate(future.getDate() + 30);
    const s = future.toISOString().slice(0, 10);
    expect(isValidPastDate(s)).toBe(false);
  });

  it("isValidPastDate rejects malformed input", () => {
    expect(isValidPastDate("not-a-date")).toBe(false);
    expect(isValidPastDate("")).toBe(false);
    expect(isValidPastDate("2026-13-01")).toBe(false);
  });
});

describe("useTimeAnchorStore — transitions", () => {
  it("starts in live mode with null asOfDate", () => {
    const s = useTimeAnchorStore.getState();
    expect(s.asOfDate).toBeNull();
    expect(s.mode).toBe("live");
    expect(s.tourSeen).toBe(false);
  });

  it("enterReplay sets asOfDate and mode=replay", () => {
    const past = new Date();
    past.setDate(past.getDate() - 7);
    const d = past.toISOString().slice(0, 10);
    useTimeAnchorStore.getState().enterReplay(d);
    const s = useTimeAnchorStore.getState();
    expect(s.asOfDate).toBe(d);
    expect(s.mode).toBe("replay");
    expect(s.pendingLiveConfirm).toBe(false);
  });

  it("enterReplay rejects future date (no state change)", () => {
    useTimeAnchorStore.getState().enterReplay("2099-01-01");
    expect(useTimeAnchorStore.getState().asOfDate).toBeNull();
    expect(useTimeAnchorStore.getState().mode).toBe("live");
  });

  it("enterLive from replay requires confirmation when requireConfirm=true", () => {
    const past = new Date();
    past.setDate(past.getDate() - 7);
    const d = past.toISOString().slice(0, 10);
    useTimeAnchorStore.getState().enterReplay(d);
    const ok = useTimeAnchorStore.getState().enterLive(true);
    expect(ok).toBe(false);
    expect(useTimeAnchorStore.getState().pendingLiveConfirm).toBe(true);
    expect(useTimeAnchorStore.getState().mode).toBe("replay");
  });

  it("confirmPendingLive transitions back to live", () => {
    const past = new Date();
    past.setDate(past.getDate() - 7);
    const d = past.toISOString().slice(0, 10);
    useTimeAnchorStore.getState().enterReplay(d);
    useTimeAnchorStore.getState().enterLive(true);
    useTimeAnchorStore.getState().confirmPendingLive();
    const s = useTimeAnchorStore.getState();
    expect(s.asOfDate).toBeNull();
    expect(s.mode).toBe("live");
    expect(s.pendingLiveConfirm).toBe(false);
  });

  it("cancelPendingLive keeps replay mode", () => {
    const past = new Date();
    past.setDate(past.getDate() - 7);
    const d = past.toISOString().slice(0, 10);
    useTimeAnchorStore.getState().enterReplay(d);
    useTimeAnchorStore.getState().enterLive(true);
    useTimeAnchorStore.getState().cancelPendingLive();
    const s = useTimeAnchorStore.getState();
    expect(s.mode).toBe("replay");
    expect(s.pendingLiveConfirm).toBe(false);
  });

  it("enterReplayWorkbench forces override (does not inherit from live)", () => {
    // 先 live
    expect(useTimeAnchorStore.getState().mode).toBe("live");
    const past = new Date();
    past.setDate(past.getDate() - 14);
    const d = past.toISOString().slice(0, 10);
    useTimeAnchorStore.getState().enterReplayWorkbench(d);
    const s = useTimeAnchorStore.getState();
    expect(s.asOfDate).toBe(d);
    expect(s.mode).toBe("replay");
  });

  it("enterBacktestSweep sets mode=backtest_sweep", () => {
    const past = new Date();
    past.setDate(past.getDate() - 7);
    const d = past.toISOString().slice(0, 10);
    useTimeAnchorStore.getState().enterBacktestSweep(d);
    const s = useTimeAnchorStore.getState();
    expect(s.asOfDate).toBe(d);
    expect(s.mode).toBe("backtest_sweep");
  });

  it("markTourSeen persists", () => {
    useTimeAnchorStore.getState().markTourSeen();
    expect(useTimeAnchorStore.getState().tourSeen).toBe(true);
  });

  it("setAsOfDate(null) goes back to live", () => {
    const past = new Date();
    past.setDate(past.getDate() - 7);
    const d = past.toISOString().slice(0, 10);
    useTimeAnchorStore.getState().enterReplay(d);
    useTimeAnchorStore.getState().setAsOfDate(null);
    const s = useTimeAnchorStore.getState();
    expect(s.asOfDate).toBeNull();
    expect(s.mode).toBe("live");
  });

  it("setAsOfDate rejects future date (no state change)", () => {
    useTimeAnchorStore.getState().setAsOfDate("2099-01-01");
    expect(useTimeAnchorStore.getState().asOfDate).toBeNull();
    expect(useTimeAnchorStore.getState().mode).toBe("live");
  });

  // P2-7: enterReplay 必须重置本地降级显示,避免上次 replay 残留
  it("enterReplay resets degradation state (count and log to 0)", () => {
    // 假装有残留降级
    useTimeAnchorStore.setState({
      degradationCount: 5,
      degradationLog: [
        { vendor: "old", method: "old_method", reason: "stale", as_of: "2026-01-01", kind: "failure" },
      ],
    });
    const past = new Date();
    past.setDate(past.getDate() - 7);
    const d = past.toISOString().slice(0, 10);
    useTimeAnchorStore.getState().enterReplay(d);
    const s = useTimeAnchorStore.getState();
    expect(s.degradationCount).toBe(0);
    expect(s.degradationLog).toEqual([]);
  });
});

// ────────────────────────────────────────────────────────────
// T14（2026-09-27）：降级面板按严重度分档
// 缺陷形态：`个股没有场内期权` 与 `接口 301 挂了` 在面板上同一种权重，
// 用户只能读出「还是没好」，读不出哪条值得去修。
// ────────────────────────────────────────────────────────────
describe("summarizeDegradations - 严重度分档", () => {
  const e = (
    method: string,
    kind?: string,
  ) => ({ vendor: "astock-data", method, reason: `${method} 的原因`, as_of: "2026-09-11", kind } as never);

  it("配色取最重的一档：混有真故障时不得退回中性", () => {
    const s = summarizeDegradations([e("get_hot_stocks", "structuralGap"), e("get_money_flow", "failure")]);
    expect(s.worst).toBe("failure");
    expect(s.counts).toEqual({ failure: 1, noData: 0, structuralGap: 1 });
    expect(s.grouped.map((g) => g.kind)).toEqual(["failure", "structuralGap"]);
  });

  it("全是结构性不适用 ⇒ worst 退到 structuralGap（面板不再橙色示警）", () => {
    const s = summarizeDegradations([e("get_option_pcr", "structuralGap"), e("get_social_sentiment", "structuralGap")]);
    expect(s.worst).toBe("structuralGap");
    expect(s.total).toBe(2);
  });

  it("后端旧二进制没发 kind ⇒ 按真故障算，宁可多标红也不洗白", () => {
    const s = summarizeDegradations([e("get_news", undefined)]);
    expect(s.counts.failure).toBe(1);
    expect(s.worst).toBe("failure");
  });
});
