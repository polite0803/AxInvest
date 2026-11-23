// SPDX-License-Identifier: AGPL-3.0-only

/**
 * P3 编排编辑器锁（T6'''）：DAG 预校验错误码映射与保存前部门校验。
 */

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { App } from "antd";
import { describe, expect, it, vi } from "vitest";

import type { Bot, BotDepartment, BotWorkflow } from "@/types";
import { BotWorkflowEditor } from "../BotWorkflowEditor";

const BOTS: Bot[] = [{
  id: "bot-1",
  name: "执行 Bot",
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
}];

const DEPARTMENTS: BotDepartment[] = [{
  id: "dept-1",
  name: "研究部",
  description: "",
  chiefBotId: null,
  botIds: [],
  createdAt: 0,
  updatedAt: 0,
}];

function renderEditor(editing: BotWorkflow | null = null) {
  const onSubmit = vi.fn(async () => {});
  const onCancelEdit = vi.fn();
  render(
    <App>
      <BotWorkflowEditor
        bots={BOTS}
        departments={DEPARTMENTS}
        workflows={[]}
        editing={editing}
        onSubmit={onSubmit}
        onCancelEdit={onCancelEdit}
      />
    </App>,
  );
  return { onSubmit, onCancelEdit };
}

describe("BotWorkflowEditor", () => {
  it("空步骤提交触发 BOT_WORKFLOW_STEP_INVALID 预校验，不发 IPC", async () => {
    const user = userEvent.setup();
    const { onSubmit } = renderEditor();
    await user.type(screen.getByTestId("bot-workflow-name"), "日报编排");
    await user.click(screen.getByTestId("bot-workflow-save"));
    expect(screen.getByTestId("bot-workflow-validation-errors").textContent).toBeTruthy();
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("addStep 增加步骤卡片", async () => {
    const user = userEvent.setup();
    renderEditor();
    expect(screen.getAllByTestId("bot-workflow-step-card")).toHaveLength(1);
    await user.click(screen.getByTestId("bot-workflow-add-step"));
    expect(screen.getAllByTestId("bot-workflow-step-card")).toHaveLength(2);
  });
});
