import { act, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { UseTrackedTaskResult } from "../useTrackedTask";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

const invokeMock = vi.fn();
const listenMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: (...args: unknown[]) => listenMock(...args),
  logIpcError: () => () => {},
}));

import { useTrackedTask } from "../useTrackedTask";

type Report = { steps: number; cancelled?: boolean };

function row(status: string, report?: Report) {
  return {
    id: "t1",
    title: "slow",
    description: "",
    taskType: "market-close-backfill",
    status,
    output: status === "failed" ? "引擎报错" : "摘要",
    resultJson: report === undefined ? undefined : JSON.stringify(report),
    attempt: 0,
    createdAt: 1,
    updatedAt: 2,
    finishedAt: status === "running" ? undefined : 2,
  };
}

/** 用一个空组件承载 hook，并把最新状态暴露给测试。 */
function Harness({
  apiRef,
}: {
  apiRef: { current: UseTrackedTaskResult<Report> | null };
}) {
  apiRef.current = useTrackedTask<Report>();
  return null;
}

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockReset();
  listenMock.mockResolvedValue(() => {});
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("useTrackedTask — 异步任务消费", () => {
  it("受理 → 运行中 → completed 时从 resultJson 取回报告", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_background_task_output") {
        // 第一次是受理后的立刻探测，第二次是轮询
        return invokeMock.mock.calls.filter((c) => c[0] === "get_background_task_output").length === 1
          ? row("running")
          : row("completed", { steps: 3 });
      }
      return undefined;
    });

    const apiRef: { current: UseTrackedTaskResult<Report> | null } = { current: null };
    render(<Harness apiRef={apiRef} />);
    await act(async () => {
      await apiRef.current!.start(() => Promise.resolve({ taskId: "t1" }));
    });
    expect(apiRef.current!.running).toBe(true);
    expect(apiRef.current!.result).toBeNull();

    await act(async () => {
      vi.advanceTimersByTime(2000);
    });
    expect(apiRef.current!.running).toBe(false);
    expect(apiRef.current!.result).toEqual({ steps: 3 });
    expect(apiRef.current!.error).toBeNull();
  });

  it("failed 终态 → error 取 output（不造一份空报告）", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_background_task_output") {
        return row("failed");
      }
      return undefined;
    });

    const apiRef: { current: UseTrackedTaskResult<Report> | null } = { current: null };
    render(<Harness apiRef={apiRef} />);
    await act(async () => {
      await apiRef.current!.start(() => Promise.resolve({ taskId: "t1" }));
    });
    expect(apiRef.current!.running).toBe(false);
    expect(apiRef.current!.error).toBe("引擎报错");
    expect(apiRef.current!.result).toBeNull();
  });

  it("stopped（用户取消）→ cancelled 且保留部分报告", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_background_task_output") {
        return row("stopped", { steps: 1, cancelled: true });
      }
      return undefined;
    });

    const apiRef: { current: UseTrackedTaskResult<Report> | null } = { current: null };
    render(<Harness apiRef={apiRef} />);
    await act(async () => {
      await apiRef.current!.start(() => Promise.resolve({ taskId: "t1" }));
    });
    expect(apiRef.current!.cancelled).toBe(true);
    expect(apiRef.current!.result).toEqual({ steps: 1, cancelled: true });
    expect(apiRef.current!.error).toBeNull();
  });

  it("受理失败（IPC 报错）→ 不置 running，错误上抛到 error", async () => {
    const apiRef: { current: UseTrackedTaskResult<Report> | null } = { current: null };
    render(<Harness apiRef={apiRef} />);
    await act(async () => {
      const ok = await apiRef.current!.start(() => Promise.reject(new Error("boom")));
      expect(ok).toBe(false);
    });
    expect(apiRef.current!.running).toBe(false);
    expect(apiRef.current!.error).toContain("boom");
  });
});
