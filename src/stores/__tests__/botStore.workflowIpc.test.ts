// SPDX-License-Identifier: AGPL-3.0-only

/**
 * P3 IPC 对齐回归：botStore 的编排写命令必须按 docs/bot-integration/P3/IPC-P3.md §2.4
 * 的冻结入参调用，不能依赖后端为旧前端保留的 `{ workflowId, input }` 兼容分支。
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
  listen: vi.fn(() => Promise.resolve(() => {})),
  isTauri: () => false,
  logIpcError: vi.fn(() => vi.fn()),
}));

vi.mock("@/lib/errorI18n", () => ({
  backendErrorText: (e: unknown) => (e instanceof Error ? e.message : String(e)),
}));

import { useBotStore } from "@/stores/feature/botStore";
import type { BotWorkflow } from "@/types";

const saved = { id: "wf-1", name: "日报编排" } as unknown as BotWorkflow;
const run = { id: "run-1", workflowId: "wf-1", status: "queued" } as never;

describe("botStore P3 编排命令入参（IPC-P3 §2.4）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useBotStore.setState({
      workflows: [],
      workflowRuns: [],
      workflowSaving: false,
      error: null,
    });
  });

  it("update_bot_workflow：扁平 `{ id, ...patch }`，不是 `{ workflowId, input }`", async () => {
    invokeMock.mockResolvedValueOnce(saved);
    await useBotStore.getState().updateWorkflow("wf-1", {
      name: "日报编排",
      ownerScope: "instance",
    });
    expect(invokeMock).toHaveBeenCalledWith("update_bot_workflow", {
      id: "wf-1",
      name: "日报编排",
      ownerScope: "instance",
    });

    // 同时钉住“返回值回写 store”：先种一条同 id 旧记录，再验证被替换。
    const before = { ...saved, name: "旧名" } as unknown as BotWorkflow;
    useBotStore.setState({ workflows: [before] });
    invokeMock.mockResolvedValueOnce(saved);
    await useBotStore.getState().updateWorkflow("wf-1", { name: "改名" });
    expect(useBotStore.getState().workflows).toEqual([saved]);
  });

  it("delete_bot_workflow：用冻结参数 `{ id }`", async () => {
    useBotStore.setState({ workflows: [saved] });
    invokeMock.mockResolvedValueOnce(undefined);
    await useBotStore.getState().deleteWorkflow("wf-1");
    expect(invokeMock).toHaveBeenCalledWith("delete_bot_workflow", { id: "wf-1" });
    expect(useBotStore.getState().workflows).toEqual([]);
  });

  it("run_bot_workflow：显式 workflowId / requesterConversationId / inputs", async () => {
    invokeMock.mockResolvedValueOnce(run);
    await useBotStore.getState().runWorkflow({
      workflowId: "wf-1",
      requesterConversationId: "conv-1",
      inputs: { topic: "AI" },
    });
    expect(invokeMock).toHaveBeenCalledWith("run_bot_workflow", {
      workflowId: "wf-1",
      requesterConversationId: "conv-1",
      inputs: { topic: "AI" },
    });
  });

  it("run_bot_workflow：无线程时显式 null，不做隐式建线程", async () => {
    invokeMock.mockResolvedValueOnce(run);
    await useBotStore.getState().runWorkflow({
      workflowId: "wf-1",
      requesterConversationId: null,
    });
    expect(invokeMock).toHaveBeenCalledWith("run_bot_workflow", {
      workflowId: "wf-1",
      requesterConversationId: null,
    });
  });
});
