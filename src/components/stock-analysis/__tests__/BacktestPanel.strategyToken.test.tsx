import { act, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    // 不返回 options 对象,只回 key(避免把 {count: N} 渲染为 React 子节点)
    t: (key: string) => key,
  }),
}));

const timeAnchorState: Record<string, unknown> = {
  asOfDate: null,
  mode: "live",
};
vi.mock("@/stores/feature/timeAnchorStore", () => ({
  useTimeAnchorStore: (selector: (s: typeof timeAnchorState) => unknown) => selector(timeAnchorState),
}));

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

import { BacktestPanel } from "../BacktestPanel";

/**
 * 收尾：等挂载副作用（数据请求）落地。
 *
 * 为什么必须有：页面挂载后会异步拉数据，测试同步结束 ⇒ setState 落在测试之外 ⇒
 * React 报 "not wrapped in act"。为什么必须放在**断言之后**：这些用例断言的是
 * **初始渲染态**，提前 flush 会把组件推进"加载完成"态，断言随之失效（实测会让 3 条用例变红）。
 * 为什么不能用 afterEach：React 是在 setState 那一刻检查的，而那一刻在测试函数返回后、
 * afterEach 之前 —— 收尾钩子永远太晚。
 */
const settle = async () => {
  await act(async () => {});
};

beforeEach(() => {
  invokeMock.mockReset();
});

afterEach(() => {
  timeAnchorState.asOfDate = null;
  timeAnchorState.mode = "live";
});

/** 构造一条终态任务行（hook 只读 status / output / resultJson）。 */
function taskRow(report: unknown) {
  return {
    id: "x",
    title: "t",
    description: "",
    taskType: "reco-strategy-backtest",
    status: "completed",
    output: "done",
    resultJson: JSON.stringify(report),
    attempt: 0,
    createdAt: 1,
    updatedAt: 2,
    finishedAt: 2,
  };
}

describe("BacktestPanel — 异步任务归属：旧任务的报告不得覆盖新任务", () => {
  it("快速双击后,即使先发的任务报告后到,UI 仍只展示后发任务的结果", async () => {
    // 每次任务受理回执给一个新 taskId；报告经 get_background_task_output 由测试控制何时返回
    let strategyCallIndex = 0;
    const rowResolvers: Record<string, (row: unknown) => void> = {};
    invokeMock.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "backtest_reco_strategies") {
        strategyCallIndex += 1;
        return { taskId: `task-${strategyCallIndex}` };
      }
      if (cmd === "get_background_task_output") {
        const id = String(args?.taskId ?? "");
        return new Promise((resolve) => {
          rowResolvers[id] = resolve;
        });
      }
      return null;
    });

    const { container } = render(<BacktestPanel />);

    // 等首次 backtest_all_history 触发完成
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    invokeMock.mockClear();
    strategyCallIndex = 0;

    // 找到策略回测按钮并双击（antd loading 会替换内部 DOM,每次点击前重新查询）
    const findRunBtn = () => {
      const btns = Array.from(container.querySelectorAll("button"));
      const btn = btns.find((b) => {
        const t = b.textContent || "";
        return t.includes("stockAnalysis.backtest.strategyRun")
          || t.includes("stockAnalysis.backtest.strategyRunning");
      });
      if (!btn) { throw new Error("策略回测按钮未找到"); }
      return btn as HTMLElement;
    };
    // 两次点击在同一个 act 同步作用域里：不等待 React 提交，模拟用户快速双击
    const btn1 = findRunBtn();
    const btn2 = findRunBtn();
    act(() => {
      btn1.click();
      btn2.click();
    });

    // 两次都发起了任务（第二次受理会重置轨道，但请求本身照发）
    await waitFor(() => {
      const strategyCalls = invokeMock.mock.calls.filter((c) => c[0] === "backtest_reco_strategies");
      expect(strategyCalls.length).toBe(2);
    });

    // 后发任务（task-2）先到终态 ⇒ UI 展示 2nd
    await act(async () => {
      rowResolvers["task-2"]?.(
        taskRow({
          positive: { label: "正向(2nd)", stockCount: 99, strategies: {} },
          negative: { label: "负向(2nd)", stockCount: 88, strategies: {} },
          skipped: [],
        }),
      );
    });
    await waitFor(() => {
      expect(container.textContent).toContain("正向(2nd)");
    });

    // 先发任务（task-1）的报告后到 ⇒ 必须被归属判断丢弃（taskId 已不是它）
    await act(async () => {
      rowResolvers["task-1"]?.(
        taskRow({
          positive: { label: "正向(1st)", stockCount: 1, strategies: {} },
          negative: { label: "负向(1st)", stockCount: 2, strategies: {} },
          skipped: [],
        }),
      );
    });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });

    expect(container.textContent).toContain("正向(2nd)");
    expect(container.textContent).not.toContain("正向(1st)");
    await settle();
  });
});
