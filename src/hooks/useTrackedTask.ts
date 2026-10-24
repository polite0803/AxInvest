// SPDX-License-Identifier: AGPL-3.0-only

import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { invoke, listen, logIpcError } from "@/lib/invoke";
import type { BackgroundTaskInfo, TaskAccepted } from "@/types";

/** 终态集合：与后端 `TaskStatus` 的落库字符串一致（completed / failed / stopped）。 */
const TERMINAL_STATUSES = new Set(["completed", "failed", "stopped"]);
/** 事件丢失时的兜底轮询（事件是主路径，轮询只防「listen 注册晚于终态」）。 */
const POLL_MS = 2000;

export interface UseTrackedTaskResult<T> {
  taskId: string | null;
  /** 已受理、未到终态 */
  running: boolean;
  /** 终态为 stopped（用户取消）—— 此时 `result` 可能是部分报告 */
  cancelled: boolean;
  result: T | null;
  error: string | null;
  /** 发任务：`submit` 返回受理回执（后端只回 taskId，不等执行结束）。`false` = 受理失败 */
  start: (submit: () => Promise<TaskAccepted>) => Promise<boolean>;
  cancel: () => Promise<void>;
  reset: () => void;
}

/**
 * 慢命令的统一消费端：发任务 → 等终态（事件 + 轮询兜底）→ 从
 * `background_tasks.result_json` 取回结构化报告。
 *
 * 组件卸载 ≠ 取消任务：任务照跑、报告照落库（任务中心可见），这里只是停止监听。
 * 卸载后重新挂载不会自动接回旧任务 —— 面板需要的是「本次请求的报告」，
 * 历史报告在任务中心里。
 */
export function useTrackedTask<T>(): UseTrackedTaskResult<T> {
  const { t } = useTranslation();
  // `t` 不进回调依赖：i18next 的 `t` 在语言切换时会换身份（测试替身则每次渲染都换）。
  // 一旦它进了 useCallback 依赖，消费方（如 RecommendationPanel 的 triggerBacktest →
  // 挂载 effect）就会每次渲染拿到新回调 ⇒ effect 重跑 ⇒ setState ⇒ 再渲染，恒不收敛。
  // 这里只在需要时读最新值，回调身份保持稳定。
  const tRef = useRef(t);
  tRef.current = t;
  const [taskId, setTaskId] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [cancelled, setCancelled] = useState(false);
  const [result, setResult] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);

  // 回调里判断「这条更新是不是我的」/「还要不要轮询」——state 在闭包里会过期
  const taskIdRef = useRef<string | null>(null);
  const settledRef = useRef(true);

  const reset = useCallback(() => {
    taskIdRef.current = null;
    settledRef.current = true;
    setTaskId(null);
    setRunning(false);
    setCancelled(false);
    setResult(null);
    setError(null);
  }, []);

  const applyRow = useCallback(
    (id: string, row: BackgroundTaskInfo | null) => {
      // 过期任务的结果不得覆盖新任务（连续点击两次时两路事件会交错）
      if (taskIdRef.current !== id) {
        return;
      }
      if (!row) {
        settledRef.current = true;
        setRunning(false);
        setError(tRef.current("tasksCenter.tracked.rowMissing"));
        return;
      }
      if (!TERMINAL_STATUSES.has(row.status)) {
        setRunning(true);
        return;
      }
      settledRef.current = true;
      setRunning(false);
      if (row.status === "failed") {
        setError(row.output || tRef.current("tasksCenter.tracked.failedNoDetail"));
        return;
      }
      setCancelled(row.status === "stopped");
      if (!row.resultJson) {
        // 成功/取消却没有报告：这是缺陷，不能渲染成「完成但没有数据」
        setError(tRef.current("tasksCenter.tracked.resultParseFailed"));
        return;
      }
      try {
        setResult(JSON.parse(row.resultJson) as T);
      } catch (e) {
        console.error("[useTrackedTask] resultJson 解析失败", e);
        setError(tRef.current("tasksCenter.tracked.resultParseFailed"));
      }
    },
    [],
  );

  const refresh = useCallback(
    async (id: string) => {
      // 同一任务已落终态：事件与在途轮询可能各触发一次，重复读会让消费方的
      // 「终态提示」effect 重跑（同一句 toast 弹两遍）。
      if (settledRef.current) {
        return;
      }
      try {
        const row = await invoke<BackgroundTaskInfo | null>("get_background_task_output", {
          taskId: id,
        });
        applyRow(id, row);
      } catch (e) {
        logIpcError("Failed to read tracked task")(e);
      }
    },
    [applyRow],
  );

  const start = useCallback(
    async (submit: () => Promise<TaskAccepted>): Promise<boolean> => {
      reset();
      try {
        const accepted = await submit();
        taskIdRef.current = accepted.taskId;
        settledRef.current = false;
        setTaskId(accepted.taskId);
        setRunning(true);
        // 任务可能已极快结束（如幂等回补「无需补齐」）⇒ 立刻探一次
        await refresh(accepted.taskId);
        return true;
      } catch (e) {
        settledRef.current = true;
        setRunning(false);
        setError(String(e));
        return false;
      }
    },
    [refresh, reset],
  );

  const cancel = useCallback(async () => {
    const id = taskIdRef.current;
    if (!id) {
      return;
    }
    try {
      await invoke("stop_background_task", { taskId: id });
    } catch (e) {
      logIpcError("Failed to cancel tracked task")(e);
    }
  }, []);

  useEffect(() => {
    if (!taskId) {
      return;
    }
    let disposed = false;
    let unlisten: (() => void) | undefined;
    // 后端 emit 的是裸 id 字符串（与 TaskPanel 同形态）
    listen<string>("background-task:updated", (event) => {
      if (event.payload === taskId) {
        void refresh(taskId);
      }
    }).then((fn) => {
      if (disposed) {
        fn();
      } else {
        unlisten = fn;
      }
    });
    const timer = setInterval(() => {
      if (!settledRef.current) {
        void refresh(taskId);
      }
    }, POLL_MS);
    return () => {
      disposed = true;
      unlisten?.();
      clearInterval(timer);
    };
  }, [taskId, refresh]);

  return { taskId, running, cancelled, result, error, start, cancel, reset };
}
