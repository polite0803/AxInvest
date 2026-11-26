// SPDX-License-Identifier: AGPL-3.0-only

/**
 * BotAssignmentsPanel 回归锁（P2-b T5''，PLAN-P2B §5 ②）：
 * 状态徽章映射（GAP G-B7 五态）、取消按钮终态禁用（IPC-P2B §1）。
 */

import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";
import { useBotStore } from "@/stores/feature/botStore";
import type { Bot, BotAssignment, BotAssignmentStatus } from "@/types";
import { BotAssignmentsPanel, isTerminalStatus, statusBadgeColor, statusI18nKey } from "../BotAssignmentsPanel";

function makeBot(id: string, name: string): Bot {
  return {
    id,
    name,
    title: null,
    description: null,
    soul: "",
    soulHash: "",
    soulDrift: false,
    icon: "",
    color: null,
    section: "",
    modelId: null,
    providerId: null,
    effort: null,
    workdir: null,
    approvalMode: "edits",
    agentProfileId: null,
    isEnabled: true,
    sortOrder: 0,
    createdAt: 0,
    updatedAt: 0,
  };
}

function makeAssignment(status: BotAssignmentStatus): BotAssignment {
  const now = Date.now();
  return {
    id: `a-${status}`,
    rootId: `a-${status}`,
    parentId: null,
    depth: 1,
    origin: "person",
    requesterBotId: null,
    requesterConversationId: null,
    assigneeBotId: "b",
    assigneeName: "Executor",
    mode: "dispatch",
    taskText: "整理研报",
    status,
    resultText: status === "completed" ? "已完成的结果" : null,
    errorCode: status === "failed" ? "BOT_DELEGATE_TIMEOUT" : null,
    errorDetail: status === "failed" ? "委派执行超时" : null,
    startedAt: null,
    finishedAt: null,
    createdAt: now,
    updatedAt: now,
  };
}

describe("状态映射纯函数", () => {
  it("五态徽章色映射（GAP G-B7）", () => {
    expect(statusBadgeColor("queued")).toBe("default");
    expect(statusBadgeColor("running")).toBe("processing");
    expect(statusBadgeColor("completed")).toBe("success");
    expect(statusBadgeColor("failed")).toBe("error");
    expect(statusBadgeColor("cancelled")).toBe("warning");
  });

  it("五态 i18n 键映射", () => {
    expect(statusI18nKey("queued")).toBe("bot.assignment.statusQueued");
    expect(statusI18nKey("running")).toBe("bot.assignment.statusRunning");
    expect(statusI18nKey("completed")).toBe("bot.assignment.statusCompleted");
    expect(statusI18nKey("failed")).toBe("bot.assignment.statusFailed");
    expect(statusI18nKey("cancelled")).toBe("bot.assignment.statusCancelled");
  });

  it("终态判据：queued/running 可取消，其余禁用（IPC-P2B §1）", () => {
    expect(isTerminalStatus("queued")).toBe(false);
    expect(isTerminalStatus("running")).toBe(false);
    expect(isTerminalStatus("completed")).toBe(true);
    expect(isTerminalStatus("failed")).toBe(true);
    expect(isTerminalStatus("cancelled")).toBe(true);
  });
});

describe("BotAssignmentsPanel 渲染", () => {
  beforeEach(async () => {
    if (!i18n.isInitialized) {
      await i18n.init();
    }
    await i18n.changeLanguage("zh-CN");
    localStorage.clear();
    useBotStore.setState({
      listBotAssignments: vi.fn().mockResolvedValue([]),
      cancelBotAssignment: vi.fn().mockResolvedValue(undefined),
      createBotAssignment: vi.fn().mockRejectedValue(new Error("blocked in list tests")),
    });
  });

  it("空列表：空状态可见（empty 键）", async () => {
    render(<BotAssignmentsPanel bot={makeBot("a", "A")} bots={[makeBot("a", "A"), makeBot("b", "B")]} />);
    expect(await screen.findByText("暂无委派记录")).toBeTruthy();
  });

  it("在途单渲染取消按钮；终态单不渲染取消按钮", async () => {
    useBotStore.setState({
      listBotAssignments: vi
        .fn()
        .mockResolvedValue([makeAssignment("queued"), makeAssignment("completed"), makeAssignment("failed")]),
    });
    render(<BotAssignmentsPanel bot={makeBot("a", "A")} bots={[makeBot("a", "A"), makeBot("b", "B")]} />);
    await waitFor(() => {
      expect(screen.getAllByTestId("bot-assignment-item")).toHaveLength(3);
    });
    // queued 1 个可取消；completed/failed 不可取消
    expect(screen.getAllByTestId("bot-assignment-cancel")).toHaveLength(1);
    expect(screen.getByText("已完成")).toBeTruthy();
    expect(screen.getByText("失败")).toBeTruthy();
    expect(screen.getByText("委派执行超时")).toBeTruthy(); // errorDetail 如实展示
  });
});
