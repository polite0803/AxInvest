// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 后端 `BackgroundTaskInfo`（src-tauri/src/commands/background_tasks.rs）的前端投影。
 *
 * 字段名必须是 camelCase：后端结构体带 `#[serde(rename_all = "camelCase")]`
 * （AGENTS.md 禁区 13），IPC 层的键名就是 `taskType` / `exitCode` 这一套。
 * 单一权威：chat 的 TaskPanel 与任务中心 backgroundTasks 适配器共同消费，
 * 不得在组件内重定义。
 */
export interface BackgroundTaskInfo {
  id: string;
  title: string;
  description: string;
  taskType: string;
  command?: string;
  prompt?: string;
  status: string;
  output: string;
  /**
   * 结构化报告（JSON 字符串）。`output` 只是给人看的摘要；慢命令的消费方
   * （回测/收盘面板）解析这一列。仅成功/取消的任务有值。
   */
  resultJson?: string;
  exitCode?: number;
  conversationId?: string;
  idempotencyKey?: string;
  attempt: number;
  resumeFrom?: string;
  createdAt: number;
  updatedAt: number;
  finishedAt?: number;
}

/**
 * 慢命令的受理回执（后端 `TaskAccepted`）。命令不再 wait 到执行结束，
 * 只回任务 id；报告经 `get_background_task_output` 的 `resultJson` 取回。
 */
export interface TaskAccepted {
  taskId: string;
}
