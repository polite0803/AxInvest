// SPDX-License-Identifier: AGPL-3.0-only

// 工作流执行状态 → 前端 i18n 统一翻译层。
//
// 后端状态值（落库与 IPC 同形）是 snake_case 原文，而既有译文键在
// `chat.workflow.workflowStatus.*` 下是 camelCase（如 partiallyCompleted），
// 故需这层映射。未命中映射时回退后端原文 —— 宁可显示英文，不编造译文。

import i18n from "@/i18n";

const STATUS_I18N_KEY: Record<string, string> = {
  completed: "completed",
  running: "running",
  paused: "paused",
  created: "created",
  failed: "failed",
  cancelled: "cancelled",
  partially_completed: "partiallyCompleted",
  // 启动回收写进来的终态（`scheduler/restore.rs`）：上一进程消失、没有结论地中断。
  // 与 `cancelled` 严格不同 —— 那是用户主动取消，二者不可互替。
  interrupted: "interrupted",
};

const T_KEY_PREFIX = "chat.workflow.workflowStatus.";

/** 执行状态的用户可见译文。 */
export function workflowStatusLabel(status: string): string {
  const key = STATUS_I18N_KEY[status];
  return key ? i18n.t(`${T_KEY_PREFIX}${key}`) : status;
}
