// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Bot 例程面板与表单（P2-a T4'）组件测试：
 *   - 表单校验：坏 cron / 坏时区报错文案走 i18n（返回键名形态断言）；
 *   - 创建走 create_bot_scheduled_task（T5' 契约入参面）；
 *   - 编辑走 update_scheduled_task 完整 DTO + 顶层 timezone 参数。
 */

import type { ScheduledTask } from "@/types";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { App as AntdApp } from "antd";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { BotRoutineFormModal, isValidTimeZone } from "../BotRoutineFormModal";

const h = vi.hoisted(() => ({
  createBotRoutine: vi.fn(),
  updateBotRoutine: vi.fn(),
}));

vi.mock("@/stores/feature/botStore", () => ({
  useBotStore: (selector: (state: typeof h) => unknown) => selector(h),
}));

// t 直接回键：报错文案断言 = 键名断言（i18n 11 语言齐备由门禁保证）
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock("@/lib/errorI18n", () => ({
  backendErrorText: (e: unknown) => String(e),
}));

function makeTask(over: Partial<ScheduledTask> = {}): ScheduledTask {
  return {
    id: "task-1",
    name: "每日晨报",
    description: "跑一轮晨报",
    taskType: "bot_routine",
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
      retryDelaySeconds: 60,
      notificationEnabled: false,
      runOnStartup: false,
    },
    createdAt: "2026-11-23T00:00:00Z",
    updatedAt: "2026-11-23T00:00:00Z",
    botId: "bot-1",
    timezone: null,
    ...over,
  };
}

function renderModal(props: Partial<Parameters<typeof BotRoutineFormModal>[0]> = {}) {
  return render(
    <AntdApp>
      <BotRoutineFormModal
        open
        botId="bot-1"
        editing={null}
        onClose={() => {}}
        onSaved={() => {}}
        {...props}
      />
    </AntdApp>,
  );
}

beforeEach(() => {
  h.createBotRoutine.mockReset().mockResolvedValue(makeTask());
  h.updateBotRoutine.mockReset().mockResolvedValue(undefined);
});

describe("BotRoutineFormModal", () => {
  it("坏 cron 提交时报 bots.routineCronInvalid 且不发起创建", async () => {
    const user = userEvent.setup();
    renderModal();
    await user.type(screen.getByLabelText("bots.routineName"), "晨报");
    await user.type(screen.getByLabelText("bots.routinePrompt"), "跑一轮");
    const cronInput = screen.getByLabelText("bots.routineCron");
    await user.clear(cronInput);
    await user.type(cronInput, "* * * * *"); // 每分钟（后端拒绝形态）
    await user.click(screen.getByRole("button", { name: "bots.routineAdd" }));
    await waitFor(() => {
      expect(screen.getAllByText("bots.routineCronInvalid").length).toBeGreaterThan(0);
    });
    expect(h.createBotRoutine).not.toHaveBeenCalled();
  });

  it("时区校验：IANA 名可解析，偏移串与瞎填拒绝（isValidTimeZone 单元口径，GATES-P2 §2 A3）", () => {
    expect(isValidTimeZone("Asia/Shanghai")).toBe(true);
    expect(isValidTimeZone("UTC")).toBe(true);
    expect(isValidTimeZone("UTC+8")).toBe(false);
    expect(isValidTimeZone("Not/AZone")).toBe(false);
    expect(isValidTimeZone("")).toBe(false);
  });

  it("创建走 create_bot_scheduled_task 契约入参（timezone null=跟随系统）", async () => {
    const user = userEvent.setup();
    renderModal();
    await user.type(screen.getByLabelText("bots.routineName"), "晨报");
    await user.type(screen.getByLabelText("bots.routinePrompt"), "跑一轮");
    await user.click(screen.getByRole("button", { name: "bots.routineAdd" }));
    await waitFor(() => {
      expect(h.createBotRoutine).toHaveBeenCalledTimes(1);
    });
    expect(h.createBotRoutine).toHaveBeenCalledWith({
      botId: "bot-1",
      name: "晨报",
      prompt: "跑一轮",
      cronExpression: "0 9 * * *",
      timezone: null,
    });
    expect(h.updateBotRoutine).not.toHaveBeenCalled();
  });

  it("编辑走 update_scheduled_task：完整 DTO 回传 + 顶层 timezone（undefined=不改）", async () => {
    const user = userEvent.setup();
    const editing = makeTask({ timezone: "Asia/Shanghai" });
    renderModal({ editing });
    // 编辑态 Modal 标题也是 bots.routineName ⇒ 标题与表单 label 撞名，取输入框本体
    const nameInput = screen.getAllByLabelText("bots.routineName").at(-1)!;
    await user.clear(nameInput);
    await user.type(nameInput, "改名后的晨报");
    await user.click(screen.getByRole("button", { name: "common.edit" }));
    await waitFor(() => {
      expect(h.updateBotRoutine).toHaveBeenCalledTimes(1);
    });
    const [taskId, updatedTask, timezone] = h.updateBotRoutine.mock.calls[0];
    expect(taskId).toBe("task-1");
    expect(updatedTask.name).toBe("改名后的晨报");
    expect(updatedTask.status).toBe("active"); // 完整 DTO：原任务为底
    expect(timezone).toBe("Asia/Shanghai"); // 回填的时区原样上送
    expect(h.createBotRoutine).not.toHaveBeenCalled();
  });
});
