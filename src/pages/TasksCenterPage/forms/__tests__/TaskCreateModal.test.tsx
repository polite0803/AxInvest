// SPDX-License-Identifier: AGPL-3.0-only

import type { ScheduledTask } from "@/types";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { App } from "antd";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TaskCreateModal } from "../TaskCreateModal";

// ── mocks ───────────────────────────────────────────────────────────────────
vi.mock("@/lib/invoke", () => ({
  invoke: vi.fn(),
  logIpcError: () => (e: unknown) => {
    console.warn("[ipc]", e);
  },
}));
vi.mock("@/lib/errorI18n", () => ({ showBackendError: vi.fn() }));
vi.mock("react-i18next", () => ({
  // t 回键名；模板卡片用到的 defaultValue 原样返回
  useTranslation: () => ({
    t: (k: string, opts?: Record<string, unknown>) => typeof opts?.defaultValue === "string" ? opts.defaultValue : k,
  }),
}));
vi.mock("@/stores", () => ({
  // 打开 Modal 瞬间带入当前股票
  useStockAnalysisStore: (selector: (s: { stockCode: string; stockName: string }) => unknown) =>
    selector({ stockCode: "600519", stockName: "贵州茅台" }),
}));

import { invoke } from "@/lib/invoke";
const invokeMock = vi.mocked(invoke);

function renderModal(editing: ScheduledTask | null) {
  const onClose = vi.fn();
  const onSaved = vi.fn();
  const result = render(
    <App>
      <TaskCreateModal open editing={editing} onClose={onClose} onSaved={onSaved} />
    </App>,
  );
  return { ...result, onClose, onSaved };
}

/** 点击 Modal 底部确定/更新按钮（文案随 t mock 回键名） */
function clickOk(key: string) {
  fireEvent.click(screen.getByText(key));
}

/** 切换顶部任务类别（antd Radio.Button：点 label 文本） */
function switchCategory(key: string) {
  fireEvent.click(screen.getByText(key).closest("label")!);
}

function makeEditingTask(overrides: Partial<ScheduledTask> = {}): ScheduledTask {
  return {
    id: "custom-1",
    name: "我的任务",
    description: "desc",
    taskType: "custom",
    cronExpression: "0 10 * * 1",
    intervalSeconds: null,
    nextRunAt: null,
    lastRunAt: null,
    lastResult: null,
    status: "active",
    config: {
      timeoutSeconds: 300,
      retryOnFailure: true,
      maxRetries: 3,
      retryDelaySeconds: 60,
      notificationEnabled: false,
      runOnStartup: false,
    },
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
    workflowId: null,
    params: undefined,
    ...overrides,
  };
}

describe("TaskCreateModal TR-7.1 三条新建路径", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) =>
      Promise.resolve(cmd === "get_scheduled_task_templates" ? [] : undefined)
    );
  });
  afterEach(() => cleanup());

  it("投资路径：个股分析提交 create_stock_cron（camelCase 三参数）", async () => {
    const { onSaved, onClose } = renderModal(null);

    // 默认类别即投资、默认种类个股分析；股票代码已从当前持仓带入
    const codeInput = screen.getByPlaceholderText("stockAnalysis.scheduledAnalysis.placeholder.stockCode");
    fireEvent.change(codeInput, { target: { value: "000001" } });
    fireEvent.change(
      screen.getByPlaceholderText("stockAnalysis.scheduledAnalysis.placeholder.stockName"),
      { target: { value: "平安银行" } },
    );

    await act(async () => {
      clickOk("settings.scheduler.createTask");
    });

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("create_stock_cron", {
        stockCode: "000001",
        stockName: "平安银行",
        // 种类默认 cron（每日 09:00）
        cronExpression: "0 9 * * *",
      })
    );
    expect(onSaved).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("自定义路径：提交 create_scheduled_task（taskType=custom + TaskConfig 默认值）", async () => {
    renderModal(null);
    switchCategory("tasksCenter.create.category.custom");

    fireEvent.change(screen.getByPlaceholderText("settings.scheduler.taskName"), {
      target: { value: "每周巡检" },
    });

    await act(async () => {
      clickOk("settings.scheduler.createTask");
    });

    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    const createCall = invokeMock.mock.calls.find(([cmd]) => cmd === "create_scheduled_task");
    expect(createCall, "必须调用 create_scheduled_task").toBeTruthy();
    expect(createCall?.[1]).toMatchObject({
      name: "每周巡检",
      cronExpression: "0 9 * * *",
      taskType: "custom",
      workflowId: null,
      config: {
        timeoutSeconds: 300,
        retryOnFailure: true,
        maxRetries: 3,
        retryDelaySeconds: 60,
        notificationEnabled: false,
        runOnStartup: false,
      },
    });
  });

  it("自定义路径：选中工作流模板后 taskType=workflow 且带 workflowId", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      Promise.resolve(
        cmd === "get_scheduled_task_templates"
          ? [
            {
              template_type: "workflow",
              name: "周报工作流",
              description: "周报",
              workflow_id: "weekly-summary",
              default_schedule: "0 9 * * 1",
            },
          ]
          : undefined,
      )
    );
    renderModal(null);
    switchCategory("tasksCenter.create.category.custom");

    // 模板名走 t 的 defaultValue（"周报工作流"）
    const card = await screen.findByText("周报工作流");
    fireEvent.click(card);
    // 模板已回填名称；直接提交
    await act(async () => {
      clickOk("settings.scheduler.createTask");
    });

    await waitFor(() => {
      const call = invokeMock.mock.calls.find(([cmd]) => cmd === "create_scheduled_task");
      expect(call?.[1]).toMatchObject({
        name: "周报工作流",
        taskType: "workflow",
        workflowId: "weekly-summary",
        cronExpression: "0 9 * * 1",
      });
    });
  });

  it("系统路径：一键备份提交 create_backup_task（name/cron 显式传参）", async () => {
    const { onSaved } = renderModal(null);
    switchCategory("tasksCenter.create.category.system");

    // 系统类别无 Modal footer，点击卡片按钮直接创建（按钮文本为 settings.scheduler.backup）
    const backupButton = screen.getByText("settings.scheduler.backup").closest("button")!;
    await act(async () => {
      fireEvent.click(backupButton);
    });

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("create_backup_task", {
        name: "settings.scheduler.backup",
        description: "settings.scheduler.backupDesc",
        cronExpression: "0 2 * * *",
      })
    );
    expect(onSaved).toHaveBeenCalledTimes(1);
  });
});

describe("TaskCreateModal 编辑态", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
  });
  afterEach(() => cleanup());

  it("仅编辑名称/描述/cron，提交 update_scheduled_task 完整 DTO", async () => {
    const editing = makeEditingTask();
    const { onSaved, onClose } = renderModal(editing);

    // 标题为编辑态
    expect(screen.getByText("tasksCenter.create.editTitle")).toBeTruthy();
    // 不存在类别切换
    expect(screen.queryByText("tasksCenter.create.category.invest")).toBeNull();

    fireEvent.change(screen.getByPlaceholderText("settings.scheduler.taskName"), {
      target: { value: "改名后的任务" },
    });

    await act(async () => {
      clickOk("settings.scheduler.updateTask");
    });

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("update_scheduled_task", {
        taskId: "custom-1",
        task: expect.objectContaining({
          ...editing,
          name: "改名后的任务",
          cronExpression: "0 10 * * 1",
        }),
      })
    );
    expect(onSaved).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
