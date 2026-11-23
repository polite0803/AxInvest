// SPDX-License-Identifier: AGPL-3.0-only

/**
 * P3 回归锁：origin="workflow" 的 assignment 不破坏 P2-b 列表渲染与取消交互。
 * 不改动 P2-b 既有测试，只是新开一个面向 P3 数据形态的用例。
 */

import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";
import { useBotStore } from "@/stores/feature/botStore";
import type { Bot, BotAssignment } from "@/types";
import { BotAssignmentsPanel } from "../BotAssignmentsPanel";

function makeBot(id: string): Bot {
  return {
    id,
    name: id,
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

function workflowAssignment(): BotAssignment {
  return {
    id: "a-wf",
    rootId: "root",
    parentId: null,
    depth: 2,
    origin: "workflow",
    workflowRunId: "run-1",
    stepKey: "build",
    requesterBotId: null,
    requesterConversationId: null,
    assigneeBotId: "b",
    assigneeName: "Executor",
    mode: "dispatch",
    taskText: "编排步骤",
    status: "running",
    resultText: null,
    errorCode: null,
    errorDetail: null,
    startedAt: null,
    finishedAt: null,
    createdAt: 0,
    updatedAt: 0,
  };
}

describe("P3 workflow origin 对 P2-b 列表的兼容性", () => {
  beforeEach(async () => {
    if (!i18n.isInitialized) {
      await i18n.init();
    }
    await i18n.changeLanguage("zh-CN");
    localStorage.clear();
    useBotStore.setState({
      listBotAssignments: vi.fn().mockResolvedValue([workflowAssignment()]),
      cancelBotAssignment: vi.fn().mockResolvedValue(undefined),
      createBotAssignment: vi.fn().mockRejectedValue(new Error("blocked")),
    });
  });

  it("workflow 来源 assignment 正常入列且保留取消入口", async () => {
    render(<BotAssignmentsPanel bot={makeBot("a")} bots={[makeBot("a"), makeBot("b")]} />);
    await waitFor(() => {
      expect(screen.getAllByTestId("bot-assignment-item")).toHaveLength(1);
    });
    expect(screen.getByText("编排步骤")).toBeTruthy();
    expect(screen.getAllByTestId("bot-assignment-cancel")).toHaveLength(1);
  });
});
