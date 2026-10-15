// 运行门：监控「未在取数」这类缺席必须各占一句、且只在有证据时出现（PLAN §9 H-4 + §11 裁定 12 ①）。
//
// 回归对象（2026-10-15）：轮询取数失败此前只有逐只 `tracing::warn!`，没有聚合面，
// 整条链路长期全挂时唯一发现方式是人工翻日志，而监控面板仍显示「监控运行中」。
// 现在引擎每轮落 `PollRoundStats`，命令 `monitor_status` 把它连同「装配与否 / 时段档位」
// 一起给出，面板按这三类缺席各出一句。
//
// 两层各测：
// - 纯函数层 `monitorAbsenceSentences` —— 生产侧写下的是哪几个键、参数是什么；
// - 渲染层 —— 组件出的是那几句、缺席时**不**出另一句（只测渲染会放过「读了新键、算了旧格」的半改态）。
// i18next 替身回 key 原文（带参数时 `key|{json}`）⇒ 三态按 key 断言、插值断得到值。

import { act, render, waitFor } from "@testing-library/react";
import { App } from "antd";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: unknown) => opts && typeof opts === "object" ? `${key}|${JSON.stringify(opts)}` : key,
    i18n: { language: "zh-CN" },
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

const invokeMock = vi.fn();

vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: () => Promise.resolve(() => {}),
  isTauri: () => true,
}));

import type { MonitorStatus } from "@/types";
import { monitorAbsenceSentences, monitorSessionKey, PriceAlertPanel } from "../PriceAlertPanel";
import type { MonitorStatusState } from "../PriceAlertPanel";

/** 齐备态基线：引擎在、盘中、本轮全部取数成功、已落过数。任何一条测试只改自己那一格。 */
function status(overrides: Partial<MonitorStatus> = {}): MonitorStatus {
  return {
    assembled: true,
    session: "morning",
    tradingNow: true,
    pollIntervalSecs: 30,
    configsActive: 4,
    roundCodesTotal: 4,
    roundCodesFailed: 0,
    consecutiveAllFailedRounds: 0,
    roundFinishedAtMs: 1_760_000_000_000,
    ...overrides,
  };
}

const ok = (value: MonitorStatus): MonitorStatusState => ({ kind: "ok", value });
const keysOf = (sentences: ReturnType<typeof monitorAbsenceSentences>) => sentences.map((s) => s.key);

describe("monitorAbsenceSentences —— 缺席分句（纯函数层）", () => {
  it("齐备态一条都不出", () => {
    expect(monitorAbsenceSentences(ok(status()))).toEqual([]);
  });

  it("加载中不出句（不把「还没读到」演成「一切正常」之外的第三种误读）", () => {
    expect(monitorAbsenceSentences({ kind: "loading" })).toEqual([]);
  });

  it("读失败只出 readFailed，绝不塌成「未装配」", () => {
    const out = monitorAbsenceSentences({ kind: "readFailed", reason: "boom" });
    expect(keysOf(out)).toEqual(["stockAnalysis.monitor.readFailed"]);
    expect(out[0].params).toEqual({ reason: "boom" });
    expect(out[0].level).toBe("error");
  });

  it("未装配独占一句，且计数句全部闭嘴（那些 0 的含义是「没人在取数」）", () => {
    const out = monitorAbsenceSentences(
      ok(
        status({
          assembled: false,
          tradingNow: false,
          roundCodesFailed: 0,
          roundCodesTotal: 0,
          roundFinishedAtMs: 0,
        }),
      ),
    );
    expect(keysOf(out)).toEqual(["stockAnalysis.monitor.notAssembled"]);
  });

  it("非交易时段出停摆句，并把档位码换成可翻译的 sessionKey", () => {
    const out = monitorAbsenceSentences(ok(status({ tradingNow: false, session: "lunch_break" })));
    expect(keysOf(out)).toEqual(["stockAnalysis.monitor.pausedBySession"]);
    expect(out[0].params).toEqual({
      sessionKey: "stockAnalysis.monitor.sessionLunchBreak",
    });
  });

  it("未见过的档位码落 sessionUnknown，不塞进已知那一档", () => {
    expect(monitorSessionKey("pre_market_auction")).toBe("stockAnalysis.monitor.sessionUnknown");
    expect(monitorSessionKey("morning")).toBe("stockAnalysis.monitor.sessionMorning");
  });

  it("本轮部分失败 ⇒ 只出 x/y 那一句；连击满 1 轮不再重复立一句", () => {
    const out = monitorAbsenceSentences(
      ok(status({ roundCodesFailed: 3, roundCodesTotal: 5, consecutiveAllFailedRounds: 1 })),
    );
    expect(keysOf(out)).toEqual(["stockAnalysis.monitor.fetchFailed"]);
    expect(out[0].params).toEqual({ failed: 3, total: 5 });
  });

  it("连击满两轮才单独立句，且与本轮失败句并存（不收敛成一个数）", () => {
    const out = monitorAbsenceSentences(
      ok(status({ roundCodesFailed: 4, roundCodesTotal: 4, consecutiveAllFailedRounds: 2 })),
    );
    expect(keysOf(out)).toEqual([
      "stockAnalysis.monitor.fetchFailed",
      "stockAnalysis.monitor.allFailedRounds",
    ]);
    expect(out[1].params).toEqual({ rounds: 2 });
    expect(out[1].level).toBe("error");
  });

  it("空配置不得被算成「整轮全失败」：total=0 时只有时段/首轮那类句，没有失败句", () => {
    const out = monitorAbsenceSentences(
      ok(status({ configsActive: 0, roundCodesTotal: 0, roundCodesFailed: 0, roundFinishedAtMs: 0 })),
    );
    expect(keysOf(out)).not.toContain("stockAnalysis.monitor.fetchFailed");
    expect(keysOf(out)).not.toContain("stockAnalysis.monitor.allFailedRounds");
  });

  it("盘中、有启用告警但还没跑完第一轮 ⇒ 出首轮未完成，而不是「0/0 失败」", () => {
    const out = monitorAbsenceSentences(ok(status({ roundFinishedAtMs: 0 })));
    expect(keysOf(out)).toEqual(["stockAnalysis.monitor.firstRoundPending"]);
    expect(out[0].params).toEqual({ interval: 30 });
  });
});

async function renderWith(monitorStatus: unknown) {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "monitor_status") {
      return monitorStatus instanceof Error
        ? Promise.reject(monitorStatus)
        : Promise.resolve(monitorStatus);
    }
    return Promise.resolve([]);
  });
  const view = render(
    <App>
      <PriceAlertPanel />
    </App>,
  );
  await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("monitor_status"));
  await act(async () => {
    await Promise.resolve();
  });
  return view;
}

describe("PriceAlertPanel —— 缺席句上屏（渲染层）", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    document.body.innerHTML = "";
  });

  it("未装配时只出「未装配」那一句，不出现任何计数句", async () => {
    await renderWith(
      status({
        assembled: false,
        tradingNow: false,
        roundCodesTotal: 0,
        roundCodesFailed: 0,
        consecutiveAllFailedRounds: 0,
        roundFinishedAtMs: 0,
      }),
    );
    const text = document.body.textContent ?? "";
    expect(text).toContain("stockAnalysis.monitor.notAssembled");
    expect(text).not.toContain("stockAnalysis.monitor.fetchFailed");
    expect(text).not.toContain("stockAnalysis.monitor.allFailedRounds");
    expect(text).not.toContain("stockAnalysis.monitor.pausedBySession");
    expect(text).not.toContain("stockAnalysis.monitor.firstRoundPending");
  });

  it("本轮 3/5 取数失败 ⇒ 句子把两个数都带出来", async () => {
    await renderWith(status({ roundCodesFailed: 3, roundCodesTotal: 5 }));
    const text = document.body.textContent ?? "";
    expect(text).toContain("stockAnalysis.monitor.fetchFailed");
    expect(text).toContain('"failed":3');
    expect(text).toContain('"total":5');
  });

  it("连击四轮 ⇒ 本轮句与连击句各占一条，不并成一个数", async () => {
    await renderWith(
      status({ roundCodesFailed: 4, roundCodesTotal: 4, consecutiveAllFailedRounds: 4 }),
    );
    const text = document.body.textContent ?? "";
    expect(text).toContain("stockAnalysis.monitor.fetchFailed");
    expect(text).toContain("stockAnalysis.monitor.allFailedRounds|{");
    expect(text).toContain('"rounds":4');
  });

  it("午休时段停摆 ⇒ 句里点出是哪一个时段", async () => {
    await renderWith(status({ tradingNow: false, session: "lunch_break" }));
    const text = document.body.textContent ?? "";
    expect(text).toContain("stockAnalysis.monitor.pausedBySession");
    expect(text).toContain('"session":"stockAnalysis.monitor.sessionLunchBreak"');
  });

  it("monitor_status 抛错 ⇒ 报「读取失败」并带原因，不报「未装配」", async () => {
    await renderWith(new Error("ipc down"));
    const text = document.body.textContent ?? "";
    expect(text).toContain("stockAnalysis.monitor.readFailed");
    expect(text).toContain('"reason":"ipc down"');
    expect(text).not.toContain("stockAnalysis.monitor.notAssembled");
  });

  it("命令回空（mock 或旧后端）⇒ 报「后端未返回读数」，不静默也不报未装配", async () => {
    await renderWith(undefined);
    const text = document.body.textContent ?? "";
    expect(text).toContain("stockAnalysis.monitor.readFailed");
    expect(text).toContain("stockAnalysis.monitor.emptyResponse");
    expect(text).not.toContain("stockAnalysis.monitor.notAssembled");
  });

  it("齐备态 ⇒ 一条缺席句都不出", async () => {
    await renderWith(status());
    const text = document.body.textContent ?? "";
    for (
      const key of [
        "readFailed",
        "notAssembled",
        "pausedBySession",
        "fetchFailed",
        "allFailedRounds",
        "firstRoundPending",
      ]
    ) {
      expect(text).not.toContain(`stockAnalysis.monitor.${key}`);
    }
  });
});
