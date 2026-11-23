// SPDX-License-Identifier: AGPL-3.0-only

/**
 * P3 运行面板锁（T6'''）：六态徽章、waiting/running 投影、终态取消禁用、outputs 展开。
 */

import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { App } from "antd";
import { describe, expect, it, vi } from "vitest";

import type { BotAssignment, BotWorkflowRun } from "@/types";
import { BotWorkflowRunsPanel } from "../BotWorkflowRunsPanel";

function makeRun(status: BotWorkflowRun["status"], id = "run-1"): BotWorkflowRun {
  return {
    id,
    workflowId: "wf-1",
    workflowName: `编排-${id}`,
    status,
    origin: "person",
    requesterBotId: null,
    requesterConversationId: "conv-1",
    anchorMessageId: null,
    context: {
      inputs: {},
      outputs: { build: { assignmentId: "a-1", resultText: "构建完成" } },
      skipped: ["deploy"],
    },
    completedSteps: 1,
    totalSteps: 2,
    errorCode: null,
    errorDetail: null,
    startedAt: 0,
    finishedAt: null,
    createdAt: 0,
    updatedAt: 0,
  };
}

function assignment(id: string, status: BotAssignment["status"], workflowRunId: string): BotAssignment {
  return {
    id,
    rootId: "root",
    parentId: null,
    depth: 1,
    origin: "workflow",
    workflowRunId,
    stepKey: "build",
    requesterBotId: null,
    requesterConversationId: null,
    assigneeBotId: "bot-a",
    assigneeName: "A",
    mode: "dispatch",
    taskText: "task",
    status,
    resultText: null,
    errorCode: null,
    errorDetail: null,
    startedAt: null,
    finishedAt: null,
    createdAt: 0,
    updatedAt: 0,
  };
}

function renderPanel(runs: BotWorkflowRun[], assignments: BotAssignment[] = []) {
  const onCancel = vi.fn(async () => {});
  render(
    <App>
      <BotWorkflowRunsPanel runs={runs} assignments={assignments} onCancel={onCancel} />
    </App>,
  );
  return onCancel;
}

describe("BotWorkflowRunsPanel", () => {
  it("waiting 徽章独立映射，终态取消按钮禁用", () => {
    renderPanel([makeRun("waiting")]);
    const item = screen.getByTestId("bot-workflow-run-item");
    expect(item).toHaveAttribute("data-status", "waiting");
    expect(within(item).getByTestId("bot-workflow-run-status").textContent).toBeTruthy();
    expect(within(item).getByTestId("bot-workflow-run-cancel")).toBeEnabled();
  });

  it("根步骤 completed 而子在途时 run 显示 running", () => {
    renderPanel([makeRun("completed")], [assignment("child", "running", "run-1")]);
    expect(screen.getByTestId("bot-workflow-run-item")).toHaveAttribute("data-status", "running");
    expect(screen.getByTestId("bot-workflow-run-cancel")).toBeEnabled();
  });

  it("completed 终态取消禁用，failed / cancelled 也禁用", () => {
    const { rerender } = render(
      <App>
        <BotWorkflowRunsPanel runs={[makeRun("completed")]} assignments={[]} onCancel={vi.fn(async () => {})} />
      </App>,
    );
    expect(screen.getByTestId("bot-workflow-run-cancel")).toBeDisabled();
    for (const status of ["failed", "cancelled"] as const) {
      rerender(
        <App>
          <BotWorkflowRunsPanel runs={[makeRun(status)]} assignments={[]} onCancel={vi.fn(async () => {})} />
        </App>,
      );
      expect(screen.getByTestId("bot-workflow-run-cancel")).toBeDisabled();
    }
  });

  it("outputs 展开显示结果与 skipped", async () => {
    const user = userEvent.setup();
    renderPanel([makeRun("running")]);
    expect(screen.queryByTestId("bot-workflow-run-outputs")).toBeNull();
    await user.click(screen.getByTestId("bot-workflow-run-outputs-toggle"));
    expect(screen.getByTestId("bot-workflow-run-outputs")).toHaveTextContent("构建完成");
    expect(screen.getByTestId("bot-workflow-run-skipped")).toHaveTextContent("deploy");
  });
});
