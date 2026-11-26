// SPDX-License-Identifier: AGPL-3.0-only

/**
 * P4 A4/B4 前端回归锁（PLAN-P4 §6 A4/B4）：
 * - 同 askId 不双卡（pickVisibleAskUsers 纯函数）
 * - 配额面板：默认态提示、越界预校验、保存回显、恢复默认二次确认
 * - 持久提问卡：pending 空答复禁用、四态渲染、作答/取消调用
 */

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { App } from "antd";
import { beforeEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";
import { pickVisibleAskUsers } from "@/lib/askUserVisibility";
import { useBotStore } from "@/stores/feature/botStore";
import type { BotDepartmentQuota, BotInputRequest, BotInputRequestStatus } from "@/types";
import { BotInputRequestCard } from "../BotInputRequestCard";
import { BotQuotaPanel } from "../BotQuotaPanel";

function makeQuota(over: Partial<BotDepartmentQuota> = {}): BotDepartmentQuota {
  return {
    departmentId: "dep-1",
    maxConcurrency: 2,
    totalTimeoutSecs: 900,
    maxSteps: 12,
    maxInFlightPerDepth: 2,
    stepTimeoutSecs: 300,
    maxConcurrentRuns: 2,
    isDefault: true,
    updatedAt: null,
    ...over,
  };
}

function makeRequest(status: BotInputRequestStatus, over: Partial<BotInputRequest> = {}): BotInputRequest {
  const now = Date.now();
  return {
    id: `req-${status}`,
    askId: `ask-${status}`,
    conversationId: "conv-1",
    botId: "bot-1",
    workflowRunId: null,
    assignmentId: null,
    origin: "person",
    question: "选哪个方案？",
    options: ["A", "B"],
    status,
    answer: status === "answered" ? "A" : null,
    answeredAt: status === "answered" ? now : null,
    expiresAt: now + 300_000,
    createdAt: now,
    updatedAt: now,
    ...over,
  };
}

function renderInApp(ui: React.ReactElement) {
  return render(<App>{ui}</App>);
}

beforeEach(async () => {
  await i18n.changeLanguage("zh-CN");
  useBotStore.setState({
    departmentQuota: makeQuota(),
    quotaLoading: false,
    quotaSaving: false,
    getDepartmentQuota: vi.fn().mockResolvedValue(makeQuota()) as never,
    setDepartmentQuota: vi.fn().mockResolvedValue(makeQuota({ isDefault: false })) as never,
    resetDepartmentQuota: vi.fn().mockResolvedValue(undefined) as never,
    answerInputRequest: vi.fn().mockResolvedValue(makeRequest("answered")) as never,
    cancelInputRequest: vi.fn().mockResolvedValue(makeRequest("cancelled")) as never,
  });
});

describe("pickVisibleAskUsers（同 askId 不双卡）", () => {
  it("隐藏与持久卡相同的 askId，保留其余", () => {
    const asks = [{ askId: "a" }, { askId: "b" }, { askId: "c" }];
    expect(pickVisibleAskUsers(asks, ["b"])).toEqual([{ askId: "a" }, { askId: "c" }]);
  });

  it("无隐藏集合时原样返回", () => {
    const asks = [{ askId: "a" }];
    expect(pickVisibleAskUsers(asks)).toBe(asks);
    expect(pickVisibleAskUsers(asks, [])).toBe(asks);
  });
});

describe("BotQuotaPanel", () => {
  it("默认态显示「未设置时使用默认限制」", async () => {
    renderInApp(<BotQuotaPanel departmentId="dep-1" />);
    await waitFor(() => expect(screen.getByTestId("bot-quota-default-hint")).toBeTruthy());
    expect(screen.getByTestId("bot-quota-max-concurrency")).toBeTruthy();
  });

  it("越界值被前端拦下，不发保存请求", async () => {
    const user = userEvent.setup();
    renderInApp(<BotQuotaPanel departmentId="dep-1" />);
    const input = await screen.findByTestId("bot-quota-max-concurrency");
    await user.clear(input);
    await user.type(input, "9");
    await user.click(screen.getByTestId("bot-quota-save"));
    await waitFor(() => expect(screen.getByTestId("bot-quota-error")).toBeTruthy());
    expect(useBotStore.getState().setDepartmentQuota).not.toHaveBeenCalled();
  });

  it("合法值保存时提交完整六字段", async () => {
    const user = userEvent.setup();
    renderInApp(<BotQuotaPanel departmentId="dep-1" />);
    await screen.findByTestId("bot-quota-max-steps");
    await user.click(screen.getByTestId("bot-quota-save"));
    await waitFor(() => expect(useBotStore.getState().setDepartmentQuota).toHaveBeenCalledTimes(1));
    expect(useBotStore.getState().setDepartmentQuota).toHaveBeenCalledWith({
      departmentId: "dep-1",
      maxConcurrency: 2,
      totalTimeoutSecs: 900,
      maxSteps: 12,
      maxInFlightPerDepth: 2,
      stepTimeoutSecs: 300,
      maxConcurrentRuns: 2,
    });
  });

  it("后端 BOT_QUOTA_INVALID 的 detail 字段名透传到错误位", async () => {
    useBotStore.setState({
      setDepartmentQuota: vi.fn().mockRejectedValue({
        code: "BOT_QUOTA_INVALID",
        category: "validation",
        detail: "maxSteps=0 (range 1..=12)",
      }) as never,
    });
    const user = userEvent.setup();
    renderInApp(<BotQuotaPanel departmentId="dep-1" />);
    await screen.findByTestId("bot-quota-save");
    await user.click(screen.getByTestId("bot-quota-save"));
    const err = await screen.findByTestId("bot-quota-error");
    // 本地化码文案 + 后端 detail（含字段名）同时可见
    expect(err.textContent).toContain("Bot 部门配额取值不合法");
    expect(err.textContent).toContain("maxSteps");
  });

  it("恢复默认需二次确认后才调用 reset", async () => {
    const user = userEvent.setup();
    renderInApp(<BotQuotaPanel departmentId="dep-1" />);
    await screen.findByTestId("bot-quota-reset");
    expect(useBotStore.getState().resetDepartmentQuota).not.toHaveBeenCalled();
    await user.click(screen.getByTestId("bot-quota-reset"));
    // antd 会在两个汉字的按钮文案间插空格（「确 认」），按正则取。
    const confirm = await screen.findByRole("button", { name: /确\s*认/ });
    await user.click(confirm);
    await waitFor(() => expect(useBotStore.getState().resetDepartmentQuota).toHaveBeenCalledWith("dep-1"));
  });
});

describe("BotInputRequestCard", () => {
  it("pending 空答复禁用提交，填写后可提交并调用 answer", async () => {
    const user = userEvent.setup();
    const request = makeRequest("pending");
    renderInApp(<BotInputRequestCard request={request} />);
    const submit = screen.getByTestId("bot-input-request-submit") as HTMLButtonElement;
    expect(submit.disabled).toBe(true);
    await user.type(screen.getByTestId("bot-input-request-answer"), "就用 A");
    await waitFor(() =>
      expect((screen.getByTestId("bot-input-request-submit") as HTMLButtonElement).disabled).toBe(false)
    );
    await user.click(screen.getByTestId("bot-input-request-submit"));
    await waitFor(() =>
      expect(useBotStore.getState().answerInputRequest).toHaveBeenCalledWith("req-pending", "就用 A")
    );
  });

  it.each(
    [
      ["answered", "已回答"],
      ["expired", "该问题已过期"],
      ["cancelled", "已取消"],
    ] as const,
  )("%s 终态渲染", (status, text) => {
    renderInApp(<BotInputRequestCard request={makeRequest(status)} />);
    expect(screen.getByTestId("bot-input-request-status").textContent).toContain(text);
    expect((screen.getByTestId("bot-input-request-submit") as HTMLButtonElement).disabled).toBe(true);
    if (status !== "cancelled") {
      expect(screen.queryByTestId("bot-input-request-cancel")).toBeNull();
    }
  });
});
