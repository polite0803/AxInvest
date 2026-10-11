import i18n from "@/i18n";
import { useSerenityStore } from "@/stores/feature/serenityStore";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { MemoryRouter } from "react-router-dom";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { SerenityScreeningPanel } from "../SerenityScreeningPanel";

/**
 * 趋势智选「本轮取数口径」呈现层的防回归测试（2026-10-11）。
 *
 * 背景：serenity 命令此前收下 `as_of_date` 却只用裸 `AS_OF.scope` 包住 `run_workflow`，
 * 而引擎为每个节点单独 spawn ⇒ 节点内读不到截止日，回放静默跑 live；UI 上既没有
 * 实时/回放区分，也没有本轮降级归因（`docs/plans` 里的 H 轮同型缺陷在智选链未收编）。
 *
 * 本测试锁三件事：
 *   ① 口径显示绑**本轮 payload 的 `asOfDate`**，而不是渲染时读全局时间锚点 ——
 *     T11 的复发形态就是「按 as-of 取数却显示实时分析」（此处反向：按实时却显示回放）；
 *   ② 本轮降级归因用后端 `reason` 原文逐行呈现，前端不重写措辞；
 *   ③ 新一轮运行时上一轮的回放横幅必须消失（残留会把上一轮的口径挂到这一轮）。
 */
const { invokeMock, listenHandlers } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenHandlers: new Map<string, (event: { payload: unknown }) => void>(),
}));
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: (name: string, handler: (event: { payload: unknown }) => void) => {
    listenHandlers.set(name, handler);
    return Promise.resolve(() => {});
  },
  isTauri: () => false,
  TimeoutError: class MockTimeoutError extends Error {},
}));

const REPLAY_DATE = "2026-07-21";
const BANNER_TEXT = `数据截至 ${REPLAY_DATE}`;
const NOT_PERSISTED_TEXT = "回放结果不写入趋势智选历史，仅本轮展示";
const HOT_REASON = "热门股榜无 as-of 通道，且当日快照未命中";

async function renderPanel() {
  const result = render(
    <MemoryRouter>
      <I18nextProvider i18n={i18n}>
        <SerenityScreeningPanel />
      </I18nextProvider>
    </MemoryRouter>,
  );
  await act(async () => {});
  return result;
}

function emitCompleted(payload: Record<string, unknown>) {
  const handler = listenHandlers.get("serenity-screening-completed");
  expect(handler).toBeTruthy();
  act(() => {
    handler!({ payload });
  });
}

beforeAll(async () => {
  await i18n.changeLanguage("zh-CN");
});

beforeEach(() => {
  invokeMock.mockReset();
  // 按命令名分派：面板挂载会拉「上一次运行」历史，未识别的命令回 null，
  // 否则同一个返回值串到无关命令上会在测试里造出真·报错噪音。
  invokeMock.mockImplementation(() => Promise.resolve(null));
  useSerenityStore.setState({
    steps: [],
    running: false,
    error: null,
    errorDetail: null,
    candidates: [],
    trends: [],
    emptyReason: null,
    asofDegradations: [],
    lastRunAsOfDate: null,
  });
  useTimeAnchorStore.setState({ mode: "live", asOfDate: null });
});

describe("SerenityScreeningPanel · 本轮取数口径（payload 绑定，不读全局锚点）", () => {
  it("回放轮：payload 带 asOfDate ⇒ 出横幅 + 不落库声明 + 后端归因逐行", async () => {
    await renderPanel();
    emitCompleted({
      status: "completed",
      candidates: [],
      asOfDate: REPLAY_DATE,
      asofDegradations: [
        { vendor: "eastmoney", method: "get_hot_stocks", reason: HOT_REASON, kind: "structuralGap" },
      ],
    });

    expect(screen.getByText(BANNER_TEXT)).toBeTruthy();
    expect(screen.getByText(NOT_PERSISTED_TEXT)).toBeTruthy();
    // 归因用后端原文，前端不另写一套措辞
    expect(screen.getByText(HOT_REASON)).toBeTruthy();
  });

  it("实盘轮（payload 无 asOfDate）⇒ 不出横幅，也不写 store", async () => {
    await renderPanel();
    emitCompleted({ status: "completed", candidates: [] });

    expect(screen.queryByText(BANNER_TEXT)).toBeNull();
    expect(screen.queryByText(NOT_PERSISTED_TEXT)).toBeNull();
    expect(useSerenityStore.getState().lastRunAsOfDate).toBeNull();
  });

  it("负控：全局锚点处于 replay，但本轮 payload 没带日期 ⇒ 不得凭空显示回放口径", async () => {
    // 这条锁的正是 T11 的反向形态：渲染层若改成读 useTimeAnchorStore，本用例会变绿/红错位
    useTimeAnchorStore.setState({ mode: "replay", asOfDate: REPLAY_DATE });
    await renderPanel();
    emitCompleted({ status: "completed", candidates: [] });

    expect(screen.queryByText(BANNER_TEXT)).toBeNull();
    expect(useSerenityStore.getState().lastRunAsOfDate).toBeNull();
  });

  it("负控：上一轮是回放、这一轮是实盘 ⇒ 横幅必须随本轮 payload 消失", async () => {
    await renderPanel();
    emitCompleted({ status: "completed", candidates: [], asOfDate: REPLAY_DATE });
    expect(screen.getByText(BANNER_TEXT)).toBeTruthy();

    emitCompleted({ status: "completed", candidates: [] });
    expect(screen.queryByText(BANNER_TEXT)).toBeNull();
    expect(useSerenityStore.getState().asofDegradations).toEqual([]);
  });

  it("invoke 兜底路径与事件路径同源：只有返回值（completed 事件未到）时也要出横幅", async () => {
    invokeMock.mockImplementation((name: unknown) =>
      Promise.resolve(
        name === "run_serenity_screening"
          ? {
            status: "completed",
            candidates: [],
            asOfDate: REPLAY_DATE,
            asofDegradations: [],
          }
          : null,
      )
    );
    await renderPanel();

    // 真跑 handleRun（不手工 setState）：这样锁的是「兜底路径确实写了本轮口径」这个事实
    await act(async () => {
      fireEvent.click(screen.getByText("运行 Serenity 筛选"));
    });

    expect(screen.getByText(BANNER_TEXT)).toBeTruthy();
    expect(useSerenityStore.getState().lastRunAsOfDate).toBe(REPLAY_DATE);
  });
});
