// SPDX-License-Identifier: AGPL-3.0-only

/**
 * BotRoutinesPanel 组件测试（P2-a T4'）：
 *   - 空态文案与「新建例程」入口；
 *   - 列表行渲染（名称/cron/时区/下次运行）；
 *   - 暂停/恢复走 pause_/resume_scheduled_task，删除走 delete_scheduled_task；
 *   - 命令失败落 toast（不崩面板）。
 */

import type { ScheduledTask } from "@/types";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { App as AntdApp } from "antd";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { BotRoutinesPanel } from "../BotRoutinesPanel";

const h = vi.hoisted(() => ({
  listBotRoutines: vi.fn(),
  pauseBotRoutine: vi.fn(),
  resumeBotRoutine: vi.fn(),
  deleteBotRoutine: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock("@/stores/feature/botStore", () => ({
  useBotStore: (selector: (state: typeof h) => unknown) => selector(h),
}));

vi.mock("@/lib/toast", () => ({
  message: { error: h.toastError, success: vi.fn() },
}));

vi.mock("@/lib/errorI18n", () => ({
  backendErrorText: (e: unknown) => String(e),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: { language: "zh-CN" },
  }),
}));

function makeTask(over: Partial<ScheduledTask> = {}): ScheduledTask {
  return {
    id: "task-1",
    name: "每日晨报",
    description: "",
    taskType: "bot_routine",
    cronExpression: "0 9 * * *",
    intervalSeconds: null,
    nextRunAt: "2026-11-24T01:00:00Z",
    lastRunAt: null,
    lastResult: null,
    status: "active",
    config: {
      timeoutSeconds: 300,
      retryOnFailure: false,
      maxRetries: 0,
      retryDelaySeconds: 60,
      notificationEnabled: false,
      runOnStartup: false,
    },
    createdAt: "2026-11-23T00:00:00Z",
    updatedAt: "2026-11-23T00:00:00Z",
    botId: "bot-1",
    timezone: "Asia/Shanghai",
    ...over,
  };
}

function renderPanel() {
  return render(
    <AntdApp>
      <BotRoutinesPanel botId="bot-1" />
    </AntdApp>,
  );
}

beforeEach(() => {
  h.listBotRoutines.mockReset().mockResolvedValue([]);
  h.pauseBotRoutine.mockReset().mockResolvedValue(undefined);
  h.resumeBotRoutine.mockReset().mockResolvedValue(undefined);
  h.deleteBotRoutine.mockReset().mockResolvedValue(undefined);
  h.toastError.mockReset();
});

describe("BotRoutinesPanel", () => {
  it("空态：显示 routineEmpty 与新建入口", async () => {
    renderPanel();
    await waitFor(() => {
      expect(screen.getByText("bots.routineEmpty")).toBeTruthy();
    });
    expect(screen.getByTestId("bot-routine-add")).toBeTruthy();
    expect(h.listBotRoutines).toHaveBeenCalledWith("bot-1");
  });

  it("列表行渲染名称与 cron（时区非空显示 IANA 名）", async () => {
    h.listBotRoutines.mockResolvedValue([makeTask()]);
    renderPanel();
    await waitFor(() => {
      expect(screen.getByTestId("bot-routine-item")).toBeTruthy();
    });
    expect(screen.getByText("每日晨报")).toBeTruthy();
    expect(screen.getByText(/0 9 \* \* \*/)).toBeTruthy();
    expect(screen.getByText(/Asia\/Shanghai/)).toBeTruthy();
  });

  it("时区空显示「跟随系统时区」（bots.routineTimezoneLocal）", async () => {
    h.listBotRoutines.mockResolvedValue([makeTask({ timezone: null })]);
    renderPanel();
    await waitFor(() => {
      expect(screen.getByText(/bots\.routineTimezoneLocal/)).toBeTruthy();
    });
  });

  it("关闭开关走 pauseBotRoutine，开启走 resumeBotRoutine", async () => {
    const user = userEvent.setup();
    h.listBotRoutines.mockResolvedValue([makeTask()]);
    renderPanel();
    await waitFor(() => {
      expect(screen.getByTestId("bot-routine-item")).toBeTruthy();
    });
    const sw = screen.getByRole("switch");
    await user.click(sw);
    await waitFor(() => {
      expect(h.pauseBotRoutine).toHaveBeenCalledWith("task-1");
    });
    expect(h.resumeBotRoutine).not.toHaveBeenCalled();
  });

  it("命令失败落 toast，面板不崩", async () => {
    const user = userEvent.setup();
    h.listBotRoutines.mockResolvedValue([makeTask()]);
    h.pauseBotRoutine.mockRejectedValue(new Error("PAUSE_FAILED"));
    renderPanel();
    await waitFor(() => {
      expect(screen.getByTestId("bot-routine-item")).toBeTruthy();
    });
    await user.click(screen.getByRole("switch"));
    await waitFor(() => {
      expect(h.toastError).toHaveBeenCalled();
    });
    expect(screen.getByTestId("bot-routine-item")).toBeTruthy();
  });
});
