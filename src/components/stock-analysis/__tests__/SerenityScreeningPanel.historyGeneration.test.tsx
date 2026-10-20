import i18n from "@/i18n";
import { useSerenityStore } from "@/stores/feature/serenityStore";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { App } from "antd";
import { I18nextProvider } from "react-i18next";
import { MemoryRouter } from "react-router-dom";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { SerenityScreeningPanel } from "../SerenityScreeningPanel";

/**
 * 趋势智选「历史」弹窗的代际切换（2026-10-20）。
 *
 * 背景：2026-10-12 裁定退役了「回放跳过写库」⇒ 回放候选照落 `reco_picks`（带 `as_of_date`），
 * 但 `list_reco_history` 不传 `generation` 时后端只给实盘代。于是回放批次在库里、却在界面上
 * 无处可看 —— 本测试锁这条接线：
 *   ① 打开弹窗默认按实盘代查（`generation: null`，与后端默认口径一致）；
 *   ② 切到回放代 ⇒ 把**解析出的截止日**作为 `generation` 传下去，标签上显示同一个日期；
 *   ③ 本轮与时间锚点都没有截止日 ⇒ 回放代禁用，并单独说一句为什么禁用（不静默少一格）；
 *   ④ 空态按代际分两句：实盘代没历史 ≠ 该截止日没回放批次；
 *   ⑤ 切换代际清空多选 —— 跨代残留的勾选会删错批次。
 */
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: () => Promise.resolve(() => {}),
  isTauri: () => false,
  TimeoutError: class MockTimeoutError extends Error {},
}));

const REPLAY_DATE = "2026-07-21";
const LIVE_ROW = { generatedAt: "2026-08-01T09:30:00", stockCount: 3, createdAt: "2026-08-01", styles: "serenity" };
const GEN_LABEL = "代际";
const LIVE_GEN_LABEL = "实盘代";
const REPLAY_GEN_LABEL = `回放代 · 截至 ${REPLAY_DATE}`;
const NO_CUTOFF_LABEL = "回放代（无截止日）";
const NO_CUTOFF_HINT = "本轮与时间锚点都没有截止日 ⇒ 库里没有可定位的回放代";
const EMPTY_LIVE =
  "趋势智选历史为空：list_reco_history 返回 0 行（可能是 DB 暂无风格为 serenity/bottleneck 的历史，或后端命令失败被静默）";
const EMPTY_REPLAY = `该截止日（${REPLAY_DATE}）没有回放批次：list_reco_history 在代际过滤下返回 0 行`;

async function renderPanel() {
  const result = render(
    <MemoryRouter>
      <I18nextProvider i18n={i18n}>
        <App>
          <SerenityScreeningPanel />
        </App>
      </I18nextProvider>
    </MemoryRouter>,
  );
  await act(async () => {});
  return result;
}

/** 取弹窗那次调用（挂载恢复走的是 limit:1，按 styleFilter 区分）。 */
function historyCalls(): Array<[unknown, unknown]> {
  return invokeMock.mock.calls.filter(
    ([, args]) => (args as { styleFilter?: string })?.styleFilter === "serenity,bottleneck",
  ) as Array<[unknown, unknown]>;
}

async function openHistory() {
  await act(async () => {
    fireEvent.click(screen.getByText("查看趋势智选历史"));
  });
  await act(async () => {});
}

beforeAll(async () => {
  await i18n.changeLanguage("zh-CN");
});

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation((_name: unknown, args: unknown) =>
    Promise.resolve(
      (args as { styleFilter?: string })?.styleFilter === "serenity,bottleneck" ? [LIVE_ROW] : null,
    )
  );
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

describe("SerenityScreeningPanel · 历史弹窗代际切换", () => {
  it("① 打开弹窗默认按实盘代查，generation 显式传 null", async () => {
    await renderPanel();
    await openHistory();

    expect(screen.getByText(GEN_LABEL)).toBeTruthy();
    expect(screen.getByText(LIVE_GEN_LABEL)).toBeTruthy();
    const calls = historyCalls();
    expect(calls.length).toBe(1);
    expect(calls[0][0]).toBe("list_reco_history");
    expect(calls[0][1]).toEqual({ styleFilter: "serenity,bottleneck", limit: 50, generation: null });
  });

  it("② 本轮是回放 ⇒ 标签带截止日，点它把同一日期传给后端", async () => {
    useSerenityStore.setState({ lastRunAsOfDate: REPLAY_DATE });
    await renderPanel();
    await openHistory();

    const option = screen.getByText(REPLAY_GEN_LABEL);
    expect(option).toBeTruthy();
    await act(async () => {
      fireEvent.click(option);
    });

    const calls = historyCalls();
    expect(calls.length).toBe(2);
    expect(calls[1][1]).toEqual({ styleFilter: "serenity,bottleneck", limit: 50, generation: REPLAY_DATE });
  });

  it("③ 无截止日 ⇒ 回放代禁用且单独说一句为什么（不静默少一格）", async () => {
    await renderPanel();
    await openHistory();

    expect(screen.getByText(NO_CUTOFF_LABEL)).toBeTruthy();
    expect(screen.getByText(NO_CUTOFF_HINT)).toBeTruthy();
    expect(screen.queryByText(REPLAY_GEN_LABEL)).toBeNull();
    // 禁用态下点它不得发出第二次查询
    await act(async () => {
      fireEvent.click(screen.getByText(NO_CUTOFF_LABEL));
    });
    expect(historyCalls().length).toBe(1);
  });

  it("④ 空态分两代各说一句", async () => {
    invokeMock.mockImplementation((_name: unknown, args: unknown) =>
      Promise.resolve(
        (args as { styleFilter?: string })?.styleFilter === "serenity,bottleneck" ? [] : null,
      )
    );
    useSerenityStore.setState({ lastRunAsOfDate: REPLAY_DATE });
    await renderPanel();
    await openHistory();
    expect(await screen.findByText(EMPTY_LIVE)).toBeTruthy();

    await act(async () => {
      fireEvent.click(screen.getByText(REPLAY_GEN_LABEL));
    });
    expect(await screen.findByText(EMPTY_REPLAY)).toBeTruthy();
  });

  it("⑤ 切代时清掉上一代的多选，避免跨代删错批次", async () => {
    useSerenityStore.setState({ lastRunAsOfDate: REPLAY_DATE });
    await renderPanel();
    await openHistory();

    // 勾一行 ⇒ footer 出现「已选 1 项」
    const rowChecks = screen.getAllByRole("checkbox");
    await act(async () => {
      fireEvent.click(rowChecks[rowChecks.length - 1]);
    });
    expect(screen.getByText("已选 1 项")).toBeTruthy();

    await act(async () => {
      fireEvent.click(screen.getByText(REPLAY_GEN_LABEL));
    });
    expect(screen.queryByText("已选 1 项")).toBeNull();
  });

  it("负控：全局锚点在 replay 但本轮是实盘 ⇒ 仍按锚点日期给出可切换的回放代", async () => {
    // 这一格查的是库里已有的批次，与横幅那条「只读本轮 payload」判据不是同一面
    useTimeAnchorStore.setState({ mode: "replay", asOfDate: REPLAY_DATE });
    await renderPanel();
    await openHistory();

    await act(async () => {
      fireEvent.click(screen.getByText(REPLAY_GEN_LABEL));
    });
    expect(historyCalls().at(-1)?.[1]).toEqual({
      styleFilter: "serenity,bottleneck",
      limit: 50,
      generation: REPLAY_DATE,
    });
  });
});
