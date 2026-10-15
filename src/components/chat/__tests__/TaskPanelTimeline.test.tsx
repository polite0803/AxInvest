import { act, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("react-i18next", () => ({
  // 返回 key 本身，断言命中了哪个 key
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) => opts ? `${key}|${JSON.stringify(opts)}` : key,
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

const invokeMock = vi.fn();

vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => true,
  logIpcError: () => () => {},
}));

import { TaskPanel } from "../TaskPanel";

/**
 * 后端 `BackgroundTaskInfo` 的实际 IPC 载荷（`#[serde(rename_all = "camelCase")]`）。
 * 这是契约样本 —— 若前端 interface 退回 snake_case，本组测试会失败。
 */
function makeTask(over: Partial<Record<string, unknown>> = {}) {
  return {
    id: "t1",
    title: "构建",
    description: "",
    taskType: "bash",
    command: "npm run build",
    prompt: null,
    status: "running",
    output: "ok",
    exitCode: 0,
    conversationId: null,
    idempotencyKey: null,
    attempt: 0,
    resumeFrom: null,
    createdAt: 1_700_000_000_000,
    updatedAt: 1_700_000_000_000,
    finishedAt: null,
    ...over,
  };
}

function mockTasks(tasks: unknown[]) {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "list_background_tasks") { return Promise.resolve(tasks); }
    if (cmd === "list_task_events") { return Promise.resolve([]); }
    return Promise.resolve(undefined);
  });
}

beforeEach(() => {
  invokeMock.mockReset();
});

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

describe("TaskPanel 字段契约与状态诚实性", () => {
  it("按 camelCase 消费后端字段：taskType 命中本地化标签", async () => {
    mockTasks([makeTask({ taskType: "bash" })]);
    render(<TaskPanel />);

    // task.bash 命中 = task.taskType 取到了值。
    // 若 interface 写回 snake_case（task_type），此处取到 undefined，
    // 标签会渲染成空字符串，断言失败。
    expect(await screen.findByText("task.bash")).toBeTruthy();
    expect(await screen.findByText("task.statusRunning")).toBeTruthy();
  });

  it("exitCode 按 camelCase 取到并渲染", async () => {
    mockTasks([makeTask({ exitCode: 7, status: "failed" })]);
    render(<TaskPanel />);

    // 展开任务详情
    const header = await screen.findByText("构建");
    // 为什么包在 act 里: raw dispatchEvent 的展开点击会在 act 外同步提交 3 个 setState
    // (setExpandedId/setEventsLoading/setEventsError)，每次点击刷 3 条 act 警告；
    // act 同步作用域只影响提交时机，点击后的异步取数链仍由后面的断言等待。
    act(() => {
      header.closest('[role="button"]')?.dispatchEvent(
        new MouseEvent("click", { bubbles: true }),
      );
    });

    await waitFor(() => {
      expect(screen.getByText(/task\.exitCode/)).toBeTruthy();
    });
    await settle();
  });

  it("未知 status 不映射为 pending（归因不得说谎）", async () => {
    mockTasks([makeTask({ status: "runing" })]);
    render(<TaskPanel />);

    // 必须显式报「未知状态」，并带上原始值
    expect(await screen.findByText(/task\.statusUnknown/)).toBeTruthy();
    // 关键否定断言：不能把漂移值渲染成「等待中」
    expect(screen.queryByText("task.statusPending")).toBeNull();
  });

  it("空 status 也按未知处理，不回退 pending", async () => {
    // 后端 TaskStatus::Unknown.as_db_str() 返回 ""
    mockTasks([makeTask({ status: "" })]);
    render(<TaskPanel />);

    expect(await screen.findByText("task.statusUnknown")).toBeTruthy();
    expect(screen.queryByText("task.statusPending")).toBeNull();
  });

  it("时间线读失败时不渲染成空列表，而是显式报错", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_background_tasks") { return Promise.resolve([makeTask()]); }
      if (cmd === "list_task_events") { return Promise.reject(new Error("db down")); }
      return Promise.resolve(undefined);
    });
    render(<TaskPanel />);

    const header = await screen.findByText("构建");
    // 为什么包在 act 里: raw dispatchEvent 的展开点击会在 act 外同步提交 3 个 setState
    // (setExpandedId/setEventsLoading/setEventsError)，每次点击刷 3 条 act 警告；
    // act 同步作用域只影响提交时机，点击后的异步取数链仍由后面的断言等待。
    act(() => {
      header.closest('[role="button"]')?.dispatchEvent(
        new MouseEvent("click", { bubbles: true }),
      );
    });

    // 失败必须出错误文案，而不是 task.timelineEmpty（那等于宣称「没有状态迁移」）
    expect(await screen.findByText("task.timelineLoadFail")).toBeTruthy();
    expect(screen.queryByText("task.timelineEmpty")).toBeNull();
    await settle();
  });

  it("时间线渲染迁移记录（读路径真被消费）", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_background_tasks") { return Promise.resolve([makeTask()]); }
      if (cmd === "list_task_events") {
        return Promise.resolve([
          {
            id: "e1",
            taskId: "t1",
            source: "command",
            fromStatus: "pending",
            toStatus: "running",
            actor: "system",
            reason: "任务创建",
            payload: null,
            createdAt: 1_700_000_000_000,
          },
        ]);
      }
      return Promise.resolve(undefined);
    });
    render(<TaskPanel />);

    const header = await screen.findByText("构建");
    // 为什么包在 act 里: raw dispatchEvent 的展开点击会在 act 外同步提交 3 个 setState
    // (setExpandedId/setEventsLoading/setEventsError)，每次点击刷 3 条 act 警告；
    // act 同步作用域只影响提交时机，点击后的异步取数链仍由后面的断言等待。
    act(() => {
      header.closest('[role="button"]')?.dispatchEvent(
        new MouseEvent("click", { bubbles: true }),
      );
    });

    await waitFor(() => {
      // 来源标签命中本地化 key ⇒ task_events.source 被真正消费
      expect(screen.getByText("task.sourceCommand")).toBeTruthy();
    });
    // from → to 两段都命中本地化标签，证明 fromStatus/toStatus 都被消费
    expect(screen.getByText(/task\.statusPending → /)).toBeTruthy();
    await settle();
  });
});
