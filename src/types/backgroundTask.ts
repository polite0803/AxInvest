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
  exitCode?: number;
  conversationId?: string;
  idempotencyKey?: string;
  attempt: number;
  resumeFrom?: string;
  createdAt: number;
  updatedAt: number;
  finishedAt?: number;
}
