// SPDX-License-Identifier: AGPL-3.0-only

import i18n from "@/i18n";
import { invokePluginCommand } from "@/lib/pluginCommand";

/**
 * 股票域长任务的前端出口（#349 异步任务 seam）。
 *
 * 协议：`stock_start_task { task, input }` → `{ taskId }` → 以固定间隔
 * `stock_poll_task { taskId }` 轮询，终态 `completed` 解出 `result` 返回、
 * `failed` / `stopped` 抛出（`output` 即宿主错误文案）、超时抛出。
 * 执行体跑在宿主进程（`spawn_tracked` 台账）⇒ 任务同时出现在任务中心，
 * 可在面板停止（协作式取消，语义与宿主 `stop_background_task` 同源）。
 *
 * 取代的是旧宿主命令的「await 到结束」语义：调用方代码形不变，只换入口。
 */
export async function invokeStockTask<T>(
  task: string,
  input?: Record<string, unknown>,
  opts?: { pollMs?: number; timeoutMs?: number },
): Promise<T> {
  const { taskId } = await invokePluginCommand<{ taskId: string }>("stock_start_task", {
    task,
    input: input ?? {},
  });
  const timeoutMs = opts?.timeoutMs ?? 30 * 60_000;
  const pollMs = opts?.pollMs ?? 1500;
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    // 先查后等：任务可能已秒完成（mock / 缓存态），首轮不引入固定延迟。
    const snap = await invokePluginCommand<{
      taskId: string;
      status: string;
      output: string;
      result?: T | null;
    }>("stock_poll_task", { taskId });
    if (snap.status === "completed") { return snap.result as T; }
    if (snap.status === "failed") {
      throw new Error(snap.output || i18n.t("error.STOCK_TASK_FAILED", { task }));
    }
    if (snap.status === "stopped") {
      throw new Error(snap.output || i18n.t("error.STOCK_TASK_CANCELLED", { task }));
    }
    if (Date.now() > deadline) {
      throw new Error(i18n.t("error.STOCK_TASK_TIMEOUT", {
        task,
        seconds: Math.round(timeoutMs / 1000),
        taskId,
      }));
    }
    await new Promise((r) => setTimeout(r, pollMs));
  }
}
