// SPDX-License-Identifier: AGPL-3.0-only

import type { AggregatedRuns, RunQuery, SourceFetchError, SourceId, UnifiedRun } from "../model";
import { filterRuns, sortRuns } from "../model";
import { backgroundTasksAdapter } from "./backgroundTasks.adapter";
import { cronAdapter } from "./cron.adapter";
import { moverRecallAnalysisAdapter } from "./moverRecallAnalysis.adapter";
import { pipelineAdapter } from "./pipeline.adapter";
import { stockAnalysisAdapter } from "./stockAnalysis.adapter";
import { timeTravelAdapter } from "./timeTravel.adapter";
import type { TaskSourceAdapter } from "./types";

/**
 * 适配器注册表：新增任务源 = 在此数组追加一行，中心代码零改动。
 * 列表顺序即概览/Runs 的源默认顺序。
 */
const REGISTRY: TaskSourceAdapter[] = [
  cronAdapter,
  pipelineAdapter,
  stockAnalysisAdapter,
  timeTravelAdapter,
  moverRecallAnalysisAdapter,
  backgroundTasksAdapter,
];

const BY_ID = new Map<SourceId, TaskSourceAdapter>(
  REGISTRY.map((adapter) => [adapter.id, adapter]),
);

export function getAllAdapters(): readonly TaskSourceAdapter[] {
  return REGISTRY;
}

export function getAdapter(id: SourceId): TaskSourceAdapter | undefined {
  return BY_ID.get(id);
}

export function registerAdapter(adapter: TaskSourceAdapter): void {
  if (!BY_ID.has(adapter.id)) {
    REGISTRY.push(adapter);
    BY_ID.set(adapter.id, adapter);
  }
}

function toErrorMessage(e: unknown): string {
  if (e instanceof Error) {
    return e.message;
  }
  return String(e);
}

/**
 * 并行聚合各源 run（Promise.allSettled 隔离）：
 * 单源失败只产生一条 SourceFetchError，不拖垮整页；
 * 源内过滤（源只支持自己的 list 语义），跨源过滤在合并后统一做。
 */
export async function aggregateRuns(query: RunQuery = {}): Promise<AggregatedRuns> {
  const adapters = query.sources && query.sources.length > 0
    ? REGISTRY.filter((a) => query.sources!.includes(a.id))
    : REGISTRY;

  const settled = await Promise.allSettled(
    adapters.map((adapter) => adapter.list(query)),
  );

  const runs: UnifiedRun[] = [];
  const errors: SourceFetchError[] = [];
  settled.forEach((result, idx) => {
    const adapter = adapters[idx];
    if (!adapter) {
      return;
    }
    if (result.status === "fulfilled") {
      runs.push(...result.value);
    } else {
      errors.push({ source: adapter.id, message: toErrorMessage(result.reason) });
    }
  });

  return {
    runs: sortRuns(filterRuns(runs, query)),
    errors,
  };
}
