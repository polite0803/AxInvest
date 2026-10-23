import i18n from "@/i18n";
import { act, render, screen, waitFor } from "@testing-library/react";
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

/** 阶段 0 的四个计数域：申报总数 / 已枚举 / clist 确认 / 最新收盘票数。 */
const COVERAGE = {
  universeSize: 5804,
  confirmedByClist: 1180,
  reportedTotal: 5925,
  latestCloseCodes: 2690,
  closeDates: ["2026-10-21", "2026-10-22"],
  rowsPerDate: [2600, 2690],
};

const VIEW = {
  chain: "reco",
  from: "2026-09-30",
  to: "2026-10-22",
  dataSince: "2026-07-01",
  collectedDays: 90,
  universeSize: 5804,
  universeConfirmed: 1180,
  rules: [
    { period: "ultra_short", gainPct: 10, windowDays: 2, varName: "mover_gain_ultra_short" },
    { period: "short", gainPct: 20, windowDays: 5, varName: "mover_gain_short" },
    { period: "mid", gainPct: 30, windowDays: 28, varName: "mover_gain_mid" },
    { period: "long", gainPct: 40, windowDays: 90, varName: "mover_gain_long" },
  ],
  rates: {
    reachability: null,
    coverage: null,
    unexplainedShare: null,
    events: 0,
    misses: 0,
    judgeableEvents: null,
  },
  codesWithClose: 2783,
  judgeableCodesByTier: null,
  gateVersion: "mover-layers-v2",
  eventsByTier: {},
  layers: [],
  misses: [],
};

function wire(historyReport: unknown) {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "backfill_market_close_history") {
      return Promise.resolve(historyReport);
    }
    if (cmd === "get_market_coverage") {
      return Promise.resolve(COVERAGE);
    }
    return Promise.resolve([VIEW]);
  });
}

/**
 * 只取最后一条 toast：`.ant-message` 是全局容器，上一条测试的 toast 还在场时读整个容器
 * 会把两句拼在一起（`message.destroy()` 在 jsdom 下并不真的摘掉节点）。
 * 每条断言判自己那一条，才拆得开三种缺席。
 */
const lastToast = (): string => {
  const notices = document.querySelectorAll(".ant-message-notice");
  if (notices.length === 0) {
    return "";
  }
  return notices[notices.length - 1].textContent ?? "";
};

const clickBackfill = async () => {
  const btn = screen.getByRole("button", { name: "补窗口历史" });
  await act(async () => {
    btn.click();
  });
};

beforeEach(() => {
  invokeMock.mockReset();
});

/**
 * 阶段 0 的三个采集动作里，「补窗口历史」的结果有三种互不相等的缺席，各自成句：
 * 有新增 / 零新增（可用日线不足 90 根）/ 写库失败。把失败或零进展说成补齐，
 * 用户就会停止点击，样本停在半覆盖状态（2026-10-23 现网「500 只 / 0 行 / 失败 500」即此）。
 */
describe("MoverRecallPanel 全 A 股样本动作", () => {
  it("零新增票要点名：句子里给出「无新增 N」，不得冒充「本批已齐」", async () => {
    wire({
      requested: 500,
      codes: 500,
      rows: 0,
      noNew: 500,
      failed: 0,
      targetDays: 90,
      error: null,
    });
    render(
      <I18nextProvider i18n={i18n}>
        <MemoryRouter>
          <MoverRecallPanel />
        </MemoryRouter>
      </I18nextProvider>,
    );
    await clickBackfill();
    await waitFor(() => expect(lastToast()).toContain("无新增 500"));
    expect(lastToast()).not.toContain("本批历史已齐");
  });

  it("写库失败 ⇒ 报后端首条原因，不得显示成补齐成功", async () => {
    wire({
      requested: 500,
      codes: 500,
      rows: 0,
      noNew: 0,
      failed: 500,
      targetDays: 90,
      error: "001367: 落库失败: None of the records are inserted",
    });
    render(
      <I18nextProvider i18n={i18n}>
        <MemoryRouter>
          <MoverRecallPanel />
        </MemoryRouter>
      </I18nextProvider>,
    );
    await clickBackfill();
    await waitFor(() => expect(lastToast()).toContain("001367"));
    expect(lastToast()).not.toContain("历史补齐");
  });

  it("无可补代码（requested=0）才是「本批历史已齐」", async () => {
    wire({
      requested: 0,
      codes: 0,
      rows: 0,
      noNew: 0,
      failed: 0,
      targetDays: 90,
      error: null,
    });
    render(
      <I18nextProvider i18n={i18n}>
        <MemoryRouter>
          <MoverRecallPanel />
        </MemoryRouter>
      </I18nextProvider>,
    );
    await clickBackfill();
    await waitFor(() => expect(lastToast()).toContain("本批历史已齐"));
  });

  it("命令没返回报告（浏览器模式 mock 回 undefined）⇒ 报「没返回字段」，不得读成已齐", async () => {
    wire(undefined);
    render(
      <I18nextProvider i18n={i18n}>
        <MemoryRouter>
          <MoverRecallPanel />
        </MemoryRouter>
      </I18nextProvider>,
    );
    await clickBackfill();
    await waitFor(() => expect(lastToast()).toContain("returned no numeric"));
    expect(lastToast()).not.toContain("本批历史已齐");
  });
});
