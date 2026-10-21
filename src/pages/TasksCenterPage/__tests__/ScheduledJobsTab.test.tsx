// SPDX-License-Identifier: AGPL-3.0-only

import type { ScheduledTask } from "@/types";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { App } from "antd";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ScheduledJobsTab } from "../ScheduledJobsTab";

const { invokeMock, navigateMock, showBackendErrorMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  navigateMock: vi.fn(),
  showBackendErrorMock: vi.fn(),
}));

vi.mock("@/lib/invoke", async (importOriginal) => {
  // TaskCreateModal → @/stores barrel 会消费 isTauri 等其他导出，保留原模块仅替换 invoke
  const actual = await importOriginal<typeof import("@/lib/invoke")>();
  return { ...actual, invoke: invokeMock };
});
vi.mock("@/lib/errorI18n", () => ({
  showBackendError: showBackendErrorMock,
  // 历史详情列对非 null 的 output 会走翻译；这个导出缺失会让「有 output 的历史行」整棵树崩
  // （D-5 加轮次跳转时才第一次把这条路径跑到）。
  translateBackendError: (s: string) => s,
}));
vi.mock("react-router-dom", () => ({ useNavigate: () => navigateMock }));

// t 直接回键（插值参数不影响断言目标键）
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: { language: "zh-CN" },
  }),
  // TaskCreateModal → @/stores barrel 会拉起 @/i18n 初始化，需要该导出
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

function makeTask(over: Partial<ScheduledTask>): ScheduledTask {
  return {
    id: "t-id",
    name: "任务",
    description: "",
    taskType: "custom",
    cronExpression: "0 9 * * *",
    intervalSeconds: null,
    nextRunAt: null,
    lastRunAt: null,
    lastResult: null,
    status: "active",
    config: {
      timeoutSeconds: 300,
      retryOnFailure: false,
      maxRetries: 0,
      retryDelaySeconds: 30,
      notificationEnabled: true,
      runOnStartup: false,
    },
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
    workflowId: null,
    ...over,
  };
}

const TASK_POOL = makeTask({
  id: "t-pool",
  name: "股票池扫描",
  taskType: "pool-scan",
  status: "active",
  cronExpression: "0 18 * * 1-5",
});
const TASK_CUSTOM = makeTask({
  id: "t-custom",
  name: "我的自定义任务",
  taskType: "custom",
  workflowId: null,
  status: "paused",
});
const TASK_OPC = makeTask({
  id: "t-opc",
  name: "OPC 需求发现",
  taskType: "opc-demand-discovery",
  workflowId: "opc-demand-discovery",
  status: "active",
});

function renderTab() {
  return render(
    <App>
      <ScheduledJobsTab />
    </App>,
  );
}

/** 按任务名定位表格行（antd Table 行内含名称单元格）。 */
async function rowNamed(name: string): Promise<HTMLElement> {
  const cell = await screen.findByText(name);
  const row = cell.closest("tr");
  if (!row) {
    throw new Error(`找不到任务行: ${name}`);
  }
  return row;
}

describe("ScheduledJobsTab 列表与行操作（TR-6.2）", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    navigateMock.mockReset();
    showBackendErrorMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_scheduled_tasks") {
        return Promise.resolve([TASK_POOL, TASK_CUSTOM, TASK_OPC]);
      }
      if (cmd === "get_task_execution_history") {
        return Promise.resolve([
          {
            id: "r1",
            task_id: "t-pool",
            started_at: Date.now(),
            completed_at: Date.now() + 1200,
            success: true,
            output: null,
            error: null,
            duration_ms: 1200,
          },
        ]);
      }
      return Promise.resolve(undefined);
    });
  });
  afterEach(() => cleanup());

  it("渲染四个分组 Tag（投资 / 自定义 / OPC）", async () => {
    renderTab();
    const poolRow = await rowNamed("股票池扫描");
    expect(within(poolRow).getByText("tasksCenter.group.invest")).toBeTruthy();
    const customRow = await rowNamed("我的自定义任务");
    expect(within(customRow).getByText("tasksCenter.group.custom")).toBeTruthy();
    const opcRow = await rowNamed("OPC 需求发现");
    expect(within(opcRow).getByText("tasksCenter.group.opc")).toBeTruthy();
  });

  it("Switch 关闭→pause、开启→resume，载荷 camelCase", async () => {
    renderTab();
    const poolRow = await rowNamed("股票池扫描");
    // pool 当前 active → 点一下走暂停
    fireEvent.click(within(poolRow).getByRole("switch"));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("pause_scheduled_task", { taskId: "t-pool" }));

    const customRow = await rowNamed("我的自定义任务");
    // custom 当前 paused → 点一下走恢复
    fireEvent.click(within(customRow).getByRole("switch"));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("resume_scheduled_task", { taskId: "t-custom" }));
  });

  it("立即执行：内置任务调用 execute_scheduled_task；裸 custom 置灰", async () => {
    renderTab();
    const poolRow = await rowNamed("股票池扫描");
    const executeBtn = within(poolRow).getByLabelText("tasksCenter.action.execute");
    expect(executeBtn).not.toBeDisabled();
    fireEvent.click(executeBtn);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("execute_scheduled_task", { taskId: "t-pool" }));

    const customRow = await rowNamed("我的自定义任务");
    expect(within(customRow).getByLabelText("tasksCenter.action.execute")).toBeDisabled();
  });

  it("立即执行等到终态 ⇒ 直接展示这一轮结论（含 run_id），不再只弹「已触发」", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_scheduled_tasks") { return [TASK_POOL, TASK_CUSTOM, TASK_OPC]; }
      if (cmd === "execute_scheduled_task") {
        return {
          success: true,
          output: "[reco] 区间 09-30~10-20 漏检=62 | run_id=mrr-1",
          awaited: true,
        };
      }
      return undefined;
    });
    renderTab();
    const poolRow = await rowNamed("股票池扫描");
    fireEvent.click(within(poolRow).getByLabelText("tasksCenter.action.execute"));
    await waitFor(() => expect(screen.getByText(/run_id=mrr-1/)).toBeTruthy());
  });

  it("只回「已触发」（长任务）⇒ ack 不得被当成结果弹出来", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_scheduled_tasks") { return [TASK_POOL, TASK_CUSTOM, TASK_OPC]; }
      if (cmd === "execute_scheduled_task") {
        return { success: true, output: "任务 '股票池扫描' 已触发，请稍后刷新查看执行结果", awaited: false };
      }
      return undefined;
    });
    renderTab();
    const poolRow = await rowNamed("股票池扫描");
    fireEvent.click(within(poolRow).getByLabelText("tasksCenter.action.execute"));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("execute_scheduled_task", { taskId: "t-pool" }));
    expect(screen.queryByText(/请稍后刷新查看执行结果/)).toBeNull();
  });

  it("历史按钮拉取执行历史并打开 Drawer", async () => {
    renderTab();
    const poolRow = await rowNamed("股票池扫描");
    fireEvent.click(within(poolRow).getByLabelText("tasksCenter.action.history"));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("get_task_execution_history", { taskId: "t-pool" }));
    expect(await screen.findByText("tasksCenter.history.title")).toBeTruthy();
    // 负控：这条历史行的 output 是 null（没有 run_id）⇒ 不该长出轮次跳转
    expect(screen.queryByTestId("open-mover-run")).toBeNull();
  });

  it("妖股漏检历史行带 run_id ⇒ 出跳转链接，点击深链到漏检面板并锁定该轮", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_scheduled_tasks") {
        return Promise.resolve([makeTask({ id: "mrr-job", name: "mover-check", taskType: "mover-recall" })]);
      }
      if (cmd === "get_task_execution_history") {
        return Promise.resolve([
          {
            id: "r9",
            task_id: "mrr-job",
            started_at: Date.now(),
            completed_at: Date.now() + 1900,
            success: true,
            output: "区间 2026-09-30~2026-10-20 达标事件=77 漏检=65 | run_id=mrr-6eaaeed5-1",
            error: null,
            duration_ms: 1900,
            partial: false,
          },
        ]);
      }
      return Promise.resolve(undefined);
    });
    renderTab();
    const row = await rowNamed("mover-check");
    fireEvent.click(within(row).getByLabelText("tasksCenter.action.history"));
    const link = await screen.findByTestId("open-mover-run");
    fireEvent.click(link);
    await waitFor(() =>
      expect(navigateMock).toHaveBeenCalledWith("/invest?tab=workspace&view=backtest&moverRun=mrr-6eaaeed5-1")
    );
  });

  it("删除：Popconfirm 确认后调 delete_scheduled_task（camelCase 载荷）", async () => {
    renderTab();
    const poolRow = await rowNamed("股票池扫描");
    fireEvent.click(within(poolRow).getByLabelText("tasksCenter.action.delete"));
    // 内置任务显示额外警示文案
    expect(await screen.findByText("tasksCenter.delete.builtinWarning")).toBeTruthy();
    const confirmBtn = await screen.findByRole("button", { name: "tasksCenter.delete.ok" });
    fireEvent.click(confirmBtn);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("delete_scheduled_task", { taskId: "t-pool" }));
  });

  it("OPC 行展示投递配置跳转按钮，点击导航到 DemandDiscoveryPage", async () => {
    renderTab();
    const opcRow = await rowNamed("OPC 需求发现");
    fireEvent.click(within(opcRow).getByText("tasksCenter.action.opcDelivery"));
    expect(navigateMock).toHaveBeenCalledWith("/demand-discovery");
  });
});

describe("ScheduledJobsTab 自动刷新（TR-6.3）", () => {
  // 不用全局 fake timers（antd Table / rc 组件渲染依赖 rAF 与真实调度，假定时器下会卡渲染）；
  // 截获全部 setInterval 注册，按周期定位任务中心自己的 15s 回调。
  let registrations: Array<{ id: number; cb: () => void; ms: number }>;
  let nextTimerId: number;
  let setIntervalSpy: ReturnType<typeof vi.spyOn>;
  let clearIntervalSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    registrations = [];
    nextTimerId = 900_700;
    invokeMock.mockReset().mockResolvedValue([]);
    setIntervalSpy = vi
      .spyOn(globalThis, "setInterval")
      .mockImplementation(
        (((cb: TimerHandler, ms?: number) => {
          const id = nextTimerId++;
          registrations.push({ id, cb: cb as () => void, ms: Number(ms ?? 0) });
          return id as unknown as ReturnType<typeof setInterval>;
        }) as unknown) as typeof setInterval,
      );
    clearIntervalSpy = vi.spyOn(globalThis, "clearInterval").mockImplementation(() => {});
  });
  afterEach(() => {
    setIntervalSpy.mockRestore();
    clearIntervalSpy.mockRestore();
    cleanup();
  });

  it("以 15s 周期注册自动刷新，卸载时 clearInterval", async () => {
    const { unmount } = renderTab();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(1));

    const auto = registrations.find((r) => r.ms === 15_000);
    expect(auto, "必须注册 15s 自动刷新定时器").toBeTruthy();

    // 手动触发一次定时回调 → 静默再拉一次列表
    await act(async () => {
      auto?.cb();
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledTimes(2);

    // 卸载必须清理该定时器
    unmount();
    expect(clearIntervalSpy).toHaveBeenCalledWith(auto!.id);
  });
});
