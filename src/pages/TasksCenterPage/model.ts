// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 任务中心 v2 统一运行模型（Unified Run Model）。
 *
 * 设计参照 Temporal Web UI / Windmill Runs / GitHub Actions：
 * 中心只理解一种语言——UnifiedRun；各任务源（cron / pipeline / 后台任务 /
 * 索引队列 / workflow ……）通过 TaskSourceAdapter 把原生记录映射成 UnifiedRun。
 * 详见 `.trae/specs/tasks-center/redesign-v2.md`。
 *
 * 本文件为纯类型 + 纯函数，不允许引 React，便于单测。
 */

import type { ScheduledTask } from "@/types";

// ── 源标识：新增源时在此扩联合并注册适配器 ─────────────────────────────
export type SourceId =
  | "scheduled-cron"
  | "stock-pipeline"
  | "stock-analysis"
  | "time-travel-batch"
  | "mover-recall-analysis"
  // P2-2 起接入：
  | "background-tasks"
  | "workflow-dag"
  // P2-3：
  | "index-jobs"
  // P2-4：
  | "fine-tune-jobs"
  | "opc-demand-delivery"
  | "device-sync"
  | "fleet"
  | "evolution-engine"
  | "knowledge-import";

/** 统一运行状态机（中心唯一渲染口径，原生状态由适配器映射）。 */
export type RunState =
  | "queued"
  | "running"
  | "waiting"
  | "paused"
  | "succeeded"
  | "failed"
  | "canceled";

export type RunTrigger = "schedule" | "manual" | "system" | "external" | "unknown";

/**
 * 后端触发来源 → 统一口径。
 *
 * 缺失/未登记一律 `unknown`：**不得**猜成 schedule 或 manual —— 旧数据没有这个信息，
 * 猜一类就是把「不知道」伪装成可信读数（与状态映射里「未知不得默认 running」同一条纪律）。
 */
export function mapRunTrigger(value: string | null | undefined): RunTrigger {
  switch (value) {
    case "schedule":
      return "schedule";
    case "manual":
      return "manual";
    case "system":
      return "system";
    case "external":
      return "external";
    default:
      return "unknown";
  }
}

/** 不改变主状态的修饰标记（部分成功/重试中/中断/僵死/跳过）。 */
export type RunBadge =
  | "partial"
  | "retrying"
  | "interrupted"
  | "stale"
  | "skipped";

/** 该 run 在当前状态下可执行/可查看的能力（由适配器按源命令齐备性给出）。 */
export interface RunCapabilities {
  cancel?: boolean;
  retry?: boolean;
  pause?: boolean;
  resume?: boolean;
  viewLogs?: boolean;
  viewSteps?: boolean;
  viewIO?: boolean;
  viewHistory?: boolean;
  openSource?: boolean;
}

export interface RunError {
  code?: string;
  message: string;
}

/** 跨源统一运行记录。时间字段一律 epoch ms。 */
export interface UnifiedRun {
  /** `${source}:${nativeId}`，全局唯一 */
  id: string;
  source: SourceId;
  /** 源内原生 id（调源命令时用它） */
  nativeId: string;
  /** 展示名（适配器已按当前语言渲染） */
  name: string;
  trigger: RunTrigger;
  state: RunState;
  /** 子阶段（已本地化文案，如「分析」/「反思」、training/validating） */
  stage?: string;
  /** 0–100；无源数据时留空 */
  progress?: number;
  attempts: number;
  queuedAt?: number;
  startedAt?: number;
  finishedAt?: number;
  durationMs?: number;
  error?: RunError;
  badges?: RunBadge[];
  /** DAG/父子执行关联 */
  parentRunId?: string;
  /** 跳源页面的站内路由 */
  deepLink?: string;
  capabilities: RunCapabilities;
  /** 源原生记录（源专属详情抽屉使用，中心不解读） */
  raw?: unknown;
}

// ── 查询 / 分页 / 详情附件 ────────────────────────────────────────────

export interface RunQuery {
  sources?: SourceId[];
  states?: RunState[];
  triggers?: RunTrigger[];
  /** 仅返回该时间点之后开始/结束的 run */
  sinceMs?: number;
  search?: string;
  limit?: number;
}

export interface RunStep {
  id: string;
  name: string;
  state: RunState | "skipped";
  startedAt?: number;
  finishedAt?: number;
  message?: string;
}

export interface RunLogLine {
  ts: number;
  stream?: "out" | "err";
  text: string;
}

export interface RunArtifact {
  label: string;
  value: unknown;
}

/** 单源拉取失败（allSettled 隔离，不拖垮整页）。 */
export interface SourceFetchError {
  source: SourceId;
  message: string;
}

export interface AggregatedRuns {
  runs: UnifiedRun[];
  errors: SourceFetchError[];
}

/** 概览健康卡四计数。 */
export interface RunHealth {
  running: number;
  queued: number;
  failed: number;
  waiting: number;
}

// ── 纯函数：状态归并、健康计数、过滤/排序 ─────────────────────────────

export function runIdOf(source: SourceId, nativeId: string): string {
  return `${source}:${nativeId}`;
}

export function computeHealth(runs: readonly UnifiedRun[]): RunHealth {
  const health: RunHealth = { running: 0, queued: 0, failed: 0, waiting: 0 };
  for (const r of runs) {
    if (r.state === "running") {
      health.running += 1;
    } else if (r.state === "queued") {
      health.queued += 1;
    } else if (r.state === "failed") {
      health.failed += 1;
    } else if (r.state === "waiting") {
      health.waiting += 1;
    }
  }
  return health;
}

/** 需要关注：失败 / 等待（审批、阻塞）/ 运行僵死。 */
export function attentionRuns(runs: readonly UnifiedRun[]): UnifiedRun[] {
  return runs.filter(
    (r) =>
      r.state === "failed"
      || r.state === "waiting"
      || r.badges?.includes("stale") === true,
  );
}

export function activeRuns(runs: readonly UnifiedRun[]): UnifiedRun[] {
  return runs.filter((r) => r.state === "running" || r.state === "queued");
}

/** 列表过滤（源/状态/触发/时间窗/全文）。sinceMs 同时匹配开始与结束时间。 */
export function filterRuns(runs: readonly UnifiedRun[], q: RunQuery): UnifiedRun[] {
  const needle = q.search?.trim().toLowerCase();
  return runs.filter((r) => {
    if (q.sources && q.sources.length > 0 && !q.sources.includes(r.source)) {
      return false;
    }
    if (q.states && q.states.length > 0 && !q.states.includes(r.state)) {
      return false;
    }
    if (q.triggers && q.triggers.length > 0 && !q.triggers.includes(r.trigger)) {
      return false;
    }
    if (q.sinceMs !== undefined) {
      const anchor = r.finishedAt ?? r.startedAt ?? r.queuedAt;
      if (anchor === undefined || anchor < q.sinceMs) {
        return false;
      }
    }
    if (needle) {
      const hay = `${r.name} ${r.nativeId} ${r.error?.message ?? ""}`.toLowerCase();
      if (!hay.includes(needle)) {
        return false;
      }
    }
    return true;
  });
}

/** 统一排序：运行中优先，其次按最近活动时间倒序。 */
export function sortRuns(runs: readonly UnifiedRun[]): UnifiedRun[] {
  const activityAt = (r: UnifiedRun): number => r.finishedAt ?? r.startedAt ?? r.queuedAt ?? 0;
  const stateWeight = (r: UnifiedRun): number => {
    if (r.state === "running") {
      return 0;
    }
    if (r.state === "queued") {
      return 1;
    }
    if (r.state === "waiting") {
      return 2;
    }
    if (r.state === "failed") {
      return 3;
    }
    if (r.state === "paused") {
      return 4;
    }
    return 5;
  };
  return [...runs].sort((a, b) => {
    const wa = stateWeight(a);
    const wb = stateWeight(b);
    if (wa !== wb) {
      return wa - wb;
    }
    return activityAt(b) - activityAt(a);
  });
}

/** 近 N 毫秒内终态 run 的成功/失败计数（概览时间序列摘要用）。 */
export function summarizeWindow(
  runs: readonly UnifiedRun[],
  sinceMs: number,
  now: number = Date.now(),
): { succeeded: number; failed: number; canceled: number } {
  const out = { succeeded: 0, failed: 0, canceled: 0 };
  for (const r of runs) {
    if (r.finishedAt === undefined || r.finishedAt < sinceMs || r.finishedAt > now) {
      continue;
    }
    if (r.state === "succeeded") {
      out.succeeded += 1;
    } else if (r.state === "failed") {
      out.failed += 1;
    } else if (r.state === "canceled") {
      out.canceled += 1;
    }
  }
  return out;
}

/** next_run 停滞：档期已过（或为 0 = 待补发）且距上次运行超过该时长。 */
const NEXT_RUN_STALE_AFTER_MS = 24 * 3600 * 1000;

/**
 * P1-10：`next_run_at` 停滞判据（计划任务面板）。
 *
 * 调度器是「到期即派发 + 错过补发一次」模型：正常任务跑完后 `next_run_at` 会被
 * `record_run` 推进到下一档期。若档期已过 —— 含后端的 `next_run_at = 0`
 * （"待补发"）经 RFC3339 序列化成的 1970-01-01 形态 —— 且距上次运行超过 24 小时，
 * 说明派发一直没落地（长任务被关机打断、`record_run` 从未执行；pool-scan
 * 十月中旬的实际状态正是如此），面板须显式标出而不是让 1970 日期静默躺着。
 * 排除：非活跃任务、从未运行过的任务（新建待首跑，交给别的信号）。
 */
export function isNextRunStale(task: ScheduledTask, now: number = Date.now()): boolean {
  if (task.status !== "active" || !task.lastRunAt) {
    return false;
  }
  const lastMs = Date.parse(task.lastRunAt);
  if (!Number.isFinite(lastMs)) {
    return false;
  }
  const nextMs = task.nextRunAt ? Date.parse(task.nextRunAt) : Number.NaN;
  const overdue = !Number.isFinite(nextMs) || nextMs < now;
  return overdue && now - lastMs > NEXT_RUN_STALE_AFTER_MS;
}
