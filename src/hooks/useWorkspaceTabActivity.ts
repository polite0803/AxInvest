// SPDX-License-Identifier: AGPL-3.0-only
/**
 * 工作台各 Tab 的「活动中」信号 —— 供切换栏渲染右上角活动点。
 *
 * 判据（**三条缺一不可**，否则活动点会退化成噪声）：
 *   1. 状态在**全局 store** 里，不依赖该页是否挂载
 *      —— 活动点的存在意义正是「用户不在那个 Tab 时也能看到它在干活」；
 *      若信号只存在于页面局部 state，非活跃时页面已卸载/隐藏，读不到。
 *   2. 状态**会自行归零**：不是「曾经发生过」，也不是「连接还开着」。
 *   3. 语义是「有正在进行、会结束的工作」，而非「有东西存在」。
 *
 * ⚠ 反例（本仓实测，**刻意不配活动点**；勿凭直觉"补齐 5 个"）：
 *   - terminal：唯一候选是 `sessions[].status === "running"`，但 PTY 会话一旦创建就长期
 *     处于 running（直到 exited），**恒亮** ⇒ 是噪声不是信号。终端 store 里没有
 *     「命令执行中」这一维状态（`activeSessionId` 同理恒真）。
 *   - knowledge：`useKnowledgeSourceStore` 只有 `loading`（全局唯一、瞬时的请求态），
 *     **没有**索引 / 抓取任务的进行中状态。
 *   - files / dashboard：无相应状态。
 *   宁可少 4 个点，也不要 4 个常亮或永假的假信号。
 */

import type { WorkspaceTab } from "@/lib/workspaceTabs";
import { useMultiAgentStore, useRlTrainingStore, useStreamStore, useWorkflowStore } from "@/stores";

/** Tab → 是否正有工作进行中。未列出的 Tab 恒为 `false`（无信号，见文件头反例）。 */
export function useWorkspaceTabActivity(): Partial<Record<WorkspaceTab, boolean>> {
  // ⚠ 每个 selector 都只取 **boolean**：zustand 按 Object.is 比较，
  // 流式期间每 50ms 到达的 chunk 不会触发调用方重渲染，只有 true↔false 翻转才重渲染。
  // 不要把整个 store 或对象/数组取出来，否则切换栏会跟着流式高频重渲染。
  const chatStreaming = useStreamStore((s) => s.streaming);
  const workflowExecuting = useWorkflowStore((s) => s.isExecuting);
  const delegating = useMultiAgentStore((s) => s.delegating);
  const rlTraining = useRlTrainingStore((s) => s.status === "running");

  return {
    chat: chatStreaming,
    workflow: workflowExecuting,
    multiAgent: delegating,
    devtools: rlTraining,
  };
}
