// SPDX-License-Identifier: AGPL-3.0-only

import type { PluginTaskSourcesInfoDto } from "@/types";
import { useSyncExternalStore } from "react";
import type { AggregatedRuns, RunQuery, SourceFetchError, SourceId, UnifiedRun } from "../model";
import { filterRuns, sortRuns } from "../model";
import { backgroundTasksAdapter } from "./backgroundTasks.adapter";
import { cronAdapter } from "./cron.adapter";
import { moverRecallAnalysisAdapter } from "./moverRecallAnalysis.adapter";
import { pipelineAdapter } from "./pipeline.adapter";
import { createPluginTaskSourceAdapter } from "./pluginTaskSource.adapter";
import { timeTravelAdapter } from "./timeTravel.adapter";
import type { TaskSourceAdapter } from "./types";

/**
 * 适配器注册表：新增任务源 = 在此数组追加一行，中心代码零改动。
 * 列表顺序即概览/Runs 的源默认顺序。
 *
 * 插件贡献的源走同一条 `REGISTRY`（`registerPluginTaskSources`，任务 #148），
 * 不另建第二份清单 ⇒ 「概览/Runs/筛选器」这些按 `getAllAdapters()` 渲染的地方
 * 不需要为插件源加分支。
 */
const REGISTRY: TaskSourceAdapter[] = [
  cronAdapter,
  pipelineAdapter,
  timeTravelAdapter,
  moverRecallAnalysisAdapter,
  backgroundTasksAdapter,
];

const BY_ID = new Map<SourceId, TaskSourceAdapter>(
  REGISTRY.map((adapter) => [adapter.id, adapter]),
);

/**
 * 注册表版本号（外置 store 的快照值）：每次成功追加源就 +1。
 *
 * 为什么需要它：`REGISTRY` 是模块级可变数组，React 看不见它的变化。插件源在页面挂载后
 * 才异步注册进来，没有这个版本号，筛选器/源名 chip 会一直显示旧快照（表现为「源装上了
 * 但列表里没有」）。消费侧用 `useTaskSourceRegistry()` 订阅。
 */
let registryVersion = 0;
const registryListeners = new Set<() => void>();

function bumpRegistryVersion(): void {
  registryVersion += 1;
  for (const listener of registryListeners) {
    listener();
  }
}

function subscribeRegistry(listener: () => void): () => void {
  registryListeners.add(listener);
  return () => {
    registryListeners.delete(listener);
  };
}

/** 订阅「注册表是否多了一个源」；返回值只用来触发重渲染，不要拿它做业务判断。 */
export function useTaskSourceRegistry(): number {
  return useSyncExternalStore(subscribeRegistry, () => registryVersion);
}

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

/**
 * 把 `plugin_list_task_sources` 的投影注册成任务源（任务 #148）。
 *
 * 三条口径：
 * 1. **只挂 `enabled` 的插件**：禁用插件的源挂上去只会多一个永远为空的标签。
 * 2. **重名宿主优先**：`registerAdapter` 本身幂等（已在 `BY_ID` 里就不追加），
 *    宿主源在模块加载期先占位 ⇒ 插件抢同名必然失败，与 B10-1「宿主命令优先」同判据。
 * 3. **id 原样用作 `SourceId`**：宿主源是字面量联合、插件源是运行期字符串，
 *    `model.ts` 的 `SourceId` 已并入 `(string & {})` 承接后者。
 *
 * @returns 实际新增的源数量（0 表示没有可挂的，或全部与宿主重名）。
 */
export function registerPluginTaskSources(infos: readonly PluginTaskSourcesInfoDto[]): number {
  let added = 0;
  for (const info of infos) {
    if (!info.enabled) {
      continue;
    }
    for (const source of info.taskSources) {
      if (BY_ID.has(source.id)) {
        console.warn(`[tasksCenter] 插件 ${info.id} 的任务源 id「${source.id}」与已注册源重名，按宿主优先忽略`);
        continue;
      }
      const adapter = createPluginTaskSourceAdapter(info.id, source);
      REGISTRY.push(adapter);
      BY_ID.set(source.id, adapter);
      added += 1;
    }
  }
  if (added > 0) {
    bumpRegistryVersion();
  }
  return added;
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
