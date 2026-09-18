// SPDX-License-Identifier: AGPL-3.0-only

import { useWorkspaceTabActivity } from "@/hooks/useWorkspaceTabActivity";
import { useMultiAgentStore, useRlTrainingStore, useStreamStore, useWorkflowStore } from "@/stores";
import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

/**
 * 活动点信号接线测试。
 *
 * 这组用例的价值在**接线**：4 个信号各自来自 4 个不同 store 的**不同字段**，
 * 最容易出的错不是「逻辑写错」而是「接错字段/接错 store」—— 那种错在 UI 上
 * 表现为「点了半天不亮」或「恒亮」，光靠肉眼看切换栏很难定位。
 */
describe("useWorkspaceTabActivity — 信号接线", () => {
  beforeEach(() => {
    // 全部置为静默，避免用例间泄漏
    useStreamStore.setState({ streaming: false });
    useWorkflowStore.setState({ isExecuting: false });
    useMultiAgentStore.setState({ delegating: false });
    useRlTrainingStore.setState({ status: "idle" });
  });

  it("全部静默时没有任何 Tab 被点亮", () => {
    const { result } = renderHook(() => useWorkspaceTabActivity());
    expect(result.current.chat).toBe(false);
    expect(result.current.workflow).toBe(false);
    expect(result.current.multiAgent).toBe(false);
    expect(result.current.devtools).toBe(false);
  });

  it("对话流式中点亮 chat，且不误点其它 Tab", () => {
    useStreamStore.setState({ streaming: true });
    const { result } = renderHook(() => useWorkspaceTabActivity());
    expect(result.current.chat).toBe(true);
    expect(result.current.workflow).toBe(false);
    expect(result.current.multiAgent).toBe(false);
    expect(result.current.devtools).toBe(false);
  });

  it("工作流执行中点亮 workflow（信号源是 workflowStore.isExecuting，不是 agentStore）", () => {
    useWorkflowStore.setState({ isExecuting: true });
    const { result } = renderHook(() => useWorkspaceTabActivity());
    expect(result.current.workflow).toBe(true);
    expect(result.current.chat).toBe(false);
  });

  it("多智能体委派中点亮 multiAgent", () => {
    useMultiAgentStore.setState({ delegating: true });
    const { result } = renderHook(() => useWorkspaceTabActivity());
    expect(result.current.multiAgent).toBe(true);
  });

  it("RL 训练中点亮 devtools", () => {
    useRlTrainingStore.setState({ status: "running" });
    const { result } = renderHook(() => useWorkspaceTabActivity());
    expect(result.current.devtools).toBe(true);
  });

  it("信号会自行归零 —— 工作结束必须熄灭，否则活动点退化成常亮噪声", () => {
    useStreamStore.setState({ streaming: true });
    const { result } = renderHook(() => useWorkspaceTabActivity());
    expect(result.current.chat).toBe(true);

    act(() => {
      useStreamStore.setState({ streaming: false });
    });
    expect(result.current.chat).toBe(false);
  });

  it("无信号的 Tab 恒为 false —— 不是漏写，是那些 Tab 没有「会自行归零」的状态", () => {
    // 先把 4 个真实信号**全部点亮**：这样断言才有区分力 ——
    // 若只是全静默时 falsy，连"忘了接线"也照样通过。
    useStreamStore.setState({ streaming: true });
    useWorkflowStore.setState({ isExecuting: true });
    useMultiAgentStore.setState({ delegating: true });
    useRlTrainingStore.setState({ status: "running" });

    const { result } = renderHook(() => useWorkspaceTabActivity());

    expect(result.current.chat).toBe(true);
    expect(result.current.workflow).toBe(true);
    expect(result.current.multiAgent).toBe(true);
    expect(result.current.devtools).toBe(true);

    // terminal 的 PTY running 是「连接还开着」（恒亮噪声），
    // knowledge 只有全局 loading，files / dashboard 无相应状态 ⇒ 都不该亮
    expect(result.current.terminal).toBeFalsy();
    expect(result.current.knowledge).toBeFalsy();
    expect(result.current.files).toBeFalsy();
    expect(result.current.dashboard).toBeFalsy();
  });
});
