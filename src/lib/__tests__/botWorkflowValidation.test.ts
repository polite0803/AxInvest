// SPDX-License-Identifier: AGPL-3.0-only

/**
 * P3 前端 DAG 预校验与多层链投影锁（T6'''）。
 * 断言错误码与 IPC-P3 §4 / MODEL-P3 §2 冻结语义一致，不依赖 locale 落盘。
 */

import {
  delegationRootIds,
  effectiveRunStatus,
  filterWorkflowsByDepartment,
  FRONTEND_VALIDATION_CODES,
  isAssignmentTerminal,
  isRunTerminal,
  runStatusI18nKey,
  selectDelegationChain,
  validateWorkflowSteps,
  workflowErrorI18nKey,
} from "@/lib/botWorkflowValidation";
import type { BotAssignment, BotWorkflow, BotWorkflowRun, BotWorkflowStep } from "@/types";
import { describe, expect, it } from "vitest";

function step(stepKey: string, dependsOn: string[] = []): BotWorkflowStep {
  return {
    stepKey,
    botId: `bot-${stepKey}`,
    promptTemplate: `prompt for ${stepKey}`,
    dependsOn,
    timeoutSecs: 30,
    onFailure: "abort",
  };
}

function run(status: BotWorkflowRun["status"], overrides: Partial<BotWorkflowRun> = {}): BotWorkflowRun {
  return {
    id: "run-1",
    workflowId: "wf-1",
    workflowName: "编排",
    status,
    origin: "person",
    requesterBotId: null,
    requesterConversationId: "conv-1",
    anchorMessageId: null,
    context: { inputs: {}, outputs: {}, skipped: [] },
    completedSteps: 1,
    totalSteps: 2,
    errorCode: null,
    errorDetail: null,
    startedAt: 0,
    finishedAt: null,
    createdAt: 0,
    updatedAt: 0,
    ...overrides,
  };
}

function assignment(id: string, overrides: Partial<BotAssignment> = {}): BotAssignment {
  return {
    id,
    rootId: id,
    parentId: null,
    depth: 1,
    origin: "workflow",
    workflowRunId: null,
    stepKey: null,
    requesterBotId: null,
    requesterConversationId: null,
    assigneeBotId: "bot-a",
    assigneeName: "A",
    mode: "dispatch",
    taskText: "task",
    status: "completed",
    resultText: null,
    errorCode: null,
    errorDetail: null,
    startedAt: null,
    finishedAt: null,
    createdAt: 0,
    updatedAt: 0,
    ...overrides,
  };
}

describe("DAG 前端预校验", () => {
  it("合法 DAG 通过", () => {
    const result = validateWorkflowSteps([step("a"), step("b", ["a"])]);
    expect(result.ok).toBe(true);
    expect(result.issues).toEqual([]);
  });

  it("自引用与互相依赖报 BOT_WORKFLOW_CYCLE", () => {
    expect(validateWorkflowSteps([step("a", ["a"])]).issues.map((i) => i.code)).toContain("BOT_WORKFLOW_CYCLE");
    const cycle = validateWorkflowSteps([step("a", ["b"]), step("b", ["a"])]);
    expect(cycle.ok).toBe(false);
    expect(cycle.issues.map((i) => i.code)).toContain("BOT_WORKFLOW_CYCLE");
  });

  it("非法 key / 未知依赖 / 超时越界报 BOT_WORKFLOW_STEP_INVALID", () => {
    const badKey = { ...step("bad key") };
    expect(validateWorkflowSteps([badKey]).issues.map((i) => i.code)).toContain("BOT_WORKFLOW_STEP_INVALID");
    expect(validateWorkflowSteps([step("a", ["missing"])]).issues.map((i) => i.code)).toContain(
      "BOT_WORKFLOW_STEP_INVALID",
    );
    expect(validateWorkflowSteps([{ ...step("a"), timeoutSecs: 301 }]).issues.map((i) => i.code)).toContain(
      "BOT_WORKFLOW_STEP_INVALID",
    );
  });

  it("空步骤与超过 12 步报 BOT_WORKFLOW_STEP_INVALID", () => {
    expect(validateWorkflowSteps([]).issues.map((i) => i.code)).toContain("BOT_WORKFLOW_STEP_INVALID");
    const many = Array.from({ length: 13 }, (_, i) => step(`s${i + 1}`));
    expect(validateWorkflowSteps(many).issues.map((i) => i.code)).toContain("BOT_WORKFLOW_STEP_INVALID");
  });

  it("前端只暴露后端语义常量的子集，错误码映射到 error.<CODE>", () => {
    expect(FRONTEND_VALIDATION_CODES).toEqual([
      "BOT_WORKFLOW_STEP_INVALID",
      "BOT_WORKFLOW_CYCLE",
      "BOT_WORKFLOW_DEPARTMENT_REQUIRED",
    ]);
    expect(workflowErrorI18nKey("BOT_WORKFLOW_CYCLE")).toBe("error.BOT_WORKFLOW_CYCLE");
    expect(workflowErrorI18nKey("BOT_DELEGATE_MAX_DEPTH")).toBe("error.BOT_DELEGATE_MAX_DEPTH");
    expect(workflowErrorI18nKey("NOT_A_BOT_CODE")).toBeNull();
  });
});

describe("多层链与运行状态投影", () => {
  it("终态 assignment 判定覆盖 completed / failed / cancelled", () => {
    expect(isAssignmentTerminal("completed")).toBe(true);
    expect(isAssignmentTerminal("failed")).toBe(true);
    expect(isAssignmentTerminal("cancelled")).toBe(true);
    expect(isAssignmentTerminal("queued")).toBe(false);
    expect(isAssignmentTerminal("running")).toBe(false);
  });

  it("根步骤 completed 但子在途时 run 仍显示 running", () => {
    const child = assignment("child", {
      rootId: "root",
      parentId: "root",
      depth: 2,
      workflowRunId: "run-1",
      status: "running",
    });
    expect(effectiveRunStatus(run("completed"), [child])).toBe("running");
    expect(isRunTerminal(run("completed"), [child])).toBe(false);
  });

  it("failed / cancelled 不被在途子 assignment 覆盖", () => {
    const child = assignment("child", { workflowRunId: "run-1", status: "running" });
    expect(effectiveRunStatus(run("failed"), [child])).toBe("failed");
    expect(effectiveRunStatus(run("cancelled"), [child])).toBe("cancelled");
  });

  it("waiting 与 queued 状态徽章键分开映射", () => {
    expect(runStatusI18nKey("waiting")).toBe("bot.workflowRun.statusWaiting");
    expect(runStatusI18nKey("running")).toBe("bot.workflowRun.statusRunning");
    expect(runStatusI18nKey("queued")).toBe("bot.workflowRun.statusQueued");
  });

  it("rootId 过滤并按 depth 再 createdAt 排序", () => {
    const all = [
      assignment("root", { rootId: "root", createdAt: 10 }),
      assignment("l3-late", { id: "l3-late", rootId: "root", parentId: "l2", depth: 3, createdAt: 40 }),
      assignment("other", { id: "other", rootId: "other", createdAt: 5 }),
      assignment("l2", { id: "l2", rootId: "root", parentId: "root", depth: 2, createdAt: 30 }),
      assignment("l3-early", { id: "l3-early", rootId: "root", parentId: "l2", depth: 3, createdAt: 20 }),
    ];
    expect(selectDelegationChain(all, "root").map((a) => a.id)).toEqual([
      "root",
      "l2",
      "l3-early",
      "l3-late",
    ]);
    expect(delegationRootIds(all)).toEqual(["other", "root"]);
  });

  it("部门筛选保留 instance 级编排", () => {
    const workflows = [
      { id: "w-instance", ownerScope: "instance" as const, departmentId: null },
      { id: "w-dept-a", ownerScope: "department" as const, departmentId: "dept-a" },
      { id: "w-dept-b", ownerScope: "department" as const, departmentId: "dept-b" },
    ] as BotWorkflow[];
    expect(filterWorkflowsByDepartment(workflows, null).map((w) => w.id)).toEqual([
      "w-instance",
      "w-dept-a",
      "w-dept-b",
    ]);
    expect(filterWorkflowsByDepartment(workflows, "dept-a").map((w) => w.id)).toEqual([
      "w-instance",
      "w-dept-a",
    ]);
  });
});
